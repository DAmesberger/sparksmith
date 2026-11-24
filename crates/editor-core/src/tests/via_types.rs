//! Filled and capped vias: one setting of one via, held to the fab
//! profile's drill rules (`layout::via_fill`). The limits here are test
//! numbers, not a fabricator's.
use crate::fabrication::preflight::{self, FabricationCommand, PreflightItem};
use crate::layout::{self, LayoutCommand, ViaRequest, via_fill};
use crate::library::PadKind;
use crate::model::{Project, Via, ViaFill, ViaSpan};
use crate::rules::{self, RuleKey, RuleProfile, RuleValue, RulesCommand, fixtures};
use crate::{Engine, Id, Operation};

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The prototype fixture that fills holes of 0.2 to 0.4 mm, told by size;
/// `stated` false: it offers the fill without saying what it makes.
fn filling(stated: bool) -> Fallible<RuleProfile> {
    let mut value: serde_json::Value = serde_json::from_str(fixtures::PROTOTYPE_JSON)?;
    let p = &mut value["fab_profile"];
    p["id"] = "test-filling".into();
    p["name"] = "Test filling".into();
    p["drill_rules"] = serde_json::json!([{
        "id": "through", "kind": "mechanical", "layers": {"span": "board"},
        "fills": [
            {"fill": "none"},
            {"fill": "capped", "not_stated": !stated, "source": "Test fixture.",
             "order": {"told_by_size": true}}
        ]
    }]);
    for (key, mm) in [("capped_drill_min", 0.2), ("capped_drill_max", 0.4)] {
        p["rules"][key] = serde_json::json!({"value": mm, "source": "Test fixture."});
    }
    Ok(fixtures::parse(&value.to_string())?)
}

fn under(profile: RuleProfile) -> Fallible<Engine> {
    let mut engine = Engine::demo()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(profile, true)),
    )?;
    Ok(engine)
}

fn via(id: Id, net: Id, position: [f64; 2], drill: f64, fill: ViaFill) -> Via {
    Via {
        id,
        net,
        position,
        drill,
        diameter: drill + 0.3,
        span: ViaSpan::Through,
        tented: true,
        fill,
        locked: false,
    }
}

/// The board with vias of these drills and fills in its far corner.
fn with(engine: &Engine, vias: &[(f64, ViaFill)]) -> Fallible<Project> {
    let mut project = engine.project().clone();
    let net = project.nets.first().map(|n| n.id).ok_or("no net")?;
    let [w, h] = project.board_size;
    let first = project.next_id;
    project.next_id += vias.len() as Id;
    project.vias = vias
        .iter()
        .enumerate()
        .map(|(k, (drill, fill))| {
            via(
                first + k as Id,
                net,
                [w - 1.0 - k as f64, h - 1.0],
                *drill,
                *fill,
            )
        })
        .collect();
    Ok(project)
}

fn codes(project: &Project) -> Vec<&'static str> {
    via_fill::findings(project)
        .into_iter()
        .filter_map(|f| {
            [
                via_fill::NOT_MADE,
                via_fill::NOT_STATED,
                via_fill::IN_PAD_OPEN,
                via_fill::TOLD,
            ]
            .into_iter()
            .find(|c| f.code.as_str() == *c)
        })
        .collect()
}

/// Red without the profile check: a board with a filled via passes under a
/// profile that makes open vias only.
#[test]
fn a_fill_the_profile_does_not_make_is_found() -> Fallible<()> {
    let plain = under(fixtures::prototype()?)?;
    assert!(codes(&with(&plain, &[(0.3, ViaFill::None)])?).is_empty());
    let filled = with(&plain, &[(0.3, ViaFill::Capped)])?;
    assert_eq!(codes(&filled), [via_fill::NOT_MADE]);
    let found = via_fill::findings(&filled);
    assert_eq!(
        found.first().map(|f| f.message.as_str()),
        Some(
            format!(
                "{} does not make filled and capped vias; the board has 1 via",
                fixtures::prototype()?.name
            )
            .as_str()
        )
    );
    // What it needs of a profile is said.
    assert!(found.iter().all(|f| f.requirement.is_some()));
    let filling = under(filling(true)?)?;
    assert!(codes(&with(&filling, &[(0.3, ViaFill::Capped)])?).is_empty());
    Ok(())
}

