//! Slice B4's gate (acceptance 6 and 7 of the design note) through the
//! JSON transport, over the ESP32-S31 dev board example (with J1's package
//! carrying its USB4105 part file's opening, as in B3's gate), with an
//! original headless SolidCraft assembly test document: the published
//! board linked and grounded; a 2 mm wall across J1's opening with a
//! cutout projected from the opening ("USB opening"); a boss projected
//! from mounting hole 1 ("Boss"); and a far wall beyond the board's top
//! edge ("Far wall"), saved as a `.solidcraft` file.
use super::Session;
use super::interface_publish_tests::{
    TestResult, apply, faces, instance, linked_design, ok, open, publish, remove,
};
use serde_json::{Value, json};
use sparksmith_core::Operation;
use sparksmith_core::layout::outline::{BoardCommand, BoardEdit};
use sparksmith_core::library::footprint_source::{
    DimKind, DimValue, ParamName, SketchConstraintId, SketchCurveId, SketchEdit, SketchPointId,
    SketchRef,
};
use sparksmith_core::model::BoardShape;
use sparksmith_solidcraft::linked_design::LinkedDesign;
use std::path::Path;

/// Where the far wall's inner face stands in the test document, mm: half
/// a millimetre beyond the board's top edge (y = 64).
pub(super) const FAR_WALL_Y: f64 = 64.5;

/// The test linked design with its far wall's inner face at `far_y`, saved at
/// `path`.
pub(super) fn save_linked_design(
    design: &Path,
    opening: &str,
    far_y: f64,
    path: &Path,
) -> TestResult {
    let (mut assembly_link, _) = linked_design(design, opening)?;
    assembly_link.solid_box("Far wall", [-5.0, far_y, -2.0], [50.0, 2.0, 12.0])?;
    let _ = std::fs::remove_file(path);
    assembly_link.save(crate::native_files::new_file(&path.display().to_string()))?;
    Ok(())
}

/// The far wall's inner face (normal -y) in a saved linked design: its name
/// and a point on it.
pub(super) fn far_face(path: &Path) -> TestResult<(String, [f64; 3])> {
    let (mut assembly_link, newer) = LinkedDesign::open(&path.display().to_string())?;
    assert!(!newer);
    let face = faces(&mut assembly_link, "Far wall")?
        .into_iter()
        .find(|f| f.normal.is_some_and(|n| n[1] < -0.99))
        .ok_or("the far wall's inner face")?;
    Ok((face.name, face.point))
}

/// `assembly.status`, the document's view of the link.
pub(super) fn status(session: &mut Session) -> TestResult<Value> {
    Ok(ok(
        session,
        json!({"command": "project.query", "params": {
            "query": "assembly.status", "params": {}}}),
    )?["result"]
        .clone())
}

/// Whether `assembly.preview` finds the linked design changed since the board
/// read it.
pub(super) fn changed(
    session: &mut Session,
    assembly_link: &Path,
    design: &Path,
) -> TestResult<bool> {
    let preview = ok(
        session,
        json!({"command": "assembly.preview", "params": files(assembly_link, design)}),
    )?;
    preview["changed"].as_bool().ok_or_else(|| "changed".into())
}

/// The files every linked design request names.
pub(super) fn files(assembly_link: &Path, design: &Path) -> Value {
    json!({"path": assembly_link.display().to_string(), "design": design.display().to_string()})
}

pub(super) fn with(mut params: Value, more: Value) -> Value {
    if let (Some(a), Some(b)) = (params.as_object_mut(), more.as_object()) {
        a.extend(b.clone());
    }
    params
}

/// `assembly.update` with no assembly linked: link it.
pub(super) fn link(
    session: &mut Session,
    assembly_link: &Path,
    design: &Path,
) -> TestResult<Value> {
    let revision = session.engine().revision();
    ok(
        session,
        json!({"command": "assembly.update", "params": {
            "path": assembly_link.display().to_string(),
            "design": design.display().to_string(),
            "expected_revision": revision}}),
    )
}

/// The production check's items with `code`.
pub(super) fn preflight(session: &mut Session, code: &str) -> TestResult<Vec<Value>> {
    let reply = ok(
        session,
        json!({"command": "project.query", "params": {
            "query": "fabrication.preflight", "params": {}}}),
    )?;
    Ok(reply["result"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["code"] == code)
        .cloned()
        .collect())
}

