//! Offline help shared by the desktop, command surface and static manual.
mod findings;
pub use findings::finding_entries;

mod parameters;
pub use parameters::{parameter_type, parameter_value};
mod search;
use crate::{Class, Stage, diagnostics::Severity, rules::profile::ProfileSource};
use schemars::JsonSchema;
pub use search::{WORD_MATCH, match_score, score};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sparksmith_generator_abi::{
    surface::Kind as EntryKind,
    v1::{Param, Preset},
};
use std::{collections::BTreeMap, fmt, str::FromStr};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HelpKind {
    Entry,
    Finding,
    Command,
    Setting,
    Rule,
    Generator,
    Term,
    Guide,
    Workflow,
    Panel,
}

#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct HelpId(String);

impl HelpId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn name(&self) -> &str {
        self.0.split_once('/').map_or("", |(_, name)| name)
    }
    pub fn kind(&self) -> HelpKind {
        // Construction and deserialization both validate the prefix.
        match self.0.split('/').next() {
            Some("entry") => HelpKind::Entry,
            Some("finding") => HelpKind::Finding,
            Some("command") => HelpKind::Command,
            Some("setting") => HelpKind::Setting,
            Some("rule") => HelpKind::Rule,
            Some("generator") => HelpKind::Generator,
            Some("term") => HelpKind::Term,
            Some("workflow") => HelpKind::Workflow,
            Some("panel") => HelpKind::Panel,
            _ => HelpKind::Guide,
        }
    }
}

impl FromStr for HelpId {
    type Err = String;
    fn from_str(raw: &str) -> Result<Self, String> {
        fn slug(s: &str) -> bool {
            !s.is_empty()
                && s.as_bytes()[0].is_ascii_alphanumeric()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
                && !s.contains("..")
        }
        let valid = raw.split_once('/').is_some_and(|(kind, name)| match kind {
            "rule" => name
                .split_once('/')
                .is_some_and(|(a, b)| slug(a) && slug(b)),
            "generator" => name
                .split_once('#')
                .map_or_else(|| slug(name), |(a, b)| slug(a) && slug(b)),
            "entry" | "finding" | "command" | "setting" | "term" | "guide" | "workflow"
            | "panel" => slug(name),
            _ => false,
        });
        if !valid || raw.len() > 512 {
            return Err(format!("Invalid help id ‘{raw}’"));
        }
        Ok(Self(raw.to_owned()))
    }
}
impl TryFrom<String> for HelpId {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        s.parse()
    }
}
impl From<HelpId> for String {
    fn from(id: HelpId) -> Self {
        id.0
    }
}
impl fmt::Display for HelpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Inline {
    Text(String),
    Reference(HelpId),
    Key(HelpId),
}

/// Parse the two inline forms used by topic authors. Plain text stays literal.
pub fn inlines(mut s: &str) -> Result<Vec<Inline>, String> {
    if s.contains("://") {
        return Err("Use a help reference in place of a URL".into());
    }
    let mut out = Vec::new();
    while !s.is_empty() {
        let Some((pos, opener)) = s
            .match_indices("[[")
            .next()
            .into_iter()
            .chain(s.match_indices("{key:").next())
            .min_by_key(|(i, _)| *i)
        else {
            out.push(Inline::Text(s.into()));
            break;
        };
        if pos > 0 {
            out.push(Inline::Text(s[..pos].into()));
        }
        let key = opener == "{key:";
        let (target, rest) = s[pos + opener.len()..]
            .split_once(if key { "}" } else { "]]" })
            .ok_or_else(|| format!("Unclosed help reference in ‘{s}’"))?;
        out.push(if key {
            Inline::Key(format!("command/{target}").parse()?)
        } else {
            Inline::Reference(target.parse()?)
        });
        s = rest;
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Block {
    Paragraph(Vec<Inline>),
    List(Vec<Vec<Inline>>),
    Table(Vec<Vec<Vec<Inline>>>),
    Note(Vec<Inline>),
    Image {
        file: String,
        alt: String,
        version: String,
    },
    Example(CallExample),
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallExample {
    pub entry: HelpId,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Facts {
    Entry {
        module: String,
        class: Class,
        entry_kind: Option<EntryKind>,
        schema_digest: String,
        examples: Vec<CallExample>,
    },
    Finding {
        severity: Severity,
        blocks: Option<Stage>,
        group: String,
        fix: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        cause: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        explanation: Option<String>,
        fixed_by: Vec<HelpId>,
        checks: Option<HelpId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule_key: Option<String>,
    },
    Command {
        context: String,
        default_key: Option<String>,
        reaches: Option<HelpId>,
    },
    Setting {
        schema: Value,
        unit: Option<String>,
        default: Value,
        source: String,
    },
    Rule {
        value: Value,
        unit: String,
        source: ProfileSource,
        row: String,
        author_choice: bool,
    },
    Generator {
        parameters: Vec<Param>,
        presets: Vec<Preset>,
    },
    Term {
        definition: String,
        avoid: Vec<String>,
        developer_only: bool,
    },
    Guide,
    Workflow,
    Panel,
}

impl Facts {
    pub fn kind(&self) -> HelpKind {
        match self {
            Self::Entry { .. } => HelpKind::Entry,
            Self::Finding { .. } => HelpKind::Finding,
            Self::Command { .. } => HelpKind::Command,
            Self::Setting { .. } => HelpKind::Setting,
            Self::Rule { .. } => HelpKind::Rule,
            Self::Generator { .. } => HelpKind::Generator,
            Self::Term { .. } => HelpKind::Term,
            Self::Guide => HelpKind::Guide,
            Self::Workflow => HelpKind::Workflow,
            Self::Panel => HelpKind::Panel,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelpEntry {
    pub id: HelpId,
    pub title: String,
    pub summary: String,
    pub since: String,
    pub facts: Facts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<HelpId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
}

impl HelpEntry {
    /// The intro in a reading pane. Findings still carry a searchable summary
    /// when their cause supplies it, and the Cause section shows that text.
    pub fn introduction(&self) -> Option<&str> {
        if self.summary.is_empty()
            || self.summary == self.title
            || matches!(&self.facts, Facts::Finding { cause, .. } if cause == &self.summary)
        {
            None
        } else {
            Some(self.summary.as_str())
        }
    }
}

pub type Index = BTreeMap<HelpId, HelpEntry>;

#[cfg(test)]
mod tests;