/// Red without the size check: 0.15 and 0.5 mm pass a 0.2 to 0.4 mm rule.
#[test]
fn a_filled_hole_outside_the_sizes_made_is_found() -> Fallible<()> {
    let engine = under(filling(true)?)?;
    for (drill, expected) in [
        (0.15, vec![via_fill::NOT_MADE]),
        (0.2, vec![]),
        (0.4, vec![]),
        (0.5, vec![via_fill::NOT_MADE]),
    ] {
        assert_eq!(
            codes(&with(&engine, &[(drill, ViaFill::Capped)])?),
            expected,
            "{drill}"
        );
        // The sizes are the filled vias' own.
        assert!(codes(&with(&engine, &[(drill, ViaFill::None)])?).is_empty());
    }
    Ok(())
}

/// Red without the told-by check; red the other way when every mixed size
/// warns whatever the fabricator is told by.
#[test]
fn filled_and_open_vias_of_one_size_are_found_where_size_tells_them_apart() -> Fallible<()> {
    let engine = under(filling(true)?)?;
    let mixed = [(0.3, ViaFill::Capped), (0.3, ViaFill::None)];
    assert_eq!(codes(&with(&engine, &mixed)?), [via_fill::TOLD]);
    let apart = [(0.3, ViaFill::Capped), (0.25, ViaFill::None)];
    assert!(codes(&with(&engine, &apart)?).is_empty());
    let mut untold = filling(true)?;
    for rule in &mut untold.drill_rules {
        for fill in &mut rule.fills {
            fill.order = None;
        }
    }
    assert!(codes(&with(&under(untold)?, &mixed)?).is_empty());
    Ok(())
}

/// Red without the paste check: an open via in a paste opening passes.
#[test]
fn an_open_via_under_paste_is_found_and_a_filled_one_is_not() -> Fallible<()> {
    let engine = under(filling(true)?)?;
    let pad = layout::placed_pads(engine.project())
        .into_iter()
        .find(|p| p.pad.kind == PadKind::Smd && p.pad.hole.is_none())
        .ok_or("no pad")?;
    for (fill, expected) in [
        (ViaFill::None, vec![via_fill::IN_PAD_OPEN]),
        (ViaFill::Capped, vec![]),
    ] {
        let mut project = with(&engine, &[(0.3, fill)])?;
        for v in &mut project.vias {
            v.position = pad.center();
        }
        assert_eq!(codes(&project), expected);
    }
    Ok(())
}

#[test]
fn a_fill_is_set_on_the_named_vias_in_one_step() -> Fallible<()> {
    let mut engine = under(filling(true)?)?;
    let net = engine
        .project()
        .nets
        .first()
        .map(|n| n.id)
        .ok_or("no net")?;
    let [w, h] = engine.project().board_size;
    for k in 0..2 {
        engine.apply(
            engine.revision(),
            Operation::Layout(LayoutCommand::AddVia {
                via: ViaRequest::fixture(net, [w - 1.0 - 2.0 * f64::from(k), h - 1.0]),
            }),
        )?;
    }
    let ids: Vec<Id> = engine.project().vias.iter().map(|v| v.id).collect();
    let fills = |e: &Engine| -> Vec<ViaFill> { e.project().vias.iter().map(|v| v.fill).collect() };
    let set = |ids: Vec<Id>| {
        Operation::Layout(LayoutCommand::SetVias {
            ids,
            fill: ViaFill::Capped,
        })
    };
    // One unknown via refuses the whole command.
    let first = ids.first().copied().ok_or("no via")?;
    assert!(
        engine
            .apply(engine.revision(), set(vec![first, Id::MAX]))
            .is_err()
    );
    assert!(fills(&engine).iter().all(ViaFill::is_none));
    engine.apply(engine.revision(), set(ids))?;
    assert_eq!(fills(&engine), [ViaFill::Capped; 2]);
    // It is saved and read back.
    assert_eq!(fills(&Engine::from_file(engine.to_file())?), fills(&engine));
    engine.undo()?;
    assert!(fills(&engine).iter().all(ViaFill::is_none));
    Ok(())
}

fn item(engine: &Engine, code: &str) -> Fallible<PreflightItem> {
    preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?
    .items
    .into_iter()
    .find(|i| i.code == code)
    .ok_or_else(|| format!("no {code}").into())
}

