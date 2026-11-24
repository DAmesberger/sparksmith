//! Typed command failures and design-check findings. Codes are stable strings
//! for automation; messages are for people and may change wording.
mod finding_code;
pub use finding_code::{FindingCode, KNOWN_CODES};

use crate::ids::{DefinitionId, Endpoint, Id, PadId, UnitId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest one-line message, in characters: an issue title or a
/// refusal a person reads in a toast or under a field
/// (`issues::MAX_TITLE`).
pub const MAX_LINE: usize = 80;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Malformed or out-of-range input.
    #[error("{}", shown(.0))]
    Invalid(String),
    /// A referenced object, terminal or net does not exist.
    #[error("{}", shown(.0))]
    NotFound(String),
    /// Valid input that the current document state does not permit
    /// (locked objects, routed copper, duplicates, newer revisions).
    #[error("{}", shown(.0))]
    Conflict(String),
    /// A supported size, count or identity limit would be exceeded.
    #[error("{}", shown(.0))]
    Limit(String),
    /// The request was made against an older document revision.
    #[error("stale revision: expected {expected}, current {actual}")]
    Stale { expected: u64, actual: u64 },
    /// A file or feature version this build cannot represent.
    #[error("{}", shown(.0))]
    Unsupported(String),
    /// Undo or redo with an empty history.
    #[error("{}", shown(.0))]
    History(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// A refusal text split into its leading rule code and the sentence for
/// people: `"footprint.explicit: this footprint has explicit pads"` gives
/// `(Some("footprint.explicit"), "this footprint has explicit pads")`. A
/// rule code is two or more lower-case words joined by dots.
pub fn split_rule(text: &str) -> (Option<&str>, &str) {
    let Some((head, rest)) = text.split_once(": ") else {
        return (None, text);
    };
    let word =
        |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase() || c == '_');
    if head.split('.').count() > 1 && head.split('.').all(word) {
        (Some(head), rest)
    } else {
        (None, text)
    }
}

/// The text people see: the sentence without its rule code. A sentence
/// that followed a code starts with a capital letter.
fn shown(text: &str) -> String {
    let (rule, sentence) = split_rule(text);
    if rule.is_none() {
        return sentence.to_owned();
    }
    let mut chars = sentence.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Error {
    /// This failure of operation `index` (from 0) of a batch of `count`,
    /// which the message names after its own text, so a rule code it leads
    /// with stays where [`split_rule`] reads it.
    pub fn in_batch(self, index: usize, count: usize) -> Self {
        let place =
            |text: String| format!("{text} (operation {} of {count})", index.saturating_add(1));
        match self {
            Self::Invalid(text) => Self::Invalid(place(text)),
            Self::NotFound(text) => Self::NotFound(place(text)),
            Self::Conflict(text) => Self::Conflict(place(text)),
            Self::Limit(text) => Self::Limit(place(text)),
            Self::Unsupported(text) => Self::Unsupported(place(text)),
            Self::History(text) => Self::History(place(text)),
            other @ (Self::Stale { .. } | Self::Json(_)) => other,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid_input",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Limit(_) => "limit_exceeded",
            Self::Stale { .. } => "stale_revision",
            Self::Unsupported(_) => "unsupported",
            Self::History(_) => "history_empty",
            Self::Json(_) => "malformed_request",
        }
    }
    /// The rule code a refusal names, such as `footprint.explicit`, for
    /// hosts that choose wording or a follow-up by code rather than by text.
    pub fn rule(&self) -> Option<&str> {
        match self {
            Self::Invalid(text)
            | Self::NotFound(text)
            | Self::Conflict(text)
            | Self::Limit(text)
            | Self::Unsupported(text)
            | Self::History(text) => split_rule(text).0,
            Self::Stale { .. } | Self::Json(_) => None,
        }
    }
    pub fn diagnostic(&self) -> Diagnostic {
        let (expected_revision, actual_revision) = match self {
            Self::Stale { expected, actual } => (Some(*expected), Some(*actual)),
            _ => (None, None),
        };
        Diagnostic {
            code: self.code().into(),
            rule: self.rule().map(str::to_owned),
            message: self.to_string(),
            expected_revision,
            actual_revision,
        }
    }
}

/// Serializable form of a failure, shared by every transport.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Diagnostic {
    pub code: String,
    /// The rule a refusal names, such as `footprint.explicit`, when it
    /// names one. The message never repeats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<u64>,
}

/// An object a finding refers to, by stable identity.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectRef {
    Instance {
        id: Id,
    },
    Terminal {
        endpoint: Endpoint,
    },
    Net {
        id: Id,
    },
    Track {
        id: Id,
    },
    Pad {
        instance: Id,
        pad: PadId,
    },
    /// A definition lineage (draft or published revision).
    Definition {
        id: DefinitionId,
    },
    /// A local object of a definition (terminal, unit, package terminal or
    /// pad), by its local ID within that lineage.
    LibraryObject {
        definition: DefinitionId,
        local: u32,
    },
    /// A symbol occurrence on the sheet.
    Occurrence {
        id: Id,
    },
    /// One unit of one instance, placed or not.
    Unit {
        instance: Id,
        unit: UnitId,
    },
    /// One field of one instance, by field lineage.
    Property {
        instance: Id,
        field: DefinitionId,
    },
    /// A board via.
    Via {
        id: Id,
    },
    /// A copper fill (zone).
    Fill {
        id: Id,
    },
    /// A keepout region.
    Keepout {
        id: Id,
    },
    /// The board as a whole, for a finding about the board rather than any
    /// one object on it (a check that stopped at its limit). Locating it
    /// frames the board.
    Board,
    /// One feature of the board's shape, by its board feature id
    /// (`model::board::BoardFeatureId`): a mounting hole, a cutout or a
    /// slot. Two holes are two objects, so a finding (and a waiver) about
    /// one never names the other.
    BoardFeature {
        feature: u32,
    },
    /// One via-in-pad hole of a placed pad: the pad's via `index`, in the
    /// footprint's order. The pad's own hole is the pad itself.
    PadVia {
        instance: Id,
        pad: PadId,
        index: u32,
    },
    /// One side of the board, for a finding about that side as a whole (a
    /// side's fiducial count), so the two sides' findings are two.
    BoardSide {
        top: bool,
    },
    /// A net label or power port on a schematic sheet.
    Label {
        id: Id,
    },
    /// A hierarchical sheet symbol.
    SheetSymbol {
        id: Id,
    },
    /// A pin on a sheet symbol.
    SheetPin {
        id: Id,
    },
    /// A schematic sheet.
    Sheet {
        sheet: crate::ids::SheetId,
    },
    /// A schematic wire: its whole path.
    Wire {
        id: Id,
    },
}