pub(super) fn undo(session: &mut Session) -> TestResult {
    let revision = session.engine().revision();
    ok(
        session,
        json!({"command": "project.undo", "params": {"expected_revision": revision}}),
    )?;
    Ok(())
}

/// The board's solved edge `name`: its two ends.
pub(super) fn edge(session: &Session, name: &str) -> TestResult<([f64; 2], [f64; 2])> {
    let BoardShape::Sketch(sketch) = &session.engine().project().board_shape else {
        return Err("a sketch board".into());
    };
    let e = sketch
        .solved
        .edges
        .iter()
        .find(|e| e.name.0 == name)
        .ok_or(name.to_owned())?;
    Ok((e.a, e.b))
}

pub(super) fn position(session: &Session, designator: &str) -> TestResult<[f64; 2]> {
    let id = instance(session, designator)?;
    Ok(session
        .engine()
        .project()
        .placement_of(id)
        .ok_or("placed")?
        .position)
}

/// Acceptance 6, as slice P1 decided it: deleting J1, the publish
/// preview's dry run names the linked design's cutout that would fail, and
/// says the publish waits, before anything is written. `assembly.status`
/// reads "broken reference" at once; the production check lists
/// `assembly.reference_lost`, naming the cutout and J1, as a warning that
/// blocks no fabrication output; publishing is refused, writing nothing,
/// until it is resolved; and undo restores everything.
#[test]
fn b4_deleting_j1_breaks_the_cutout_and_blocks_publishing() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let j1 = instance(&session, "J1")?;
    let opening = format!("i{j1}.opening");
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    let linked = link(&mut session, &assembly_link, &design)?;
    let consumers = linked["assembly"]["consumers"]
        .as_array()
        .ok_or("consumers")?;
    assert!(
        consumers
            .iter()
            .any(|c| c["feature"] == "USB opening" && c["token"] == opening.as_str()),
        "the cutout uses J1's opening: {consumers:?}"
    );
    assert!(
        consumers.iter().any(|c| c["feature"] == "Boss"),
        "the boss uses mounting hole 1: {consumers:?}"
    );
    assert_eq!(status(&mut session)?["state"], "linked");
    assert!(!changed(&mut session, &assembly_link, &design)?);
    let before = session.engine().project().clone();
    let linked_design_bytes = std::fs::read(&assembly_link)?;

    remove(&mut session, j1)?;
    let design_bytes = std::fs::read(&design)?;
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly_link.display().to_string()}}),
    )?;
    assert_eq!(
        std::fs::read(&design)?,
        design_bytes,
        "a preview writes nothing"
    );
    assert_eq!(
        std::fs::read(&assembly_link)?,
        linked_design_bytes,
        "the linked design is never written"
    );
    let effects = preview["dry_run"]["effects"]
        .as_array()
        .ok_or("the dry run")?;
    let fails: Vec<&Value> = effects.iter().filter(|e| e["effect"] == "fails").collect();
    let [cutout] = fails.as_slice() else {
        return Err(format!("one failing feature: {effects:?}").into());
    };
    assert_eq!(cutout["feature"], "USB opening");
    assert_eq!(
        cutout["message"],
        "The linked design's ‘USB opening’ uses J1 opening, which this publish removes, so it would \
         fail"
    );
    assert!(cutout["datum"].is_u64(), "{cutout}");

    let blocked = preview["blocked"].as_array().ok_or("the publish waits")?;
    assert_eq!(blocked.len(), 1, "{blocked:?}");

    // Broken as soon as J1 is gone.
    let broken = status(&mut session)?;
    assert_eq!(broken["state"], "broken_reference", "{broken}");
    assert_eq!(broken["words"], "broken reference");
    let items = preflight(&mut session, "assembly.reference_lost")?;
    let [item] = items.as_slice() else {
        return Err(format!("one broken reference: {items:?}").into());
    };
    assert_eq!(item["severity"], "warning", "fabrication output goes ahead");
    let title = item["title"].as_str().ok_or("a title")?;
    assert!(
        title.contains("‘USB opening’") && title.contains("J1 was deleted"),
        "{title}"
    );
    // Publishing waits, writing nothing.
    let sidecar = sparksmith_io::interface::sidecar_path(&design);
    let sidecar_bytes = std::fs::read(&sidecar)?;
    let revision = session.engine().revision();
    let refused = session.dispatch(json!({"command": "interface.publish", "params": {
        "path": design.display().to_string(), "tier": "interface",
        "expected_revision": revision}}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"]["rule"], "assembly.publish_blocked",
        "{refused}"
    );
    let message = refused["error"]["message"].as_str().ok_or("a message")?;
    assert!(message.contains("J1 was deleted"), "{message}");
    assert_eq!(session.engine().revision(), revision);
    assert_eq!(std::fs::read(&design)?, design_bytes, "nothing written");
    assert_eq!(std::fs::read(&sidecar)?, sidecar_bytes);

    undo(&mut session)?;
    assert_eq!(
        *session.engine().project(),
        before,
        "undo restores the document"
    );
    assert!(preflight(&mut session, "assembly.reference_lost")?.is_empty());
    assert_eq!(status(&mut session)?["state"], "linked");
    Ok(())
}

