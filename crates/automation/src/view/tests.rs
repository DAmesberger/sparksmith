//! The `view` entries through the front door: headless they answer
//! `host.unavailable`; in a window they run there, change no revision and
//! take no undo step, and wait while the person is busy.
use super::*;
use crate::front::{Call, CallCx, NoOne, Surface};
use crate::gate::{Grant, Mode, Principal};
use crate::session::Session;
use serde_json::{Value, json};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn call(entry: &str, params: Value) -> Call {
    Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision: None,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    }
}

fn run(session: &mut Session, window: Option<&mut Fake>, call: Call) -> Result<Value, Box<Error>> {
    let grant = Grant::new(Mode::AskEveryTime, []);
    session.call(
        CallCx {
            who: Principal::RemoteAgent {
                grant: &grant,
                client: "test",
            },
            asker: &mut NoOne,
            verbose: false,
            window: window.map(|w| w as &mut dyn WindowHost),
        },
        call,
    )
}

/// The first placed part's id and designator.
fn first_part(session: &Session) -> Fallible<(Id, String)> {
    let project = session.engine().project();
    let placement = project.placements.first().ok_or("no placed part")?;
    let instance = project
        .instance(placement.instance)
        .ok_or("placement without its part")?;
    Ok((instance.id, instance.designator.clone()))
}

/// No window: every view entry is refused with `host.unavailable`, and
/// nothing changes.
#[test]
fn headless_there_is_no_window() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let (id, _) = first_part(&session)?;
    let revision = session.engine().revision();
    for (entry, params) in [
        ("view.context", json!({})),
        ("view.select", json!({"ids": [id]})),
        ("view.open", json!({"workspace": "pcb", "view": "3d"})),
    ] {
        let Err(error) = run(&mut session, None, call(entry, params)) else {
            return Err(format!("{entry} ran without a window").into());
        };
        assert_eq!(error.code, "host.unavailable", "{entry}: {error:?}");
        assert!(error.instead.is_some(), "{entry}");
    }
    assert_eq!(session.engine().revision(), revision);
    Ok(())
}

/// In a window, under ask-every-time: select a part by id, open 3D and a
/// sheet, read what the window shows. No revision, no undo step, nobody
/// asked.
#[test]
fn the_window_selects_and_opens_without_a_revision() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let (id, designator) = first_part(&session)?;
    let revision = session.engine().revision();
    let step = session.engine().undo_step();
    let mut window = Fake::idle();

    let selected = run(
        &mut session,
        Some(&mut window),
        call("view.select", json!({"ids": [id]})),
    )
    .map_err(|e| format!("{e:?}"))?;
    assert_eq!(selected, json!({"selected": 1, "workspace": "pcb"}));
    assert_eq!(window.selection, vec![ObjectRef::Instance { id }]);

    let opened = run(
        &mut session,
        Some(&mut window),
        call("view.open", json!({"workspace": "pcb", "view": "3d"})),
    )
    .map_err(|e| format!("{e:?}"))?;
    assert_eq!(opened["view"], "3d", "{opened}");
    assert_eq!(opened["revision"], revision);

    let sheet = session.engine().project().sheets.first();
    let opened = run(
        &mut session,
        Some(&mut window),
        call(
            "view.open",
            json!({"workspace": "schematic", "sheet": sheet}),
        ),
    )
    .map_err(|e| format!("{e:?}"))?;
    assert_eq!(opened["workspace"], "schematic", "{opened}");
    assert_eq!(opened["sheet"]["id"], json!(sheet));

    let context = run(
        &mut session,
        Some(&mut window),
        call("view.context", json!({})),
    )
    .map_err(|e| format!("{e:?}"))?;
    assert_eq!(
        context["selection"],
        json!([{"kind": "instance", "id": id, "name": designator}]),
        "{context}"
    );
    assert_eq!(session.engine().revision(), revision);
    assert_eq!(session.engine().undo_step(), step);
    Ok(())
}