/// Red when either code takes a waiver like any other finding.
#[test]
fn not_made_is_never_waived_and_not_stated_only_with_a_reason() -> Fallible<()> {
    let filled = |profile: RuleProfile| -> Fallible<Engine> {
        let engine = under(profile)?;
        let mut file = engine.to_file();
        file.project = with(&engine, &[(0.3, ViaFill::Capped)])?;
        Ok(Engine::from_file(file)?)
    };
    let waive = |key: &str, reason: &str| {
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![key.to_owned()],
            reason: reason.to_owned(),
            profile: None,
        })
    };

    let mut engine = filled(fixtures::prototype()?)?;
    let found = item(&engine, via_fill::NOT_MADE)?;
    assert!(found.blocks() && !found.offers_waiver());
    let refused = engine.apply(engine.revision(), waive(&found.key, "they make it"));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.rule() == Some("fabrication.not_waivable")),
        "{refused:?}"
    );

    let mut engine = filled(filling(false)?)?;
    let found = item(&engine, via_fill::NOT_STATED)?;
    assert!(found.blocks() && found.offers_waiver());
    // There is no number to accept.
    assert!(found.measured.is_empty());
    let refused = engine.apply(engine.revision(), waive(&found.key, " "));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.rule() == Some("fabrication.waiver_reason")),
        "{refused:?}"
    );
    engine.apply(
        engine.revision(),
        waive(&found.key, "they confirmed 0.3 mm by mail"),
    )?;
    assert!(!item(&engine, via_fill::NOT_STATED)?.blocks());
    Ok(())
}

/// Red when a waiver of what is not stated goes by its key alone: it then
/// holds under another profile, another reading of it and other vias.
#[test]
fn a_not_stated_waiver_is_for_its_profile_and_its_vias() -> Fallible<()> {
    let filled = |profile: RuleProfile, vias: &[(f64, ViaFill)]| -> Fallible<Engine> {
        let engine = under(profile)?;
        let mut file = engine.to_file();
        file.project = with(&engine, vias)?;
        Ok(Engine::from_file(file)?)
    };
    let one = [(0.3, ViaFill::Capped)];
    let mut engine = filled(filling(false)?, &one)?;
    let key = item(&engine, via_fill::NOT_STATED)?.key;
    engine.apply(
        engine.revision(),
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![key],
            reason: "they confirmed 0.3 mm by mail".into(),
            profile: None,
        }),
    )?;
    let waivers = engine.project().production.waivers.clone();
    let given = waivers.first().and_then(|w| w.given_for.clone());
    let given = given.ok_or("what it was given for")?;
    assert_eq!(given.profile, "test-filling");
    assert_eq!(given.vias.len(), 1);
    assert!(given.vias.iter().all(|v| v.drill_nm == 300_000));
    assert!(waivers.iter().all(|w| w.accepted.is_empty()));

    let lapsed = |profile: RuleProfile, vias: &[(f64, ViaFill)]| -> Fallible<Option<String>> {
        let engine = filled(profile, vias)?;
        let mut file = engine.to_file();
        file.project.production.waivers.clone_from(&waivers);
        let found = item(&Engine::from_file(file)?, via_fill::NOT_STATED)?;
        assert_eq!(found.blocks(), found.lapsed.is_some());
        Ok(found.lapsed.map(|l| l.text))
    };
    // The same, and the same with one more via of the size.
    assert_eq!(lapsed(filling(false)?, &one)?, None);
    let two = [(0.3, ViaFill::Capped), (0.3, ViaFill::Capped)];
    assert_eq!(lapsed(filling(false)?, &two)?, None);
    let sizes = Some("Was accepted for vias of other sizes".to_owned());
    assert_eq!(lapsed(filling(false)?, &[(0.25, ViaFill::Capped)])?, sizes);
    let more = [(0.3, ViaFill::Capped), (0.25, ViaFill::Capped)];
    assert_eq!(lapsed(filling(false)?, &more)?, sizes);
    let other = Some("Was accepted under another fab profile".to_owned());
    let mut renamed = filling(false)?;
    renamed.id = "test-filling-2".into();
    assert_eq!(lapsed(renamed, &one)?, other);
    let mut read = filling(false)?;
    read.source.date = "2026-01-02".into();
    assert_eq!(lapsed(read, &one)?, other);
    let mut said = filling(false)?;
    for rule in &mut said.drill_rules {
        rule.source = "Says something else.".into();
    }
    assert_eq!(lapsed(said, &one)?, other);
    Ok(())
}