/// The dry run of a move names the linked design feature that follows, by its
/// SolidCraft name (acceptance 2's second half): J1 moved 2 mm, the
/// cutout moves 2 mm along X; nothing fails.
#[test]
fn b4_the_dry_run_names_the_feature_that_follows() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    link(&mut session, &assembly_link, &design)?;
    super::interface_publish_tests::move_j1(&mut session, 2.0)?;
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly_link.display().to_string()}}),
    )?;
    let effects = preview["dry_run"]["effects"]
        .as_array()
        .ok_or("the dry run")?;
    let [moves] = effects.as_slice() else {
        return Err(format!("one effect: {effects:?}").into());
    };
    assert_eq!(moves["effect"], "moves");
    assert_eq!(moves["feature"], "USB opening");
    assert_eq!(
        moves["message"],
        "The linked design's ‘USB opening’ follows J1 opening: it moves 2 mm along X"
    );
    Ok(())
}

/// Acceptance 7: the linked design → board direction. The board's top edge
/// is dimensioned 0.5 mm from the far wall's inner face, followed as a
/// followed face, and SW2 is dimensioned 7 mm from the top edge. The
/// wall moved 1 mm in the test document: `assembly.preview` finds the
/// linked design changed and lists the reference, the top
/// edge and SW2, each 1 mm along Y; `assembly.update` at the previewed hash
/// applies it as one undo step.
#[test]
fn b4_the_board_follows_the_linked_design_wall() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    link(&mut session, &assembly_link, &design)?;
    let (face, pick) = far_face(&assembly_link)?;
    let revision = session.engine().revision();
    let added = ok(
        &mut session,
        json!({"command": "assembly.follow", "params": with(files(&assembly_link, &design), json!({
            "body": "Far wall", "face": face, "pick": pick, "name": "Far wall · inner face",
            "expected_revision": revision}))}),
    )?;
    let reference = &added["assembly"]["references"][0];
    assert_eq!(reference["status"], "resolved");
    let curve = reference["sketch"]["curve"]
        .as_str()
        .ok_or("the reference owns a fixed line")?
        .to_owned();
    let traced = &reference["geometry"];
    for end in ["a", "b"] {
        let y = traced[end][1].as_f64().ok_or("a line")?;
        assert!((y - FAR_WALL_Y).abs() <= 1e-9, "{traced}");
    }

    // The top edge follows the wall: the height dimension gives way to
    // 0.5 mm from the top-right corner to the wall's trace.
    let (top_a, _) = edge(&session, "top")?;
    assert!((top_a[1] - 64.0).abs() <= 1e-9);
    apply(
        &mut session,
        Operation::Board(BoardCommand::Edit {
            edits: vec![
                BoardEdit::Sketch {
                    edit: SketchEdit::Delete {
                        ids: vec![SketchRef::Constraint(SketchConstraintId("k6".into()))],
                    },
                },
                BoardEdit::RemoveParameter {
                    name: ParamName("height".into()),
                },
                BoardEdit::Sketch {
                    edit: SketchEdit::AddDimension {
                        kind: DimKind::Distance,
                        refs: vec![
                            SketchRef::Point(SketchPointId("p2".into())),
                            SketchRef::Curve(SketchCurveId(curve)),
                        ],
                        value: DimValue::Fixed(FAR_WALL_Y - 64.0),
                    },
                },
            ],
        }),
    )?;
    assert!((edge(&session, "top")?.0[1] - 64.0).abs() <= 1e-9);
    // SW2 held 7 mm below the top edge.
    let sw2 = instance(&session, "SW2")?;
    apply(
        &mut session,
        serde_json::from_value(json!({"editor": "Dimension", "operation": {
            "command": "layout.dimension_add", "params": {"dimension": {
                "kind": {"dim": "distance"},
                "refs": [
                    {"part": {"feature": "origin", "target": {"kind": "placement", "id": sw2}}},
                    {"board": {"curve": "l3"}}],
                "value": {"fixed": 7.0}}}}}))?,
    )?;
    let sw2_before = position(&session, "SW2")?;
    assert!((sw2_before[1] - 57.0).abs() <= 1e-9, "{sw2_before:?}");
    let j1_before = position(&session, "J1")?;
    assert!(!changed(&mut session, &assembly_link, &design)?);

    // The wall moves 1 mm in the test document.
    save_linked_design(&design, &opening, FAR_WALL_Y + 1.0, &assembly_link)?;
    assert_eq!(far_face(&assembly_link)?.0, face, "the same face, by name");

    let before = session.engine().project().clone();
    let revision = session.engine().revision();
    let preview = ok(
        &mut session,
        json!({"command": "assembly.preview", "params": files(&assembly_link, &design)}),
    )?;
    assert_eq!(
        session.engine().revision(),
        revision,
        "a preview changes nothing"
    );
    assert!(preview["refused"].is_null(), "{preview}");
    assert_eq!(preview["changed"], true);
    let summaries: Vec<&str> = preview["objects"]
        .as_array()
        .ok_or("objects")?
        .iter()
        .filter_map(|o| o["summary"].as_str())
        .collect();
    assert!(
        summaries.contains(&"Board edge top, 1 mm along Y"),
        "{summaries:?}"
    );
    assert!(summaries.contains(&"SW2, 1 mm along Y"), "{summaries:?}");
    assert!(
        !summaries.iter().any(|s| s.starts_with("J1")),
        "J1 is held to the bottom edge: {summaries:?}"
    );
    let references = preview["references"].as_array().ok_or("references")?;
    let [row] = references.as_slice() else {
        return Err(format!("{references:?}").into());
    };
    assert_eq!(row["summary"], "Far wall · inner face, 1 mm along Y");

    // A stale hash is refused, changing nothing.
    let stale = session.dispatch(json!({"command": "assembly.update", "params": with(
        files(&assembly_link, &design), json!({
        "expect_hash": before.assembly_link.as_ref().ok_or("linked")?.file_sha256.to_string(),
        "expected_revision": revision}))}));
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(session.engine().revision(), revision);

    ok(
        &mut session,
        json!({"command": "assembly.update", "params": with(files(&assembly_link, &design), json!({
            "expect_hash": preview["expect_hash"], "expected_revision": revision}))}),
    )?;
    assert_eq!(session.engine().revision(), revision + 1, "one step");
    let (top_a, top_b) = edge(&session, "top")?;
    assert!((top_a[1] - 65.0).abs() <= 1e-9 && (top_b[1] - 65.0).abs() <= 1e-9);
    let sw2_after = position(&session, "SW2")?;
    assert!((sw2_after[1] - sw2_before[1] - 1.0).abs() <= 1e-9);
    assert!((sw2_after[0] - sw2_before[0]).abs() <= 1e-9);
    assert_eq!(position(&session, "J1")?, j1_before);
    assert!(!changed(&mut session, &assembly_link, &design)?);
    assert_eq!(status(&mut session)?["state"], "linked");

    undo(&mut session)?;
    assert_eq!(
        *session.engine().project(),
        before,
        "one undo step restores it"
    );
    Ok(())
}

