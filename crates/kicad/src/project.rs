//! The design rules of a KiCad project file (`.kicad_pro`, JSON): its net
//! classes (clearance, track width, via size, differential-pair width and
//! gap), their name patterns and explicit net assignments, and the
//! board-wide minimums under `board.design_settings.rules`. They become
//! the project's own rule set, marked "From the KiCad project", over the
//! fab profile, which keeps the fab limits: a KiCad limit looser than the
//! profile's is not taken (and is counted). Read for facts only; nothing
//! else of the file is used.
use crate::report::{LooserRule, Report};
use sparksmith_core::rules::{CopperObject, DesignRules, RuleKey, RuleValue};

/// Largest project file read, bytes.
pub const MAX_PROJECT: usize = 16 * 1024 * 1024;
/// Most net classes, patterns and assignments read.
const MAX_ENTRIES: usize = 65_536;

/// What the importer says the project's own rule set comes from.
pub const SOURCE: &str = "From the KiCad project";

/// One KiCad net class's values, mm.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KClass {
    pub name: String,
    pub clearance: Option<f64>,
    pub track_width: Option<f64>,
    pub via_diameter: Option<f64>,
    pub via_drill: Option<f64>,
    pub diff_pair_width: Option<f64>,
    pub diff_pair_gap: Option<f64>,
}

/// The board-wide minimums of `board.design_settings.rules`, mm.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoardMinimums {
    /// The least copper-to-copper clearance (`min_clearance`).
    pub clearance: Option<f64>,
    pub edge_clearance: Option<f64>,
    pub hole_clearance: Option<f64>,
    pub hole_to_hole: Option<f64>,
    pub track_width_min: Option<f64>,
    pub via_annular_min: Option<f64>,
    pub drill_min: Option<f64>,
    pub text_height_min: Option<f64>,
}

/// The rules a project file states.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectRules {
    /// The `Default` class (else the first).
    pub default: Option<KClass>,
    /// Every other class, in file order.
    pub classes: Vec<KClass>,
    /// `(pattern, class)` in file order.
    pub patterns: Vec<(String, String)>,
    /// `(net name, class)`.
    pub assignments: Vec<(String, String)>,
    pub board: BoardMinimums,
}

fn mm(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().filter(|v| v.is_finite() && *v >= 0.0)
}

fn positive(v: &serde_json::Value) -> Option<f64> {
    mm(v).filter(|v| *v > 0.0)
}

fn class(c: &serde_json::Value) -> KClass {
    KClass {
        name: c["name"].as_str().unwrap_or_default().to_owned(),
        clearance: mm(&c["clearance"]),
        track_width: positive(&c["track_width"]),
        via_diameter: positive(&c["via_diameter"]),
        via_drill: positive(&c["via_drill"]),
        diff_pair_width: positive(&c["diff_pair_width"]),
        diff_pair_gap: positive(&c["diff_pair_gap"]),
    }
}

