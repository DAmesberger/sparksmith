//! The shipped Standard profile, the one a new board starts on
//! (`library/profiles/standard.json`): it loads, cites every rule, is no
//! looser than any shipped fabricator's profile at its standard options, and
//! a new project on it routes and fills at once and cannot lose it.
use sparksmith_core::rules::{RuleKey, RuleProfile, RuleValue, RulesCommand};
use sparksmith_core::{Engine, Operation};
use sparksmith_io::rule_profiles::{self, ProfileLibrary};
use sparksmith_io::shipped::ShippedLibrary;
use std::collections::BTreeMap;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// This checkout's shipped library.
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

/// The shipped profiles by id, each at its options' defaults.
fn shipped() -> Fallible<BTreeMap<String, RuleProfile>> {
    Ok(
        rule_profiles::shipped_profiles(&ShippedLibrary::at(&root())?)?
            .into_iter()
            .map(|p| (p.id.clone(), p))
            .collect(),
    )
}

fn standard() -> Fallible<RuleProfile> {
    Ok(shipped()?.remove("standard").ok_or("standard")?)
}

#[test]
fn it_is_the_shipped_default_and_no_fabricators_own() -> Fallible {
    let library = ShippedLibrary::at(&root())?;
    let defaults = sparksmith_io::new_project::load(&library)?;
    assert_eq!(defaults.fab_profile, "standard");
    let p = standard()?;
    assert_eq!(p.name, "Standard");
    assert_eq!(p.fabricator, None);
    assert!(p.options().is_empty(), "nothing to choose");
    let boards = p.boards.as_ref().ok_or("boards")?;
    assert_eq!(boards.copper_layers, [1, 2, 4, 6]);
    assert!(!boards.source.is_empty());
    let layers = u32::try_from(defaults.board.stackup.copper.len())?;
    assert!(
        boards.copper_layers.contains(&layers),
        "the new board's own count"
    );
    assert_eq!(rule_profiles::decode(&rule_profiles::encode(&p)?)?, p);
    Ok(())
}

/// Every rule has a source: a fabricator's page with its date for a limit,
/// and words that say what it is for any other.
#[test]
fn every_rule_is_cited() -> Fallible {
    let basis = ShippedLibrary::at(&root())?
        .root()
        .join("profiles/basis/standard.json");
    let basis: BTreeMap<String, String> = serde_json::from_slice(&std::fs::read(basis)?)?;
    let p = standard()?;
    for key in RuleKey::all() {
        let name = key.name();
        // It makes no filled vias, so it has no sizes for them.
        if matches!(key, RuleKey::CappedDrillMin | RuleKey::CappedDrillMax) {
            assert!(p.entry(key).is_none() && !key.is_set(p.rules()), "{name}");
            continue;
        }
        let entry = p.entry(key).ok_or_else(|| format!("no {name}"))?;
        assert!(entry.cases.is_empty(), "{name}");
        assert!(!entry.source.trim().is_empty(), "{name}");
        let limit = basis
            .get(&name)
            .ok_or_else(|| format!("{name}: no basis"))?;
        // A formula of other rules is as sourced as they are.
        let formula = matches!(entry.setting, sparksmith_core::rules::Setting::Formula(_));
        if limit == "profile_limit" && !formula {
            assert!(
                entry.source.contains("https://") && entry.source.contains("read 2026-11-"),
                "{name}: {}",
                entry.source
            );
        }
    }
    Ok(())
}

