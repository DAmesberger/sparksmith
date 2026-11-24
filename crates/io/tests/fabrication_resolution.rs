//! A fab profile other than the board's own resolves to a production
//! snapshot only when the design can take it, under the shipped profiles
//! and the shipped new board:
//!
//! - a valid alternate keeps the board's foils, chooses the profile's copper
//!   weight that orders them, and every file states those foils and that
//!   weight's rules;
//! - a profile that offers no weight for the board's foils (3 oz copper
//!   ordered from a fabricator offering 1 and 2 oz) is refused;
//! - a profile a kept override conflicts with is refused, and no waiver
//!   turns the refusal into an export under the saved profile.
//!
//! A refusal writes nothing: the output folder is neither created nor
//! filled.
use sparksmith_core::fabrication::{
    self, FabricationCommand, FabricationOptions, ProductionSnapshot, preflight,
};
use sparksmith_core::rules::{RuleEntry, RuleKey, RuleProfile, RuleValue, RulesCommand};
use sparksmith_core::{Engine, Operation};
use sparksmith_io::fabrication::{ExportRequest, export};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A shipped profile, read from this checkout's library.
fn shipped(id: &str) -> Fallible<RuleProfile> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../library/profiles")
        .join(format!("{id}.json"));
    Ok(sparksmith_io::rule_profiles::read_profile_file(&path)?)
}

/// `profile` with its copper weight chosen.
fn weighed(profile: &RuleProfile, weight: &str) -> Fallible<RuleProfile> {
    Ok(profile.with_choices(&BTreeMap::from([(
        "copper_weight".to_owned(),
        weight.to_owned(),
    )]))?)
}

/// The shipped new board under the shipped profile `id`, its copper weight
/// chosen through the rules edit a person makes (the foils follow it).
fn board(id: &str, weight: &str) -> Fallible<Engine> {
    let mut engine =
        sparksmith_io::new_project::engine(&sparksmith_io::standards::shipped_context()?)?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(shipped(id)?, false)),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOptions {
            options: BTreeMap::from([("copper_weight".into(), weight.into())]),
        }),
    )?;
    Ok(engine)
}

fn request(output: PathBuf, profile: Option<RuleProfile>) -> ExportRequest {
    ExportRequest {
        output,
        zip: false,
        profile,
        options: FabricationOptions {
            base_name: Some("b".into()),
            ..Default::default()
        },
    }
}

/// Export `engine`'s design under `profile` into `output`.
fn run(
    engine: &Engine,
    output: &Path,
    profile: Option<RuleProfile>,
) -> sparksmith_io::Result<sparksmith_io::fabrication::ExportReport> {
    export(
        engine.project(),
        engine.drafts(),
        &request(output.to_owned(), profile),
        engine.check_settings(),
        &mut |_| std::ops::ControlFlow::Continue(()),
    )
}

/// The job file's outer foils (mm) and minimum line width (mm).
fn job(dir: &Path) -> Fallible<(Vec<f64>, f64)> {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("b.gbrjob"))?)?;
    let foils = json["MaterialStackup"]
        .as_array()
        .ok_or("no MaterialStackup")?
        .iter()
        .filter(|m| m["Type"] == "Copper")
        .filter_map(|m| m["Thickness"].as_f64())
        .collect();
    let width = json["DesignRules"][0]["MinLineWidth"]
        .as_f64()
        .ok_or("no MinLineWidth")?;
    Ok((foils, width))
}

/// What a refusal must say and leave: `fabrication.profile_refused` from
/// the snapshot, the check and the export alike, naming `reason`, and
/// nothing written: neither a new folder nor a file in an empty one.
fn assert_refused(engine: &Engine, profile: &RuleProfile, reason: &str) -> Fallible<()> {
    let project = engine.project();
    let checks = engine.check_settings();
    let refusal = |error: &sparksmith_core::Error| {
        assert_eq!(error.rule(), Some("fabrication.profile_refused"), "{error}");
        assert!(error.to_string().contains(reason), "{error}");
        error.to_string()
    };
    let snapshot = ProductionSnapshot::resolve(project, Some(profile))
        .err()
        .ok_or("the snapshot is refused")?;
    let check = preflight::report(project, engine.drafts(), Some(profile), checks)
        .err()
        .ok_or("the check is refused")?;
    assert_eq!(refusal(&check), refusal(&snapshot));
    let exported = |output: &Path| -> Fallible<()> {
        match run(engine, output, Some(profile.clone())) {
            Err(sparksmith_io::Error::Document(error)) => {
                refusal(&error);
                Ok(())
            }
            other => Err(format!("the export is refused: {other:?}").into()),
        }
    };
    let dir = tempfile::tempdir()?;
    let fresh = dir.path().join("fresh");
    exported(&fresh)?;
    assert!(!fresh.exists(), "a refused export creates no folder");
    let empty = dir.path().join("empty");
    std::fs::create_dir(&empty)?;
    exported(&empty)?;
    assert_eq!(std::fs::read_dir(&empty)?.count(), 0, "nor fills one");
    // Nothing beside the output either (no staging folder left behind).
    let left: Vec<_> = std::fs::read_dir(dir.path())?
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(left, ["empty"]);
    Ok(())
}

