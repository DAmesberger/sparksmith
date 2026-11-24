//! What a board needs of a fab profile, in words.
//!
//! A board check whose limit is the fab profile's own rule records what the
//! board measured as a [`Requirement`] (`diagnostics`): the bound another
//! profile would have to allow for the finding to clear. [`describe`] reads
//! one against a profile: the measured value, the profile's limit and the
//! source the profile gives for it, and the requirement in neutral words
//! ("needs pad to unplated hole clearance ≤ 0.175 mm"). The words name the
//! rule and the value, never a fabricator: which profiles allow it is
//! computed (`fabrication::suggest`).
use super::{RuleKey, RuleProfile, RuleValue};
use crate::diagnostics::Requirement;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A requirement as words, against the profile the board was checked
/// under.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Needed {
    /// What the finding records.
    pub requirement: Requirement,
    /// The rule's name ("pad to unplated hole clearance", "copper layers").
    pub rule: String,
    /// What the board has, with its unit ("0.175 mm", "4 copper layers").
    pub measured: String,
    /// The profile's limit, with its unit ("0.2 mm", "2-layer boards"),
    /// when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
    /// The profile the limit is from, by name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile: String,
    /// Where the profile says the limit comes from (the rule's source in the
    /// profile file, else the profile's document).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    /// One line: "Needs pad to unplated hole clearance ≤ 0.175 mm".
    pub text: String,
}

/// A number for a message: at most three decimals, no trailing zeros
/// (`0.2`, `0.102`, `0`).
pub fn number(value: f64) -> String {
    fixed(value, 3)
}

