//! Slice C3: typed pinout import. The agent (or the Pins
//! tab's Paste table) reads the datasheet and states every field
//! explicitly in a [`PinoutDocument`]; Sparksmith never parses free text and
//! never splits "USART1_TX" into peripheral and role. `library.pinout_preview`
//! returns a [`PinoutPlan`] (changes, findings, review items);
//! `library.pinout_apply` applies it atomically as one undo step, refused
//! when the draft changed since the plan's `base`. The result stays a draft,
//! every imported row unreviewed (founder decision F8: publishing is
//! blocked while rows flagged `uncertain` remain unresolved).
//!
//! Rules arrive as Rhai pin-rule scripts (F2, slice C1), shown in the
//! review like any other row.
mod stage;
pub use stage::{StagedDraft, StagedImport, check_document, stage};

use super::{DefinitionMeta, Drafts, Library};
use crate::diagnostics::{Error, Finding, ObjectRef, Result, Stage};
use crate::functions::{Direction, Drive, GpioRef, Pull, SignalKind, SourceSpan};
use crate::ids::{DefinitionId, PackageRowRef, TerminalId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The only pinout document format this build reads.
pub const PINOUT_FORMAT: u32 = 1;
/// Pins one document lists.
pub const MAX_PINOUT_PINS: usize = 2048;
/// Peripherals one document lists.
pub const MAX_PINOUT_PERIPHERALS: usize = 256;
/// Package variants one document lists.
pub const MAX_PINOUT_VARIANTS: usize = 16;
/// Rules one document lists.
pub const MAX_PINOUT_RULES: usize = 1024;
/// Bytes of a free-text note.
pub const MAX_PINOUT_NOTE: usize = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutReference {
    pub title: String,
    #[serde(default)]
    pub document_revision: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutDevice {
    pub name: String,
    pub reference: PinoutReference,
}

/// One package variant and its terminal count (exposed pads included).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutVariant {
    pub name: String,
    #[serde(default)]
    pub package_row: Option<PackageRowRef>,
    pub terminal_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutRole {
    pub label: String,
    #[serde(default)]
    pub signal: Option<String>,
    #[serde(default = "one")]
    pub capacity: u16,
    #[serde(default)]
    pub required_when_used: bool,
}

fn one() -> u16 {
    1
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutPeripheral {
    pub label: String,
    #[serde(default)]
    pub kind: Option<String>,
    pub roles: Vec<PinoutRole>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutFunction {
    pub label: String,
    pub direction: Direction,
    pub signal: SignalKind,
    #[serde(default = "no_drive")]
    pub drive: Drive,
    /// Peripheral and role labels, stated explicitly.
    #[serde(default)]
    pub peripheral: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// The mux code exactly as the datasheet's table names it ("AF7",
    /// "F2", "ALT5").
    #[serde(default)]
    pub mux: Option<String>,
    /// The table the code belongs to when the datasheet numbers several
    /// tables from the same codes, e.g. "IO MUX", "LP IO MUX" or "analog"
    /// (each such table has its own F0). Leave it out for a part with one
    /// table.
    #[serde(default)]
    pub mux_table: Option<String>,
}

fn no_drive() -> Drive {
    Drive::None
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PinoutClass {
    Electrical,
    DeviceNc,
    Reserved,
}

/// Declared options, by option name.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutOptions {
    #[serde(default)]
    pub pull: Option<Vec<Pull>>,
    #[serde(default)]
    pub speed: Option<Vec<String>>,
    #[serde(default)]
    pub initial_level: bool,
    #[serde(default)]
    pub schmitt: bool,
}

/// One pin row.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutPin {
    /// The document's own row key, stable across re-imports ("r12").
    pub row: String,
    /// Package designation per variant name ("QFN-48" → "12").
    pub number: BTreeMap<String, String>,
    pub name: String,
    pub class: PinoutClass,
    #[serde(default)]
    pub gpio: Option<GpioRef>,
    #[serde(default)]
    pub options: PinoutOptions,
    #[serde(default)]
    pub functions: Vec<PinoutFunction>,
    /// Label of the design-default function.
    #[serde(default)]
    pub default: Option<String>,
    #[serde(default)]
    pub source: Option<SourceSpan>,
    /// The author is unsure; blocks publishing until resolved (F8).
    #[serde(default)]
    pub uncertain: bool,
    #[serde(default)]
    pub note: Option<String>,
}

/// One rule as a Rhai pin-rule script (C1).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutRule {
    pub label: String,
    /// A Rhai script ending in one verdict: ok(), forbid(msg, [pins]),
    /// require(msg, [pins]) or warn(msg, [pins]); it reads the assignment
    /// only through active, active_count, function, used, role, kind,
    /// option, excludes and requires (`rule_api` of
    /// `library.pinout_schema` lists them with examples).
    pub script: String,
    #[serde(default)]
    pub source: Option<SourceSpan>,
}

/// The typed import document (`pinout.schema`).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutDocument {
    /// Must be [`PINOUT_FORMAT`].
    pub format: u32,
    pub device: PinoutDevice,
    pub variants: Vec<PinoutVariant>,
    #[serde(default)]
    pub peripherals: Vec<PinoutPeripheral>,
    pub pins: Vec<PinoutPin>,
    #[serde(default)]
    pub rules: Vec<PinoutRule>,
}

/// What the import targets. Rows match the terminals of an existing
/// draft by package number, through the bindings of that component (the
/// first variant in document order that numbers the row and resolves);
/// a component without any binding matches by exact terminal name. Every
/// other row is a new terminal, and draft terminals no row matches are
/// removed; the plan lists each of these changes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "target", rename_all = "snake_case", deny_unknown_fields)]
pub enum PinoutTarget {
    /// An existing component draft; terminals are kept by number.
    Draft { id: DefinitionId },
    /// New drafts: the component (designator prefix `U`), a block symbol
    /// with a pin per terminal, and per variant a package (terminals by
    /// the variant's numbers; the form of the variant's row, else of
    /// `row` for the first variant, else generic) and a binding mapping
    /// them by number. No footprint: lands come from a template
    /// (`library.footprint_from_template`) and are mapped through
    /// `library.assign_terminals`.
    New {
        name: String,
        #[serde(default)]
        row: Option<PackageRowRef>,
    },
}

/// The kind of one planned change.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Changed,
    Removed,
}

