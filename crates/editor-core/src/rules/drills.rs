//! What a fabricator makes of the holes a board may have, and what it says
//! of their price: data of a fab profile beside its rules. A profile states
//! rules over layers ([`DrillRule`]), never a list of combinations; the
//! numbers stay in the rules (`capped_drill_min`, `capped_drill_max`).
//!
//! Today's one drill passes through the whole board, so a rule says which
//! fills of a through via the fabricator makes. A profile that has no rules
//! makes through drills that stay open.
use super::{MAX_PROFILE_NOTES, MAX_PROFILE_TEXT};
use crate::diagnostics::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// What closes a via's hole: a setting of the one via.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum ViaFill {
    /// The hole stays open under any solder mask.
    #[default]
    None,
    /// Filled, then plated over with copper: solderable.
    Capped,
}

impl ViaFill {
    pub const ALL: [Self; 2] = [Self::None, Self::Capped];

    pub fn is_none(&self) -> bool {
        *self == Self::None
    }

    /// The fill in the user's words.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Open",
            Self::Capped => "Filled and capped",
        }
    }

    /// The fill's name in files and commands.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Capped => "capped",
        }
    }

    /// The IPC-4761 type the drill and job files state (Ucamco, Gerber
    /// Job File Format 2020.08, p. 8: `VII`, Filled and Capped); none for
    /// an open hole.
    pub fn ipc_4761(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Capped => Some("VII"),
        }
    }
}

/// How a drill is made.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrillProcess {
    Mechanical,
}

/// Which layers a rule's drills pass.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DrillSpan {
    /// Every copper layer: the through drill.
    Board,
}

/// Which layers a rule's drills pass.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DrillLayers {
    pub span: DrillSpan,
}

/// One kind of drill the fabricator makes, with the fills it makes of it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DrillRule {
    pub id: String,
    pub kind: DrillProcess,
    pub layers: DrillLayers,
    pub fills: Vec<FillRule>,
    /// Where it comes from ("" means the profile's source).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// One fill a fabricator makes of a drill, or says nothing usable about.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FillRule {
    pub fill: ViaFill,
    /// The fabricator offers it without stating what it makes: neither made
    /// nor not made. `source` says what it does state.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not_stated: bool,
    /// How it is ordered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<FillOrder>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// How the fabricator is told which vias take a fill.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FillOrder {
    /// What to choose on the order form.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form: Vec<FormChoice>,
    /// The vias are named to it by their hole size, so a filled and an open
    /// via of one size cannot be told apart.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub told_by_size: bool,
    /// What it asks for, quoted.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub asks: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

/// One choice on an order form.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FormChoice {
    pub label: String,
    pub choice: String,
    /// The form's own label was not read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unverified: bool,
}

/// A charge the fabricator publishes, and the boards it falls on. No amount:
/// no page read gives one.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Charge {
    pub when: ChargeWhen,
    /// The published sentence.
    pub text: String,
    pub source: String,
}

/// When a charge falls: every condition given must hold of one via.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChargeWhen {
    /// On boards of these copper layer counts (empty: of any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copper_layers: Vec<u32>,
    /// A via with this fill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ViaFill>,
    /// A via whose hole is under this, mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drill_below_mm: Option<f64>,
    /// A via whose pad is under this, mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter_below_mm: Option<f64>,
    /// A via whose pad is at most this, mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diameter_at_most_mm: Option<f64>,
}

impl ChargeWhen {
    /// Whether a via of `drill` and `diameter` mm with `fill` on a board of
    /// `layers` copper layers meets every condition given.
    pub fn holds(&self, layers: usize, fill: ViaFill, drill: f64, diameter: f64) -> bool {
        // A size the fabricator names is met by the same size entered.
        const SAME: f64 = 1e-9;
        (self.copper_layers.is_empty()
            || self
                .copper_layers
                .iter()
                .any(|n| usize::try_from(*n).is_ok_and(|n| n == layers)))
            && self.fill.is_none_or(|f| f == fill)
            && self.drill_below_mm.is_none_or(|mm| drill < mm - SAME)
            && self.diameter_below_mm.is_none_or(|mm| diameter < mm - SAME)
            && self
                .diameter_at_most_mm
                .is_none_or(|mm| diameter <= mm + SAME)
    }
}