/// How much an issue matters, most severe first. Core decides it; hosts only
/// map it to a glyph and ink.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Blocks a stage (publish, board placement, electrical resolution) or
    /// breaks a manufacturing or electrical rule. The only red severity.
    /// Anything that blocks publishing is an error, except a package
    /// terminal not mapped yet, a to-do that still blocks publishing.
    Error,
    /// Lint, and purchasing or rating mismatches.
    Warning,
    /// Work that is expected to remain while designing: unrouted
    /// connections, unplaced instances, unresolved purchasing, package
    /// terminals not mapped yet.
    ToDo,
    /// Information that needs no action.
    Info,
}

/// A derived completeness state a finding can block. Nothing stores these
/// states; they hold when no current finding blocks them.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// A definition revision may be published.
    Publish,
    /// An instance can be placed on the board.
    BoardPlaceable,
    /// Every unit and terminal of an instance has an explicit decision.
    ElectricallyResolved,
}

/// One check result: an electrical rule, a board check or a completeness
/// finding. Circuit and board findings never block a component stage.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Finding {
    /// Stable machine-readable rule code, e.g. `board.clearance`.
    #[schemars(with = "String")]
    pub code: FindingCode,
    pub message: String,
    pub objects: Vec<ObjectRef>,
    /// The stage this finding prevents, if any. Advisory findings have none.
    #[serde(default)]
    pub blocks: Option<Stage>,
    /// What the board needs of a fab profile for this finding to clear,
    /// when its limit is the fab profile's own (not an override's or a net
    /// class's): the measured value the profile's rule would have to
    /// allow. Another profile that allows it clears the finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement: Option<Requirement>,
    /// What the check measured against its limits, as it compared them: a
    /// waiver accepts these numbers and no worse ones. None for a finding
    /// that is a fact without a number. Told with a production finding
    /// (`fabrication.preflight`), not with every finding.
    #[serde(skip)]
    pub measured: Vec<Measured>,
}

