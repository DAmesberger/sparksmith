//! Read the same offline help entries as the desktop and the static manual.
use super::Session;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, HelpKind, Index, Inline};
use sparksmith_core::{
    Error,
    rules::{RuleKey, RuleProfile, RuleValue, profile::ProfileSource},
};
use sparksmith_help_search::{Audience, Model, Search};
use sparksmith_io::shipped::ShippedLibrary;
use std::sync::OnceLock;

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    /// A question or words. Empty lists the index.
    pub text: String,
    #[serde(default)]
    pub kinds: Vec<HelpKind>,
    pub limit: Option<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Cause,
    Explanation,
    Body,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TopicParams {
    pub id: HelpId,
    /// Maximum serialized excerpt bytes. Facts are always returned.
    pub budget_bytes: Option<usize>,
    #[serde(default)]
    pub sections: Vec<Section>,
    /// Block offset returned by a preceding page.
    #[serde(default)]
    pub offset: usize,
    /// Set by the front door when a client asks about a particular project.
    #[serde(skip)]
    pub(crate) contextual: bool,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Hit {
    pub id: HelpId,
    pub kind: HelpKind,
    pub title: String,
    pub summary: String,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchReply {
    pub hits: Vec<Hit>,
    pub total: usize,
    /// With no hits: the topics closest to the question, which may not
    /// answer it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub nearest: Vec<Hit>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct ActiveRule {
    pub name: String,
    pub value: RuleValue,
    pub row: String,
    pub source: ProfileSource,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TopicReply {
    pub id: HelpId,
    pub kind: HelpKind,
    pub since: String,
    pub title: String,
    pub summary: String,
    pub facts: Facts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_rule: Option<ActiveRule>,
    pub excerpt: Vec<Block>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<TopicParams>,
    pub related: Vec<HelpId>,
    pub uri: String,
}

/// The embedding model of `shipped`, or `None` without one the search can
/// read: words alone then rank.
fn model(shipped: Option<ShippedLibrary>) -> Option<Model> {
    let files = sparksmith_io::help::model(&shipped?).ok()??;
    Model::read(&files.tokenizer, &files.weights, &files.config).ok()
}

/// The search `find` reads: the shipped help, or the catalog alone where no
/// library is installed. It belongs to no session.
pub(crate) fn catalog_search() -> std::result::Result<&'static Search, String> {
    static SEARCH: OnceLock<std::result::Result<Search, String>> = OnceLock::new();
    let made = SEARCH.get_or_init(|| {
        let shipped = ShippedLibrary::locate().ok();
        let index = shipped
            .as_ref()
            .and_then(|s| sparksmith_io::help::load(s).ok())
            .unwrap_or_default();
        // An entry the installed help does not know is found all the same.
        let index: Index = entries().chain(index).collect();
        Search::new(&index, model(shipped).as_ref())
    });
    made.as_ref().map_err(Clone::clone)
}

/// The catalog's entries as topics.
fn entries() -> impl Iterator<Item = (HelpId, HelpEntry)> {
    super::metadata::entries().iter().filter_map(|e| {
        let id: HelpId = format!("entry/{}", e.name).parse().ok()?;
        let facts = Facts::Entry {
            module: String::new(),
            class: e.class,
            entry_kind: None,
            schema_digest: String::new(),
            examples: Vec::new(),
        };
        let entry = HelpEntry {
            id: id.clone(),
            title: e.title.into(),
            summary: e.summary.clone(),
            since: String::new(),
            facts,
            body: Vec::new(),
            related: Vec::new(),
            keywords: Vec::new(),
        };
        Some((id, entry))
    })
}

impl Session {
    fn help_index(&self) -> Result<&Index> {
        if self.shipped.is_none()
            && let Some(context) = self.engine().context()
        {
            return Ok(context.help()?);
        }
        let index = self.help_index.get_or_init(|| {
            self.shipped()
                .and_then(|s| Ok(sparksmith_io::help::load(&s)?))
                .map_err(|e| e.to_string())
        });
        Ok(index.as_ref().map_err(|why| Error::Invalid(why.clone()))?)
    }
    pub(super) fn search_help(&self, params: &SearchParams) -> Result<Value> {
        if params.text.len() > 4096 {
            return Err(Error::Invalid("Help search text exceeds 4096 bytes".into()).into());
        }
        let index = self.help_index()?;
        let found =
            self.help_search()?
                .search(&params.text, Audience::Agents, &params.kinds, usize::MAX);
        let total = found.hits.len();
        let hit = |id: HelpId| {
            let e = index.get(&id)?;
            Some(Hit {
                kind: id.kind(),
                id,
                title: e.title.clone(),
                summary: e.summary.clone(),
            })
        };
        let limit = params.limit.unwrap_or(total);
        Ok(serde_json::to_value(SearchReply {
            hits: found.hits.into_iter().take(limit).filter_map(hit).collect(),
            total,
            nearest: found.nearest.into_iter().filter_map(hit).collect(),
        })?)
    }

    /// The search over this session's help index, made on first use.
    fn help_search(&self) -> Result<&Search> {
        let index = self.help_index()?;
        let made = self
            .help_search
            .get_or_init(|| Search::new(index, model(self.shipped().ok()).as_ref()));
        Ok(made.as_ref().map_err(|why| Error::Invalid(why.clone()))?)
    }
    pub(super) fn read_help(&self, params: TopicParams) -> Result<Value> {
        let index = self.help_index()?;
        let e = index
            .get(&params.id)
            .ok_or_else(|| Error::NotFound(format!("No help topic {}", params.id)))?;
        let profile = params
            .contextual
            .then(|| self.engine().project().rules.profile.as_ref())
            .flatten();
        let answer = topic(e, params, profile, index)?;
        Ok(serde_json::to_value(answer)?)
    }
}

fn topic(
    e: &HelpEntry,
    mut params: TopicParams,
    profile: Option<&RuleProfile>,
    index: &Index,
) -> Result<TopicReply> {
    let mut facts = e.facts.clone();
    let mut active_rule = None;
    let mut prose = Vec::new();
    let wanted = |s| params.sections.is_empty() || params.sections.contains(&s);
    if let Facts::Finding {
        cause,
        explanation,
        checks,
        rule_key,
        ..
    } = &mut facts
    {
        if wanted(Section::Cause) && !cause.is_empty() {
            prose.push(Block::Paragraph(vec![Inline::Text(cause.clone())]));
        }
        cause.clear();
        if let Some(note) = explanation.take()
            && wanted(Section::Explanation)
        {
            prose.push(Block::Paragraph(vec![Inline::Text(note)]));
        }
        if let Some(key) = rule_key.as_deref().and_then(RuleKey::from_name) {
            *checks = None;
            if let Some(p) = profile
                && let Some((_, row)) = p.active(key)
            {
                let id = format!("rule/{}/{}", p.id, key.name())
                    .parse::<HelpId>()
                    .ok();
                *checks = id.filter(|id| index.contains_key(id));
                active_rule = Some(ActiveRule {
                    name: p.name.clone(),
                    value: key.get(p.rules()),
                    row: row.into(),
                    source: p.source.clone(),
                });
            }
        }
    }
    if wanted(Section::Body) {
        prose.extend_from_slice(&e.body);
    }
    if params.offset > prose.len() {
        return Err(Error::Invalid("Help offset is beyond the last block".into()).into());
    }
    let mut bytes = 2usize; // JSON array delimiters
    let mut excerpt = Vec::new();
    let mut next = None;
    for (i, block) in prose.into_iter().enumerate().skip(params.offset) {
        let encoded = serde_json::to_vec(&block)?.len();
        bytes = bytes
            .saturating_add(usize::from(!excerpt.is_empty()))
            .saturating_add(encoded);
        if let Some(max) = params.budget_bytes
            && bytes > max
        {
            params.offset = i;
            // Offer enough bytes for precisely one whole block when it cannot
            // fit. Following a continuation must never silently remove a budget.
            params.budget_bytes = Some(max.max(encoded.saturating_add(2)));
            next = Some(params);
            break;
        }
        excerpt.push(block);
    }
    Ok(TopicReply {
        id: e.id.clone(),
        kind: e.id.kind(),
        since: e.since.clone(),
        title: e.title.clone(),
        summary: e.summary.clone(),
        facts,
        active_rule,
        excerpt,
        truncated: next.is_some(),
        next,
        related: e.related.clone(),
        uri: format!("sparksmith://help/{}", e.id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_byte_budget_preserves_facts_and_resumes_at_the_first_unread_block()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let entry = HelpEntry {
            id: "guide/example".parse()?,
            title: "Example".into(),
            summary: "Two paragraphs.".into(),
            since: "0.1.0".into(),
            facts: Facts::Guide,
            related: vec![],
            keywords: vec![],
            body: vec![
                Block::Paragraph(vec![Inline::Text("Étape".into())]),
                Block::Note(vec![Inline::Text("最後".into())]),
            ],
        };
        let mut params: TopicParams = serde_json::from_value(serde_json::json!({"id":entry.id}))?;
        let first_block_bytes = serde_json::to_vec(&entry.body[..1])?.len();
        params.budget_bytes = Some(2);
        let short = topic(&entry, params.clone(), None, &Index::new())?;
        assert!(short.excerpt.is_empty());
        let resume = short.next.ok_or("Missing bounded continuation")?;
        assert_eq!(resume.budget_bytes, Some(first_block_bytes));
        assert_eq!(resume.offset, 0);
        params.budget_bytes = Some(first_block_bytes);
        let index = Index::new();
        let first = topic(&entry, params, None, &index)?;
        assert_eq!(first.excerpt, entry.body[..1]);
        assert!(first.truncated);
        let next = first.next.ok_or("Missing continuation")?;
        let tail = topic(&entry, next, None, &index)?;
        assert_eq!(tail.excerpt, entry.body[1..]);
        assert!(!tail.truncated);
        assert_eq!(tail.facts, entry.facts);
        Ok(())
    }
}
