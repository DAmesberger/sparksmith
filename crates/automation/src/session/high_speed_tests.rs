//! High-speed requests over the JSON-lines transport (the CLI's `stdio`
//! and the MCP tools dispatch the same `project.apply` and
//! `project.query`): pairs and length groups, the impedance calculator
//! and table, the length report and a refused tuning.
use super::*;
use sparksmith_core::Engine;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn query(name: &str, params: Value) -> Value {
    json!({"command": "project.query", "params": {"query": name, "params": params}})
}

#[test]
fn pairs_groups_and_impedance_travel_as_json() -> TestResult {
    let mut session = Session::new(Engine::demo()?);
    let nets: Vec<u64> = session
        .engine()
        .project()
        .nets
        .iter()
        .map(|n| n.id)
        .collect();
    let [a, b, ..] = nets[..] else {
        return Err("the demo has nets".into());
    };
    let revision = session.engine().revision();
    let reply = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Rules", "operation": {"command": "diff_pair.add", "params": {
            "pair": {"name": "D", "positive": a, "negative": b}
        }}}
    }}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["result"]["outcome"]["outcome"], "diff_pair_added");
    let revision = session.engine().revision();
    let reply = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Rules", "operation": {"command": "length_group.add", "params": {
            "group": {"name": "Bus", "pairs": [0], "target": 12.5, "tolerance": 0.25}
        }}}
    }}));
    assert_eq!(reply["ok"], true, "{reply}");
    let report = session.dispatch(query("layout.length_report", json!({})));
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["result"]["result"]["groups"][0]["target"], 12.5);
    let z = session.dispatch(query(
        "impedance.calculate",
        json!({"request": {"on": "structure",
            "structure": {"kind": "microstrip", "h": 1.27, "er": 2.2},
            "t": 0.0001, "width": 3.913}}),
    ));
    assert_eq!(z["ok"], true, "{z}");
    let z0 = z["result"]["result"]["z0"].as_f64().unwrap_or_default();
    assert!((z0 - 50.0).abs() < 0.6, "{z0}");
    let table = session.dispatch(query("impedance.table", json!({})));
    assert_eq!(table["ok"], true, "{table}");
    let suggest = session.dispatch(query("diff_pair.suggest", json!({})));
    assert_eq!(suggest["ok"], true, "{suggest}");
    // A tuning of a track that does not exist is refused and changes
    // nothing.
    let revision = session.engine().revision();
    let refused = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Layout", "operation": {"command": "layout.tune_length", "params": {
            "request": {"track": 99999, "amount": {"by": "add", "mm": 1.0}}
        }}}
    }}));
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(session.engine().revision(), revision);
    Ok(())
}

#[test]
fn suggested_usb_target_can_be_reviewed_then_applied_over_stdio() -> TestResult {
    let mut engine = Engine::demo()?;
    let ids: Vec<_> = engine.project().nets.iter().take(2).map(|n| n.id).collect();
    let [p, n] = ids[..] else {
        return Err("fixture needs two nets".into());
    };
    for (net, name) in [(p, "USB_D+"), (n, "USB_D-")] {
        engine.apply(
            engine.revision(),
            sparksmith_core::Operation::Schematic(
                sparksmith_core::schematic::SchematicCommand::RenameNet {
                    net,
                    name: name.into(),
                },
            ),
        )?;
    }
    let revision = engine.revision();
    let mut session = Session::new(engine);
    let reply = session.dispatch(query("diff_pair.suggest", json!({})));
    let row = &reply["result"]["result"][0];
    assert_eq!(row["positive"], p, "{reply}");
    assert_eq!(row["negative"], n);
    let target = &row["target"];
    assert_eq!(target["ohms"], 90.0);
    assert_eq!(target["tolerance_pct"], 15.0);
    assert!(
        target["source"]
            .as_str()
            .is_some_and(|s| s.contains("§5.3"))
    );
    assert!(session.engine().project().rules.diff_pairs.is_empty());
    let reply = session.dispatch(json!({"command":"project.apply","params":{
        "expected_revision":revision,"operation":{"editor":"Rules","operation":{
            "command":"diff_pair.add","params":{"pair":{
                "name":row["name"],"positive":p,"negative":n,"target":target
            }}
        }}
    }}));
    assert_eq!(reply["ok"], true, "{reply}");
    let saved = &session.engine().project().rules.diff_pairs[0];
    assert_eq!(&serde_json::to_value(&saved.target)?, target);
    Ok(())
}
