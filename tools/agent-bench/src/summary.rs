//! Aggregate all planned arms, including interrupted or failed attempts.
//! Charges come from persisted provider receipts. Paired differences use
//! the same model, fixture and repetition in both treatments.
use super::{Experiment, RunPlan, Treatment, ledger_path};
use crate::{
    Result,
    spend::{Balance, Ledger},
    trace::{Metrics, Row},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader},
    path::Path,
};

#[derive(Serialize, Default)]
pub(super) struct Spread {
    pub(super) low: f64,
    pub(super) median: f64,
    pub(super) high: f64,
}
pub(super) fn spread(values: &[f64]) -> Option<Spread> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    };
    Some(Spread {
        low: sorted[0],
        median,
        high: *sorted.last()?,
    })
}

type Measures = BTreeMap<&'static str, f64>;

#[derive(Serialize, Default)]
struct Observed {
    finished: bool,
    artifact_verified: Option<bool>,
    verification_reason: Option<String>,
    turn_end: Option<String>,
    success: Option<bool>,
    reason: Option<String>,
    revision: Option<u64>,
    saved_sha256: Option<String>,
    metrics: Option<Metrics>,
    wall_ms: Option<f64>,
    request_frame_bytes: u64,
    reply_frame_bytes: u64,
    observed_tool_calls: u64,
    observed_model_attempts: u64,
    observed_model_receipts: u32,
}

impl Observed {
    fn read(dir: &Path) -> Result<Self> {
        let mut o = Self::default();
        let path = dir.join("trace.jsonl");
        if path.try_exists()? {
            let mut reader = BufReader::new(File::open(path)?);
            let mut line = Vec::new();
            loop {
                line.clear();
                if reader.read_until(b'\n', &mut line)? == 0 || !line.ends_with(b"\n") {
                    break;
                }
                match serde_json::from_slice(&line)? {
                    Row::Call(call) => {
                        o.observed_tool_calls += 1;
                        o.request_frame_bytes += call.request_bytes as u64;
                        o.reply_frame_bytes += call.reply_bytes as u64;
                    }
                    Row::Model { .. } => {
                        o.observed_model_attempts += 1;
                        o.observed_model_receipts += 1;
                    }
                    Row::ModelError { .. } => o.observed_model_attempts += 1,
                    Row::Finish {
                        success,
                        artifact_verified,
                        verification_reason,
                        turn_end,
                        reason,
                        revision,
                        saved_sha256,
                        metrics,
                        wall_ms,
                        ..
                    } => {
                        o.finished = true;
                        o.artifact_verified = artifact_verified;
                        o.verification_reason = verification_reason;
                        o.turn_end = turn_end;
                        o.success = Some(success);
                        o.reason = reason;
                        o.revision = revision;
                        o.saved_sha256 = saved_sha256;
                        o.metrics = Some(metrics);
                        o.wall_ms = Some(wall_ms);
                    }
                    Row::Start { .. } => (),
                }
            }
        }
        if !o.finished && dir.join("failure.txt").try_exists()? {
            o.reason = Some(fs::read_to_string(dir.join("failure.txt"))?);
            o.success = Some(false);
        }
        Ok(o)
    }
}