/// Read a project file; `None` when it is not one (too large, not JSON).
pub fn read(bytes: &[u8]) -> Option<ProjectRules> {
    if bytes.len() > MAX_PROJECT {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let rules = &json["board"]["design_settings"]["rules"];
    let net = &json["net_settings"];
    let mut classes: Vec<KClass> = net["classes"]
        .as_array()
        .map(|c| c.iter().take(MAX_ENTRIES).map(class).collect())
        .unwrap_or_default();
    let default = classes
        .iter()
        .position(|c| c.name == "Default")
        .or((!classes.is_empty()).then_some(0))
        .map(|k| classes.remove(k));
    let patterns = net["netclass_patterns"]
        .as_array()
        .map(|a| {
            a.iter()
                .take(MAX_ENTRIES)
                .filter_map(|p| {
                    Some((
                        p["pattern"].as_str()?.to_owned(),
                        p["netclass"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    // A net's class by name: a string, or a list whose first entry counts.
    let assignments = net["netclass_assignments"]
        .as_object()
        .map(|o| {
            o.iter()
                .take(MAX_ENTRIES)
                .filter_map(|(net, class)| {
                    let class = match class {
                        serde_json::Value::String(s) => Some(s.as_str()),
                        serde_json::Value::Array(a) => a.first().and_then(|v| v.as_str()),
                        _ => None,
                    }?;
                    Some((net.clone(), class.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    Some(ProjectRules {
        default,
        classes,
        patterns,
        assignments,
        board: BoardMinimums {
            clearance: mm(&rules["min_clearance"]),
            edge_clearance: mm(&rules["min_copper_edge_clearance"]),
            hole_clearance: mm(&rules["min_hole_clearance"]),
            hole_to_hole: mm(&rules["min_hole_to_hole"]),
            track_width_min: positive(&rules["min_track_width"]),
            via_annular_min: positive(&rules["min_via_annular_width"]),
            drill_min: positive(&rules["min_through_hole_diameter"]),
            text_height_min: positive(&rules["min_text_height"]),
        },
    })
}

/// Whether `key` is a limit (a least value the fab profile may also set),
/// as opposed to a default a tool starts from.
fn is_limit(key: RuleKey) -> bool {
    !matches!(
        key,
        RuleKey::TrackWidthDefault | RuleKey::ViaDrillDefault | RuleKey::ViaDiameterDefault
    )
}

/// The KiCad settings of the default class, as the report names them.
const CLASS_CLEARANCE: &str = "default net class clearance";
const CLASS_TRACK: &str = "default net class track width";
const CLASS_VIA: &str = "default net class via diameter";
const CLASS_DRILL: &str = "default net class via drill";

/// The rule overrides the project's default class and board minimums
/// give, less the limits looser than `profile`'s (each counted as
/// [`crate::report::Case::RuleLooserThanFab`], each named): the fab
/// profile keeps its limits.
pub fn overrides(
    rules: &ProjectRules,
    profile: &DesignRules,
    report: &mut Report,
) -> Vec<(RuleKey, RuleValue)> {
    use CopperObject::*;
    let copper = [Track, Pad, Via, Fill];
    let holes = [ViaHole, PthHole, NpthHole];
    let b = &rules.board;
    let default = rules.default.clone().unwrap_or_default();
    // Each wanted value with the KiCad setting it comes from.
    let mut wanted: Vec<(RuleKey, f64, &'static str)> = Vec::new();
    let mut pair = |a: CopperObject, b: CopperObject, value: Option<f64>, setting| {
        if let Some(v) = value {
            wanted.push((RuleKey::Clearance { a, b }, v, setting));
        }
    };
    for (i, a) in copper.iter().enumerate() {
        for c in &copper[i..] {
            pair(*a, *c, b.clearance, "min_clearance");
            pair(*a, *c, default.clearance, CLASS_CLEARANCE);
        }
        pair(*a, BoardEdge, b.edge_clearance, "min_copper_edge_clearance");
        for hole in holes {
            pair(*a, hole, b.hole_clearance, "min_hole_clearance");
        }
    }
    for (i, a) in holes.iter().enumerate() {
        for c in &holes[i..] {
            pair(*a, *c, b.hole_to_hole, "min_hole_to_hole");
        }
    }
    for (key, value, setting) in [
        (RuleKey::TrackWidthMin, b.track_width_min, "min_track_width"),
        (
            RuleKey::ViaAnnularRingMin,
            b.via_annular_min,
            "min_via_annular_width",
        ),
        (RuleKey::DrillMin, b.drill_min, "min_through_hole_diameter"),
        (
            RuleKey::SilkTextHeightMin,
            b.text_height_min,
            "min_text_height",
        ),
        (RuleKey::TrackWidthDefault, default.track_width, CLASS_TRACK),
        (RuleKey::ViaDiameterDefault, default.via_diameter, CLASS_VIA),
        (RuleKey::ViaDrillDefault, default.via_drill, CLASS_DRILL),
    ] {
        if let Some(v) = value {
            wanted.push((key, v, setting));
        }
    }
    let mut out = Vec::new();
    for (key, v, setting) in wanted {
        if is_limit(key)
            && let RuleValue::Length(fab) = key.get(profile)
            && v < fab - 1e-9
        {
            report.looser_rule(LooserRule {
                rule: key.label(),
                kicad_setting: setting,
                kicad_mm: v,
                profile_mm: fab,
            });
            continue;
        }
        // The board's least clearance and the default class's both set a
        // copper pair: the larger governs.
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, RuleValue::Length(taken))) => *taken = taken.max(v),
            _ => out.push((key, RuleValue::Length(v))),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::Case;

    #[test]
    fn classes_patterns_assignments_and_board_rules_are_read() {
        let json = br#"{"board":{"design_settings":{"rules":{
            "min_copper_edge_clearance":0.5,"min_hole_clearance":0.25,"min_track_width":0.0,
            "min_hole_to_hole":0.3,"min_through_hole_diameter":0.2}}},
            "net_settings":{"classes":[{"name":"Power","clearance":0.4,"track_width":0.6},
            {"name":"Default","clearance":0.2,"track_width":0.25,"via_diameter":0.6,"via_drill":0.3,
            "diff_pair_width":0.2,"diff_pair_gap":0.25}],
            "netclass_patterns":[{"netclass":"Power","pattern":"V*"}],
            "netclass_assignments":{"GND":"Power","X":["Power"]}}}"#;
        let rules = read(json).unwrap_or_default();
        assert_eq!(
            rules.default.as_ref().map(|c| c.name.as_str()),
            Some("Default")
        );
        assert_eq!(rules.classes.len(), 1);
        assert_eq!(rules.classes[0].track_width, Some(0.6));
        assert_eq!(rules.patterns, vec![("V*".to_owned(), "Power".to_owned())]);
        assert_eq!(rules.assignments.len(), 2);
        assert_eq!(rules.board.track_width_min, None);
        assert_eq!(read(b"not json"), None);

        // Over a profile with 0.25 mm hole clearance and a 0.3 mm least
        // drill: the KiCad drill minimum (0.2) is looser and not taken.
        let mut profile = DesignRules::default();
        profile.holes.drill_min = 0.3;
        let mut report = Report::default();
        let overrides = overrides(&rules, &profile, &mut report);
        assert_eq!(report.count(Case::RuleLooserThanFab), 1);
        assert!(
            report
                .summary()
                .contains("1 KiCad rule looser than the fab profile was not taken")
        );
        assert!(!overrides.iter().any(|(k, _)| *k == RuleKey::DrillMin));
        // The rule not taken is named with both values.
        assert_eq!(
            report.looser_rules(),
            &[LooserRule {
                rule: "min drill".into(),
                kicad_setting: "min_through_hole_diameter",
                kicad_mm: 0.2,
                profile_mm: 0.3,
            }]
        );
        assert!(
            report.summary_named().contains(
                "    min drill: KiCad 0.2 mm, fab profile 0.3 mm (min_through_hole_diameter)"
            ),
            "{}",
            report.summary_named()
        );
        assert!(!report.summary().contains("min drill"));
        assert!(overrides.contains(&(
            RuleKey::Clearance {
                a: CopperObject::Track,
                b: CopperObject::BoardEdge
            },
            RuleValue::Length(0.5)
        )));
        assert!(overrides.contains(&(RuleKey::ViaDrillDefault, RuleValue::Length(0.3))));
        // Ten copper pairs, four edge, twelve hole and six hole-hole pairs,
        // three defaults.
        assert_eq!(overrides.len(), 10 + 4 + 12 + 6 + 3);
    }

    /// Run 7, finding 3: KiCad's board `min_clearance` is read and taken
    /// like the other board minimums: below the fab profile's clearance it
    /// is not taken, and each copper pair is counted and named; above it,
    /// it sets every copper pair, the default class's larger clearance
    /// winning where both are set.
    #[test]
    fn the_board_min_clearance_is_a_board_minimum() -> Result<(), String> {
        let track = RuleKey::Clearance {
            a: CopperObject::Track,
            b: CopperObject::Track,
        };
        let profile = DesignRules::default();
        let RuleValue::Length(fab) = track.get(&profile) else {
            return Err("a clearance is a length".into());
        };
        let file = |min: f64, class: Option<f64>| {
            let class = class.map_or(String::new(), |c| {
                format!(r#","net_settings":{{"classes":[{{"name":"Default","clearance":{c}}}]}}"#)
            });
            format!(
                r#"{{"board":{{"design_settings":{{"rules":{{"min_clearance":{min}}}}}}}{class}}}"#
            )
        };
        // Looser than the profile: not taken, ten pairs named.
        let rules = read(file(fab / 2.0, None).as_bytes()).unwrap_or_default();
        assert_eq!(rules.board.clearance, Some(fab / 2.0));
        let mut report = Report::default();
        let taken = overrides(&rules, &profile, &mut report);
        assert!(taken.is_empty(), "{taken:?}");
        assert_eq!(report.count(Case::RuleLooserThanFab), 10);
        assert!(
            report
                .looser_rules()
                .iter()
                .all(|r| r.kicad_setting == "min_clearance"),
            "{:?}",
            report.looser_rules()
        );
        // Tighter: every copper pair; the class's larger value governs.
        let rules = read(file(fab + 0.05, Some(fab + 0.1)).as_bytes()).unwrap_or_default();
        let mut report = Report::default();
        let taken = overrides(&rules, &profile, &mut report);
        assert_eq!(report.count(Case::RuleLooserThanFab), 0);
        assert_eq!(taken.len(), 10);
        assert!(taken.contains(&(track, RuleValue::Length(fab + 0.1))));
        let rules = read(file(fab + 0.1, Some(fab + 0.05)).as_bytes()).unwrap_or_default();
        let taken = overrides(&rules, &profile, &mut Report::default());
        assert!(taken.contains(&(track, RuleValue::Length(fab + 0.1))));
        Ok(())
    }
}