/// A board that passes is one each shipped profile's fabricator makes at
/// its standard values: every rule is at least as strict as that profile's
/// (a least value as large, a greatest ratio as small). Read from the data.
#[test]
fn every_limit_is_at_least_as_strict_as_the_fabricators_standard_ones() -> Fallible {
    let mut profiles = shipped()?;
    let own = profiles.remove("standard").ok_or("standard")?;
    for id in ["jlcpcb-2-layer", "jlcpcb-4-layer"] {
        let p = profiles.get(id).ok_or(id)?;
        assert_eq!(
            p.chosen().get("limits").map(String::as_str),
            Some("recommended")
        );
    }
    assert!(profiles.len() >= 4);
    for (id, other) in &profiles {
        for key in RuleKey::all() {
            let name = key.name();
            // Standard makes no filled vias, stricter than any size of them.
            if matches!(key, RuleKey::CappedDrillMin | RuleKey::CappedDrillMax) {
                let fill = sparksmith_core::model::ViaFill::Capped;
                assert!(matches!(
                    sparksmith_core::rules::drills::through_fill(&own.drill_rules, fill),
                    sparksmith_core::rules::drills::Verdict::NotMade(_)
                ));
                continue;
            }
            let (ours, theirs) = (key.get(own.rules()), key.get(other.rules()));
            let strict = match (ours, theirs) {
                (RuleValue::Length(a), RuleValue::Length(b)) => a + 1e-9 >= b,
                (RuleValue::Count(a), RuleValue::Count(b)) => a >= b,
                (RuleValue::Ratio(a), RuleValue::Ratio(b)) => {
                    b.is_none_or(|b| a.is_some_and(|a| a <= b + 1e-9))
                }
                // Defaults and switches, not limits.
                _ => true,
            };
            assert!(strict, "{name}: {ours:?}, {id} {theirs:?}");
        }
    }
    // The defaults a tool starts from are allowed by the limits.
    let r = own.rules();
    assert!(r.copper.track_width_default >= r.copper.track_width_min);
    assert!(r.holes.via_drill_default >= r.holes.drill_min);
    let ring = (r.holes.via_diameter_default - r.holes.via_drill_default) / 2.0;
    assert!(ring + 1e-9 >= r.holes.via_annular_ring_min);
    Ok(())
}

/// A new project's rules let a track, a via and a pour go in with nothing
/// chosen first. The parts are the test fixture's, on the new project's
/// rules.
#[test]
fn routing_and_filling_work_at_once() -> Fallible {
    let context = sparksmith_io::standards::shipped_context()?;
    let new = sparksmith_io::new_project::engine_on(standard()?, &context)?;
    let mut file = Engine::demo()?.to_file();
    file.project.rules = new.project().rules.clone();
    let mut engine = Engine::from_file(file)?;
    let net = engine.project().nets.first().ok_or("a net")?.id;
    let via = sparksmith_core::layout::class_via(engine.project(), net, [30.0, 30.0]);
    for operation in [
        serde_json::json!({"editor": "Layout", "operation": {
            "command": "layout.route", "params": {"request": {
                "net": net, "start": [20.0, 30.0], "end": [30.0, 30.0], "width": 0.25,
                "layer": "Top"}}}}),
        serde_json::json!({"editor": "Layout", "operation": {
            "command": "layout.add_via", "params": {"via": via}}}),
        serde_json::json!({"editor": "Fill", "operation": {
            "command": "fill.add", "params": {"fill": {
                "net": net, "layer": "Bottom", "region": "board"}}}}),
    ] {
        engine.apply(engine.revision(), serde_json::from_value(operation)?)?;
    }
    assert_eq!(engine.refill_stale()?, 1);
    assert!(engine.project().fills.iter().all(|f| f.computed.is_some()));
    Ok(())
}

/// No undo reaches a state without a profile, and neither does a choice of
/// another one taken back.
#[test]
fn a_new_project_cannot_lose_its_profile() -> Fallible {
    let context = sparksmith_io::standards::shipped_context()?;
    let mut engine = sparksmith_io::new_project::engine_on(standard()?, &context)?;
    assert!(!engine.can_undo());
    assert_eq!(engine.revision(), 0, "as scripts start");
    let other = shipped()?.remove("jlcpcb-2-layer").ok_or("another")?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(other, false)),
    )?;
    engine.undo()?;
    assert!(!engine.can_undo());
    assert_eq!(engine.project().rules.profile, Some(standard()?));
    let run = sparksmith_io::new_project::run_engine(&context)?;
    assert!(run.project().rules.profile.is_some() && !run.can_undo());
    Ok(())
}

/// Every host's lookup ends in a profile or names the one that is missing:
/// with a default, with one that is gone (refused; a window starts on
/// Standard and says so) and with none.
#[test]
fn no_lookup_for_a_new_board_is_empty() -> Fallible {
    let dir = tempfile::tempdir()?;
    let library = ProfileLibrary::new(dir.path().join("none"), ShippedLibrary::at(&root())?);
    let id = |default| -> Fallible<String> {
        Ok(rule_profiles::for_new_board(&library, None, default)?.id)
    };
    assert_eq!(id(None)?, "standard");
    assert!(id(Some("gone")).is_err_and(|e| e.to_string().contains("gone")));
    let (profile, said) =
        rule_profiles::for_new_board_shown(&library, Ok(Some("gone".to_owned())))?;
    assert_eq!(profile.id, "standard");
    assert!(said.is_some_and(|said| said.contains("‘gone’") && said.contains("Standard")));
    assert_eq!(id(Some("pcbway-2-layer"))?, "pcbway-2-layer");
    Ok(())
}
