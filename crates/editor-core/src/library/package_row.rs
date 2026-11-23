//! Package rows: revisioned library lineages of kind `PackageRow`, each a
//! cited, searchable set of values for one generator (format 5,
//! part-generation slice D2). A row names its generator by the module's
//! description id and gives parameter values as the generator ABI carries
//! them (lengths in nanometres, angles in millidegrees, counts, choice keys
//! and switches); `library.generate` makes the part from them. Aliases are
//! text, never identity. The shipped rows are the generators' presets.
use crate::diagnostics::FindingCode;
use crate::diagnostics::{Finding, ObjectRef, Stage};
use crate::ids::{DefinitionId, MAX_LABEL, printable};
use crate::library::generated::{GeneratorParams, MAX_PARAMS, ParamValue};
use crate::library::part::{Basis, Provenance};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Aliases one row lists.
pub const MAX_ROW_ALIASES: usize = 64;
/// Bytes of one alias.
pub const MAX_ALIAS: usize = 64;
/// Bytes of one size code (`01005`, `1608`).
pub const MAX_SIZE_CODE: usize = 8;
/// Bytes of a citation's document revision.
pub const MAX_CITATION_REVISION: usize = 64;
/// Bytes of a generator id.
pub const MAX_GENERATOR_ID: usize = 64;
/// Bytes of a parameter key, and of a text value.
const MAX_PARAM_TEXT: usize = 256;
/// Items of a list value.
const MAX_PARAM_LIST: usize = 256;

/// A chip passive's size code, both systems explicit (`0603` imperial is
/// `1608` metric). `pkg:0402` matches both meanings; nothing is guessed.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SizeCode {
    pub imperial: String,
    pub metric: String,
}

impl SizeCode {
    /// Both codes are 1–[`MAX_SIZE_CODE`] ASCII digits.
    pub fn valid(&self) -> bool {
        let code = |c: &str| {
            !c.is_empty() && c.len() <= MAX_SIZE_CODE && c.bytes().all(|b| b.is_ascii_digit())
        };
        code(&self.imperial) && code(&self.metric)
    }

    /// "0402 imperial · 1005 metric": both meanings, nothing guessed.
    pub fn label(&self) -> String {
        format!("{} imperial · {} metric", self.imperial, self.metric)
    }
}

/// A public package outline drawing (IEC 60191 / JEDEC registered outline
/// or a manufacturer's package drawing). Never an EDA library or table.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RowCitation {
    pub document: String,
    #[serde(default)]
    pub document_revision: Option<String>,
    #[serde(default)]
    pub page: Option<u16>,
}

impl RowCitation {
    /// The citation records the drawing's revision and page, so a reader
    /// can check the values against it. A citation naming only the
    /// document reads "cited (unverified)".
    pub fn checked(&self) -> bool {
        self.document_revision.is_some() && self.page.is_some()
    }
}

/// One package row: a generator and its values, with citations.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageRowDefinition {
    /// The generator the values are for (its description's `id`); empty
    /// only in an empty draft.
    pub generator: String,
    /// The generator's parameters by key; the rest come from project
    /// settings and the module's defaults when the part is made.
    #[serde(default)]
    pub params: GeneratorParams,
    /// "SOIC-8", "SO-8": text, never identity.
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub size_code: Option<SizeCode>,
    /// Origin and licence of the row.
    #[serde(default)]
    pub provenance: Provenance,
    /// The public outline drawing the values were typed from; `None` marks
    /// the row estimated.
    #[serde(default)]
    pub citation: Option<RowCitation>,
}

/// Case-folded text for comparing aliases.
fn fold_text(text: &str) -> String {
    text.to_lowercase().replace('\u{b5}', "\u{3bc}")
}

fn value_within_bounds(value: &ParamValue, depth: u8) -> bool {
    match value {
        ParamValue::Int(_) | ParamValue::Bool(_) => true,
        ParamValue::Text(t) => t.len() <= MAX_PARAM_TEXT,
        ParamValue::List(items) => {
            depth < 2
                && items.len() <= MAX_PARAM_LIST
                && items.iter().all(|v| value_within_bounds(v, depth + 1))
        }
    }
}