/// A missing linked design: the preview says so in the user's words, the
/// project keeps its snapshot, and a relink to its new place, previewed
/// there, links it again.
#[test]
fn b4_a_missing_linked_design_keeps_the_snapshot() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    link(&mut session, &assembly_link, &design)?;
    let moved = dir.path().join("moved.solidcraft");
    std::fs::rename(&assembly_link, &moved)?;
    let missing = session.dispatch(json!({"command": "assembly.preview",
        "params": files(&assembly_link, &design)}));
    assert_eq!(missing["ok"], false);
    assert!(
        missing
            .to_string()
            .contains("The linked design file was not found"),
        "{missing}"
    );
    // The project keeps its snapshot.
    assert_eq!(status(&mut session)?["state"], "linked");
    // Without a preview's hash nothing is applied.
    let revision = session.engine().revision();
    let unreviewed = session.dispatch(json!({"command": "assembly.update", "params": with(
        files(&moved, &design), json!({"expected_revision": revision}))}));
    assert_eq!(unreviewed["ok"], false, "{unreviewed}");
    assert_eq!(session.engine().revision(), revision);
    // Relinked at its new place, as previewed there.
    let there = ok(
        &mut session,
        json!({"command": "assembly.preview", "params": files(&moved, &design)}),
    )?;
    assert!(there["refused"].is_null(), "{there}");
    assert_eq!(there["changed"], false, "the same file, moved");
    ok(
        &mut session,
        json!({"command": "assembly.update", "params": with(files(&moved, &design), json!({
            "expect_hash": there["expect_hash"], "expected_revision": revision}))}),
    )?;
    let linked = status(&mut session)?;
    assert_eq!(
        linked["assembly_hint"],
        moved.display().to_string(),
        "{linked}"
    );
    assert!(!changed(&mut session, &moved, &design)?);
    Ok(())
}

