//! A waiver accepts the value it was given for and no worse one, against
//! the limit it was given against and no stricter one.
use super::Fallible;
use crate::diagnostics::{Measured, Sense, Unit};
use crate::fabrication::{FabricationCommand, PreflightItem, preflight};
use crate::model::production::{Waiver, accepted_words};
use crate::rules::{RuleKey, RuleValue, RulesCommand};
use crate::{Engine, Id, Operation};

const CODE: &str = "board.track_width";

fn set(engine: &mut Engine, key: RuleKey, mm: f64) -> Fallible<()> {
    let command = RulesCommand::SetOverride {
        key,
        value: RuleValue::Length(mm),
    };
    engine.apply(engine.revision(), Operation::Rules(command))?;
    Ok(())
}

/// The board's own minimum track width, with the default kept above it.
fn minimum(engine: &mut Engine, mm: f64) -> Fallible<()> {
    set(engine, RuleKey::TrackWidthDefault, mm.max(0.5))?;
    set(engine, RuleKey::TrackWidthMin, mm)
}

/// The showcase whose 0.2 mm signal tracks are under its own 0.4 mm
/// minimum, and the key of one of them.
fn narrow() -> Fallible<(Engine, Id, String)> {
    let mut engine = Engine::showcase()?;
    minimum(&mut engine, 0.4)?;
    let id = engine
        .project()
        .tracks
        .iter()
        .find(|t| (t.width - 0.2).abs() < 1e-9)
        .ok_or("a 0.2 mm track")?
        .id;
    let key = preflight::key(CODE, &[crate::diagnostics::ObjectRef::Track { id }]);
    let found = item(&engine, &key)?;
    assert!(found.blocks() && found.lapsed.is_none());
    assert_eq!(
        (found.measured[0].value, found.measured[0].limit),
        (0.2, 0.4)
    );
    Ok((engine, id, key))
}

fn item(engine: &Engine, key: &str) -> Fallible<PreflightItem> {
    preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?
    .items
    .into_iter()
    .find(|i| i.key == key)
    .ok_or_else(|| format!("no finding {key}").into())
}

fn waive(engine: &mut Engine, key: &str) -> Fallible<()> {
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![key.to_owned()],
            reason: "the fabricator takes it".into(),
            profile: None,
        }),
    )?;
    Ok(())
}

/// The document with one track's width changed, as a file holds it.
fn widened(engine: &Engine, id: Id, mm: f64) -> Fallible<Engine> {
    let mut file = engine.to_file();
    for t in file.project.tracks.iter_mut().filter(|t| t.id == id) {
        t.width = mm;
    }
    Ok(Engine::from_file(file)?)
}

#[test]
fn a_waiver_holds_for_the_same_and_better_and_not_for_worse() -> Fallible<()> {
    let (mut engine, id, key) = narrow()?;
    waive(&mut engine, &key)?;
    let held = item(&engine, &key)?;
    let waiver = held.waiver.clone().ok_or("the waiver")?;
    assert!(!held.blocks() && held.lapsed.is_none());
    assert_eq!(
        accepted_words(&waiver.accepted).as_deref(),
        Some("0.2 mm, needs at least 0.4 mm")
    );
    for (mm, holds) in [(0.2, true), (0.3, true), (0.199, false), (0.199_999, false)] {
        let other = widened(&engine, id, mm)?;
        let found = item(&other, &key)?;
        assert_eq!(found.waiver.is_some(), holds, "{mm}: {found:?}");
        assert_eq!(found.blocks(), !holds, "{mm}");
        if !holds {
            let text = found.lapsed.ok_or("why it lapsed")?.text;
            // Never shown as what it is not.
            assert_eq!(text, format!("Was accepted at 0.2 mm, is now {mm} mm"));
        }
    }
    // Issues lists it again, and says why.
    let worse = widened(&engine, id, 0.15)?;
    for (board, listed) in [(&engine, false), (&worse, true)] {
        let report = crate::issues::report(
            board.project(),
            board.drafts(),
            &crate::issues::IssueScope::All,
            board.check_settings(),
        )?;
        let row = report.rows.iter().find(|r| {
            std::iter::once(&r.finding)
                .chain(&r.also)
                .any(|f| preflight::key(&f.code, &f.objects) == key)
        });
        assert_eq!(row.is_some(), listed);
        let cause = row.and_then(|r| r.detail.cause.clone()).unwrap_or_default();
        assert_eq!(cause.contains("Was accepted at 0.2 mm"), listed, "{cause}");
    }
    Ok(())
}

