//! A board parameter may carry connected copper or refuse atomically, but
//! may not commit the interactive router's partially disconnected result.
use sparksmith_automation::session::{ApplyParams, Request, Session};
use sparksmith_core::layout;
use sparksmith_core::{Engine, Operation};
use std::path::Path;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn operation(editor: &str, command: &str, params: serde_json::Value) -> Fallible<Operation> {
    Ok(serde_json::from_value(serde_json::json!({
        "editor": editor, "operation": {"command": command, "params": params}
    }))?)
}

fn apply(
    session: &mut Session,
    operation: Operation,
) -> sparksmith_automation::Result<serde_json::Value> {
    session.execute(Request::Apply(ApplyParams {
        expected_revision: session.engine().revision(),
        operation,
    }))
}

#[test]
fn raising_the_top_held_module_keeps_connections_or_refuses_atomically() -> Fallible {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith");
    let mut engine = Engine::from_file(sparksmith_io::read_project(&path)?)?;
    engine.refill_stale()?;
    let mut session = Session::new(engine);
    let u1 = session
        .engine()
        .project()
        .instance_by_designator("U1")
        .ok_or("U1")?
        .id;
    let top = serde_json::to_value(&session.engine().project().board_shape)?["sketch"]["edges"]
        .as_array()
        .ok_or("edges")?
        .iter()
        .find(|e| e["name"] == "top")
        .ok_or("top")?["curve"]
        .clone();
    let baseline: Vec<_> = layout::checks(session.engine().project(), &Default::default())
        .into_iter()
        .filter(|f| {
            matches!(
                f.code.as_str(),
                "board.unrouted" | "board.track_dangling" | "board.via_dangling"
            )
        })
        .collect();
    assert!(
        baseline.is_empty(),
        "baseline has lost connections: {baseline:#?}"
    );
    let original = session.engine().to_file();
    apply(
        &mut session,
        operation(
            "Dimension",
            "layout.dimension_add",
            serde_json::json!({
                "dimension": {"kind": {"dim": "distance"}, "refs": [
                    {"part": {"feature": "origin", "target": {"kind": "placement", "id": u1}}},
                    {"board": {"curve": top}}
                ], "value": {"fixed": 15}}
            }),
        )?,
    )?;
    let before = session.engine().to_file();
    let before_project = session.engine().project().clone();
    let revision = session.engine().revision();
    let undo = session.engine().undo_step();
    let resize = operation(
        "Board",
        "board.edit",
        serde_json::json!({"edits": [
            {"op": "set_parameter", "param": {"name": "height", "expr": "70 mm", "unit": "mm", "comment": ""}}
        ]}),
    )?;
    match apply(&mut session, resize) {
        Ok(_) => {
            apply(
                &mut session,
                operation(
                    "Fill",
                    "fill.refill",
                    serde_json::json!({"target": {"fills": "all"}}),
                )?,
            )?;
            let lost: Vec<_> = layout::checks(session.engine().project(), &Default::default())
                .into_iter()
                .filter(|f| {
                    matches!(
                        f.code.as_str(),
                        "board.unrouted" | "board.track_dangling" | "board.via_dangling"
                    )
                })
                .collect();
            assert!(
                lost.is_empty(),
                "board resize committed lost connections: {lost:#?}"
            );
        }
        Err(error) => {
            assert!(
                error.to_string().to_lowercase().contains("cannot keep"),
                "{error}"
            );
            assert_eq!(session.engine().to_file(), before);
            assert_eq!(session.engine().project(), &before_project);
            assert_eq!(session.engine().revision(), revision);
            assert_eq!(session.engine().undo_step(), undo);
            session.execute(Request::Undo(
                sparksmith_automation::session::RevisionParams {
                    expected_revision: revision,
                },
            ))?;
            assert_eq!(
                session.engine().to_file(),
                original,
                "refusal added no undo step"
            );
        }
    }
    Ok(())
}