#[test]
fn a_publish_between_preview_reads_refuses_mixed_generations() -> TestResult {
    use std::time::Duration;
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly)?;
    link(&mut session, &assembly, &design)?;
    let mut publisher = Session::new(session.engine().fork());
    super::interface_publish_tests::move_j1(&mut publisher, 2.0)?;
    let (start, started) = std::sync::mpsc::channel();
    let (finish, finished) = std::sync::mpsc::channel();
    let destination = design.clone();
    let worker = std::thread::spawn(sparksmith_core::library::standards::carried(move || {
        let result = started
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())
            .and_then(|()| {
                publish(&mut publisher, &destination, "interface")
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        let _ = finish.send(result);
    }));
    session.after_interface_read = Some(Box::new(move || {
        let failed =
            |message: String| -> crate::Error { sparksmith_io::Error::Invalid(message).into() };
        start.send(()).map_err(|e| failed(e.to_string()))?;
        finished
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| failed(e.to_string()))?
            .map_err(failed)
    }));
    let before = session.engine().to_file();
    let revision = session.engine().revision();
    let reply = session.dispatch(json!({"command": "interface.publish_preview", "params": {
        "path": design.display().to_string(), "tier": "interface",
        "assembly_path": assembly.display().to_string()
    }}));
    worker.join().map_err(|_| "publisher panicked")?;
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(reply.to_string().contains("interface.changed"), "{reply}");
    assert_eq!(session.engine().to_file(), before);
    assert_eq!(session.engine().revision(), revision);
    // A fresh preview against the new pair succeeds; refusal is retryable.
    ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly.display().to_string()
        }}),
    )?;
    Ok(())
}

/// The publish request bound to `preview`: its token and linked design.
fn bound_publish(
    session: &Session,
    design: &Path,
    assembly_link: &Path,
    tier: &str,
    token: &Value,
) -> Value {
    json!({"command": "interface.publish", "params": {
        "path": design.display().to_string(), "tier": tier,
        "expected_revision": session.engine().revision(),
        "assembly_path": assembly_link.display().to_string(),
        "token": token}})
}

/// The bytes of the published design and its sidecar.
fn publication(design: &Path) -> TestResult<(Vec<u8>, Vec<u8>)> {
    Ok((
        std::fs::read(design)?,
        std::fs::read(sparksmith_io::interface::sidecar_path(design))?,
    ))
}

