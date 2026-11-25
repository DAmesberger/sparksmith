//! The fabricators' stackups over the JSON-lines transport (the CLI's
//! `stdio` and the MCP tools dispatch the same `project.apply` and
//! `project.query`): listing, one in full, taking one by id, undo, and the
//! Standard profile's list of every maker.
use super::*;
use sparksmith_core::Engine;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn query(session: &mut Session, params: Value) -> Value {
    session.dispatch(json!({"command": "project.query", "params": {
        "query": "layout.stackups", "params": params}}))
}

fn apply(session: &mut Session, editor: &str, command: &str, params: Value) -> Value {
    let revision = session.engine().revision();
    session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": editor,
            "operation": {"command": command, "params": params}}}}))
}

fn ids(reply: &Value) -> Vec<String> {
    reply["result"]["result"]["stackups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| s["id"].as_str().map(str::to_owned))
        .collect()
}

fn on_profile(profile: &str) -> std::result::Result<Session, Box<dyn std::error::Error>> {
    let mut session = Session::new(Engine::demo()?);
    let reply = apply(
        &mut session,
        "Rules",
        "rules.set_fab_profile",
        json!({"profile": profile}),
    );
    if reply["ok"] != true {
        return Err(format!("{reply}").into());
    }
    Ok(session)
}

#[test]
fn a_stackup_is_listed_read_taken_and_undone() -> TestResult {
    let mut session = on_profile("jlcpcb-4-layer")?;
    let listed = query(&mut session, json!({"copper_layers": 4, "board_mm": 1.6}));
    assert_eq!(listed["ok"], true, "{listed}");
    assert_eq!(listed["result"]["report"], "stackups");
    let found = ids(&listed);
    assert!(found.contains(&"jlc04161h-7628".to_owned()), "{found:?}");
    assert!(
        found.iter().all(|id| id.starts_with("jlc0416")),
        "{found:?}"
    );

    // One in full: its sources and disagreements, the build, and what
    // taking it does. Nothing changes.
    let revision = session.engine().revision();
    let one = query(&mut session, json!({"id": "jlc04161h-7628"}));
    assert_eq!(one["result"]["report"], "stackup", "{one}");
    let report = &one["result"]["result"];
    assert!(report["stackup"]["source"]["url"].is_string());
    assert!(
        !report["stackup"]["disagreements"]
            .as_array()
            .ok_or("none")?
            .is_empty()
    );
    assert_eq!(report["build"][0]["thickness_mm"], 0.2104);
    assert_eq!(report["pick"].get("refused"), None, "{report}");
    let changes = report["pick"]["changes"].as_array().ok_or("no changes")?;
    assert!(
        changes.iter().any(|c| c == "2 copper layers become 4"),
        "{changes:?}"
    );
    assert_eq!(session.engine().revision(), revision);

    let before = session.engine().project().clone();
    let taken = apply(
        &mut session,
        "Layout",
        "layout.use_stackup",
        json!({"stackup": "jlc04161h-7628"}),
    );
    assert_eq!(taken["ok"], true, "{taken}");
    let stackup = &session.engine().project().stackup;
    assert_eq!(stackup.copper.len(), 4);
    assert_eq!(stackup.dielectrics[0].dk, Some(4.4));
    let rows = query(&mut session, json!({"board_mm": 1.6}));
    let board: Vec<&Value> = rows["result"]["result"]["stackups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["board"] == true)
        .collect();
    assert_eq!(board.len(), 1, "{rows}");
    assert_eq!(board[0]["id"], "jlc04161h-7628");
    // The board keeps its name until a layer is edited.
    assert_eq!(board[0]["picked"], true, "{rows}");
    assert_eq!(rows["result"]["result"]["picked"]["id"], "jlc04161h-7628");
    let mut thicker = serde_json::to_value(&session.engine().project().stackup)?;
    thicker["dielectrics"][0]["thickness_mm"] = json!(0.22);
    let edited = apply(&mut session, "Layout", "layout.set_stackup", thicker);
    assert_eq!(edited["ok"], true, "{edited}");
    let rows = query(&mut session, json!({"board_mm": 1.6}));
    assert_eq!(rows["result"]["result"].get("picked"), None, "{rows}");
    let revision = session.engine().revision();
    let undone = session.dispatch(json!({"command": "project.undo", "params": {
        "expected_revision": revision}}));
    assert_eq!(undone["ok"], true, "{undone}");
    let rows = query(&mut session, json!({"board_mm": 1.6}));
    assert_eq!(rows["result"]["result"]["picked"]["id"], "jlc04161h-7628");

    let revision = session.engine().revision();
    let undone = session.dispatch(json!({"command": "project.undo", "params": {
        "expected_revision": revision}}));
    assert_eq!(undone["ok"], true, "{undone}");
    assert_eq!(session.engine().project(), &before);
    Ok(())
}

#[test]
fn an_unknown_stackup_and_a_profile_for_other_boards_are_said() -> TestResult {
    let mut session = on_profile("jlcpcb-2-layer")?;
    let unknown = query(&mut session, json!({"id": "none"}));
    assert_eq!(unknown["ok"], false, "{unknown}");
    assert!(
        unknown.to_string().contains("layout.stackup_unknown"),
        "{unknown}"
    );
    let revision = session.engine().revision();
    let refused = apply(
        &mut session,
        "Layout",
        "layout.use_stackup",
        json!({"stackup": "none"}),
    );
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(session.engine().revision(), revision);
    // Six layers under the two-layer profile: said, not refused.
    let six = query(&mut session, json!({"id": "jlc06161h-3313"}));
    let pick = &six["result"]["result"]["pick"];
    let conflicts = pick["conflicts"].as_array().ok_or("no conflicts")?;
    assert!(
        conflicts.iter().any(|c| c
            .as_str()
            .is_some_and(|c| c.contains("the board has 6 copper layers"))),
        "{conflicts:?}"
    );
    Ok(())
}

/// The Standard profile names no maker: every maker's are listed, each with
/// its maker; a maker's own profile lists its own.
#[test]
fn the_standard_profile_lists_every_maker() -> TestResult {
    let makers = |session: &mut Session, params: Value| -> Vec<String> {
        let reply = query(session, params);
        let mut out: Vec<String> = reply["result"]["result"]["stackups"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s["fabricator"].as_str().map(str::to_owned))
            .collect();
        out.dedup();
        out
    };
    let four = || json!({"copper_layers": 4});
    let mut standard = on_profile("standard")?;
    assert_eq!(makers(&mut standard, four()), ["jlcpcb", "pcbway"]);
    let mut pcbway = on_profile("pcbway-4-layer")?;
    assert_eq!(makers(&mut pcbway, four()), ["pcbway"]);
    assert_eq!(
        makers(
            &mut pcbway,
            json!({"copper_layers": 4, "fabricator": "jlcpcb"})
        ),
        ["jlcpcb"]
    );
    let all = query(&mut pcbway, json!({"all": true}));
    assert_eq!(ids(&all).len(), 122);
    assert_eq!(all["result"]["result"]["left_out"], 0);
    // Pressed and listed thicknesses are both there.
    let one = query(
        &mut pcbway,
        json!({"id": "pcbway-4-layer-1-6mm-1oz-1oz-70pc"}),
    );
    let report = &one["result"]["result"];
    let open = report["pick"]["open"].as_array().ok_or("nothing open")?;
    assert!(
        open.iter()
            .any(|o| o.as_str().is_some_and(|o| o.contains("before pressing"))),
        "{open:?}"
    );
    Ok(())
}
