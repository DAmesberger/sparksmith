//! A batch is its operations sent one after another: the same document
//! bytes, as one undo step, whatever the host looks up for an operation
//! (a fab profile by its id, the page a part is placed on) read from the
//! document the operations before it made. A batch that fails changes
//! nothing.
use super::*;
use crate::front::{Call, CallCx, NoOne, Surface};
use crate::gate::{Grant, Mode, Principal};
use sparksmith_core::InstanceCommand;
use sparksmith_core::instances::NewInstance;
use sparksmith_core::instances::variants::VariantCommand;
use sparksmith_core::model::Population;
use sparksmith_core::model::sheet::{Paper, PaperOrientation};
use sparksmith_core::rules::{NetClassSpec, RuleProfile, RulesCommand, fixtures};
use sparksmith_core::schematic::SchematicCommand;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
/// An operation made from the document it is sent to.
type Make = fn(&Project) -> Fallible<Operation>;

/// Send `params` to `entry` at the current revision as an agent, whose
/// parts are placed on the page.
fn send(
    session: &mut Session,
    entry: &str,
    params: Value,
    dry_run: bool,
) -> std::result::Result<Value, Box<crate::front::Error>> {
    let grant = Grant::new(Mode::AllowAll, []);
    let expected_revision = Some(session.engine().revision());
    session.call(
        CallCx {
            who: Principal::LocalAgent { grant: &grant },
            asker: &mut NoOne,
            verbose: false,
            window: None,
        },
        Call {
            entry: entry.into(),
            params,
            project: None,
            expected_revision,
            dry_run,
            confirm: None,
            turn: None,
            job: false,
        },
    )
}

fn single(session: &mut Session, operation: &Operation) -> Fallible {
    send(
        session,
        "project.apply",
        json!({"operation": operation}),
        false,
    )
    .map(|_| ())
    .map_err(|e| format!("{}: {}", operation.command(), e.message).into())
}

fn batch(
    session: &mut Session,
    operations: &[Operation],
    dry_run: bool,
) -> std::result::Result<Value, Box<crate::front::Error>> {
    send(
        session,
        "project.apply_all",
        json!({"operations": operations}),
        dry_run,
    )
}

fn bytes(session: &Session) -> Fallible<Vec<u8>> {
    Ok(sparksmith_core::format::encode(
        &EditorFile::from_document(session.engine().document()),
    )?)
}

fn start() -> Fallible<Session> {
    Ok(Session::new(Engine::demo()?))
}

/// A profile no library holds.
fn custom() -> Fallible<RuleProfile> {
    let mut profile = fixtures::jlcpcb_example()?;
    profile.id = "unlisted-example".into();
    Ok(profile)
}

fn set_profile(profile: ProfileChoice, options: &[(&str, &str)]) -> Operation {
    Operation::Rules(RulesCommand::SetProfile {
        profile,
        options: options
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
        keep_overrides: false,
    })
}

fn place(project: &Project, sheet: Option<usize>, at: Point) -> Fallible<Operation> {
    let like = project.instances.iter().next().ok_or("no part")?;
    Ok(Operation::Instance(InstanceCommand::Place(NewInstance {
        component: like.component,
        designator: None,
        properties: Vec::new(),
        representation: None,
        unit: None,
        position: Some(sparksmith_core::units::PointNm::from_mm_f64_rounded(at)?),
        physical: like.physical,
        sheet: match sheet {
            Some(n) => Some(project.sheets.list.get(n).ok_or("no sheet")?.id),
            None => None,
        },
    })))
}

/// Right of an A4 page, on an A3 one.
const BEYOND_A4: Point = [320.0, -20.0];

