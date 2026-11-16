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

/// Place `component` with only its id, as the person: the reply's
/// outcome.
fn place(session: &mut Session, component: Value) -> Fallible<Value> {
    let revision = session.engine().revision();
    let placed = session
        .call(
            CallCx {
                who: Principal::Person,
                asker: &mut NoOne,
                verbose: false,
                window: None,
            },
            Call {
                expected_revision: Some(revision),
                ..call("component.place", json!({"component": component}))
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    Ok(placed["outcome"].clone())
}

/// What `component.place` returned selects what it placed: the occurrence
/// id names its part, and so does the designator, in `ids` or in
/// `designators`, in any case.
#[test]
fn what_was_just_placed_selects_by_its_occurrence_or_designator() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let component = serde_json::to_value(
        session
            .engine()
            .project()
            .instances
            .first()
            .ok_or("no part")?
            .component,
    )?;
    let outcome = place(&mut session, component)?;
    let instance = outcome["instance"].as_u64().ok_or(format!("{outcome}"))?;
    let occurrence = outcome["occurrence"].as_u64().ok_or(format!("{outcome}"))?;
    let designator = outcome["designator"]
        .as_str()
        .ok_or(format!("{outcome}"))?
        .to_owned();
    assert_ne!(instance, occurrence);
    let mut window = Fake::idle();
    for params in [
        json!({"ids": [occurrence]}),
        json!({"ids": [instance]}),
        json!({"ids": [designator]}),
        json!({"ids": [designator.to_lowercase()]}),
        json!({"designators": [designator.to_lowercase()]}),
        json!({"ids": [occurrence.to_string()]}),
    ] {
        window.selection.clear();
        let selected = run(
            &mut session,
            Some(&mut window),
            call("view.select", params.clone()),
        )
        .map_err(|e| format!("{params}: {e:?}"))?;
        assert_eq!(selected["selected"], 1, "{params}: {selected}");
        assert_eq!(
            window.selection,
            vec![ObjectRef::Instance { id: instance }],
            "{params}"
        );
    }
    Ok(())
}

/// A designator no part has is refused, naming it and the designators the
/// project has; nothing is selected.
#[test]
fn an_unknown_designator_is_refused_with_the_known_ones() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let (_, designator) = first_part(&session)?;
    let mut window = Fake::idle();
    for params in [json!({"ids": ["Q99"]}), json!({"designators": ["Q99"]})] {
        let Err(error) = run(
            &mut session,
            Some(&mut window),
            call("view.select", params.clone()),
        ) else {
            return Err(format!("{params} selected").into());
        };
        assert_eq!(error.code, "view.unknown_object", "{error:?}");
        assert!(error.message.contains("\"Q99\""), "{error:?}");
        let instead = error.instead.clone().unwrap_or_default();
        assert!(instead.contains(&designator), "{error:?}");
    }
    assert!(window.selection.is_empty());
    Ok(())
}

/// `component.place` with `params`, as `who`.
fn place_as(session: &mut Session, who: Principal<'_>, params: Value) -> Result<Value, Box<Error>> {
    let revision = session.engine().revision();
    session.call(
        CallCx {
            who,
            asker: &mut NoOne,
            verbose: false,
            window: None,
        },
        Call {
            expected_revision: Some(revision),
            ..call("component.place", params)
        },
    )
}

/// An agent's symbol goes on the page: a position outside the sheet's
/// frame is refused, naming the frame, and nothing changes; a position
/// left out lands inside it. A person may still draw beside the page.
#[test]
fn an_agent_places_symbols_inside_the_sheet_frame() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let component = serde_json::to_value(
        session
            .engine()
            .project()
            .instances
            .first()
            .ok_or("no part")?
            .component,
    )?;
    let grant = Grant::new(Mode::AllowAll, []);
    let agent = || Principal::RemoteAgent {
        grant: &grant,
        client: "test",
    };
    let revision = session.engine().revision();
    let outside = json!({"component": component, "position": [100.0, 80.0]});
    let Err(error) = place_as(&mut session, agent(), outside.clone()) else {
        return Err("placed outside the frame".into());
    };
    assert!(
        error.message.contains(
            "(100, 80) mm is outside Sheet 1 (A4 landscape): its frame spans x -30\u{2026}267 mm and y -180\u{2026}30 mm, y up"
        ),
        "{error:?}"
    );
    assert_eq!(session.engine().revision(), revision);
    let placed = place_as(&mut session, agent(), json!({"component": component}))
        .map_err(|e| format!("{e:?}"))?;
    let occurrence = placed["outcome"]["occurrence"]
        .as_u64()
        .ok_or(format!("{placed}"))?;
    let project = session.engine().project();
    let at = project
        .occurrence(occurrence)
        .ok_or("no occurrence")?
        .position;
    let first = project.sheets.first();
    assert!(sparksmith_core::schematic::spot::check_on_page(project, first, at).is_ok());
    // The person may place beside the page.
    place_as(&mut session, Principal::Person, outside).map_err(|e| format!("{e:?}"))?;
    Ok(())
}