#[derive(Serialize)]
struct Finding {
    plan: RunPlan,
    status: &'static str,
    observed: Observed,
    ledger: Balance,
    measures: Measures,
}
impl Finding {
    fn read(dir: &Path, plan: RunPlan, balance: Balance) -> Result<Self> {
        let observed = Observed::read(dir)?;
        let exists = dir.try_exists()?;
        if observed.observed_model_receipts > balance.confirmed_requests {
            return Err(
                "a recorded model reply is missing its confirmed cost in the ledger".into(),
            );
        }
        let status = match observed.success {
            Some(true) => "verified",
            Some(false) => "failed",
            None if exists || balance.attempts != 0 => "interrupted",
            None => "pending",
        };
        let mut measures = Measures::new();
        if status != "pending" {
            measures.insert("confirmed_usd", balance.confirmed_nusd as f64 / 1e9);
            measures.insert("held_usd", balance.held_nusd as f64 / 1e9);
        }
        if balance.confirmed_requests > 0 && balance.held_nusd == 0 {
            let u = balance.confirmed_usage;
            measures.extend([
                ("input_tokens", u.input_tokens as f64),
                ("cache_write_tokens", u.cache_write_tokens as f64),
                ("cache_read_tokens", u.cache_read_tokens as f64),
                ("output_tokens", u.output_tokens as f64),
            ]);
        }
        if let Some(wall) = observed.wall_ms {
            measures.insert("wall_ms", wall);
        }
        if let Some(m) = &observed.metrics {
            measures.extend([
                ("model_requests", m.model_requests as f64),
                ("model_request_bytes", m.model_request_bytes as f64),
                ("estimated_prompt_tokens", m.estimated_prompt_tokens as f64),
                ("estimated_output_tokens", m.estimated_output_tokens as f64),
                ("visible_reply_bytes", m.visible_reply_bytes as f64),
                ("server_ms", m.server_ms),
                ("model_ms", m.model_ms),
                ("tool_calls", m.calls.values().sum::<usize>() as f64),
                ("retries", m.retries as f64),
                ("refusals", m.refusals as f64),
                ("request_frame_bytes", observed.request_frame_bytes as f64),
                ("reply_frame_bytes", observed.reply_frame_bytes as f64),
            ]);
        }
        Ok(Self {
            plan,
            status,
            observed,
            ledger: balance,
            measures,
        })
    }
}
fn distributions<'a>(items: impl IntoIterator<Item = &'a Measures>) -> Value {
    let mut groups: BTreeMap<_, Vec<f64>> = BTreeMap::new();
    for m in items {
        for (key, value) in m {
            groups.entry(*key).or_default().push(*value);
        }
    }
    let summary: BTreeMap<_, _> = groups
        .into_iter()
        .map(|(key, values)| (key, json!({"n":values.len(),"range":spread(&values)})))
        .collect();
    json!(summary)
}
fn comparison(rows: &[Finding], task: &str, arms: [Treatment; 2]) -> Value {
    let group: Vec<_> = rows.iter().filter(|r| r.plan.task == task).collect();
    let mut by_arm = BTreeMap::new();
    for arm in arms {
        let arm_rows: Vec<_> = group
            .iter()
            .copied()
            .filter(|r| r.plan.treatment == arm)
            .collect();
        let counts = arm_rows
            .iter()
            .fold(BTreeMap::<&str, usize>::new(), |mut counts, row| {
                *counts.entry(row.status).or_default() += 1;
                counts
            });
        by_arm.insert(arm.label(),json!({"status_counts":counts,"measures":distributions(arm_rows.iter().map(|r|&r.measures))}));
    }
    let mut matched = Vec::new();
    let mut paired = Vec::new();
    for a in group
        .iter()
        .copied()
        .filter(|r| r.plan.treatment == arms[0])
    {
        if let Some(b) = group
            .iter()
            .copied()
            .find(|b| b.plan.pair == a.plan.pair && b.plan.treatment == arms[1])
        {
            let mut deltas = Measures::new();
            if a.observed.finished && b.observed.finished {
                for (key, value) in &a.measures {
                    if let Some(other) = b.measures.get(key) {
                        deltas.insert(*key, value - other);
                    }
                }
                matched.push(deltas.clone());
            }
            paired.push(
                json!({"pair":a.plan.pair,"first_arm":if a.plan.position==1{arms[0]}else{arms[1]},
                "status":[a.status,b.status],"delta":deltas}),
            );
        }
    }
    json!({"task":task,"delta_direction":format!("{} minus {}",arms[0].label(),arms[1].label()),
        "arms":by_arm,"pairs":paired,"completed_pairs":matched.len(),"paired_measures":distributions(&matched)})
}

pub(super) fn report(spec: &Experiment, root: &Path) -> Result<Value> {
    let mut models = Vec::new();
    let mut confirmed_nusd = 0u64;
    let mut held_nusd = 0u64;
    for op in &spec.models {
        let path = ledger_path(root, &op.pricing.model);
        let mut balances = if path.try_exists()? {
            Ledger::new(&path, &op.pricing).balances()?
        } else {
            BTreeMap::new()
        };
        let mut rows = Vec::new();
        for plan in RunPlan::for_model(&op.pricing.model, spec.paired_repeats) {
            let dir = root.join(plan.name());
            let balance = balances
                .remove(dir.to_string_lossy().as_ref())
                .unwrap_or_default();
            confirmed_nusd = confirmed_nusd
                .checked_add(balance.confirmed_nusd)
                .ok_or("batch charge overflow")?;
            held_nusd = held_nusd
                .checked_add(balance.held_nusd)
                .ok_or("batch hold overflow")?;
            rows.push(Finding::read(&dir, plan, balance)?);
        }
        if !balances.is_empty() {
            return Err("ledger contains a run outside the recorded plan".into());
        }
        let comparisons = [
            comparison(&rows, "two-header", [Treatment::Input, Treatment::Full]),
            comparison(&rows, "board-build", [Treatment::Shared, Treatment::Old]),
        ];
        models.push(json!({"model":op.pricing.model,"runs":rows,"comparisons":comparisons}));
    }
    Ok(
        json!({"models":models,"confirmed_usd":confirmed_nusd as f64/1e9,"held_usd":held_nusd as f64/1e9,
       "confirmed_nusd":confirmed_nusd,"held_nusd":held_nusd}),
    )
}
