//! Read the same offline help entries as the desktop and the static manual.
use super::Session;
use crate::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sparksmith_core::{Error, rules::{RuleKey, RuleProfile, RuleValue, profile::ProfileSource}};
use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, HelpKind, Index, Inline, search};

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchParams {
    /// Words in the title, description, id or keywords. Empty lists the index.
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
    #[serde(skip_serializing_if="Option::is_none")]
    pub active_rule: Option<ActiveRule>,
    pub excerpt: Vec<Block>,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<TopicParams>,
    pub related: Vec<HelpId>,
    pub uri: String,
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
        let hits = search(self.help_index()?, &params.text, &params.kinds, usize::MAX);
        let total = hits.len();
        let hits = hits
            .into_iter()
            .take(params.limit.unwrap_or(total))
            .map(|e| Hit {
                id: e.id.clone(),
                kind: e.id.kind(),
                title: e.title.clone(),
                summary: e.summary.clone(),
            })
            .collect();
        Ok(serde_json::to_value(SearchReply { hits, total })?)
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
            if let Some(p) = profile && let Some((_, row)) = p.active(key) {
                let id = format!("rule/{}/{}", p.id, key.name()).parse::<HelpId>().ok();
                *checks = id.filter(|id| index.contains_key(id));
                active_rule = Some(ActiveRule {
                    name: p.name.clone(), value: key.get(p.rules()), row: row.into(), source: p.source.clone(),
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
        bytes = bytes
            .saturating_add(usize::from(!excerpt.is_empty()))
            .saturating_add(serde_json::to_vec(&block)?.len());
        if params.budget_bytes.is_some_and(|max| bytes > max) {
            params.offset = i;
            if excerpt.is_empty() {
                params.budget_bytes = None;
            } // the next whole block needs more room
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
    fn a_byte_budget_preserves_facts_and_resumes_at_the_first_unread_block() -> std::result::Result<(),Box<dyn std::error::Error>> {
        let entry = HelpEntry { id:"guide/example".parse()?, title:"Example".into(), summary:"Two paragraphs.".into(),
            since:"0.1.0".into(), facts:Facts::Guide,related:vec![],keywords:vec![],body: vec![
                Block::Paragraph(vec![Inline::Text("Étape".into())]), Block::Note(vec![Inline::Text("最後".into())])] };
        let mut params:TopicParams = serde_json::from_value(serde_json::json!({"id":entry.id}))?;
        params.budget_bytes = Some(serde_json::to_vec(&entry.body[..1])?.len());
        let index = Index::new();
        let first = topic(&entry, params, None, &index)?;
        assert_eq!(first.excerpt, entry.body[..1]);
        assert!(first.truncated);
        let next = first.next.ok_or("Missing continuation")?;
        let tail = topic(&entry,next,None,&index)?;
        assert_eq!(tail.excerpt,entry.body[1..]);
        assert!(!tail.truncated);
        assert_eq!(tail.facts,entry.facts);
        Ok(())
    }
}
