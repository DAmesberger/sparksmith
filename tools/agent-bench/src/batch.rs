//! Plan, run and summarize counterbalanced comparisons on fresh project copies.
use crate::{Result, Task, external, live::Operator, run, spend::Ledger, trace::Row};
use serde::{Deserialize, Serialize};
use sparksmith_automation::session::SchemaDetail;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Experiment {
    models: Vec<Operator>,
    paired_repeats: u32,
    total_limit_cents: u64,
    old_revision: String,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Treatment {
    Input,
    Full,
    Old,
    Shared,
}
impl Treatment {
    fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Full => "full",
            Self::Old => "old",
            Self::Shared => "shared",
        }
    }
    fn detail(self) -> SchemaDetail {
        if self == Self::Full {
            SchemaDetail::Full
        } else {
            SchemaDetail::Input
        }
    }
}

#[derive(Serialize, Clone)]
struct RunPlan {
    model: String,
    task: &'static str,
    treatment: Treatment,
    pair: u32,
    position: u8,
}
impl RunPlan {
    fn for_model(model: &str, repeats: u32) -> Vec<Self> {
        let mut rows = Vec::new();
        for (task, arms) in [
            ("two-header", [Treatment::Input, Treatment::Full]),
            ("board-build", [Treatment::Old, Treatment::Shared]),
        ] {
            for pair in 1..=repeats {
                let order = if pair % 2 == 1 {
                    arms
                } else {
                    [arms[1], arms[0]]
                };
                for (position, treatment) in [1, 2].into_iter().zip(order) {
                    rows.push(Self {
                        model: model.into(),
                        task,
                        treatment,
                        pair,
                        position,
                    });
                }
            }
        }
        rows
    }
    fn name(&self) -> String {
        format!(
            "{}-{}-pair{}-{}-{}",
            self.model,
            self.task,
            self.pair,
            self.position,
            self.treatment.label()
        )
    }
}

fn load(path: &Path) -> Result<Experiment> {
    let spec: Experiment = serde_json::from_slice(&fs::read(path)?)?;
    if spec.paired_repeats < 2 || spec.models.is_empty() {
        return Err("supply a model and at least two counterbalanced pairs".into());
    }
    if !(spec.old_revision.len() >= 8
        && spec.old_revision.len() <= 40
        && spec.old_revision.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("supply the approved pre-method Git commit".into());
    }
    if spec.paired_repeats > 64 {
        return Err("at most 64 pairs per treatment are supported".into());
    }
    let mut models = BTreeSet::new();
    let mut total = 0u64;
    for op in &spec.models {
        let b = &op.pricing;
        if b.provider != "anthropic"
            || b.model.is_empty()
            || !b
                .model
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            || !models.insert(&b.model)
        {
            return Err("each operator arm needs a distinct Anthropic model identifier".into());
        }
        total = total.checked_add(b.limit_cents).ok_or("budget overflow")?;
    }
    if total == 0 || total > spec.total_limit_cents {
        return Err("model allocations exceed the authorized batch total, or are empty".into());
    }
    Ok(spec)
}

// Both historical files are shipped instruction data at the operator's
// selected revision. Only these two fixed paths may supply the old prompt.
fn old_prompt(revision: &str) -> Result<String> {
    fn read(rev: &str, path: &str) -> Result<String> {
        let cmd = std::process::Command::new("git")
            .arg("show")
            .arg(format!("{rev}:library/assist/{path}"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env_remove("ANTHROPIC_API_KEY")
            .output()?;
        if !cmd.status.success() {
            return Err(format!("couldn't load {path} from {rev}").into());
        }
        Ok(String::from_utf8(cmd.stdout)?.trim_end().to_owned())
    }
    Ok(format!(
        "{}\n\n{}",
        read(revision, "chat-prompt.md")?,
        read(revision, "chat-playbook.md")?
    )
    .replace("{product}", sparksmith_core::product_name!()))
}

struct Prompts {
    old: String,
    unattended: String,
    external: String,
}
impl Prompts {
    fn load(revision: &str) -> Result<Self> {
        let data = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/agent-tasks/board-build/unattended.txt"),
        )?;
        let external = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/agent-tasks/two-header/external.txt"),
        )?;
        Ok(Self {
            external: external.trim_end().to_owned(),
            old: old_prompt(revision)?,
            unattended: data.trim_end().to_owned(),
        })
    }
}

fn plans(spec: &Experiment) -> impl Iterator<Item = RunPlan> + '_ {
    spec.models
        .iter()
        .flat_map(|op| RunPlan::for_model(&op.pricing.model, spec.paired_repeats))
}
fn ledger_path(dir: &Path, model: &str) -> PathBuf {
    dir.join(format!("{model}-charges.jsonl"))
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let temp = path.with_extension("json.partial");
    let mut f = File::create(&temp)?;
    serde_json::to_writer_pretty(&mut f, value)?;
    writeln!(f)?;
    f.sync_all()?;
    fs::rename(temp, path)?;
    Ok(())
}