/// [`filling`] with sources that disagree: 0.15 to 0.2 mm and 0.4 to
/// 0.45 mm are unclear, and a via ring of 0.1 to 0.15 mm.
fn disagreeing() -> Fallible<serde_json::Value> {
    let mut value = serde_json::to_value(rules::ProfileFile {
        format: rules::PROFILE_FILE_FORMAT,
        profile: filling(true)?,
    })?;
    let rules = &mut value["fab_profile"]["rules"];
    for (key, limit, to) in [
        ("capped_drill_min", 0.2, 0.15),
        ("capped_drill_max", 0.4, 0.45),
        ("via_annular_ring_min", 0.15, 0.1),
    ] {
        rules[key] = serde_json::json!({
            "value": limit, "source": "Test fixture.",
            "unclear": {"to": to, "source": "Page A says one, page B the other."}
        });
    }
    Ok(value)
}

/// The test board with one filled via of `drill` and `ring` mm under
/// `profile`, with `waivers`.
fn one_filled(
    profile: &serde_json::Value,
    drill: f64,
    ring: f64,
    waivers: &[crate::model::production::Waiver],
) -> Fallible<Engine> {
    let engine = under(fixtures::parse(&profile.to_string())?)?;
    let mut file = engine.to_file();
    file.project = with(&engine, &[(drill, ViaFill::Capped)])?;
    for v in &mut file.project.vias {
        v.diameter = drill + 2.0 * ring;
    }
    file.project.production.waivers = waivers.to_vec();
    Ok(Engine::from_file(file)?)
}

/// Each side of each boundary. Red when the unclear range is read as made,
/// as not made, or without its ends.
#[test]
fn what_the_sources_disagree_on_is_not_stated() -> Fallible<()> {
    let profile = disagreeing()?;
    for (drill, expected) in [
        (0.14, Some(via_fill::NOT_MADE)),
        (0.149, Some(via_fill::NOT_MADE)),
        (0.15, Some(via_fill::NOT_STATED)),
        (0.199, Some(via_fill::NOT_STATED)),
        (0.2, None),
        (0.4, None),
        (0.401, Some(via_fill::NOT_STATED)),
        (0.45, Some(via_fill::NOT_STATED)),
        (0.451, Some(via_fill::NOT_MADE)),
    ] {
        let engine = one_filled(&profile, drill, 0.2, &[])?;
        assert_eq!(
            codes(engine.project()),
            expected.into_iter().collect::<Vec<_>>(),
            "{drill}"
        );
    }
    // A limit of the project's own leaves nothing to a source.
    let mut engine = one_filled(&profile, 0.16, 0.2, &[])?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::SetOverride {
            key: RuleKey::CappedDrillMin,
            value: RuleValue::Length(0.18),
        }),
    )?;
    assert_eq!(codes(engine.project()), [via_fill::NOT_MADE]);
    Ok(())
}

/// Red when such a finding cannot be waived, needs no reason, cites no
/// source, or keeps its waiver for another hole or other sources.
#[test]
fn what_the_sources_disagree_on_is_waived_with_a_reason_for_that_hole() -> Fallible<()> {
    let profile = disagreeing()?;
    let mut engine = one_filled(&profile, 0.15, 0.2, &[])?;
    let found = item(&engine, via_fill::NOT_STATED)?;
    assert!(found.blocks() && found.offers_waiver() && found.measured.is_empty());
    let cited = found.not_stated.as_ref().map(|n| n.source.as_str());
    assert_eq!(cited, Some("Page A says one, page B the other."));
    let waive = |reason: &str| {
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![found.key.clone()],
            reason: reason.to_owned(),
            profile: None,
        })
    };
    let refused = engine.apply(engine.revision(), waive(""));
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.rule() == Some("fabrication.waiver_reason")),
        "{refused:?}"
    );
    engine.apply(engine.revision(), waive("confirmed by mail"))?;
    let waivers = engine.project().production.waivers.clone();
    let lapsed = |profile: &serde_json::Value, drill: f64| -> Fallible<Option<String>> {
        let engine = one_filled(profile, drill, 0.2, &waivers)?;
        Ok(item(&engine, via_fill::NOT_STATED)?.lapsed.map(|l| l.text))
    };
    assert_eq!(lapsed(&profile, 0.15)?, None);
    assert_eq!(
        lapsed(&profile, 0.16)?.as_deref(),
        Some("Was accepted for vias of other sizes")
    );
    let mut said = profile.clone();
    said["fab_profile"]["rules"]["capped_drill_min"]["unclear"]["to"] = 0.14.into();
    assert_eq!(
        lapsed(&said, 0.15)?.as_deref(),
        Some("Was accepted under another fab profile")
    );
    // A hole nothing allows is not waived by it.
    let engine = one_filled(&profile, 0.14, 0.2, &waivers)?;
    assert!(item(&engine, via_fill::NOT_MADE)?.blocks());
    Ok(())
}

