//! `design-body`: give a placed part of an existing project its 3D body
//! from datasheets with a configured model. It runs the body workflow
//! (`library/assist/workflows/datasheet-to-body`) headless through the one
//! runner the chat uses, on the project opened in an in-process session:
//! the fitted body is applied without asking and the model's generator
//! choice stands. The footprint is kept. The result is saved as a new
//! project at `--out` with the review report beside it (`.review.json`,
//! `.review.md`); a run that makes no body writes nothing and says why.
//! The model flags, `--record` and `--replay` are `design-part`'s.
use super::Result;
use super::design_part::{Console, ModelFlags, installed_workflow};
use serde_json::Value;
use sparksmith_assist::Datasheet;
use sparksmith_assist::model::record::{Recording, Replay};
use sparksmith_assist::model::{self, ModelBackend};
use sparksmith_assist::workflow::event::RunEnd;
use sparksmith_assist::workflow::runner::{self, WorkflowRun};
use sparksmith_core::product::SHORT_NAME as CLI;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

pub const USAGE: &str =
    "design-body --project FILE.sparksmith --designator REF --datasheet PDF [--datasheet …]
              [--photo URL|FILE]… [--notes TEXT] --out NEW.sparksmith [model flags as design-part]
                                   give a placed part a generated 3D body fitted to its footprint
                                   (the body workflow, headless); every part sharing its package
                                   changes; writes the project and its .review.md/.review.json";

/// The workflow `design-body` runs.
const WORKFLOW: &str = "datasheet-to-body";

struct Args {
    project: String,
    designator: String,
    datasheets: Vec<String>,
    photos: Vec<String>,
    notes: String,
    out: PathBuf,
    model: ModelFlags,
}

fn parse(args: &[String]) -> Result<Args> {
    let mut parsed = Args {
        project: String::new(),
        designator: String::new(),
        datasheets: Vec::new(),
        photos: Vec::new(),
        notes: String::new(),
        out: PathBuf::new(),
        model: ModelFlags::default(),
    };
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value\nusage: {CLI} {USAGE}"))?
            .clone();
        match flag.as_str() {
            "--project" => parsed.project = value,
            "--designator" => parsed.designator = value,
            "--datasheet" => parsed.datasheets.push(value),
            "--photo" => parsed.photos.push(value),
            "--notes" => parsed.notes = value,
            "--out" => parsed.out = PathBuf::from(value),
            other => {
                if !parsed.model.take(other, value)? {
                    return Err(format!("unknown option {other}\nusage: {CLI} {USAGE}").into());
                }
            }
        }
    }
    if parsed.project.is_empty()
        || parsed.designator.trim().is_empty()
        || parsed.datasheets.is_empty()
        || parsed.out.as_os_str().is_empty()
    {
        return Err(format!("usage: {CLI} {USAGE}").into());
    }
    Ok(parsed)
}

/// The workflow's inputs from the flags.
fn inputs(args: &Args) -> serde_json::Map<String, Value> {
    let mut inputs = serde_json::Map::new();
    let sheets = args
        .datasheets
        .iter()
        .map(|sheet| match Datasheet::parse(sheet) {
            Datasheet::File(path) => Value::String(path.display().to_string()),
            Datasheet::Url(url) => Value::String(url),
        })
        .collect();
    inputs.insert("datasheets".into(), Value::Array(sheets));
    inputs.insert("part".into(), Value::String(args.designator.clone()));
    if !args.photos.is_empty() {
        inputs.insert("photos".into(), Value::String(args.photos.join("\n")));
    }
    if !args.notes.trim().is_empty() {
        inputs.insert("notes".into(), Value::String(args.notes.clone()));
    }
    inputs
}

/// `design-body`.
pub fn run(args: &[String]) -> Result<()> {
    let args = parse(args)?;
    for path in std::iter::once(args.out.clone()).chain(sparksmith_assist::review_paths(&args.out))
    {
        if path.try_exists()? {
            return Err(format!("{} exists; choose a new output name", path.display()).into());
        }
    }
    let (record, replay) = (args.model.record.clone(), args.model.replay.clone());
    let replay = replay.as_deref().map(Replay::from_file).transpose()?;
    if replay.as_ref().is_some_and(|r| r.workflow.is_none()) {
        return Err("design-body replays only a recording of its workflow".into());
    }
    let inputs = inputs(&args);
    let (config, connection) = args.model.configure()?;
    let loaded = installed_workflow(WORKFLOW)?;
    let mut backend: Box<dyn ModelBackend> = match (replay, &record) {
        (Some(replay), _) => Box::new(replay),
        (None, Some(path)) => Box::new(Recording::for_workflow(
            model::connect(&config.profile, &connection)?,
            path.clone(),
            runner::mark(&loaded),
        )?),
        (None, None) => model::connect(&config.profile, &connection)?,
    };
    let mut session = super::open(Some(&args.project))?;
    let run = WorkflowRun {
        loaded: &loaded,
        inputs,
        grant: config.reach.grant(),
        config: &config,
    };
    let outcome = runner::run_workflow(
        run,
        backend.as_mut(),
        &mut session,
        &mut Console,
        &AtomicBool::new(false),
    )?;
    let review = outcome.review(&session);
    let facts = &review.facts;
    let cost = format!(
        "{} requests, {} input tokens (cache {} written, {} read), {} output tokens, {:.0} s",
        facts.requests,
        facts.input_tokens,
        facts.cache_write_tokens,
        facts.cache_read_tokens,
        facts.output_tokens,
        facts.elapsed_seconds
    );
    if !outcome.results.contains_key("apply") {
        return Err(format!(
            "no body was made ({cost}): {}",
            match &outcome.end {
                RunEnd::Done => "the run ended without applying one".to_owned(),
                other => format!("{other:?}"),
            }
        )
        .into());
    }
    let [_, written] = sparksmith_assist::write_outputs(&mut session, &args.out, &review)?;
    eprintln!(
        "{}: {cost}; saved to {} (review {})",
        review.stop,
        args.out.display(),
        written.display()
    );
    if review.report.is_none() {
        return Err(format!("no accepted review report: {}", review.stop).into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_flags_name_the_part_and_its_sources() -> Result<()> {
        let parsed = parse(&args(
            "--project p.sparksmith --designator J16 --datasheet a.pdf --photo https://x/y.jpg \
             --photo side.png --out q.sparksmith --max-requests 30",
        ))?;
        assert_eq!(parsed.designator, "J16");
        let inputs = inputs(&parsed);
        assert_eq!(inputs["part"], "J16");
        assert_eq!(inputs["photos"], "https://x/y.jpg\nside.png");
        assert_eq!(inputs["datasheets"][0], "a.pdf");
        for bad in [
            "--designator J16 --datasheet a.pdf --out q.sparksmith",
            "--project p.sparksmith --datasheet a.pdf --out q.sparksmith",
            "--project p.sparksmith --designator J16 --out q.sparksmith",
            "--project p.sparksmith --designator J16 --datasheet a.pdf",
            "--project p.sparksmith --designator J16 --datasheet a.pdf --out q --frobnicate 1",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
        Ok(())
    }
}