impl PackageRowDefinition {
    pub(crate) fn within_bounds(&self) -> bool {
        self.generator.len() <= MAX_GENERATOR_ID
            && self.params.len() <= MAX_PARAMS
            && self
                .params
                .iter()
                .all(|(k, v)| k.len() <= MAX_PARAM_TEXT && value_within_bounds(v, 0))
            && self.aliases.len() <= MAX_ROW_ALIASES
            && self.aliases.iter().all(|a| a.len() <= MAX_ALIAS)
            && self.size_code.as_ref().is_none_or(|c| {
                c.imperial.len() <= MAX_SIZE_CODE && c.metric.len() <= MAX_SIZE_CODE
            })
            && self.citation.as_ref().is_none_or(|c| {
                c.document.len() <= MAX_LABEL
                    && c.document_revision
                        .as_ref()
                        .is_none_or(|r| r.len() <= MAX_CITATION_REVISION)
            })
            && self
                .provenance
                .licence
                .as_ref()
                .is_none_or(|l| l.len() <= MAX_LABEL)
    }

    /// Integers only: nothing to be finite.
    pub(crate) fn values_finite(&self) -> bool {
        true
    }

    /// The row's values are cited from an outline drawing.
    pub fn cited(&self) -> bool {
        self.citation.is_some()
    }

    /// Publish findings: `package_row.generator` (no generator named),
    /// `package_row.alias` (blank, repeated or unprintable aliases and
    /// malformed size codes) and `package_row.provenance` (a malformed
    /// citation, or a Part-reference basis, which a row cannot have). The
    /// values themselves are the generator's to check when the part is
    /// made. A row without a citation is estimated, which is allowed.
    pub(crate) fn findings(&self, id: DefinitionId, out: &mut Vec<Finding>) {
        let this = || vec![ObjectRef::Definition { id }];
        let block =
            |code: FindingCode, message: String| Finding::blocking(code, message, this(), Stage::Publish);
        let generator_ok = !self.generator.is_empty()
            && self
                .generator
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if !generator_ok {
            out.push(block(
                FindingCode::PackageRowGenerator,
                format!(
                    "A package row names its generator: 1–{MAX_GENERATOR_ID} ASCII letters, digits, '-' or '_'."
                ),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for alias in &self.aliases {
            if !printable(alias) || alias.len() > MAX_ALIAS {
                out.push(block(
                    FindingCode::PackageRowAlias,
                    format!("An alias needs 1–{MAX_ALIAS} bytes of printable text."),
                ));
            } else if !seen.insert(fold_text(alias.trim())) {
                out.push(block(
                    FindingCode::PackageRowAlias,
                    format!("The alias ‘{}’ is listed twice.", alias.trim()),
                ));
            }
        }
        if let Some(code) = &self.size_code
            && !code.valid()
        {
            out.push(block(
                FindingCode::PackageRowAlias,
                format!(
                    "A size code gives both its imperial and its metric code, 1–{MAX_SIZE_CODE} digits each."
                ),
            ));
        }
        if let Some(citation) = &self.citation {
            if !printable(&citation.document) {
                out.push(block(
                    FindingCode::PackageRowProvenance,
                    "A citation names its public outline drawing in 1–256 bytes of printable text."
                        .into(),
                ));
            }
            if citation
                .document_revision
                .as_ref()
                .is_some_and(|r| !printable(r) || r.len() > MAX_CITATION_REVISION)
            {
                out.push(block(
                    FindingCode::PackageRowProvenance,
                    format!(
                        "A citation's document revision is 1–{MAX_CITATION_REVISION} bytes of printable text."
                    ),
                ));
            }
            if citation.page == Some(0) {
                out.push(block(
                    FindingCode::PackageRowProvenance,
                    "Citation pages are numbered from 1.".into(),
                ));
            }
        }
        if matches!(self.provenance.basis, Basis::FromReference(_)) {
            out.push(block(
                FindingCode::PackageRowProvenance,
                "A row cites its drawing with a citation, not a Part reference.".into(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(row: &PackageRowDefinition) -> Vec<String> {
        let mut out = Vec::new();
        row.findings(DefinitionId(1), &mut out);
        out.into_iter().map(|f| f.code.to_string()).collect()
    }

    #[test]
    fn a_row_names_its_generator_and_cites_cleanly() {
        let mut row = PackageRowDefinition {
            generator: "gull-wing".into(),
            params: [("pitch".to_owned(), ParamValue::Int(1_270_000))]
                .into_iter()
                .collect(),
            aliases: vec!["SOIC-8".into(), "SO-8".into()],
            ..PackageRowDefinition::default()
        };
        assert!(codes(&row).is_empty(), "{:?}", codes(&row));
        assert!(row.within_bounds());
        row.aliases.push("so-8".into());
        assert_eq!(codes(&row), ["package_row.alias"]);
        row.aliases.pop();
        row.generator.clear();
        assert_eq!(codes(&row), ["package_row.generator"]);
        row.generator = "chip".into();
        row.citation = Some(RowCitation {
            document: "JEDEC MS-012".into(),
            document_revision: None,
            page: Some(0),
        });
        assert_eq!(codes(&row), ["package_row.provenance"]);
    }
}
