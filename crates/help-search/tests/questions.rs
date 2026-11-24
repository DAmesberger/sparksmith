//! The help questions (`help-questions.json`): how many the search answers
//! within the first three topics, and the ratchet that keeps it so.
use serde::Deserialize;
use sparksmith_core::help::{HelpId, Index};
use sparksmith_help_search::{Audience, Model, Search};

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[derive(Deserialize)]
struct Questions {
    questions: Vec<Question>,
    nothing_fits: Vec<Nothing>,
}
#[derive(Deserialize)]
struct Question {
    set: String,
    #[serde(rename = "for")]
    audience: String,
    ask: String,
    right: Vec<String>,
}
#[derive(Deserialize)]
struct Nothing {
    set: String,
    ask: String,
}

fn questions() -> Fallible<Questions> {
    Ok(serde_json::from_str(include_str!("help-questions.json"))?)
}

fn library() -> Fallible<sparksmith_io::shipped::ShippedLibrary> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    Ok(sparksmith_io::shipped::ShippedLibrary::at(&root)?)
}

/// `pattern` with at most `*`s: every part in order.
fn right(id: &HelpId, pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = id.as_str();
    for (k, part) in parts.iter().enumerate() {
        let at = if k == 0 {
            rest.starts_with(part).then_some(0)
        } else if k + 1 == parts.len() {
            rest.ends_with(part).then(|| rest.len() - part.len())
        } else {
            rest.find(part)
        };
        let Some(at) = at else { return false };
        rest = &rest[at + part.len()..];
    }
    parts.len() > 1 || rest.is_empty()
}

struct Score {
    asked: usize,
    first: usize,
    three: usize,
    ten: usize,
    empty: usize,
    missed: Vec<String>,
}

fn score(
    all: &Questions,
    set: &str,
    audience: &str,
    mut ask: impl FnMut(&Question) -> Vec<HelpId>,
) -> Score {
    let mut s = Score {
        asked: 0,
        first: 0,
        three: 0,
        ten: 0,
        empty: 0,
        missed: Vec::new(),
    };
    for q in &all.questions {
        if q.set != set || q.audience != audience {
            continue;
        }
        let hits = ask(q);
        let at = hits
            .iter()
            .position(|id| q.right.iter().any(|p| right(id, p)));
        s.asked += 1;
        s.first += usize::from(at == Some(0));
        s.three += usize::from(at.is_some_and(|at| at < 3));
        s.ten += usize::from(at.is_some_and(|at| at < 10));
        s.empty += usize::from(hits.is_empty());
        if at.is_none_or(|at| at >= 3) {
            let top: Vec<&str> = hits.iter().take(4).map(HelpId::as_str).collect();
            s.missed.push(format!("{} ({at:?}) -> {top:?}", q.ask));
        }
    }
    s
}

fn audience(q: &Question) -> Audience {
    if q.audience == "agent" {
        Audience::Agents
    } else {
        Audience::People
    }
}

fn search(search: &Search, q: &Question) -> Vec<HelpId> {
    search.search(&q.ask, audience(q), &[], 50).hits
}

fn before(index: &Index, q: &Question) -> Vec<HelpId> {
    sparksmith_core::help::search(index, &q.ask, &[], 50)
        .into_iter()
        .map(|e| e.id.clone())
        .collect()
}

/// The measurement: `cargo test -p sparksmith-help-search --test questions
/// -- --ignored --nocapture measure`. `SPARKSMITH_HELP_MODEL` names another
/// model folder; `SPARKSMITH_HELP_SET` prints that set's misses.
#[test]
#[ignore = "a measurement, not a check"]
fn measure() -> Fallible {
    let all = questions()?;
    let index = sparksmith_io::help::load(&library()?)?;
    let folder = std::env::var("SPARKSMITH_HELP_MODEL")?;
    let read = |name: &str| std::fs::read(std::path::Path::new(&folder).join(name));
    let at = std::time::Instant::now();
    let model = Model::read(
        &read("tokenizer.json")?,
        &read("model.safetensors")?,
        &read("config.json")?,
    )?;
    println!("model read: {:?}", at.elapsed());
    let at = std::time::Instant::now();
    let text = Search::new(&index, None)?;
    println!("{} topics, text indexed: {:?}", index.len(), at.elapsed());
    let at = std::time::Instant::now();
    let both = Search::new(&index, Some(&model))?;
    println!("text and vectors: {:?}", at.elapsed());
    let at = std::time::Instant::now();
    for q in &all.questions {
        search(&both, q);
    }
    println!(
        "per question: {:?}",
        at.elapsed() / all.questions.len() as u32
    );
    let shown = std::env::var("SPARKSMITH_HELP_SET").unwrap_or_default();
    type Ask<'a> = Box<dyn FnMut(&Question) -> Vec<HelpId> + 'a>;
    for set in ["tune", "held"] {
        for who in ["person", "agent"] {
            let ways: [(&str, Ask<'_>); 3] = [
                ("before", Box::new(|q| before(&index, q))),
                ("text", Box::new(|q| search(&text, q))),
                ("text+meaning", Box::new(|q| search(&both, q))),
            ];
            for (name, ask) in ways {
                let s = score(&all, set, who, ask);
                println!(
                    "{set} {who:6} {name:13} first {:2} three {:2} ten {:2} empty {:2} of {}",
                    s.first, s.three, s.ten, s.empty, s.asked
                );
                if shown == set && name != "before" {
                    s.missed.iter().for_each(|m| println!("    {m}"));
                }
            }
        }
    }
    for n in &all.nothing_fits {
        let found = both.search(&n.ask, Audience::People, &[], 5);
        println!("{} {:?}: {found:?}", n.set, n.ask);
    }
    Ok(())
}
