//! The one help search: what people type in Help and the command palette
//! and agents send to `help.search` and `find`.
//!
//! Two ready-made rankings of the same topics, joined by their ranks
//! (reciprocal rank fusion): full text (Tantivy: BM25, the English stemmer
//! and stop words, the last word as a prefix, a misspelled word by edit
//! distance) and meaning (model2vec static embeddings, compared with every
//! topic). Without the embedding model the full text answers alone.
//!
//! Both audiences read the same ranking. The stated difference is
//! [`Audience::weight`]: the kind of topic a person, or an agent, looks for
//! first.
mod text;
mod vectors;

use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, HelpKind, Index, Inline};
use std::collections::HashMap;
pub use vectors::Model;

/// Topics each ranking contributes to the joined one.
const DEPTH: usize = 50;
/// The rank constant of reciprocal rank fusion, as published (Cormack et
/// al., 2009) and commonly used.
const FUSION: f32 = 60.0;
/// Cosine similarity under which a topic found by meaning alone is only
/// "nearest": nothing answers the question.
const FITS: f32 = 0.45;
/// Topics offered when nothing fits.
const NEAREST: usize = 3;
/// The longest question read, in bytes (cut at a character).
const MAX_QUERY: usize = 4096;

/// Who asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Audience {
    /// Guides first: a person looks for what to do.
    People,
    /// Catalog entries first: an agent looks for what to call.
    Agents,
}

impl Audience {
    /// How much the kind of a topic counts for this audience.
    pub fn weight(self, kind: HelpKind) -> f32 {
        match (self, kind) {
            (Self::People, HelpKind::Guide) => 1.5,
            (Self::People, HelpKind::Entry | HelpKind::Rule | HelpKind::Generator) => 0.6,
            (Self::Agents, HelpKind::Entry) => 1.5,
            (Self::Agents, HelpKind::Rule | HelpKind::Generator | HelpKind::Command) => 0.6,
            _ => 1.0,
        }
    }
}

/// What a search found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    /// The topics that answer the question, best first.
    pub hits: Vec<HelpId>,
    /// With no hits, the topics closest to it: they may not answer it.
    pub nearest: Vec<HelpId>,
}

/// A topic as both rankings read it.
struct Topic {
    id: HelpId,
    title: String,
    keywords: String,
    summary: String,
    body: String,
}

/// The search over one help index.
pub struct Search {
    ids: Vec<HelpId>,
    text: text::Text,
    vectors: Option<vectors::Vectors>,
}

impl Search {
    /// Index every topic of `index`. Without a `model` the full text
    /// answers alone.
    pub fn new(index: &Index, model: Option<&Model>) -> Result<Self, String> {
        let topics: Vec<Topic> = index.values().map(topic).collect();
        Ok(Self {
            text: text::Text::new(&topics)?,
            vectors: model.map(|m| m.vectors(&topics)),
            ids: topics.into_iter().map(|t| t.id).collect(),
        })
    }

    /// Whether the embedding model takes part.
    pub fn by_meaning(&self) -> bool {
        self.vectors.is_some()
    }

    /// The topics of `kinds` (any kind when empty) that answer `question`,
    /// best first, at most `limit`. A topic's exact id comes first. An
    /// empty question lists every topic by id.
    pub fn search(
        &self,
        question: &str,
        audience: Audience,
        kinds: &[HelpKind],
        limit: usize,
    ) -> Found {
        let question = clip(question.trim(), MAX_QUERY);
        let wanted = |i: usize| -> bool {
            self.kind(i)
                .is_some_and(|k| kinds.is_empty() || kinds.contains(&k))
        };
        if question.is_empty() {
            let hits = (0..self.ids.len()).filter(|i| wanted(*i));
            return Found {
                hits: self.named(hits.take(limit)),
                nearest: Vec::new(),
            };
        }
        // Both rankings are of the wanted kinds only, so a filter leaves
        // the ranks of what it keeps as they were.
        let text = self.text.ranked(question, DEPTH, &wanted);
        let meaning = self
            .vectors
            .as_ref()
            .map(|v| v.ranked(question, DEPTH, &wanted))
            .unwrap_or_default();
        let fits = !text.is_empty() || meaning.first().is_some_and(|(_, cosine)| *cosine >= FITS);
        let by_meaning = meaning.iter().map(|(i, _)| *i);
        let mut scores: HashMap<usize, f32> = HashMap::new();
        for ranking in [text, by_meaning.collect()] {
            for (rank, i) in ranking.into_iter().enumerate() {
                *scores.entry(i).or_default() += 1.0 / (FUSION + rank as f32);
            }
        }
        let mut joined: Vec<(f32, usize)> = scores
            .into_iter()
            .filter_map(|(i, score)| Some((score * audience.weight(self.kind(i)?), i)))
            .collect();
        joined.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let ranked = joined.into_iter().map(|(_, i)| i);
        if !fits {
            return Found {
                hits: Vec::new(),
                nearest: self.named(ranked.take(NEAREST.min(limit))),
            };
        }
        let exact = self
            .ids
            .iter()
            .position(|id| id.as_str() == question)
            .filter(|i| wanted(*i));
        let rest = ranked.filter(|i| Some(*i) != exact);
        Found {
            hits: self.named(exact.into_iter().chain(rest).take(limit)),
            nearest: Vec::new(),
        }
    }

    fn kind(&self, i: usize) -> Option<HelpKind> {
        self.ids.get(i).map(HelpId::kind)
    }

    fn named(&self, topics: impl Iterator<Item = usize>) -> Vec<HelpId> {
        topics.filter_map(|i| self.ids.get(i).cloned()).collect()
    }
}

fn clip(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn topic(entry: &HelpEntry) -> Topic {
    let mut keywords = entry.keywords.join(" ");
    // The name agents call it by, a word at a time ("component place").
    keywords.push(' ');
    keywords.extend(
        entry
            .id
            .name()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' }),
    );
    let mut body = String::new();
    let mut add = |text: &str| {
        body.push(' ');
        body.push_str(text);
    };
    match &entry.facts {
        Facts::Term {
            definition, avoid, ..
        } => {
            add(definition);
            keywords.push(' ');
            keywords.push_str(&avoid.join(" "));
        }
        Facts::Finding {
            fix,
            cause,
            explanation,
            ..
        } => {
            add(fix);
            add(cause);
            add(explanation.as_deref().unwrap_or_default());
        }
        _ => {}
    }
    for block in &entry.body {
        for inline in inlines(block) {
            if let Inline::Text(text) = inline {
                add(text);
            }
        }
    }
    Topic {
        id: entry.id.clone(),
        title: entry.title.clone(),
        keywords,
        summary: entry.summary.clone(),
        body,
    }
}

fn inlines(block: &Block) -> Vec<&Inline> {
    match block {
        Block::Paragraph(text) | Block::Note(text) => text.iter().collect(),
        Block::List(items) => items.iter().flatten().collect(),
        Block::Table(rows) => rows.iter().flatten().flatten().collect(),
        Block::Image { .. } | Block::Example(_) => Vec::new(),
    }
}