impl ChangeKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Changed => "changed",
            Self::Removed => "removed",
        }
    }
}

/// One planned change, by document row.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutChange {
    pub kind: ChangeKind,
    /// "terminal", "function", "peripheral", "rule", "package_terminal", "pad".
    pub object: String,
    #[serde(default)]
    pub row: Option<String>,
    #[serde(default)]
    pub terminal: Option<TerminalId>,
    pub summary: String,
}

/// Why a row needs a person's look.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewReason {
    Uncertain,
    ContradictsDraft,
    VariantsDisagree,
    /// The draft has no binding, so the row kept an existing terminal's
    /// identity only because the names are equal: a person confirms it is
    /// the same pin.
    MatchedByName,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewItem {
    pub row: String,
    pub reason: ReviewReason,
    pub message: String,
}

/// `library.pinout_preview`'s result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutPlan {
    /// The draft revision token the plan was made against; `pinout_apply`
    /// refuses when it no longer matches.
    pub base: PlanBase,
    /// How many changes of each object and kind (`terminal changed`).
    #[serde(default)]
    pub change_counts: BTreeMap<String, usize>,
    /// Each change; listed only with `detail: true`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<PinoutChange>,
    /// Every publish finding the result would have; listed one by one
    /// only with `detail: true`, else grouped in `finding_groups`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// Each row a person must look at; listed one by one only with
    /// `detail: true`, else grouped in `review_groups`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review: Vec<ReviewItem>,
    /// Without `detail`: the findings by code, with their count and the
    /// first messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub finding_groups: Vec<FindingGroup>,
    /// Without `detail`: the review items by reason, with every row and
    /// the first messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_groups: Vec<ReviewGroup>,
    /// Each pin rule evaluated without selections and over the samples it
    /// names ([`crate::functions::rules::check_samples`]): what it says
    /// and where it fails. Failures are also blocking findings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<crate::functions::rules::RuleCheck>,
    /// Whether changes, findings and review items are listed one by one.
    #[serde(default)]
    pub detail: bool,
}