/// Review finding 12: the publish writes only what its preview showed.
/// With the board linked, the preview dry-runs the linked design and
/// gives a token; changing only the linked design between preview and
/// publish refuses the publish, and neither publication file changes. A
/// publish without the token is refused too; a new preview's token
/// publishes.
#[test]
fn a_publish_is_refused_when_the_linked_design_changed_since_its_preview() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    link(&mut session, &assembly_link, &design)?;
    super::interface_publish_tests::move_j1(&mut session, 2.0)?;
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly_link.display().to_string()}}),
    )?;
    assert!(preview["dry_run"]["effects"].is_array(), "{preview}");
    let token = preview["token"].clone();
    assert!(
        token.as_str().is_some_and(|t| t.starts_with("publish-")),
        "{preview}"
    );
    let before = publication(&design)?;

    // Only the linked design changes: it gains a wall after the preview.
    save_linked_design(&design, &opening, FAR_WALL_Y + 1.0, &assembly_link)?;
    let revision = session.engine().revision();
    let refused = session.dispatch(bound_publish(
        &session,
        &design,
        &assembly_link,
        "interface",
        &token,
    ));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        refused["error"]["rule"], "interface.preview_changed",
        "{refused}"
    );
    assert_eq!(publication(&design)?, before, "nothing written");
    assert_eq!(session.engine().revision(), revision, "nothing recorded");

    // Without a token, a linked board's publish is refused as well.
    let unbound = session.dispatch(json!({"command": "interface.publish", "params": {
        "path": design.display().to_string(), "tier": "interface",
        "expected_revision": revision}}));
    assert_eq!(unbound["ok"], false, "{unbound}");
    assert_eq!(unbound["error"]["rule"], "interface.token", "{unbound}");
    assert_eq!(publication(&design)?, before, "nothing written");

    // A new preview of the changed linked design publishes.
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly_link.display().to_string()}}),
    )?;
    let published = session.dispatch(bound_publish(
        &session,
        &design,
        &assembly_link,
        "interface",
        &preview["token"],
    ));
    assert_eq!(published["ok"], true, "{published}");
    assert_ne!(publication(&design)?, before);
    Ok(())
}

/// The native publication's fit policy is the standalone export's: a
/// linked publication whose bodies do not fit the board plan is refused,
/// naming them and writing nothing, unless written anyway.
#[test]
fn an_unfit_linked_publication_is_refused_unless_written_anyway() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly_link = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "mechanical")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly_link)?;
    link(&mut session, &assembly_link, &design)?;
    super::interface_publish_tests::move_j1(&mut session, 2.0)?;
    // A model built with one track 0.1 mm narrower than the plan reads.
    let project = session.engine().project().clone();
    let unfit = {
        let _context = session.engine().enter_context();
        let model = sparksmith_solidcraft::project::build(&project)?;
        let mut planned = project.clone();
        let track = planned
            .tracks
            .iter_mut()
            .next()
            .ok_or("the example has a track")?;
        track.width += 0.1;
        let plan = sparksmith_solidcraft::project::plan::of(&planned)?;
        sparksmith_solidcraft::project::plan::check(&plan, &model, &mut || Ok(()))?
    };
    assert!(!unfit.differences.is_empty());
    session.interface_check = Some(unfit);

    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "mechanical",
            "assembly_path": assembly_link.display().to_string()}}),
    )?;
    let before = publication(&design)?;
    let refused = session.dispatch(bound_publish(
        &session,
        &design,
        &assembly_link,
        "mechanical",
        &preview["token"],
    ));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "export_unfit", "{refused}");
    assert_eq!(publication(&design)?, before, "nothing written");

    let mut anyway = bound_publish(
        &session,
        &design,
        &assembly_link,
        "mechanical",
        &preview["token"],
    );
    anyway["params"]["write_anyway"] = json!(true);
    let written = session.dispatch(anyway);
    assert_eq!(written["ok"], true, "{written}");
    assert_eq!(
        written["result"]["published"]["check"]["differs"], true,
        "{written}"
    );
    Ok(())
}