fn fixed(value: f64, places: usize) -> String {
    let text = format!("{value:.places$}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" || text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// A measured number for a message that compares it with `limit`: as
/// [`number`], with more decimals (up to six) where three would read as the
/// limit itself, so a gap just under its rule does not read as meeting it
/// (`0.1996`, not `0.2`, against 0.2).
pub fn number_against(value: f64, limit: f64) -> String {
    (3..=6)
        .map(|places| fixed(value, places))
        .zip((3..=6).map(|places| fixed(limit, places)))
        .find(|(v, l)| v != l)
        .map_or_else(|| number(value), |(v, _)| v)
}

/// `value` of a rule of `kind` with its unit.
fn with_unit(kind: RuleValue, value: &str) -> String {
    match kind {
        RuleValue::Length(_) => format!("{value} mm"),
        RuleValue::Area(_) => format!("{value} mm²"),
        RuleValue::Angle(_) => format!("{value}°"),
        RuleValue::Ratio(_) => format!("{value}:1"),
        _ => value.to_owned(),
    }
}

/// The scalar value of `value`, when it has one.
fn scalar(value: RuleValue) -> Option<f64> {
    match value {
        RuleValue::Length(v)
        | RuleValue::Area(v)
        | RuleValue::Angle(v)
        | RuleValue::Fraction(v)
        | RuleValue::Ratio(Some(v)) => Some(v),
        RuleValue::Count(n) => Some(f64::from(n)),
        _ => None,
    }
}

/// The source `profile` gives for its rule `key` under its choices, else
/// its document.
fn rule_source(profile: &RuleProfile, key: RuleKey) -> String {
    match profile.active(key) {
        Some((_, source)) if !source.trim().is_empty() => source.to_owned(),
        _ => document(profile),
    }
}

fn document(profile: &RuleProfile) -> String {
    let s = &profile.source;
    [s.publisher.as_str(), s.document.as_str()]
        .iter()
        .filter(|t| !t.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join(": ")
}

/// `requirement` in words, against `profile` (the one the board was
/// checked under; `None` gives no limit or source). `None` for a rule name
/// no rule has.
pub fn describe(requirement: &Requirement, profile: Option<&RuleProfile>) -> Option<Needed> {
    let name = profile.map(|p| p.name.clone()).unwrap_or_default();
    let needed = match requirement {
        Requirement::AtMost { rule, value } | Requirement::AtLeast { rule, value } => {
            let key = RuleKey::from_name(rule)?;
            let kind = key.kind();
            let limit = profile.and_then(|p| scalar(key.get(p.rules())));
            let measured = with_unit(
                kind,
                &limit.map_or_else(|| number(*value), |l| number_against(*value, l)),
            );
            let sign = if matches!(requirement, Requirement::AtMost { .. }) {
                "≤"
            } else {
                "≥"
            };
            Needed {
                requirement: requirement.clone(),
                rule: key.label(),
                text: format!("Needs {} {sign} {measured}", key.label()),
                measured,
                limit: limit.map(|l| with_unit(kind, &number(l))),
                source: profile.map(|p| rule_source(p, key)).unwrap_or_default(),
                profile: name,
            }
        }
        Requirement::CopperLayers { count } => {
            let boards = profile.and_then(|p| p.boards.as_ref());
            Needed {
                requirement: requirement.clone(),
                rule: "copper layers".into(),
                measured: format!("{count} copper layers"),
                limit: boards.map(|b| {
                    let counts: Vec<String> =
                        b.copper_layers.iter().map(ToString::to_string).collect();
                    format!("{}-layer boards", counts.join(", "))
                }),
                source: match (boards, profile) {
                    (Some(b), _) if !b.source.is_empty() => b.source.clone(),
                    (_, Some(p)) => document(p),
                    _ => String::new(),
                },
                profile: name,
                text: format!("Needs a profile for {count}-layer boards"),
            }
        }
        Requirement::ViaFill { fill } => {
            let fill = super::ViaFill::ALL.into_iter().find(|f| f.id() == fill)?;
            let words = fill.label().to_lowercase();
            let verdict = profile.map(|p| super::drills::through_fill(&p.drill_rules, fill));
            let source = match verdict {
                Some(super::drills::Verdict::NotMade(source)) => source.to_owned(),
                Some(super::drills::Verdict::NotStated(rule)) => rule.source.clone(),
                _ => String::new(),
            };
            Needed {
                requirement: requirement.clone(),
                rule: "via fill".into(),
                measured: format!("{words} vias"),
                limit: profile.map(|_| "open vias".into()),
                source: match profile {
                    Some(p) if source.trim().is_empty() => document(p),
                    _ => source,
                },
                profile: name,
                text: format!("Needs {words} vias"),
            }
        }
        Requirement::MaskColour { colour } => {
            let option = profile.and_then(|p| p.following(super::BoardSetting::MaskColour));
            Needed {
                requirement: requirement.clone(),
                rule: "solder mask colour".into(),
                measured: colour.clone(),
                limit: option.map(|o| {
                    o.choices
                        .iter()
                        .flat_map(|c| c.mask_colours.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(", ")
                }),
                source: option.map(|o| o.source.clone()).unwrap_or_default(),
                profile: name,
                text: format!("Needs a {colour} solder mask"),
            }
        }
    };
    Some(needed)
}

/// What `profile` does not state of `requirement`: what the board has lies
/// past the rule's limit and within what the fabricator's own sources
/// disagree on ([`super::profile::Unclear`]), or is a via fill it offers
/// without stating it. The source quotes the fabricator.
pub fn not_stated(requirement: &Requirement, profile: Option<&RuleProfile>) -> Option<Needed> {
    // A limit a source names is met by the same size entered.
    const SAME: f64 = 1e-9;
    let profile = profile?;
    if let Requirement::ViaFill { fill } = requirement {
        let fill = super::ViaFill::ALL.into_iter().find(|f| f.id() == fill)?;
        let verdict = super::drills::through_fill(&profile.drill_rules, fill);
        let need = describe(requirement, Some(profile))?;
        return matches!(verdict, super::drills::Verdict::NotStated(_)).then(|| Needed {
            text: format!(
                "{} does not state what it makes of {}",
                profile.name, need.measured
            ),
            ..need
        });
    }
    let (Requirement::AtMost { rule, value } | Requirement::AtLeast { rule, value }) = requirement
    else {
        return None;
    };
    let key = RuleKey::from_name(rule)?;
    let unclear = profile.unclear(key)?;
    let limit = scalar(key.get(profile.rules()))?;
    let within = if key.is_maximum() {
        *value <= unclear.to + SAME
    } else {
        *value >= unclear.to - SAME
    };
    let need = describe(requirement, Some(profile)).filter(|_| within)?;
    let kind = key.kind();
    let (low, high) = (unclear.to.min(limit), unclear.to.max(limit));
    Some(Needed {
        text: format!(
            "{} does not state {} between {} and {}",
            profile.name,
            key.label(),
            with_unit(kind, &number(low)),
            with_unit(kind, &number(high))
        ),
        source: unclear.source.clone(),
        ..need
    })
}

/// Whether `a` asks at least as much as `b` of a profile: the same kind of
/// requirement of the same rule, and as tight or tighter. Requirements of
/// different rules or kinds never cover each other.
pub fn covers(a: &Requirement, b: &Requirement) -> bool {
    match (a, b) {
        (
            Requirement::AtMost {
                rule: ra,
                value: va,
            },
            Requirement::AtMost {
                rule: rb,
                value: vb,
            },
        ) => ra == rb && va <= vb,
        (
            Requirement::AtLeast {
                rule: ra,
                value: va,
            },
            Requirement::AtLeast {
                rule: rb,
                value: vb,
            },
        ) => ra == rb && va >= vb,
        (a, b) => a == b,
    }
}

/// The tightest requirement of each rule among `all`, in first-seen order:
/// what a profile must allow for every one of them to clear.
pub fn tightest<'a>(all: impl IntoIterator<Item = &'a Requirement>) -> Vec<Requirement> {
    let mut out: Vec<Requirement> = Vec::new();
    for r in all {
        if let Some(i) = out.iter().position(|o| covers(o, r) || covers(r, o)) {
            if let Some(slot) = out.get_mut(i)
                && covers(r, slot)
            {
                *slot = r.clone();
            }
        } else {
            out.push(r.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Result;

    #[test]
    fn a_requirement_reads_as_the_rule_and_the_measured_value() -> Result<()> {
        let profile = crate::rules::fixtures::prototype()?;
        let key = RuleKey::Clearance {
            a: crate::rules::CopperObject::Pad,
            b: crate::rules::CopperObject::NpthHole,
        }
        .normalized();
        let need = describe(
            &Requirement::AtMost {
                rule: key.name(),
                value: 0.175,
            },
            Some(&profile),
        )
        .ok_or_else(|| crate::diagnostics::Error::Invalid("no rule".into()))?;
        assert_eq!(need.text, format!("Needs {} ≤ 0.175 mm", key.label()));
        assert_eq!(need.measured, "0.175 mm");
        assert_eq!(
            need.limit,
            scalar(key.get(profile.rules())).map(|l| format!("{} mm", number(l)))
        );
        assert_eq!(need.profile, profile.name);
        assert!(!need.source.is_empty());
        assert!(
            describe(
                &Requirement::AtMost {
                    rule: "no_such_rule".into(),
                    value: 1.0
                },
                None
            )
            .is_none()
        );
        Ok(())
    }

    #[test]
    fn a_measured_value_never_reads_as_its_limit() {
        assert_eq!(number_against(0.1996, 0.2), "0.1996");
        assert_eq!(number_against(0.123_456, 0.2), "0.123");
        assert_eq!(number(0.0), "0");
    }

    #[test]
    fn the_tightest_requirement_of_each_rule_is_kept() {
        let at_most = |rule: &str, value| Requirement::AtMost {
            rule: rule.into(),
            value,
        };
        let all = [
            at_most("drill_min", 0.2),
            at_most("track_width_min", 0.1),
            at_most("drill_min", 0.15),
            at_most("drill_min", 0.18),
            Requirement::CopperLayers { count: 4 },
            Requirement::CopperLayers { count: 4 },
        ];
        assert_eq!(
            tightest(&all),
            vec![
                at_most("drill_min", 0.15),
                at_most("track_width_min", 0.1),
                Requirement::CopperLayers { count: 4 },
            ]
        );
    }
}