fn manifest(spec: &Experiment, prompts: &Prompts) -> Result<serde_json::Value> {
    let source = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env_remove("ANTHROPIC_API_KEY")
        .output()?;
    if !source.status.success() {
        return Err("could not identify benchmark checkout".into());
    }
    let shared = sparksmith_assist::chat::ChatLibrary::locate()?.system;
    let tasks = [
        Task::load("board-build")?.mark,
        Task::load("two-header")?.mark,
    ];
    Ok(
        serde_json::json!({"format":1,"experiment":spec,"revision":String::from_utf8(source.stdout)?.trim(),
          "tasks":tasks,"runs":plans(spec).collect::<Vec<_>>(),"board_instruction":prompts.unattended,"external_instruction":prompts.external,
          "old_prompt_sha256":crate::trace::digest(prompts.old.as_bytes()),"shared_prompt_sha256":crate::trace::digest(shared.as_bytes())}),
    )
}

fn exec_run(
    task: &Task,
    dir: &Path,
    ledger: &Path,
    cli: &Path,
    op: &Operator,
    row: &RunPlan,
    prompts: &Prompts,
) -> Result<Row> {
    let detail = row.treatment.detail();
    let arm = op.clone().arm(dir, ledger, detail)?;
    match row.treatment {
        Treatment::Input | Treatment::Full => external::run(task, dir, cli, arm, &prompts.external),
        Treatment::Old | Treatment::Shared => {
            let goal = format!("{}\n\n{}", task.goal.trim_end(), prompts.unattended);
            let system = (row.treatment == Treatment::Old).then_some(prompts.old.as_str());
            run::run_with_prompt(task, dir, arm, false, system, Some(&goal))
        }
    }
}

pub fn execute(operator: &Path, out: &Path, cli: &Path) -> Result<()> {
    let spec = load(operator)?;
    let prompts = Prompts::load(&spec.old_revision)?;
    let manifest = manifest(&spec, &prompts)?;
    if !out.try_exists()? {
        fs::create_dir(out)?;
    }
    let root = out.canonicalize()?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("batch.lock"))?;
    lock.try_lock()
        .map_err(|e| format!("batch already running or lock unavailable: {e}"))?;
    let mark = root.join("manifest.json");
    if mark.try_exists()? {
        let prior: serde_json::Value = serde_json::from_slice(&fs::read(&mark)?)?;
        if prior != manifest {
            return Err("existing batch has a different plan, fixture, prompt or revision".into());
        }
    } else {
        if fs::read_dir(&root)?.count() != 1 {
            return Err("choose a new empty batch folder".into());
        }
        save(&mark, &manifest)?;
    }
    // A resumed batch cannot trust a missing ledger for an existing run.
    for op in &spec.models {
        let ledger = ledger_path(&root, &op.pricing.model);
        for row in RunPlan::for_model(&op.pricing.model, spec.paired_repeats) {
            if root.join(row.name()).try_exists()?
                && (!ledger.try_exists()? || ledger.metadata()?.len() == 0)
            {
                return Err("a previous run has lost its charge ledger".into());
            }
        }
    }
    report(&spec, &root)?;
    let binary = cli.canonicalize()?;
    for op in &spec.models {
        let ledger = ledger_path(&root, &op.pricing.model);
        for row in RunPlan::for_model(&op.pricing.model, spec.paired_repeats) {
            let dir = root.join(row.name());
            if dir.try_exists()? {
                continue;
            }
            let store = Ledger::new(&ledger, &op.pricing);
            let context = sparksmith_assist::model::ModelProfile::anthropic(&op.pricing.model)
                .capabilities
                .context_tokens;
            if !store.can_start(context, op.max_output_per_request)? {
                break;
            }
            let task = Task::load(row.task)?;
            if let Err(error) = exec_run(&task, &dir, &ledger, &binary, op, &row, &prompts) {
                // Keep the run and its confirmed usage after failed verification.
                // A missing directory means the connection never started.
                if !dir.try_exists()? {
                    return Err(error);
                }
                fs::write(dir.join("failure.txt"), error.to_string())?;
            }
            let findings = report(&spec, &root)?;
            save(&root.join("report.json"), &findings)?;
        }
    }
    save(&root.join("report.json"), &report(&spec, &root)?)?;
    Ok(())
}

#[path = "summary.rs"]
mod summary;
use summary::report;

pub fn print_report(operator: &Path, root: &Path) -> Result<()> {
    let spec = load(operator)?;
    let root = root.canonicalize()?;
    let mark: serde_json::Value = serde_json::from_slice(&fs::read(root.join("manifest.json"))?)?;
    if mark["experiment"] != serde_json::to_value(&spec)? {
        return Err("operator file differs from the recorded plan".into());
    }
    println!("{}", serde_json::to_string_pretty(&report(&spec, &root)?)?);
    Ok(())
}

pub fn plan(operator: &Path) -> Result<()> {
    let spec = load(operator)?;
    let prompts = Prompts::load(&spec.old_revision)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&manifest(&spec, &prompts)?)?
    );
    Ok(())
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
