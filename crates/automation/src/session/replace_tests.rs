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

/// An imported board with project parts of one three-pad land pattern
/// drawn about different origins: U1's footprint has its origin between
/// pads 1 and 2, U2's at pad 1, and U3's is U2's turned a quarter turn.
/// U1 is routed: a track from each of pads 1 and 2 outward.
fn offset_engine() -> Fallible<sparksmith_core::Engine> {
    let pads = |at: [(f64, f64, f64); 3], nets: [(u32, &str); 3]| -> String {
        at.iter()
            .zip(nets)
            .enumerate()
            .map(|(k, ((x, y, angle), (net, name)))| {
                let net = if net == 0 {
                    String::new()
                } else {
                    format!(r#" (net {net} "{name}")"#)
                };
                format!(
                    r#"(pad "{}" smd rect (at {x} {y} {angle}) (size 1.0 1.6) (layers "F.Cu" "F.Mask" "F.Paste"){net})"#,
                    k + 1
                )
            })
            .collect()
    };
    let none = [(0, ""), (0, ""), (0, "")];
    let board = format!(
        r#"(kicad_pcb (version 20240108) (generator "original offset fixture")
        (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
        (net 0 "") (net 1 "A") (net 2 "B") (net 3 "C")
        (gr_rect (start 0 0) (end 60 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))
        (footprint "Original:Centred" (layer "F.Cu") (at 10 10)
          (property "Reference" "U1") {})
        (footprint "Original:AtPadOne" (layer "F.Cu") (at 30 10)
          (property "Reference" "U2") {})
        (footprint "Original:Turned" (layer "F.Cu") (at 45 10)
          (property "Reference" "U3") {})
        (segment (start 8 10) (end 4 10) (width 0.25) (layer "F.Cu") (net 1))
        (segment (start 12 10) (end 16 10) (width 0.25) (layer "F.Cu") (net 2)))"#,
        pads(
            [(-2.0, 0.0, 0.0), (2.0, 0.0, 0.0), (0.0, 3.0, 0.0)],
            [(1, "A"), (2, "B"), (3, "C")]
        ),
        pads([(0.0, 0.0, 0.0), (4.0, 0.0, 0.0), (2.0, 3.0, 0.0)], none),
        pads(
            [(0.0, 0.0, 90.0), (0.0, -4.0, 90.0), (3.0, -2.0, 90.0)],
            none
        ),
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

/// U1 replaced by the component of `by`, its terminals mapped by name.
fn offset_request(session: &Session, by: &str, alignment: Option<&str>) -> Fallible<Value> {
    let target = instance(session, by)?.component;
    let mut request = json!({
        "designators": ["U1"],
        "target": {"id": target.id, "revision": target.revision},
        "terminals": [
            {"from": "1", "to": "1"}, {"from": "2", "to": "2"}, {"from": "3", "to": "3"}
        ],
    });
    if let Some(alignment) = alignment {
        request["alignment"] = json!(alignment);
    }
    Ok(request)
}

/// The pad centres of a placed part on the board, by pad number.
fn pad_positions(session: &Session, designator: &str) -> Fallible<Vec<[f64; 2]>> {
    let id = instance(session, designator)?.id;
    let mut pads: Vec<(String, [f64; 2])> =
        sparksmith_core::layout::placed_pad_centres(session.engine().project(), id)
            .into_iter()
            .map(|p| (p.label, p.position))
            .collect();
    pads.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pads.into_iter().map(|(_, p)| p).collect())
}

fn close(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(p, q)| (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6)
}

/// A footprint whose origin lies elsewhere is moved so its mapped pads
/// land on the old ones: the copper stays the same and the routing stays
/// connected. `keep_origin` keeps the old origin and moves every pad.
#[test]
fn a_replacement_fits_its_pads_to_the_old_ones() -> Fallible {
    let mut session = Session::fixture(offset_engine()?);
    let old = pad_positions(&session, "U1")?;
    assert_eq!(old.len(), 3);
    let tracks = session.engine().project().tracks.clone();
    assert_eq!(tracks.len(), 2);
    let codes_before = board_codes(&session);
    assert!(!codes_before.iter().any(|c| c == "board.unrouted"));

    // Keep origin: every pad lies 2 mm off; the routing detaches.
    let keep = offset_request(&session, "U2", Some("keep_origin"))?;
    let preview = call(&mut session, "project.preview_replace", keep);
    let plan = &preview["result"]["plan"];
    assert_eq!(plan["blockers"], json!([]), "{plan}");
    assert_eq!(plan["copper"][0]["identical"], false, "{plan}");
    let fit = &plan["fits"][0];
    assert_eq!(fit["alignment"], "keep_origin", "{plan}");
    assert_eq!(fit["moved_mm"], json!([0.0, 0.0]), "{plan}");
    assert!(
        (fit["max_residual_mm"].as_f64().ok_or("residual")? - 2.0).abs() < 1e-6,
        "{plan}"
    );
    assert!(
        plan["warnings"].to_string().contains("A track on A"),
        "{plan}"
    );

    // Fit pads (the default): moved 2 mm, every residual zero, the copper
    // identical, no track detached.
    let request = offset_request(&session, "U2", None)?;
    let preview = call(&mut session, "project.preview_replace", request.clone());
    let plan = &preview["result"]["plan"];
    assert_eq!(plan["blockers"], json!([]), "{plan}");
    assert_eq!(plan["copper"][0]["identical"], true, "{plan}");
    let fit = &plan["fits"][0];
    assert_eq!(fit["alignment"], "fit_pads", "{plan}");
    assert_eq!(fit["turned_deg"], 0.0, "{plan}");
    let moved = &fit["moved_mm"];
    assert!(
        (moved[0].as_f64().ok_or("x")?.abs() - 2.0).abs() < 1e-6
            && moved[1].as_f64().ok_or("y")?.abs() < 1e-6,
        "{plan}"
    );
    assert_eq!(fit["pads"].as_array().map(Vec::len), Some(3), "{plan}");
    assert_eq!(fit["max_residual_mm"], 0.0, "{plan}");
    assert!(
        !plan["warnings"].to_string().contains("A track on"),
        "{plan}"
    );
    assert!(
        plan["changes"]
            .to_string()
            .contains("U1 is placed so its mapped pads fit the old ones"),
        "{plan}"
    );

    let applied = call(
        &mut session,
        "project.replace_component",
        json!({"replace": request}),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    assert!(close(&pad_positions(&session, "U1")?, &old));
    assert_eq!(session.engine().project().tracks, tracks);
    // The board check finds what it found before: the routing still
    // reaches every pad.
    assert_eq!(board_codes(&session), codes_before);
    Ok(())
}

/// The board check's finding codes, sorted.
fn board_codes(session: &Session) -> Vec<String> {
    let mut codes: Vec<String> =
        sparksmith_core::layout::checks(session.engine().project(), &Default::default())
            .into_iter()
            .map(|f| f.code)
            .collect();
    codes.sort();
    codes
}

/// A footprint drawn a quarter turn from the old one is turned back onto
/// the old pads.
#[test]
fn a_replacement_turned_a_quarter_fits_by_turning() -> Fallible {
    let mut session = Session::fixture(offset_engine()?);
    let old = pad_positions(&session, "U1")?;
    let request = offset_request(&session, "U3", None)?;
    let preview = call(&mut session, "project.preview_replace", request.clone());
    let plan = &preview["result"]["plan"];
    assert_eq!(plan["blockers"], json!([]), "{plan}");
    let fit = &plan["fits"][0];
    assert_eq!(
        fit["turned_deg"].as_f64().map(f64::abs),
        Some(90.0),
        "{plan}"
    );
    assert!(
        fit["max_residual_mm"].as_f64().ok_or("residual")? < 1e-6,
        "{plan}"
    );
    let applied = call(
        &mut session,
        "project.replace_component",
        json!({"replace": request}),
    );
    assert_eq!(applied["ok"], true, "{applied}");
    assert!(close(&pad_positions(&session, "U1")?, &old));
    Ok(())
}