/// Any rule with a limit can have an unclear range: the via ring here, whose
/// finding keeps its own code. Red when only the filled holes read it.
#[test]
fn an_unclear_range_counts_for_any_rule() -> Fallible<()> {
    let profile = disagreeing()?;
    let ring = crate::diagnostics::FindingCode::BoardViaAnnularRing.as_str();
    let mut engine = one_filled(&profile, 0.3, 0.12, &[])?;
    let found = item(&engine, ring)?;
    assert!(found.blocks() && found.offers_waiver());
    assert!(found.not_stated.is_some() && found.needs.is_none());
    let waive = |reason: &str| {
        Operation::Fabrication(FabricationCommand::Waive {
            keys: vec![found.key.clone()],
            reason: reason.to_owned(),
            profile: None,
        })
    };
    assert!(engine.apply(engine.revision(), waive("")).is_err());
    engine.apply(engine.revision(), waive("confirmed by mail"))?;
    assert!(!item(&engine, ring)?.blocks());
    // Past the loosest source it needs another profile, as ever.
    let engine = one_filled(&profile, 0.3, 0.09, &[])?;
    let found = item(&engine, ring)?;
    assert!(found.needs.is_some() && found.not_stated.is_none() && !found.offers_waiver());
    let engine = one_filled(&profile, 0.3, 0.15, &[])?;
    assert!(item(&engine, ring).is_err(), "what every source allows");
    Ok(())
}

/// An unclear range lies past a limit and quotes its sources.
#[test]
fn an_unclear_range_that_is_none_is_refused() -> Fallible<()> {
    let good = disagreeing()?;
    assert!(fixtures::parse(&good.to_string()).is_ok());
    for (key, unclear) in [
        (
            "capped_drill_min",
            serde_json::json!({"to": 0.25, "source": "x"}),
        ),
        (
            "capped_drill_min",
            serde_json::json!({"to": 0.2, "source": "x"}),
        ),
        (
            "capped_drill_max",
            serde_json::json!({"to": 0.35, "source": "x"}),
        ),
        (
            "capped_drill_min",
            serde_json::json!({"to": 0.15, "source": " "}),
        ),
        ("tent_vias", serde_json::json!({"to": 0.15, "source": "x"})),
    ] {
        let mut bad = good.clone();
        bad["fab_profile"]["rules"][key]["unclear"] = unclear;
        assert!(fixtures::parse(&bad.to_string()).is_err(), "{key}");
    }
    Ok(())
}

/// The data itself: every shipped profile states the open through via, and
/// what it says of the filled one carries a source.
#[test]
fn shipped_profiles_source_what_they_say_of_filled_vias() -> Fallible<()> {
    let jlc = fixtures::jlcpcb_example()?;
    let rule = match rules::drills::through_fill(&jlc.drill_rules, ViaFill::Capped) {
        rules::drills::Verdict::Made(Some(rule)) => rule,
        other => return Err(format!("{other:?}").into()),
    };
    assert!(!rule.source.is_empty());
    assert!(!jlc.charges.is_empty());
    let holes = jlc.rules().holes;
    // Made where every page agrees; not stated up to the loosest page.
    assert_eq!(
        (holes.capped_drill_min, holes.capped_drill_max),
        (Some(0.2), Some(0.5))
    );
    for (key, to) in [
        (RuleKey::CappedDrillMin, 0.15),
        (RuleKey::CappedDrillMax, 0.55),
    ] {
        let unclear = jlc.unclear(key).ok_or("what its pages disagree on")?;
        assert_eq!(unclear.to, to);
        // Both sides are cited.
        assert!(unclear.source.contains("pcb-capabilities"));
        assert!(unclear.source.contains("pofv"));
    }
    Ok(())
}

/// A charge falls only where every condition holds of one via.
#[test]
fn a_charge_needs_every_condition() -> Fallible<()> {
    let when: rules::drills::ChargeWhen = serde_json::from_value(serde_json::json!({
        "copper_layers": [4], "fill": "capped",
        "drill_below_mm": 0.3, "diameter_at_most_mm": 0.4
    }))?;
    let (capped, open) = (ViaFill::Capped, ViaFill::None);
    assert!(when.holds(4, capped, 0.25, 0.4));
    assert!(!when.holds(6, capped, 0.25, 0.4));
    assert!(!when.holds(4, open, 0.25, 0.4));
    assert!(!when.holds(4, capped, 0.3, 0.4));
    assert!(!when.holds(4, capped, 0.25, 0.45));
    Ok(())
}