#[test]
fn a_stricter_limit_ends_a_waiver_and_a_looser_one_does_not() -> Fallible<()> {
    let (mut engine, id, key) = narrow()?;
    waive(&mut engine, &key)?;
    for (mm, holds) in [(0.35, true), (0.4, true), (0.45, false)] {
        let mut other = Engine::from_file(engine.to_file())?;
        minimum(&mut other, mm)?;
        assert_eq!(item(&other, &key)?.waiver.is_some(), holds, "{mm}");
    }
    let mut stricter = Engine::from_file(engine.to_file())?;
    minimum(&mut stricter, 0.45)?;
    let text = item(&stricter, &key)?.lapsed.ok_or("why")?.text;
    assert_eq!(
        text,
        "Was accepted at 0.2 mm (needs at least 0.4 mm), is now 0.2 mm (needs at least 0.45 mm)"
    );
    // Better, yet farther from its limit than was accepted.
    let wider = widened(&engine, id, 0.25)?;
    let mut wider = Engine::from_file(wider.to_file())?;
    minimum(&mut wider, 0.5)?;
    assert!(item(&wider, &key)?.blocks());
    // Given again, it holds for what it is now.
    waive(&mut wider, &key)?;
    assert!(!item(&wider, &key)?.blocks());
    Ok(())
}

#[test]
fn undo_save_and_reopen_keep_what_was_accepted() -> Fallible<()> {
    let (mut engine, id, key) = narrow()?;
    // A width no short decimal is.
    engine = widened(&engine, id, 0.1 + 0.1 / 3.0)?;
    let before = engine.project().production.clone();
    waive(&mut engine, &key)?;
    let after = engine.project().production.clone();
    assert_eq!(after.waivers[0].accepted[0].value, 0.1 + 0.1 / 3.0);
    engine.undo()?;
    assert_eq!(engine.project().production, before);
    assert!(item(&engine, &key)?.blocks());
    engine.redo()?;
    assert_eq!(engine.project().production, after);
    let reread = crate::format::decode(&crate::format::encode(&engine.to_file())?)?;
    let engine = Engine::from_file(reread)?;
    assert_eq!(engine.project().production, after);
    assert!(!item(&engine, &key)?.blocks());
    Ok(())
}

/// A project saved before waivers held values: its waiver counts for
/// nothing, says so, and holds once given again.
#[test]
fn a_waiver_without_its_value_counts_for_nothing_until_given_again() -> Fallible<()> {
    let (engine, _, key) = narrow()?;
    let mut file = engine.to_file();
    file.project.production.waivers = vec![Waiver {
        key: key.clone(),
        reason: "before".into(),
        accepted: Vec::new(),
        footprints: Vec::new(),
        given_for: None,
    }];
    let mut engine = Engine::from_file(file)?;
    let found = item(&engine, &key)?;
    assert!(found.blocks());
    assert_eq!(
        found.lapsed.ok_or("why")?.text,
        "Was accepted without its value; accept it again"
    );
    waive(&mut engine, &key)?;
    assert!(!item(&engine, &key)?.blocks());
    Ok(())
}

/// Another object under another id is another finding: no waiver goes over
/// to it, though it lies where the waived one lay.
#[test]
fn a_replaced_object_is_not_accepted() -> Fallible<()> {
    let (mut engine, id, key) = narrow()?;
    waive(&mut engine, &key)?;
    let mut file = engine.to_file();
    let new = file.project.allocate()?;
    for t in file.project.tracks.iter_mut().filter(|t| t.id == id) {
        t.id = new;
    }
    let engine = Engine::from_file(file)?;
    assert!(item(&engine, &key).is_err());
    let other = preflight::key(CODE, &[crate::diagnostics::ObjectRef::Track { id: new }]);
    let found = item(&engine, &other)?;
    assert!(found.blocks() && found.lapsed.is_none());
    // Its waiver still names the one that is gone, and accepts nothing.
    assert!(engine.project().production.waiver(&key).is_some());
    Ok(())
}

/// The drawing asks the fabricator to confirm what was accepted, and
/// nothing a waiver no longer accepts.
#[test]
fn the_drawing_states_the_accepted_value_of_waivers_that_hold() -> Fallible<()> {
    let (mut engine, id, key) = narrow()?;
    waive(&mut engine, &key)?;
    let notes = |engine: &Engine| -> Fallible<Vec<String>> {
        Ok(crate::fabrication::drawing::data(
            &crate::fabrication::ProductionSnapshot::resolve(engine.project(), None)?,
            &Default::default(),
            engine.check_settings(),
        )?
        .notes)
    };
    let said = "Accepted: 0.2 mm, needs at least 0.4 mm.";
    assert!(notes(&engine)?.iter().any(|n| n.contains(said)));
    let worse = widened(&engine, id, 0.15)?;
    assert!(!notes(&worse)?.iter().any(|n| n.starts_with("Waived")));
    Ok(())
}

