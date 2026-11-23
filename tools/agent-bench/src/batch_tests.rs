use super::summary::spread;
use super::*;
#[test]
fn every_treatment_runs_in_both_orders_before_its_comparison_is_reported() {
    let order = RunPlan::for_model("test-model", 2);
    let observed: Vec<_> = order.iter().map(|r| (r.task, r.treatment)).collect();
    assert_eq!(
        observed,
        vec![
            ("two-header", Treatment::Input),
            ("two-header", Treatment::Full),
            ("two-header", Treatment::Full),
            ("two-header", Treatment::Input),
            ("board-build", Treatment::Old),
            ("board-build", Treatment::Shared),
            ("board-build", Treatment::Shared),
            ("board-build", Treatment::Old),
        ]
    );
    assert_eq!(
        order
            .iter()
            .map(|r| r.name())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        8
    );
}
#[test]
fn paired_difference_and_range_use_both_orders() {
    let stats = spread(&[-4.0, 2.0]).unwrap_or_default();
    assert_eq!((stats.low, stats.median, stats.high), (-4.0, -1.0, 2.0));
    assert!(spread(&[]).is_none());
}

#[test]
fn an_interrupted_run_keeps_its_hold_and_unknown_metrics_out_of_paired_differences() -> Result<()> {
    let root = tempfile::tempdir()?;
    let model = "test-model";
    let op: Operator = serde_json::from_value(serde_json::json!({
        "pricing":{"provider":"anthropic","model":model,"limit_cents":100,"rate_source":"local fixture",
            "rates":{"input_nusd":10,"cache_write_nusd":20,"cache_read_nusd":1,"output_nusd":30}},
        "requests":2,"cumulative_output_tokens":10,"cumulative_tokens":20,"seconds":1,"max_output_per_request":3,"effort":"medium"
    }))?;
    let spec = Experiment {
        models: vec![op],
        paired_repeats: 2,
        total_limit_cents: 100,
        old_revision: "12345678".into(),
    };
    let dir = root.path().canonicalize()?;
    let mut plans = RunPlan::for_model(model, 2).into_iter();
    let first = dir.join(plans.next().ok_or("missing first arm")?.name());
    let next = dir.join(plans.next().ok_or("missing second arm")?.name());
    fs::create_dir(&first)?;
    fs::create_dir(&next)?;
    let ledger_path = ledger_path(&dir, model);
    let ledger = Ledger::new(&ledger_path, &spec.models[0].pricing);
    ledger.reserve(&first.to_string_lossy(), 1, 100, 3)?;
    ledger.settle(
        &first.to_string_lossy(),
        1,
        sparksmith_assist::usage::Usage {
            input_tokens: 2,
            output_tokens: 1,
            ..Default::default()
        },
    )?;
    let row = Row::Finish {
        metrics: Default::default(),
        usage: None,
        success: false,
        artifact_verified: Some(false),
        verification_reason: Some("track changed".into()),
        turn_end: Some("answered".into()),
        reason: Some("track changed".into()),
        wall_ms: 10.0,
        revision: Some(2),
        saved_sha256: None,
    };
    fs::write(
        first.join("trace.jsonl"),
        format!("{}\n", serde_json::to_string(&row)?),
    )?;
    ledger.reserve(&next.to_string_lossy(), 1, 100, 3)?;
    fs::write(next.join("trace.jsonl"), br#"{"record":"start"#)?;
    let output = summary::report(&spec, &dir)?;
    assert_eq!(output["confirmed_nusd"], 50);
    assert_eq!(output["held_nusd"], 2090);
    let model = &output["models"][0];
    assert_eq!(model["runs"][0]["status"], "failed");
    assert_eq!(model["runs"][0]["measures"]["input_tokens"], 2.0);
    assert_eq!(model["runs"][1]["status"], "interrupted");
    assert!(model["runs"][1]["measures"].get("input_tokens").is_none());
    assert_eq!(model["runs"][2]["status"], "pending");
    assert_eq!(model["comparisons"][0]["completed_pairs"], 0);
    Ok(())
}
