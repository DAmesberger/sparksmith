//! The help questions (`help-questions.json`): how many the search answers
//! within the first three topics, and the ratchet that keeps it so.
use serde::Deserialize;
use sparksmith_core::help::HelpId;
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
    let root = match std::env::var_os("SPARKSMITH_HELP_LIBRARY") {
        Some(folder) => folder.into(),
        None => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library"),
    };
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

/// The measurement: `cargo test -p sparksmith-automation --test help_questions
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
            let ways: [(&str, Ask<'_>); 2] = [
                ("text", Box::new(|q| search(&text, q))),
                ("text+meaning", Box::new(|q| search(&both, q))),
            ];
            for (name, ask) in ways {
                let s = score(&all, set, who, ask);
                println!(
                    "{set} {who:6} {name:13} first {:2} three {:2} ten {:2} empty {:2} of {}",
                    s.first, s.three, s.ten, s.empty, s.asked
                );
                if shown == set {
                    s.missed.iter().for_each(|m| println!("    {m}"));
                }
            }
        }
    }
    for n in &all.nothing_fits {
        let found = both.search(&n.ask, Audience::People, &[], 5);
        if n.set == shown {
            let close = both.closest(&n.ask);
            println!("{} {:?} {close:?}: {found:?}", n.set, n.ask);
        }
    }
    let mut near: Vec<_> = all
        .questions
        .iter()
        .filter(|q| q.set == shown)
        .filter_map(|q| Some((both.closest(&q.ask)?, &q.ask)))
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!(
        "least similar: {:?}",
        near.iter().take(12).collect::<Vec<_>>()
    );
    Ok(())
}

/// The ratchet: questions answered within the first three topics, per set
/// and audience, with the shipped model and by words alone. Raise a number
/// when the search or the topics get better; a lower one is a regression.
#[test]
fn the_search_answers_the_questions_in_its_first_three_topics() -> Fallible {
    let all = questions()?;
    let shipped = library()?;
    let index = sparksmith_io::help::load(&shipped)?;
    let files = sparksmith_io::help::model(&shipped)?.ok_or("the shipped model")?;
    let model = Model::read(&files.tokenizer, &files.weights, &files.config)?;
    let both = Search::new(&index, Some(&model))?;
    let text = Search::new(&index, None)?;
    assert!(both.by_meaning() && !text.by_meaning());
    // (set, audience, with the model, by words alone)
    for (set, who, with, alone) in [
        ("tune", "person", 46, 43),
        ("held", "person", 51, 45),
        ("tune", "agent", 7, 7),
        ("held", "agent", 6, 5),
    ] {
        for (by, least) in [(&both, with), (&text, alone)] {
            let s = score(&all, set, who, |q| search(by, q));
            assert!(
                s.three >= least,
                "{set} {who}, model {}: {} of {}, at least {least}; missed {:#?}",
                by.by_meaning(),
                s.three,
                s.asked,
                s.missed
            );
        }
    }
    Ok(())
}

/// A question nothing answers gets no hits: the nearest topics are offered
/// as such.
#[test]
fn a_question_no_topic_answers_says_so() -> Fallible {
    let all = questions()?;
    let shipped = library()?;
    let index = sparksmith_io::help::load(&shipped)?;
    let files = sparksmith_io::help::model(&shipped)?.ok_or("the shipped model")?;
    let model = Model::read(&files.tokenizer, &files.weights, &files.config)?;
    let search = Search::new(&index, Some(&model))?;
    let mut answered = Vec::new();
    for n in &all.nothing_fits {
        let found = search.search(&n.ask, Audience::People, &[], 10);
        if !found.hits.is_empty() {
            answered.push((n.set.as_str(), n.ask.as_str()));
        }
    }
    // Two made-up words are still taken for an answer: a known miss of the
    // set that is only measured.
    assert!(
        answered.iter().all(|a| *a == ("held", "qwxv plmk")),
        "{answered:?}"
    );
    Ok(())
}

/// People and agents read one ranking; the stated difference is the kind
/// each looks for first. An exact id comes first, an empty question lists
/// everything, and a kind filter keeps the order of what it keeps.
#[test]
fn the_audience_changes_only_the_weight_of_a_kind() -> Fallible {
    let shipped = library()?;
    let index = sparksmith_io::help::load(&shipped)?;
    let search = Search::new(&index, None)?;
    use sparksmith_core::help::HelpKind;
    let ask = "connect two pins";
    let place = |who, kind| {
        let hits = search.search(ask, who, &[], usize::MAX).hits;
        hits.iter().position(|id| id.kind() == kind)
    };
    let (people, agents) = (Audience::People, Audience::Agents);
    assert!(place(people, HelpKind::Guide) < place(agents, HelpKind::Guide));
    assert!(place(agents, HelpKind::Entry) < place(people, HelpKind::Entry));
    let first = |ask: &str, who| search.search(ask, who, &[], 5).hits.first().cloned();
    let exact = first("entry/project.save", Audience::People);
    assert_eq!(
        exact.as_ref().map(HelpId::as_str),
        Some("entry/project.save")
    );
    let all = search.search("", Audience::People, &[], usize::MAX).hits;
    assert_eq!(all.len(), index.len());
    let only = search.search(ask, Audience::Agents, &[HelpKind::Entry], 5);
    assert!(only.hits.iter().all(|id| id.kind() == HelpKind::Entry));
    assert!(!only.hits.is_empty());
    Ok(())
}
