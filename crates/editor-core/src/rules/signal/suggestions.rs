//! Pair recognition is shipped data. A polarity ending proposes only two nets.
//! Only an explicit match of both full names can offer an interface target.
use super::{DIFF_IMPEDANCE_RANGE, DiffPairSpec, TOLERANCE_RANGE, name_ok};
use crate::diagnostics::{Error, Result};
use crate::ids::Id;
use crate::rules::ProjectRules;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Published PCB differential impedance and its symmetric percentage tolerance.
/// Copied onto one pair when its suggestion is accepted.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairTarget {
    pub interface: String,
    pub ohms: f64,
    pub tolerance_pct: f64,
    /// Publisher, document and section that state both numbers.
    pub source: String,
    pub url: String,
}

impl PairTarget {
    pub(super) fn check(&self) -> Result<()> {
        if !name_ok(&self.interface)
            || !name_ok(&self.source)
            || !name_ok(&self.url)
            || !self.url.strip_prefix("https://").is_some_and(|url| {
                !url.is_empty() && !url.starts_with('/') && !url.chars().any(char::is_whitespace)
            })
            || !DIFF_IMPEDANCE_RANGE.contains(&self.ohms)
            || !TOLERANCE_RANGE.contains(&self.tolerance_pct)
        {
            return Err(Error::Invalid("A differential target needs a published source, its URL, an impedance and a tolerance in range.".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairCatalog {
    format: u32,
    endings: Vec<[String; 2]>,
    interfaces: Vec<Interface>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Interface {
    target: Option<PairTarget>,
    names: Vec<Names>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Names {
    name: String,
    positive: String,
    negative: String,
}

/// A proposal, without any change to the document.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PairSuggestion {
    pub name: String,
    pub positive: Id,
    pub negative: Id,
    pub positive_name: String,
    pub negative_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<PairTarget>,
}

impl PairSuggestion {
    /// The values the person chose to add, independent of later catalog edits.
    pub fn pair_spec(&self, with_target: bool) -> DiffPairSpec {
        DiffPairSpec {
            name: self.name.clone(),
            positive: self.positive,
            negative: self.negative,
            target: if with_target {
                self.target.clone()
            } else {
                None
            },
        }
    }
}

impl PairCatalog {
    pub fn check(&self) -> Result<()> {
        crate::library::standards::check_format(self.format, "differential pair catalog")?;
        let valid = |p: &str, n: &str| name_ok(p) && name_ok(n) && !p.eq_ignore_ascii_case(n);
        let mut endings = BTreeSet::new();
        for [p, n] in &self.endings {
            if !valid(p, n) || !endings.insert((p.to_ascii_uppercase(), n.to_ascii_uppercase())) {
                return Err(Error::Invalid(
                    "Invalid or duplicate differential pair endings.".into(),
                ));
            }
        }
        let mut names = BTreeSet::new();
        for interface in &self.interfaces {
            if let Some(target) = &interface.target {
                target.check()?;
            }
            for alias in &interface.names {
                let p = alias.positive.to_ascii_uppercase();
                let n = alias.negative.to_ascii_uppercase();
                let distinct = valid(&p, &n);
                let ordered = if p < n { (p, n) } else { (n, p) };
                if !name_ok(&alias.name) || !distinct || !names.insert(ordered) {
                    return Err(Error::Invalid(
                        "Invalid or conflicting differential pair names.".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn suggest<'a>(
        &self,
        rules: &ProjectRules,
        nets: impl IntoIterator<Item = (Id, &'a str)>,
    ) -> Vec<PairSuggestion> {
        let paired: BTreeSet<Id> = rules
            .diff_pairs
            .iter()
            .flat_map(|p| [p.positive, p.negative])
            .collect();
        // An ambiguous spelling (including case variants) cannot identify a net.
        let mut by_name = BTreeMap::new();
        for (id, name) in nets {
            by_name
                .entry(name.to_ascii_uppercase())
                .and_modify(|unique| *unique = None)
                .or_insert(Some((id, name)));
        }
        let mut candidates = BTreeMap::new();
        let mut offer = |name: &str,
                         (positive, positive_name): (Id, &str),
                         (negative, negative_name): (Id, &str),
                         target: Option<&PairTarget>| {
            if paired.contains(&positive) || paired.contains(&negative) || positive == negative {
                return;
            }
            candidates
                .entry((positive, negative))
                .or_insert_with(|| PairSuggestion {
                    name: name.to_owned(),
                    positive,
                    negative,
                    positive_name: positive_name.to_owned(),
                    negative_name: negative_name.to_owned(),
                    target: target.cloned(),
                });
        };
        // Prefer an explicit alias when a generic ending finds the same two nets.
        for interface in &self.interfaces {
            for alias in &interface.names {
                if let (Some(Some(p)), Some(Some(n))) = (
                    by_name.get(&alias.positive.to_ascii_uppercase()),
                    by_name.get(&alias.negative.to_ascii_uppercase()),
                ) {
                    offer(&alias.name, *p, *n, interface.target.as_ref());
                }
            }
        }
        for [plus, minus] in &self.endings {
            let plus = plus.to_ascii_uppercase();
            let minus = minus.to_ascii_uppercase();
            for (upper, net) in &by_name {
                let Some(p) = net else { continue };
                let Some(stem) = upper.strip_suffix(&plus).filter(|s| !s.is_empty()) else {
                    continue;
                };
                if let Some(Some(n)) = by_name.get(&format!("{stem}{minus}")) {
                    let stem = p.1[..stem.len()].trim_end_matches(['_', '-', '.']);
                    offer(if stem.is_empty() { p.1 } else { stem }, *p, *n, None);
                }
            }
        }
        // Multiple plausible partners for a net are never resolved by list order.
        let mut seen = BTreeSet::new();
        let mut ambiguous = BTreeSet::new();
        for &(p, n) in candidates.keys() {
            for id in [p, n] {
                if !seen.insert(id) {
                    ambiguous.insert(id);
                }
            }
        }
        let mut out: Vec<_> = candidates
            .into_values()
            .filter(|s| !ambiguous.contains(&s.positive) && !ambiguous.contains(&s.negative))
            .collect();
        out.sort_by(|a, b| a.positive_name.cmp(&b.positive_name));
        let mut taken: BTreeSet<_> = rules
            .diff_pairs
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        for suggestion in &mut out {
            if !taken.insert(suggestion.name.to_lowercase()) {
                let base = &suggestion.positive_name;
                let mut name = base.clone();
                let mut number = 2;
                while !taken.insert(name.to_lowercase()) {
                    name = format!("{base} {number}");
                    number += 1;
                }
                suggestion.name = name;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_unambiguous_interface_names_get_a_published_target() -> Result<()> {
        let context = crate::library::standards::Context::fixture()?;
        let catalog = &context.standards().pairs;
        let rules = ProjectRules::default();
        let rows = catalog.suggest(
            &rules,
            [
                (1, "usb_sstxp"),
                (2, "USB_SSTXM"),
                (3, "DEBUG_USB_D+"),
                (4, "DEBUG_USB_D-"),
                (5, "CLK_P"),
                (6, "CLK_N"),
                (7, "usb_dp"),
                (8, "USB_DM"),
                (9, "USB_DN"),
            ],
        );
        assert_eq!(rows.len(), 3);
        let target = rows
            .iter()
            .find(|s| s.positive == 1)
            .and_then(|s| s.target.as_ref())
            .ok_or_else(|| Error::NotFound("SuperSpeed suggestion".into()))?;
        assert!(target.source.contains("§5.4"));
        assert_eq!((target.ohms, target.tolerance_pct), (90.0, 15.0));
        assert!(
            rows.iter()
                .filter(|s| s.positive != 1)
                .all(|s| s.target.is_none())
        );
        assert!(
            catalog
                .suggest(&rules, [(1, "USB_D+"), (2, "Usb_D+"), (3, "USB_D-")])
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn shipped_mdi_lvds_and_tmds_aliases_offer_the_published_trace_targets() -> Result<()> {
        let context = crate::library::standards::Context::fixture()?;
        let catalog = &context.standards().pairs;
        let rules = ProjectRules::default();
        let mdi = ("Ethernet MDI (10BASE-T, 100BASE-TX, 1000BASE-T)", 15.0, "#page=29");
        let lvds = ("LVDS", 10.0, "#page=6");
        let tmds = ("HDMI TMDS", 10.0, "#page=199");
        for (p, n, (interface, tolerance, page)) in [
            ("eth_tx_p", "ETH_TX_N", mdi),
            ("MDI_RXP", "MDI_RXN", mdi),
            ("MDI0_P", "MDI0_N", mdi),
            ("MDI1_P", "MDI1_N", mdi),
            ("MDI_C+", "MDI_C-", mdi),
            ("ETH_MDI_D_P", "ETH_MDI_D_N", mdi),
            ("LVDS_P", "LVDS_N", lvds),
            ("LVDS_D0_P", "LVDS_D0_N", lvds),
            ("LVDS_CLKP", "LVDS_CLKN", lvds),
            ("HDMI_DP0", "HDMI_DN0", tmds),
            ("TMDS_DATA1_P", "TMDS_DATA1_N", tmds),
            ("TMDS_D2+", "TMDS_D2-", tmds),
            ("HDMI_CLKP", "HDMI_CLKN", tmds),
        ] {
            let rows = catalog.suggest(&rules, [(10, n), (20, p)]);
            let [row] = rows.as_slice() else {
                return Err(Error::Invalid(format!("Expected one suggestion for {p} and {n}")));
            };
            assert_eq!((row.positive, row.negative), (20, 10), "{p}");
            let target = row.target.as_ref().ok_or_else(|| Error::NotFound(p.into()))?;
            assert_eq!(target.interface, interface, "{p}");
            assert_eq!((target.ohms, target.tolerance_pct), (100.0, tolerance), "{p}");
            assert!(target.url.ends_with(page), "{p}");
            if interface == "LVDS" {
                assert!(target.source.contains("typical"));
            }
            assert_eq!(row.pair_spec(true).target.as_ref(), Some(target));
            assert!(row.pair_spec(false).target.is_none());
        }
        Ok(())
    }

    #[test]
    fn lookalike_names_do_not_inherit_interface_targets() -> Result<()> {
        let context = crate::library::standards::Context::fixture()?;
        let catalog = &context.standards().pairs;
        for (p, n) in [
            ("ETH_RGMII_TX_P", "ETH_RGMII_TX_N"),
            ("MDI4_P", "MDI4_N"),
            ("LVDS_D0_P", "LVDS_D1_N"),
            ("CLK_P", "CLK_N"),
            ("HDMI_FRL3_P", "HDMI_FRL3_N"),
            ("HDMI_DDC_P", "HDMI_DDC_N"),
            ("TMDS_D3+", "TMDS_D3-"),
        ] {
            let rows = catalog.suggest(&ProjectRules::default(), [(1, p), (2, n)]);
            assert!(rows.iter().all(|row| row.target.is_none()), "{p} and {n}");
        }
        Ok(())
    }
}