/// ECAD/MCAD receipts come only from completed host transactions: applied
/// directly through any transport, each is refused and changes nothing.
#[test]
fn receipts_of_file_transactions_are_refused_when_applied_directly() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let mut session = open(dir.path())?;
    let published = publish(&mut session, &design, "interface")?;
    assert!(published["published"]["revision"].is_u64(), "{published}");
    let record = session
        .engine()
        .project()
        .interface
        .published
        .clone()
        .ok_or("a recorded publish")?;
    let source_id = session
        .engine()
        .project()
        .interface
        .source_id
        .ok_or("a source id")?;
    let mut later = record;
    later.revision += 1;
    let revision = session.engine().revision();
    let forged = serde_json::to_value(Operation::Interface(
        sparksmith_core::interface::InterfaceCommand::RecordPublish {
            source_id,
            record: later,
        },
    ))?;
    for request in [
        json!({"command": "project.apply", "params": {
            "expected_revision": revision, "operation": forged}}),
        json!({"command": "project.apply_all", "params": {
            "expected_revision": revision, "operations": [forged]}}),
    ] {
        let refused = session.dispatch(request);
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["rule"], "receipt.host_only", "{refused}");
        assert_eq!(session.engine().revision(), revision);
    }
    Ok(())
}

/// Review §18 (8dea4ac3 recheck): a concurrent save of the linked design
/// after the native design is staged, before it is put in place, makes the
/// approved publish another one: refused, the published pair untouched.
#[test]
fn a_linked_design_saved_while_publishing_refuses_the_install() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly = dir.path().join("assembly.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly)?;
    link(&mut session, &assembly, &design)?;
    super::interface_publish_tests::move_j1(&mut session, 2.0)?;
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "path": design.display().to_string(), "tier": "interface",
            "assembly_path": assembly.display().to_string()}}),
    )?;
    let before = publication(&design)?;
    let (stage_design, stage_assembly) = (design.clone(), assembly.clone());
    let stage_opening = opening.clone();
    *session
        .after_interface_stage
        .lock()
        .map_err(|_| "hook lock")? = Some(Box::new(move || {
        save_linked_design(
            &stage_design,
            &stage_opening,
            FAR_WALL_Y + 1.0,
            &stage_assembly,
        )
        .map_err(|e| sparksmith_io::Error::Invalid(e.to_string()).into())
    }));
    let request = bound_publish(&session, &design, &assembly, "interface", &preview["token"]);
    let revision = session.engine().revision();
    let reply = session.dispatch(request);
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply.to_string().contains("interface.preview_changed"),
        "{reply}"
    );
    assert_eq!(publication(&design)?, before, "nothing installed");
    assert_eq!(session.engine().revision(), revision);
    Ok(())
}

/// A preview made without a destination approves none: its token does not
/// publish at a path it never saw.
#[test]
fn a_preview_without_a_path_publishes_nowhere() -> TestResult {
    let dir = tempfile::tempdir()?;
    let design = dir.path().join("board.solidcraft");
    let assembly = dir.path().join("assembly.solidcraft");
    let other = dir.path().join("other");
    std::fs::create_dir_all(&other)?;
    let elsewhere = other.join("board.solidcraft");
    let mut session = open(dir.path())?;
    let opening = format!("i{}.opening", instance(&session, "J1")?);
    publish(&mut session, &design, "interface")?;
    save_linked_design(&design, &opening, FAR_WALL_Y, &assembly)?;
    link(&mut session, &assembly, &design)?;
    let preview = ok(
        &mut session,
        json!({"command": "interface.publish_preview", "params": {
            "tier": "interface", "assembly_path": assembly.display().to_string()}}),
    )?;
    assert!(preview["dry_run"].is_null(), "{preview}");
    assert!(
        preview["token"].is_null(),
        "no authorization without a dry run: {preview}"
    );
    let unreviewed = session.dispatch(bound_publish(
        &session,
        &elsewhere,
        &assembly,
        "interface",
        &preview["token"],
    ));
    assert_eq!(unreviewed["ok"], false, "{unreviewed}");
    assert!(!elsewhere.exists());
    let complete = ok(
        &mut session,
        json!({"command":"interface.publish_preview","params":{
            "path":design.display().to_string(),"assembly_path":assembly.display().to_string(),"tier":"interface"
        }}),
    )?;
    assert!(complete["dry_run"]["effects"].is_array());
    assert!(complete["token"].is_string());
    let reply = session.dispatch(bound_publish(
        &session,
        &elsewhere,
        &assembly,
        "interface",
        &complete["token"],
    ));
    assert_eq!(reply["ok"], false, "{reply}");
    assert!(
        reply.to_string().contains("interface.preview_changed"),
        "{reply}"
    );
    assert!(!elsewhere.exists(), "nothing written there");
    Ok(())
}