/// The messages a compact plan quotes per group.
pub const PLAN_EXAMPLES: usize = 3;

/// Findings of one code in a compact plan.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FindingGroup {
    pub code: String,
    #[serde(default)]
    pub blocks: Option<Stage>,
    pub count: usize,
    /// The first [`PLAN_EXAMPLES`] messages.
    pub examples: Vec<String>,
}

/// Review items of one reason in a compact plan.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewGroup {
    pub reason: ReviewReason,
    /// Every row with this reason.
    pub rows: Vec<String>,
    /// The first [`PLAN_EXAMPLES`] messages.
    pub examples: Vec<String>,
}

impl PinoutPlan {
    /// The plan without what only `detail` lists one by one: the changes
    /// (their counts stay), the findings (grouped by code and blocking
    /// stage, counted, the first messages quoted) and the review items
    /// (grouped by reason with every row). The rule checks stay whole.
    #[must_use]
    pub fn compact(mut self) -> Self {
        self.changes.clear();
        let mut groups: Vec<FindingGroup> = Vec::new();
        for finding in std::mem::take(&mut self.findings) {
            match groups
                .iter_mut()
                .find(|g| g.code == finding.code && g.blocks == finding.blocks)
            {
                Some(group) => {
                    group.count = group.count.saturating_add(1);
                    if group.examples.len() < PLAN_EXAMPLES {
                        group.examples.push(finding.message);
                    }
                }
                None => groups.push(FindingGroup {
                    code: finding.code,
                    blocks: finding.blocks,
                    count: 1,
                    examples: vec![finding.message],
                }),
            }
        }
        self.finding_groups = groups;
        let mut review: Vec<ReviewGroup> = Vec::new();
        for item in std::mem::take(&mut self.review) {
            match review.iter_mut().find(|g| g.reason == item.reason) {
                Some(group) => {
                    group.rows.push(item.row);
                    if group.examples.len() < PLAN_EXAMPLES {
                        group.examples.push(item.message);
                    }
                }
                None => review.push(ReviewGroup {
                    reason: item.reason,
                    rows: vec![item.row],
                    examples: vec![item.message],
                }),
            }
        }
        self.review_groups = review;
        self.detail = false;
        self
    }
}

/// Which rows `pinout_apply` takes. `Except` lists pin row keys and
/// rule keys (`rule:<label>`); an excluded row of an existing draft keeps
/// what the draft had, and on a new component it is not imported.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "accept", content = "rows", rename_all = "snake_case")]
pub enum PinoutAccept {
    #[default]
    All,
    Except(Vec<String>),
}

/// What `pinout_apply` wrote.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutApplied {
    /// The drafts created or changed, component first.
    pub drafts: Vec<DefinitionId>,
    /// Imported rows, all unreviewed.
    pub unreviewed: Vec<String>,
}

/// A component's import review state (F8), part of the revision: rows an
/// import created that nobody confirmed, and rows flagged uncertain
/// (`pinout.uncertain` blocks publishing; `pinout.unreviewed` warns).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImportReview {
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub unreviewed: std::collections::BTreeSet<TerminalId>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub uncertain: std::collections::BTreeSet<TerminalId>,
}

impl ImportReview {
    pub fn is_empty(&self) -> bool {
        self.unreviewed.is_empty() && self.uncertain.is_empty()
    }
}

/// The `library.pinout_apply` request: the previewed target and
/// document, the plan's `base` and which rows to take.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PinoutApply {
    pub target: PinoutTarget,
    pub document: PinoutDocument,
    /// The plan's `base`, as the preview wrote it.
    pub base: PlanBase,
    #[serde(default)]
    pub accept: PinoutAccept,
}

/// A plan's base token (`PinoutPlan::base`): a digest, so usually above
/// 2^53. It is written as a decimal string, which a JSON client that reads
/// numbers as doubles keeps exactly; an integer is read too.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, JsonSchema)]
#[schemars(with = "String")]
pub struct PlanBase(pub u64);

