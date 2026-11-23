use super::*;

#[test]
fn a_selected_wire_is_reported_alongside_its_net() -> Fallible {
    let mut h = Harness::new()?;
    let wire = h.app.engine.project().wires.first().ok_or("no wire")?.id;
    let selected = h.call(
        live::CALL,
        json!({"entry": "view.select", "params": {"ids": [wire]}}),
    )?;
    assert_eq!(selected["result"]["selected"], 1, "{selected}");
    let context = h.call(live::CALL, json!({"entry": "view.context", "params": {}}))?;
    assert_eq!(
        selected["result"]["selection"],
        context["result"]["selection"]
    );
    let shown = context["result"]["selection"]
        .as_array()
        .ok_or("no selection")?;
    assert!(shown.contains(&json!({"kind": "wire", "id": wire})));
    assert!(shown.iter().any(|object| object["kind"] == "net"));
    Ok(())
}

#[test]
fn selecting_two_vias_reports_the_selection_the_window_keeps() -> Fallible {
    let mut h = Harness::new()?;
    h.call(
        live::CALL,
        json!({"entry": "view.open", "params": {"workspace": "pcb", "view": "layout"}}),
    )?;
    let before = h.app.engine.document().clone();
    let revision = h.app.engine.revision();
    let operations = [5, 9].map(|x| {
        let mut params = h.add_via(revision)["params"].clone();
        params["via"]["position"] = json!([x, 4]);
        json!({"editor": "Layout", "operation": {"command": "layout.add_via", "params": params}})
    });
    let added = h.call(
        live::CALL,
        json!({"entry": "project.apply_all", "expected_revision": revision,
            "params": {"operations": operations}}),
    )?;
    assert_eq!(added["ok"], true, "{added}");
    let ids = &added["result"]["changes"]["added"]["vias"];
    assert_eq!(ids.as_array().map(Vec::len), Some(2), "{added}");
    let vias = json!([{"kind": "via", "id": ids[0]}, {"kind": "via", "id": ids[1]}]);
    let selected = h.call(
        live::CALL,
        json!({"entry": "view.select", "params": {"ids": ids}}),
    )?;
    assert_eq!(selected["result"]["selected"], 2, "{selected}");
    assert_eq!(selected["result"]["selection"], vias);
    assert_eq!(selected["result"]["unselected"], json!([]));
    let context = h.call(live::CALL, json!({"entry": "view.context", "params": {}}))?;
    assert_eq!(context["result"]["selection"], vias, "{context}");
    assert_eq!(context["result"]["revision"], revision + 1);
    let selected_ids: Vec<_> = h.app.pcb.selected().vias.into_iter().collect();
    assert_eq!(json!(selected_ids), *ids);

    // A sheet cannot join a board selection. Report exactly which request
    // was left out, even if the same via was requested twice.
    let sheet = json!({"kind": "sheet", "sheet": h.app.engine.project().sheets.first()});
    let partial = h.call(
        live::CALL,
        json!({"entry": "view.select", "params": {"ids": ids, "objects": [vias[0], sheet]}}),
    )?;
    assert_eq!(partial["result"]["selected"], 2, "{partial}");
    assert_eq!(partial["result"]["selection"], vias);
    assert_eq!(partial["result"]["unselected"], json!([sheet]));
    h.app.history(false);
    assert_eq!(h.app.engine.document(), &before);
    Ok(())
}
