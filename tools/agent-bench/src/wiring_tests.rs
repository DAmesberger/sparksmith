use super::*;

#[test]
fn one_pins_read_one_intent_check_and_one_atomic_apply_wire_the_saved_subcircuit() -> Result<()> {
    let root = tempfile::tempdir()?;
    let dir = root.path().join("intent");
    let task = Task::load("subcircuit-wiring")?;
    let before = task.session()?.engine().revision();
    let outcome = offline(&task, &dir, None, SchemaDetail::Input, false)?;
    let Row::Finish {
        success,
        revision,
        metrics,
        ..
    } = outcome
    else {
        return Err("missing run outcome".into());
    };
    assert!(success);
    assert_eq!(revision, before.checked_add(1));
    assert_eq!(metrics.refusals, 0);
    let mut calls = Vec::new();
    for line in fs::read_to_string(dir.join("trace.jsonl"))?.lines() {
        if let Row::Call(call) = serde_json::from_str(line)? {
            assert!(call.ok);
            calls.push((call.verb, call.entry));
        }
    }
    assert_eq!(calls.len(), 3);
    assert_eq!(
        (calls[0].0.as_str(), calls[0].1.as_str()),
        ("query", "schematic.pins")
    );
    assert_eq!(
        (calls[1].0.as_str(), calls[1].1.as_str()),
        ("query", "schematic.check_intent")
    );
    assert_eq!(calls[2].0, "apply");

    let initial = sparksmith_io::read_project(&dir.join("start.sparksmith"))?;
    let mut saved = sparksmith_io::read_project(&dir.join("revised.sparksmith"))?;
    assert_eq!(initial.project.instances, saved.project.instances);
    assert_eq!(initial.project.occurrences, saved.project.occurrences);
    assert_eq!(initial.project.placements, saved.project.placements);
    let verify::Expected::Wiring(wanted) = &task.expected else {
        return Err("missing wiring verifier".into());
    };
    assert!(wanted.verify(&initial.project).is_err());
    wanted.verify(&saved.project)?;
    // Swapping anode and cathode labels preserves the pin count and
    // series topology. The verifier must still catch the wrong polarity.
    for net in saved.project.nets.iter_mut() {
        match net.name.as_str() {
            "GND" => net.name = "LED_A".into(),
            "LED_A" => net.name = "GND".into(),
            _ => {}
        }
    }
    assert!(wanted.verify(&saved.project).is_err());
    Ok(())
}