/// 2 oz outer copper ordered under one shipped profile, exported under the
/// other: both offer 2 oz, so the snapshot keeps the 70 µm foils and takes
/// the other profile's 2 oz rules, and the job file and drawing say so.
#[test]
fn a_valid_alternate_keeps_the_foils_and_takes_the_matching_weight() -> Fallible<()> {
    for (own, other) in [
        ("pcbway-2-layer", "jlcpcb-2-layer"),
        ("jlcpcb-2-layer", "pcbway-2-layer"),
    ] {
        let engine = board(own, "2oz")?;
        let project = engine.project();
        assert!(project.stackup.outer_copper_is(70.0), "{own}");
        let alternate = shipped(other)?;
        // The other profile's own choice is 1 oz: taken as it is, it would
        // order other foils than the board has.
        assert_eq!(alternate.outer_copper_um(), Some(35.0));
        let expected = weighed(&alternate, "2oz")?;

        let snapshot = ProductionSnapshot::resolve(project, Some(&alternate))?;
        let resolved = snapshot.project();
        assert_eq!(resolved.stackup, project.stackup, "{own} → {other}");
        let profile = resolved.rules.profile.as_ref().ok_or("a profile")?;
        assert_eq!(profile.id, other);
        assert_eq!(profile.outer_copper_um(), Some(70.0));
        assert_eq!(resolved.rules.effective(), *expected.rules());

        let dir = tempfile::tempdir()?;
        let out = dir.path().join("out");
        let report = run(&engine, &out, Some(alternate.clone()))?;
        assert!(report.preflight.profile.starts_with(&expected.name));
        assert_eq!(report.preflight.blocking, 0);
        let (foils, width) = job(&out)?;
        assert_eq!(foils, [0.07, 0.07], "{own} → {other}");
        assert_eq!(width, expected.rules().copper.track_width_min);
        // Against the file: 2 oz has its own minimum, wider than 1 oz's.
        assert!(width > weighed(&alternate, "1oz")?.rules().copper.track_width_min);

        let set = fabrication::generate(
            &snapshot,
            &FabricationOptions::default(),
            &Default::default(),
        )?;
        let drawing =
            fabrication::drawing::data(&snapshot, &set.identity, engine.check_settings())?;
        assert!(
            drawing.profile.starts_with(&expected.name),
            "{}",
            drawing.profile
        );
        let copper: Vec<f64> = drawing
            .stackup
            .iter()
            .filter(|r| r.kind == "copper")
            .map(|r| r.thickness_mm)
            .collect();
        assert_eq!(copper, [0.07, 0.07]);
    }
    Ok(())
}

/// 3 oz outer copper (105 µm) under PCBWay, exported under JLCPCB, which
/// offers 1 and 2 oz: refused, never checked under JLCPCB's 1 oz rules
/// with 105 µm foils in the job file.
#[test]
fn a_profile_without_the_boards_copper_weight_is_refused() -> Fallible<()> {
    let engine = board("pcbway-2-layer", "3oz")?;
    assert!(engine.project().stackup.outer_copper_is(105.0));
    assert_refused(
        &engine,
        &shipped("jlcpcb-2-layer")?,
        "outer copper is 105 µm",
    )?;
    // The board's own profile still exports, with its own foils.
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("own");
    run(&engine, &out, None)?;
    assert_eq!(job(&out)?.0, [0.105, 0.105]);
    Ok(())
}