/// While the person holds the window, steering it is refused with the
/// reason; reading what it shows still answers.
#[test]
fn a_busy_window_refuses_steering_but_answers_reads() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let (id, _) = first_part(&session)?;
    let mut window = Fake::held(Some("The person is dragging in the window"));
    let Err(error) = run(
        &mut session,
        Some(&mut window),
        call("view.select", json!({"ids": [id]})),
    ) else {
        return Err("selected while the person dragged".into());
    };
    assert_eq!(error.code, "live.busy", "{error:?}");
    assert!(error.message.contains("dragging"), "{error:?}");
    assert!(window.selection.is_empty());
    let context = run(
        &mut session,
        Some(&mut window),
        call("view.context", json!({})),
    );
    assert!(context.is_ok(), "{context:?}");
    Ok(())
}

/// What names nothing, or the wrong workspace's field, is refused and the
/// window is left as it was.
#[test]
fn unknown_objects_and_misplaced_fields_change_nothing() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let mut window = Fake::idle();
    let missing = session.engine().project().next_id;
    for (entry, params, code) in [
        (
            "view.select",
            json!({"ids": [missing]}),
            "view.unknown_object",
        ),
        ("view.select", json!({}), "malformed_request"),
        (
            "view.select",
            json!({"objects": [{"kind": "net", "id": 999_999}]}),
            "view.unknown_object",
        ),
        (
            "view.open",
            json!({"workspace": "schematic", "view": "3d"}),
            "malformed_request",
        ),
        (
            "view.open",
            json!({"workspace": "schematic", "sheet": 999}),
            "view.unknown_object",
        ),
        (
            "view.open",
            json!({"workspace": "component", "definition": missing}),
            "view.not_a_draft",
        ),
    ] {
        let Err(error) = run(&mut session, Some(&mut window), call(entry, params.clone())) else {
            return Err(format!("{entry} {params} ran").into());
        };
        assert_eq!(error.code, code, "{entry} {params}: {error:?}");
    }
    assert!(window.selection.is_empty());
    assert_eq!(window.workspace, Workspace::Pcb);
    Ok(())
}

/// `assist.offer` shows the workflow card in the window's chat with the
/// inputs the model has, naming those the run will still ask for; it
/// changes no revision. Headless, and for a workflow or input the host does
/// not have, nothing is shown.
#[test]
fn an_offer_shows_the_workflow_card_in_the_window() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let revision = session.engine().revision();
    let offer = json!({"workflow": "datasheet-to-component",
        "inputs": {"part": "TLV75733PDBVR"}, "why": "You attached a datasheet."});

    let Err(error) = run(&mut session, None, call("assist.offer", offer.clone())) else {
        return Err("assist.offer ran without a window".into());
    };
    assert_eq!(error.code, "host.unavailable", "{error:?}");

    let mut window = Fake::idle();
    let Err(error) = run(
        &mut session,
        Some(&mut window),
        call("assist.offer", offer.clone()),
    ) else {
        return Err("assist.offer ran without workflows".into());
    };
    assert_eq!(error.code, "assist.no_workflows", "{error:?}");

    let shipped =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library/assist/workflows");
    session.lend_workflows(Some(crate::workflow::load::list(
        Some(&shipped),
        None,
        &crate::workflow::HostCatalog,
    )));
    let shown = run(&mut session, Some(&mut window), call("assist.offer", offer))
        .map_err(|e| format!("{e:?}"))?;
    assert_eq!(shown, json!({"card": "card-1", "missing": ["datasheets"]}));
    let [card] = window.offers.as_slice() else {
        return Err(format!("{:?}", window.offers).into());
    };
    assert_eq!(card.title, "Datasheet → component");
    assert_eq!(card.why, "You attached a datasheet.");
    assert_eq!(card.inputs["part"], "TLV75733PDBVR");

    for (params, says) in [
        (json!({"workflow": "make-coffee", "why": ""}), "no workflow"),
        (
            json!({"workflow": "datasheet-to-component", "inputs": {"colour": "red"}, "why": ""}),
            "not an input",
        ),
        (
            json!({"workflow": "datasheet-to-component", "inputs": {"part": 7}, "why": ""}),
            "inputs/part",
        ),
    ] {
        let Err(error) = run(
            &mut session,
            Some(&mut window),
            call("assist.offer", params.clone()),
        ) else {
            return Err(format!("{params} was shown").into());
        };
        assert_eq!(error.code, "assist.offer", "{params}: {error:?}");
        assert!(error.message.contains(says), "{params}: {error:?}");
    }
    assert_eq!(window.offers.len(), 1);
    assert_eq!(session.engine().revision(), revision);
    Ok(())
}