/// One number a check measured and the limit it compared it with.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Measured {
    pub value: f64,
    pub limit: f64,
    pub sense: Sense,
    pub unit: Unit,
    /// Which number of a finding that measures several ("the drill").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub of: String,
    /// How far apart two numbers are the same to this check: the slack it
    /// compares with its limit. Never stored.
    #[serde(skip)]
    pub within: f64,
}

/// Which side of its limit a number must be on.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Sense {
    /// A minimum: smaller is worse.
    AtLeast,
    /// A maximum: larger is worse.
    AtMost,
}

/// What a measured number counts.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Mm,
    Degrees,
    /// To one, as an aspect ratio.
    Ratio,
    Percent,
    Count,
    Amperes,
}

impl Measured {
    /// `value` against a minimum of `limit`, the same within `within`.
    pub fn at_least(value: f64, limit: f64, unit: Unit, within: f64) -> Self {
        Self {
            value,
            limit,
            sense: Sense::AtLeast,
            unit,
            of: String::new(),
            within,
        }
    }

    /// `value` against a maximum of `limit`, the same within `within`.
    pub fn at_most(value: f64, limit: f64, unit: Unit, within: f64) -> Self {
        Self {
            sense: Sense::AtMost,
            ..Self::at_least(value, limit, unit, within)
        }
    }

    #[must_use]
    pub fn of(mut self, of: impl Into<String>) -> Self {
        self.of = of.into();
        self
    }

    /// How far past its limit the number is, the wrong way.
    fn short(&self) -> f64 {
        match self.sense {
            Sense::AtLeast => self.limit - self.value,
            Sense::AtMost => self.value - self.limit,
        }
    }

    /// Whether the two are the same number of one check.
    pub fn alike(&self, other: &Self) -> bool {
        (self.sense, self.unit, &self.of) == (other.sense, other.unit, &other.of)
    }

    /// Whether accepting `self` accepts `now`: the same number, no worse,
    /// and no farther from its limit (which may have changed).
    pub fn accepts(&self, now: &Self) -> bool {
        let worse = match now.sense {
            Sense::AtLeast => self.value - now.value,
            Sense::AtMost => now.value - self.value,
        };
        self.alike(now) && worse <= now.within && now.short() - self.short() <= now.within
    }
}

/// What a board needs of its fab profile: a profile-limited finding's
/// measured value, as the bound a profile rule must allow. The rule is
/// named as fab profile files name it (`rules::RuleKey::name`,
/// `clearance_pad_npth_hole`); `rules::requirement` reads it against a
/// profile.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Requirement {
    /// A minimum rule (a clearance, a width, a ring, a drill) the board's
    /// geometry is below: the profile's value must be at most `value`.
    AtMost { rule: String, value: f64 },
    /// A maximum rule (an aspect ratio) the board is above: the profile's
    /// value must be at least `value`.
    AtLeast { rule: String, value: f64 },
    /// A board of `count` copper layers: the profile must be for it.
    CopperLayers { count: u32 },
    /// A board with a solder mask of `colour` (its id): the profile must
    /// offer it.
    MaskColour { colour: String },
    /// A board with vias of `fill` (`capped`): the profile must make it.
    ViaFill { fill: String },
}

impl Finding {
    /// An advisory finding that blocks no stage.
    pub fn new(code: FindingCode, message: String, objects: Vec<ObjectRef>) -> Self {
        Self {
            code,
            message,
            objects,
            blocks: None,
            requirement: None,
            measured: Vec::new(),
        }
    }

    /// This finding with what the check measured (a number that is none is
    /// left out).
    #[must_use]
    pub fn measuring(mut self, measured: impl IntoIterator<Item = Measured>) -> Self {
        self.measured.extend(
            measured
                .into_iter()
                .filter(|m| m.value.is_finite() && m.limit.is_finite()),
        );
        self
    }

    /// This finding with what it needs of the fab profile (none leaves it
    /// as it is).
    #[must_use]
    pub fn requiring(mut self, requirement: Option<Requirement>) -> Self {
        self.requirement = requirement;
        self
    }
    /// A finding that prevents `stage` while it exists.
    pub fn blocking(
        code: FindingCode,
        message: String,
        objects: Vec<ObjectRef>,
        stage: Stage,
    ) -> Self {
        Self {
            blocks: Some(stage),
            ..Self::new(code, message, objects)
        }
    }
    pub fn blocks_stage(&self, stage: Stage) -> bool {
        self.blocks == Some(stage)
    }
}
