//! Layout blocks on the wire, as an agent or the CLI sends them: made, listed,
//! moved and deleted by the entries' names; and on the ESP32-S31 dev board a
//! part its dimensions hold is named, and dimensions win, also over a locked
//! block.
use super::Session;
use serde_json::{Value, json};
use sparksmith_core::{Engine, Id, Point};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const EXAMPLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith"
);

fn open() -> TestResult<Session> {
    let mut session = Session::new(Engine::default());
    let opened = session.dispatch(json!({"command": "project.open", "params": {
        "expected_revision": session.engine().revision(), "path": EXAMPLE}}));
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(session)
}

fn layout(session: &mut Session, command: &str, params: Value) -> Value {
    let revision = session.engine().revision();
    session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Layout", "operation": {"command": command, "params": params}}}}))
}

fn id(session: &Session, designator: &str) -> TestResult<Id> {
    let project = session.engine().project();
    Ok(project
        .instance_by_designator(designator)
        .ok_or_else(|| designator.to_owned())?
        .id)
}

fn at(session: &Session, part: Id) -> TestResult<Point> {
    let project = session.engine().project();
    Ok(project.placement_of(part).ok_or("not placed")?.position)
}

fn rows(session: &mut Session) -> TestResult<Value> {
    let reply = session.dispatch(json!({"command": "project.query",
        "params": {"query": "layout.blocks", "params": {}}}));
    assert_eq!(reply["ok"], true, "{reply}");
    Ok(reply["result"]["result"].clone())
}

#[test]
fn blocks_are_made_listed_moved_and_deleted_on_the_wire() -> TestResult {
    let mut session = Session::new(Engine::demo()?);
    assert_eq!(rows(&mut session)?, json!([]));
    let (r1, r2) = (id(&session, "R1")?, id(&session, "R2")?);
    let made = layout(
        &mut session,
        "layout.block_add",
        json!({"parts": [r1, r2], "name": "Divider"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let block = made["result"]["outcome"]["block"].clone();
    assert_eq!(made["result"]["outcome"]["outcome"], "block_added");
    let listed = rows(&mut session)?;
    assert_eq!(listed[0]["id"], block);
    assert_eq!(listed[0]["name"], "Divider");
    let parts = listed[0]["parts"].to_string();
    assert!(parts.contains("R1") && parts.contains("R2"), "{listed}");
    assert!(
        listed[0]["border"].as_array().is_some_and(|b| b.len() >= 3),
        "{listed}"
    );
    let before = session.engine().document().clone();
    let (a, b) = (at(&session, r1)?, at(&session, r2)?);
    let revision = session.engine().revision();
    let moved = layout(
        &mut session,
        "layout.block_move",
        json!({"id": block, "turn_deg": 180.0}),
    );
    assert_eq!(moved["ok"], true, "{moved}");
    assert_eq!(session.engine().revision(), revision + 1);
    let (c, d) = (at(&session, r1)?, at(&session, r2)?);
    for k in 0..2 {
        assert!(((b[k] - a[k]) + (d[k] - c[k])).abs() < 1e-9, "turned about");
    }
    let undone = session.dispatch(json!({"command": "project.undo",
        "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(session.engine().document(), &before);
    let gone = layout(&mut session, "layout.block_delete", json!({"id": block}));
    assert_eq!(gone["ok"], true, "{gone}");
    assert_eq!(rows(&mut session)?, json!([]));
    // A typo in a name is refused and changes nothing.
    let wrong = layout(
        &mut session,
        "layout.block_add",
        json!({"parts": [r1], "titel": "x"}),
    );
    assert_eq!(wrong["ok"], false, "{wrong}");
    Ok(())
}

/// J1 is held to the bottom edge and the centre line: a block with it does
/// not move and says J1; a wider board moves J1 all the same, also in a
/// locked block, and the border follows.
#[test]
fn dimensions_win_over_a_block() -> TestResult {
    let mut session = open()?;
    let (j1, r5) = (id(&session, "J1")?, id(&session, "R5")?);
    let made = layout(
        &mut session,
        "layout.block_add",
        json!({"parts": [j1, r5], "name": "Connector"}),
    );
    let block = made["result"]["outcome"]["block"].clone();
    let before = session.engine().document().clone();
    for how in [
        json!({"id": block, "by": [1.0, 0.0]}),
        json!({"id": block, "turn_deg": 90.0}),
        json!({"id": block, "flip": true}),
    ] {
        let refused = layout(&mut session, "layout.block_move", how);
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(refused["error"].to_string().contains("J1"), "{refused}");
        assert_eq!(session.engine().document(), &before);
    }
    let locked = layout(
        &mut session,
        "layout.block_lock",
        json!({"id": block, "locked": true}),
    );
    assert_eq!(locked["ok"], true, "{locked}");
    let border = rows(&mut session)?[0]["border"].clone();
    let revision = session.engine().revision();
    let was = at(&session, j1)?;
    let widened = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Board", "operation": {"command": "board.edit", "params": {"edits": [
            {"op": "set_parameter", "param": {"name": "width", "expr": "50 mm", "unit": "mm",
                "comment": ""}}]}}}}}));
    assert_eq!(widened["ok"], true, "{widened}");
    assert!((at(&session, j1)?[0] - was[0] - 5.0).abs() < 1e-6);
    assert_ne!(rows(&mut session)?[0]["border"], border);
    Ok(())
}