const CASES: [(&str, Make, Make); 7] = [
    (
        "a profile by its definition, then by its id with an option",
        |_| {
            Ok(set_profile(
                ProfileChoice::Profile(Box::new(custom()?)),
                &[],
            ))
        },
        |_| {
            Ok(set_profile(
                ProfileChoice::Id("unlisted-example".into()),
                &[("limits", "minimum")],
            ))
        },
    ),
    (
        "a profile, then one of its options",
        |_| {
            Ok(set_profile(
                ProfileChoice::Profile(Box::new(custom()?)),
                &[],
            ))
        },
        |_| {
            Ok(Operation::Rules(RulesCommand::SetOptions {
                options: [("limits".into(), "minimum".into())].into(),
            }))
        },
    ),
    (
        "a larger page, then a part on it",
        |project| {
            Ok(Operation::Schematic(SchematicCommand::SetPage {
                sheet: project.sheets.first(),
                paper: Paper::A3,
                orientation: PaperOrientation::Landscape,
            }))
        },
        |project| place(project, None, BEYOND_A4),
    ),
    (
        "a sheet, then a part on it",
        |_| {
            Ok(Operation::Schematic(SchematicCommand::AddSheet {
                name: None,
                paper: Some(Paper::A3),
                orientation: None,
            }))
        },
        |project| place(project, Some(1), BEYOND_A4),
    ),
    (
        "a part, then its designator",
        |project| place(project, None, [0.0, -60.0]),
        |project| {
            Ok(Operation::Instance(InstanceCommand::SetDesignator {
                instance: project.instances.last().ok_or("no part")?.id,
                designator: "R77".into(),
            }))
        },
    ),
    (
        "a variant, then a part it leaves off",
        |_| {
            Ok(Operation::Variant(VariantCommand::Add {
                name: "Lite".into(),
                copy_of: None,
            }))
        },
        |project| {
            Ok(Operation::Variant(VariantCommand::SetPopulation {
                instances: vec![project.instances.first().ok_or("no part")?.id],
                variant: Some(project.variants.list.first().ok_or("no variant")?.id),
                population: Some(Population::NotPopulated),
            }))
        },
    ),
    (
        "a net class, then a net in it",
        |_| {
            Ok(Operation::Rules(RulesCommand::AddNetClass {
                class: NetClassSpec {
                    name: "Wide".into(),
                    ..NetClassSpec::default()
                },
            }))
        },
        |project| {
            let class = project.rules.net_classes.last().ok_or("no class")?.id;
            Ok(Operation::Rules(RulesCommand::AssignNets {
                nets: vec![project.nets.first().ok_or("no net")?.id],
                class: Some(class),
            }))
        },
    ),
];

#[test]
fn two_edits_give_the_same_document_singly_and_in_a_batch() -> Fallible {
    for (name, first, second) in CASES {
        let mut singly = start()?;
        let before = bytes(&singly)?;
        let one = first(singly.engine().project())?;
        single(&mut singly, &one).map_err(|e| format!("{name}: {e}"))?;
        let two = second(singly.engine().project())?;
        single(&mut singly, &two).map_err(|e| format!("{name}: {e}"))?;
        let expected = bytes(&singly)?;
        assert_ne!(expected, before, "{name}");

        let mut batched = start()?;
        assert_eq!(bytes(&batched)?, before, "{name}");
        let steps = [one, two];
        let revision = batched.engine().revision();
        let tried = batch(&mut batched, &steps, true)
            .map_err(|e| format!("{name}, dry run: {}", e.message))?;
        assert_eq!(tried["dry_run"], true, "{name}");
        assert_eq!(
            bytes(&batched)?,
            before,
            "{name}: a dry run changes nothing"
        );
        assert_eq!(batched.engine().revision(), revision, "{name}");

        batch(&mut batched, &steps, false).map_err(|e| format!("{name}: {}", e.message))?;
        assert_eq!(bytes(&batched)?, expected, "{name}");
        assert_eq!(batched.engine().revision(), revision + 1, "{name}");
        send(&mut batched, "project.undo", json!({}), false)
            .map_err(|e| format!("{name}, undo: {}", e.message))?;
        assert_eq!(bytes(&batched)?, before, "{name}: one undo step");
    }
    Ok(())
}

/// The second operation is refused as it is when sent alone after the
/// first, naming its place, and the first is not kept.
#[test]
fn a_batch_whose_later_reference_fails_changes_nothing() -> Fallible {
    let unknown: Make = |_| {
        Ok(set_profile(
            ProfileChoice::Id("no-such-example".into()),
            &[],
        ))
    };
    let smaller: Make = |project| {
        Ok(Operation::Schematic(SchematicCommand::SetPage {
            sheet: project.sheets.first(),
            paper: Paper::A4,
            orientation: PaperOrientation::Portrait,
        }))
    };
    let cases: [(&str, Make, Make); 2] = [
        ("an unknown profile", CASES[0].1, unknown),
        // On the A4 landscape page the batch starts with, off the portrait
        // page the first operation leaves.
        (
            "a smaller page, then a part beside it",
            smaller,
            |project| place(project, None, [200.0, -20.0]),
        ),
    ];
    for (name, first, second) in cases {
        let mut session = start()?;
        let before = bytes(&session)?;
        let revision = session.engine().revision();
        let steps = [
            first(session.engine().project())?,
            second(session.engine().project())?,
        ];
        for dry_run in [true, false] {
            let Err(refused) = batch(&mut session, &steps, dry_run) else {
                return Err(format!("{name}: applied (dry run {dry_run})").into());
            };
            assert!(
                refused.message.ends_with("(operation 2 of 2)"),
                "{name}: {refused:?}"
            );
            assert_eq!(bytes(&session)?, before, "{name}");
            assert_eq!(session.engine().revision(), revision, "{name}");
        }
        single(&mut session, &steps[0])?;
        assert!(single(&mut session, &steps[1]).is_err(), "{name}");
    }
    Ok(())
}