/// What a profile says of a fill of a through via.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict<'a> {
    Made(Option<&'a FillRule>),
    /// The profile's rules exclude it; its rule's source, when it has one.
    NotMade(&'a str),
    NotStated(&'a FillRule),
}

/// What `rules` say of `fill` on a through via.
pub fn through_fill(rules: &[DrillRule], fill: ViaFill) -> Verdict<'_> {
    let through = rules
        .iter()
        .find(|r| r.kind == DrillProcess::Mechanical && r.layers.span == DrillSpan::Board);
    match through.and_then(|r| r.fills.iter().find(|f| f.fill == fill)) {
        Some(rule) if rule.not_stated => Verdict::NotStated(rule),
        Some(rule) => Verdict::Made(Some(rule)),
        None if fill.is_none() => Verdict::Made(None),
        None => Verdict::NotMade(through.map_or("", |r| r.source.as_str())),
    }
}

/// Drill rules and charges a profile lists, at most.
const MAX_ENTRIES: usize = 16;

/// Bounds of a profile's drill rules and charges: unique ids and fills,
/// printable and bounded texts, sizes above 0 mm, a charge with a condition,
/// a sentence and a source.
pub fn check(rules: &[DrillRule], charges: &[Charge]) -> Result<()> {
    let bad = |what: &str| Err(Error::Invalid(format!("rules.drill_rules: {what}")));
    let text =
        |t: &str, max: usize| t.len() <= max && !t.chars().any(|c| c.is_control() && c != '\n');
    let name = |t: &str| !t.trim().is_empty() && text(t, MAX_PROFILE_TEXT);
    if rules.len() > MAX_ENTRIES || charges.len() > MAX_ENTRIES {
        return bad("a profile lists at most 16 drill rules and 16 charges");
    }
    let mut ids = BTreeSet::new();
    for r in rules {
        if !name(&r.id) || !ids.insert(r.id.as_str()) || !text(&r.source, MAX_PROFILE_NOTES) {
            return bad("every drill rule needs an id of its own; texts are bounded");
        }
        let mut fills = BTreeSet::new();
        for f in &r.fills {
            if !fills.insert(f.fill) {
                return bad("a drill rule lists a fill once");
            }
            let order = f.order.as_ref();
            if !text(&f.source, MAX_PROFILE_NOTES)
                || order.is_some_and(|o| {
                    !text(&o.asks, MAX_PROFILE_NOTES)
                        || !text(&o.source, MAX_PROFILE_NOTES)
                        || o.form.len() > MAX_ENTRIES
                        || o.form.iter().any(|c| !name(&c.label) || !name(&c.choice))
                })
            {
                return bad("texts are printable and bounded");
            }
            if f.not_stated && f.source.trim().is_empty() {
                return bad("what is not stated needs a source that says so");
            }
        }
    }
    let size = |mm: Option<f64>| mm.is_none_or(|mm| mm.is_finite() && mm > 0.0);
    for c in charges {
        if c.when == ChargeWhen::default()
            || !size(c.when.drill_below_mm)
            || !size(c.when.diameter_below_mm)
            || !size(c.when.diameter_at_most_mm)
            || c.when.copper_layers.len() > MAX_ENTRIES
        {
            return bad("a charge needs a condition; sizes are above 0 mm");
        }
        if c.text.trim().is_empty()
            || c.source.trim().is_empty()
            || !text(&c.text, MAX_PROFILE_NOTES)
            || !text(&c.source, MAX_PROFILE_NOTES)
        {
            return bad("a charge needs its published sentence and a source");
        }
    }
    Ok(())
}