/// Numbers accept numbers: by side, by which number it is, and within the
/// check's own slack, never across units.
#[test]
fn a_number_accepts_the_same_number_no_worse() {
    let at_least = |v: f64, l: f64| Measured::at_least(v, l, Unit::Mm, 1e-6);
    let at_most = |v: f64, l: f64| Measured::at_most(v, l, Unit::Mm, 1e-6);
    for (then, now, accepted) in [
        (at_least(0.1, 0.2), at_least(0.1, 0.2), true),
        (at_least(0.1, 0.2), at_least(0.1 - 5e-7, 0.2), true),
        (at_least(0.1, 0.2), at_least(0.1 - 5e-6, 0.2), false),
        (at_least(0.1, 0.2), at_least(0.15, 0.2), true),
        (at_least(0.1, 0.2), at_least(0.15, 0.3), false),
        (at_least(0.1, 0.2), at_least(0.05, 0.1), false),
        (at_most(2.0, 1.0), at_most(1.5, 1.0), true),
        (at_most(2.0, 1.0), at_most(2.5, 1.0), false),
        (at_most(2.0, 1.0), at_most(2.0, 0.5), false),
        // Too short is not too long.
        (at_least(9.0, 10.0), at_most(9.0, 8.0), false),
        (at_least(0.1, 0.2), at_least(0.1, 0.2).of("drill"), false),
        (
            at_least(0.1, 0.2),
            Measured::at_least(0.1, 0.2, Unit::Degrees, 1e-6),
            false,
        ),
    ] {
        assert_eq!(then.accepts(&now), accepted, "{then:?} {now:?}");
    }
    assert_eq!(at_most(1.0, 0.5).sense, Sense::AtMost);
}

/// Never-waivable findings stay so, whatever a file says was accepted.
#[test]
fn a_value_makes_nothing_waivable() -> Fallible<()> {
    let mut engine = Engine::showcase()?;
    // The pours are computed for other rules: stale.
    set(
        &mut engine,
        RuleKey::Clearance {
            a: crate::rules::CopperObject::Fill,
            b: crate::rules::CopperObject::ViaHole,
        }
        .normalized(),
        1.0,
    )?;
    let stale = preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?
    .items
    .into_iter()
    .find(|i| i.code == "fill.stale")
    .ok_or("a stale pour")?
    .key;
    assert!(waive(&mut engine, &stale).is_err());
    let mut file = engine.to_file();
    file.project.production.waivers = vec![Waiver {
        key: stale.clone(),
        reason: String::new(),
        accepted: vec![Measured::at_least(0.0, 1.0, Unit::Mm, 0.0)],
        footprints: Vec::new(),
        given_for: None,
    }];
    let engine = Engine::from_file(file)?;
    let found = item(&engine, &stale)?;
    assert!(found.blocks() && found.lapsed.is_none());
    Ok(())
}

/// A pad's id is its footprint's: on another footprint it is other copper,
/// which the waiver did not accept, with a value or without one.
#[test]
fn a_pad_on_another_footprint_is_not_accepted() -> Fallible<()> {
    let mut engine = Engine::from_file(super::preflight::close_parts()?.to_file())?;
    let key = preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?
    .items
    .into_iter()
    .find(|i| i.code == "board.clearance" && i.offers_waiver())
    .ok_or("a pad clearance")?
    .key;
    waive(&mut engine, &key)?;
    assert!(item(&engine, &key)?.waiver.is_some());
    assert_eq!(
        engine.project().production.waivers[0].footprints.len(),
        2,
        "one for each pad"
    );
    for (change, text) in [
        (
            Some(crate::ids::DefinitionId(Id::MAX)),
            "Was accepted on another footprint",
        ),
        (None, "Was accepted without its footprint; accept it again"),
    ] {
        let mut file = engine.to_file();
        let waiver = &mut file.project.production.waivers[0];
        match change {
            Some(other) => waiver.footprints[0] = other,
            None => waiver.footprints.clear(),
        }
        let found = item(&Engine::from_file(file)?, &key)?;
        assert!(found.blocks());
        assert_eq!(found.lapsed.ok_or("why")?.text, text);
    }
    Ok(())
}
