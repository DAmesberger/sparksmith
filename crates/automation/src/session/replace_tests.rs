//! `project.preview_replace` and `project.replace_component`: an imported
//! chip part replaced by a generated library component of another lineage
//! whose pads sit elsewhere. The mapping is explicit (a suggestion by
//! names is offered, never applied); the copper report names the pads that
//! differ; tracks stay where they are and are listed as detached; nets
//! follow the mapping; one undo step takes it back; refusals change
//! nothing. Original synthetic data.
use super::{Session, hosted};
use serde_json::{Value, json};
use sparksmith_core::ids::Endpoint;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// An imported board: R1 a two-pad chip with pads 4 mm apart on nets
/// IN and OUT, a track from each pad outward, and R2 the same part
/// unrouted.
fn engine() -> Fallible<sparksmith_core::Engine> {
    let mut board = String::from(
        r#"(kicad_pcb (version 20240108) (generator "original replace fixture")
        (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "IN") (net 2 "OUT") (net 3 "X") (net 4 "Y")
        (gr_rect (start 0 0) (end 40 20) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
    );
    for (name, x, a, b) in [
        ("R1", 10, (1, "IN"), (2, "OUT")),
        ("R2", 25, (3, "X"), (4, "Y")),
    ] {
        board.push_str(&format!(
            r#"(footprint "Original:Wide2" (layer "F.Cu") (at {x} 10)
            (property "Reference" "{name}")
            (pad "1" smd rect (at -2 0) (size 1.0 1.2) (layers "F.Cu" "F.Mask" "F.Paste") (net {} "{}"))
            (pad "2" smd rect (at 2 0) (size 1.0 1.2) (layers "F.Cu" "F.Mask" "F.Paste") (net {} "{}")))"#,
            a.0, a.1, b.0, b.1
        ));
    }
    board.push_str(
        r#"(segment (start 8 10) (end 4 10) (width 0.25) (layer "F.Cu") (net 1))
        (segment (start 12 10) (end 16 10) (width 0.25) (layer "F.Cu") (net 2)))"#,
    );
    let defaults = sparksmith_io::new_project::shipped().map_err(|e| e.to_string())?;
    let profile =
        sparksmith_io::rule_profiles::for_new_board_from_env(None).map_err(|e| e.to_string())?;
    let imported = sparksmith_kicad::import(
        board.as_bytes(),
        &sparksmith_kicad::Setup {
            board: &defaults.board,
            presets: &defaults.presets,
            profile: Some(&profile),
            evaluators: hosted(),
            project_file: None,
            schematic: None,
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(imported.engine)
}

fn call(session: &mut Session, command: &str, mut params: Value) -> Value {
    if command != "project.preview_replace" && params.get("expected_revision").is_none() {
        params["expected_revision"] = json!(session.engine().revision());
    }
    session.dispatch(json!({"command": command, "params": params}))
}

/// The generated 0603 resistor component: another lineage.
fn target(session: &mut Session) -> Fallible<Value> {
    let made = call(
        session,
        "library.generate_part",
        json!({"generator": "chip", "scope": "component", "preset": "resistor-0603"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    made["result"]["outcome"]["report"]["revisions"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|r| r["kind"] == "component")
        .map(|r| json!({"id": r["id"], "revision": r["revision"]}))
        .ok_or_else(|| format!("no component revision: {made}").into())
}

fn instance(
    session: &Session,
    designator: &str,
) -> Fallible<sparksmith_core::model::ComponentInstance> {
    session
        .engine()
        .project()
        .instances
        .iter()
        .find(|i| i.designator == designator)
        .cloned()
        .ok_or_else(|| format!("no {designator}").into())
}

fn net_of(session: &Session, endpoint: Endpoint) -> Option<String> {
    let project = session.engine().project();
    project.net_of(endpoint).map(|n| n.name.clone())
}

#[test]
fn a_part_is_replaced_by_another_lineage_with_an_explicit_mapping() -> Fallible {
    let mut session = Session::fixture(engine()?);
    let target = target(&mut session)?;
    let r1 = instance(&session, "R1")?;
    let tracks = session.engine().project().tracks.clone();
    assert_eq!(
        tracks.len(),
        2,
        "{:?}",
        session.engine().project().refused_import_tracks
    );

    // Without a mapping: the preview offers names, the plan is blocked.
    let request = json!({"designators": ["R1"], "target": target});
    let preview = call(&mut session, "project.preview_replace", request.clone());
    assert_eq!(preview["ok"], true, "{preview}");
    let preview = &preview["result"];
    let blockers = preview["plan"]["blockers"].to_string();
    assert!(blockers.contains("mapping is incomplete"), "{preview}");
    let current = preview["terminals"]["current"]
        .as_array()
        .ok_or("current")?;
    let new = preview["terminals"]["target"].as_array().ok_or("target")?;
    assert_eq!((current.len(), new.len()), (2, 2), "{preview}");
    assert_eq!(
        preview["suggested_terminals"].as_array().map(Vec::len),
        Some(2),
        "{preview}"
    );

    // Applying without a complete mapping, with an unknown or ambiguous
    // name, or at an outdated revision is refused and changes nothing.
    let before = session.engine().document().clone();
    let revision = session.engine().revision();
    let mut half = request.clone();
    half["terminals"] = json!([{"from": current[0]["id"], "to": new[0]["id"]}]);
    for (replace, expected) in [
        (request.clone(), revision),
        (half, revision),
        (
            {
                let mut r = request.clone();
                r["terminals"] = json!([{"from": "nine", "to": new[0]["id"]}]);
                r
            },
            revision,
        ),
    ] {
        let refused = call(
            &mut session,
            "project.replace_component",
            json!({"replace": replace, "expected_revision": expected}),
        );
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(session.engine().document(), &before);
    }
    let mapping = json!([
        {"from": current[0]["id"], "to": new[0]["id"]},
        {"from": current[1]["id"], "to": new[1]["id"]},
    ]);
    let mut full = request.clone();
    full["terminals"] = mapping;
    let stale = call(
        &mut session,
        "project.replace_component",
        json!({"replace": full, "expected_revision": revision + 7}),
    );
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(session.engine().document(), &before);

    // The complete mapping: the copper differs (the new pads sit 1.3 mm
    // apart, not 4), the tracks are listed as detached and kept.
    let preview = call(&mut session, "project.preview_replace", full.clone());
    let plan = &preview["result"]["plan"];
    assert_eq!(plan["blockers"], json!([]), "{plan}");
    let copper = &plan["copper"][0];
    assert_eq!(copper["designator"], "R1", "{plan}");
    assert_eq!(copper["identical"], false, "{plan}");
    assert_eq!(
        copper["old_only"].as_array().map(Vec::len),
        Some(2),
        "{plan}"
    );
    assert_eq!(
        copper["new_only"].as_array().map(Vec::len),
        Some(2),
        "{plan}"
    );
    let warnings = plan["warnings"].to_string();
    assert!(
        warnings.contains("A track on IN") && warnings.contains("A track on OUT"),
        "{plan}"
    );
    assert_eq!(plan["conflicting_tracks"], json!([]), "{plan}");

    let applied = call(
        &mut session,
        "project.replace_component",
        json!({"replace": full, "expected_revision": revision}),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["result"]["copper"][0]["identical"], false);
    let after = instance(&session, "R1")?;
    assert_eq!(after.component.id.0, target["id"].as_u64().ok_or("id")?);
    assert_eq!(after.id, r1.id, "the part keeps its identity");
    assert_eq!(
        instance(&session, "R2")?.component,
        r1.component,
        "R2 untouched"
    );
    // Nets follow the mapping; tracks never move or go.
    let end = |t: &Value| -> Fallible<Endpoint> {
        Ok(Endpoint {
            instance: r1.id,
            terminal: serde_json::from_value(t["id"].clone())?,
        })
    };
    assert_eq!(net_of(&session, end(&new[0])?).as_deref(), Some("IN"));
    assert_eq!(net_of(&session, end(&new[1])?).as_deref(), Some("OUT"));
    assert_eq!(session.engine().project().tracks, tracks);
    // The board check now names the copper left for rerouting.
    let codes: Vec<String> =
        sparksmith_core::layout::checks(session.engine().project(), &Default::default())
            .into_iter()
            .map(|f| f.code)
            .collect();
    assert!(
        codes
            .iter()
            .any(|c| c == "board.unrouted" || c == "board.track_dangling"),
        "{codes:?}"
    );

    // One undo step restores everything.
    let undo = session.dispatch(json!({"command": "project.undo",
        "params": {"expected_revision": session.engine().revision()}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert_eq!(session.engine().project(), before.project.as_ref());
    Ok(())
}

/// A mapping naming one old terminal twice is ambiguous; a dropped
/// terminal must be stated as null.
#[test]
fn an_ambiguous_mapping_is_refused_in_plain_words() -> Fallible {
    let mut session = Session::fixture(engine()?);
    let target = target(&mut session)?;
    let preview = call(
        &mut session,
        "project.preview_replace",
        json!({"designators": ["R2"], "target": target}),
    );
    let current = preview["result"]["terminals"]["current"].clone();
    let new = preview["result"]["terminals"]["target"].clone();
    let twice = json!({"designators": ["R2"], "target": target, "terminals": [
        {"from": current[0]["id"], "to": new[0]["id"]},
        {"from": current[0]["id"], "to": new[1]["id"]},
    ]});
    let plan = call(&mut session, "project.preview_replace", twice);
    assert!(
        plan["result"]["plan"]["blockers"]
            .to_string()
            .contains("mapped twice"),
        "{plan}"
    );
    // Dropping a terminal explicitly is complete.
    let dropped = json!({"designators": ["R2"], "target": target, "terminals": [
        {"from": current[0]["id"], "to": new[0]["id"]},
        {"from": current[1]["id"], "to": null},
    ]});
    let plan = call(&mut session, "project.preview_replace", dropped);
    assert_eq!(plan["result"]["plan"]["blockers"], json!([]), "{plan}");
    Ok(())
}