/// A 0.22 mm default track width kept as an override conflicts with a
/// profile whose minimum is 0.50 mm: refused, and there is no finding to
/// waive. A waiver saved under the old refusal's key accepts nothing, so
/// the export is still refused instead of running under the saved profile.
#[test]
fn a_conflicting_kept_override_is_refused_and_cannot_be_waived() -> Fallible<()> {
    let mut engine = board("jlcpcb-2-layer", "1oz")?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOverride {
            key: RuleKey::TrackWidthDefault,
            value: RuleValue::Length(0.22),
        }),
    )?;
    let mut strict = shipped("jlcpcb-2-layer")?;
    strict.id = "test-strict".into();
    strict.name = "Test strict 0.50 mm".into();
    for (key, mm) in [
        (RuleKey::TrackWidthDefault, 0.6),
        (RuleKey::TrackWidthMin, 0.5),
    ] {
        strict.set(
            key,
            RuleEntry::value(RuleValue::Length(mm), "Test fixture."),
        )?;
    }
    assert_refused(&engine, &strict, "default track width")?;
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec!["fabrication.profile_refused []".into()],
            reason: "check release guard".into(),
            profile: None,
        }),
    )?;
    assert_refused(&engine, &strict, "default track width")?;
    Ok(())
}

/// The two demo resistors have two open pads on their common net. Piling
/// copper at the opposite end of the board cannot route either one.
#[test]
fn an_incomplete_graph_preserves_open_pads_and_blocks_release() -> Fallible<()> {
    use sparksmith_core::library::standards::Context;
    use sparksmith_core::model::production::Waiver;
    use sparksmith_core::{CopperLayer, InstanceCommand, Track};

    let _scope = Context::fixture()?.enter();
    let mut engine = Engine::demo()?;
    let unused: Vec<_> = engine
        .project()
        .nets
        .iter()
        .filter_map(|n| match n.terminals.as_slice() {
            [only] => Some(*only),
            _ => None,
        })
        .collect();
    for pin in unused {
        engine.apply(
            engine.revision(),
            Operation::Instance(InstanceCommand::SetNoConnect {
                instance: pin.instance,
                terminals: vec![pin.terminal],
                no_connect: true,
            }),
        )?;
    }
    let check = |engine: &Engine| {
        preflight::report(
            engine.project(),
            engine.drafts(),
            None,
            engine.check_settings(),
        )
    };
    let open_pads = |report: &fabrication::PreflightReport| {
        report
            .items
            .iter()
            .filter(|i| i.code == "board.unrouted" && i.blocks())
            .map(|i| i.key.clone())
            .collect::<Vec<_>>()
    };
    let control = check(&engine)?;
    assert_eq!(control.blocking, 2);
    let known_opens = open_pads(&control);
    assert_eq!(known_opens.len(), 2);
    let mut file = engine.to_file();
    let p = &mut file.project;
    let net = p
        .nets
        .iter()
        .find(|n| n.terminals.len() == 2)
        .ok_or("shared net")?
        .id;
    for _ in 0..90 {
        let id = p.allocate()?;
        p.tracks.push(Track {
            id,
            net,
            start: [25.0, 15.0],
            end: [30.0, 15.0],
            width: 0.3,
            locked: false,
            layer: CopperLayer::Top,
        });
    }
    let mut engine = Engine::from_file(file)?;
    assert!(!engine.copper_graph().complete());
    let dense = check(&engine)?;
    assert_eq!(open_pads(&dense), known_opens);
    let limit = dense
        .items
        .iter()
        .find(|i| i.code == "board.check_limit")
        .ok_or("limit")?;
    assert!(limit.blocks());
    assert!(!limit.offers_waiver());
    let revision = engine.revision();
    let error = engine
        .apply(
            revision,
            Operation::Fabrication(FabricationCommand::Waive {
                keys: vec![limit.key.clone()],
                reason: "Reviewed the dense region".into(),
                profile: None,
            }),
        )
        .err()
        .ok_or("the waiver was accepted")?;
    assert_eq!(error.rule(), Some("fabrication.not_waivable"));
    assert_eq!(engine.revision(), revision);
    assert!(engine.project().production.waivers.is_empty());

    let mut file = engine.to_file();
    file.project.production.waivers.push(Waiver {
        key: limit.key.clone(),
        reason: "Old waiver".into(),
        accepted: Vec::new(),
        footprints: Vec::new(),
        given_for: None,
    });
    let engine = Engine::from_file(file)?;
    let after = check(&engine)?;
    assert!(!after.ready());
    assert_eq!(after.waived, 0);
    assert_eq!(after.blocking, dense.blocking);
    assert_eq!(open_pads(&after), known_opens);
    let dir = tempfile::tempdir()?;
    let output = dir.path().join("release");
    match run(&engine, &output, None) {
        Err(sparksmith_io::Error::Document(error)) => {
            assert_eq!(error.rule(), Some("fabrication.blocked"))
        }
        result => return Err(format!("expected a blocked release, got {result:?}").into()),
    }
    assert!(!output.exists());
    Ok(())
}