impl std::fmt::Display for PlanBase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Serialize for PlanBase {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PlanBase {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        struct Token;
        impl serde::de::Visitor<'_> for Token {
            type Value = PlanBase;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("the plan's base as a decimal string, as the preview wrote it")
            }
            fn visit_str<E: serde::de::Error>(
                self,
                text: &str,
            ) -> std::result::Result<PlanBase, E> {
                text.parse().map(PlanBase).map_err(|_| {
                    E::custom(format!(
                        "base ‘{text}’ is not a plan base: copy the preview's base string exactly"
                    ))
                })
            }
            fn visit_u64<E: serde::de::Error>(
                self,
                value: u64,
            ) -> std::result::Result<PlanBase, E> {
                Ok(PlanBase(value))
            }
            fn visit_i64<E: serde::de::Error>(
                self,
                value: i64,
            ) -> std::result::Result<PlanBase, E> {
                u64::try_from(value)
                    .map(PlanBase)
                    .map_err(|_| E::custom("a plan base is never negative"))
            }
        }
        deserializer.deserialize_any(Token)
    }
}

/// The planned drafts of an import.
pub type PinoutDrafts = Vec<(DefinitionMeta, super::Definition)>;

/// The `accept` key of a rule: this prefix and the rule's label
/// (`rule:uart0_pins`). Row keys never start with it.
pub const RULE_ROW_PREFIX: &str = "rule:";

/// EXL-48, an original invented 48-pin controller in two packages (QFN-48
/// with an exposed pad, and LQFP-48), with alternate functions,
/// peripherals and Rhai pin rules: the core fixture of import and review.
/// It describes no real device; the automation scenario uses its own
/// single-package EXM-48.
#[cfg(test)]
pub(crate) const EXL48: &str = include_str!("pinout/exl48.json");

/// The F8 gate over a component's review state: each terminal flagged
/// uncertain blocks publishing (`pinout.uncertain`); unreviewed imported
/// terminals are one warning (`pinout.unreviewed`). A re-import whose row
/// is no longer flagged clears its uncertainty; a person clears both
/// by editing the draft's `review`.
pub fn review_findings(component: DefinitionId, review: &ImportReview) -> Vec<Finding> {
    let at = |t: &TerminalId| ObjectRef::LibraryObject {
        definition: component,
        local: t.0,
    };
    let mut out: Vec<Finding> = review
        .uncertain
        .iter()
        .map(|t| {
            Finding::blocking(
                "pinout.uncertain",
                format!(
                    "Imported terminal {t} is flagged uncertain; check it against the datasheet and clear the flag before publishing."
                ),
                vec![at(t)],
                Stage::Publish,
            )
        })
        .collect();
    if !review.unreviewed.is_empty() {
        let count = review.unreviewed.len();
        out.push(Finding::new(
            "pinout.unreviewed",
            format!(
                "{count} imported terminal{} not been reviewed yet.",
                if count == 1 { " has" } else { "s have" }
            ),
            review.unreviewed.iter().map(at).collect(),
        ));
    }
    out
}

/// The token a plan is made against (`PinoutPlan::base`): a digest of the
/// target draft and every binding draft of it (they decide how rows match
/// terminals by number), or, for a new component, of the package rows it
/// names. `pinout_apply` refuses when the token changed.
pub fn base_token(library: &Library, drafts: &Drafts, target: &PinoutTarget) -> Result<PlanBase> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    match target {
        PinoutTarget::Draft { id } => {
            let draft = drafts
                .get(*id)
                .ok_or_else(|| Error::NotFound("That draft no longer exists".into()))?;
            hasher.update(serde_json::to_vec(draft)?);
            for other in &drafts.items {
                if let super::Definition::Binding(b) = &other.definition
                    && b.component == *id
                {
                    hasher.update(serde_json::to_vec(other)?);
                }
            }
        }
        PinoutTarget::New { name, row } => {
            hasher.update(name.as_bytes());
            if let Some(r) = row {
                hasher.update(serde_json::to_vec(&library.package_row(*r))?);
            }
        }
    }
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    for (b, d) in bytes.iter_mut().zip(digest.iter()) {
        *b = *d;
    }
    Ok(PlanBase(u64::from_le_bytes(bytes)))
}

/// JSON schema of [`PinoutDocument`] (`pinout.schema`).
pub fn schema() -> serde_json::Value {
    serde_json::json!(schemars::schema_for!(PinoutDocument))
}
