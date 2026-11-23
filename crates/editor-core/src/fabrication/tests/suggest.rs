//! Profile suggestions on an original connector-like part: the demo
//! board's two-terminal parts with an unplated locating hole drawn
//! 0.175 mm from pad 1's copper, as a connector's datasheet draws its
//! locating pegs beside its contacts. Three invented fabricators publish
//! invented profiles that differ only in the pad-to-unplated-hole
//! clearance; no real fabricator's numbers are used.
use super::Fallible;
use crate::diagnostics::{ObjectRef, Requirement};
use crate::fabrication::suggest::suggest;
use crate::fabrication::{PreflightReport, ProfileSuggestions};
use crate::geometry::Point;
use crate::layout::{LayoutCommand, placed_pads};
use crate::library::{Hole, HoleShape, MaskRule, Pad, PadKind, PadShape, PasteRule};
use crate::model::{Layer, Project};
use crate::rules::{
    CopperObject, Fabricator, RuleEntry, RuleKey, RuleProfile, RuleValue, fixtures,
};
use crate::tests::support::{by_designator, nm, pt};
use crate::{Engine, Query, QueryResult};

/// The drawn gap between the locating hole and pad 1's copper.
const GAP: f64 = 0.175;
/// The locating hole's diameter.
const HOLE: f64 = 0.6;

fn key() -> RuleKey {
    RuleKey::Clearance {
        a: CopperObject::Pad,
        b: CopperObject::NpthHole,
    }
    .normalized()
}

/// The prototype fixture as `id` of fabricator `fabricator`, its
/// pad-to-unplated-hole clearance `clearance`.
fn profile(id: &str, fabricator: &str, clearance: f64) -> Fallible<RuleProfile> {
    let mut p = fixtures::prototype()?;
    p.id = id.into();
    p.name = format!("Profile {id}");
    p.fabricator = Some(fabricator.into());
    p.set(
        key(),
        RuleEntry::value(
            RuleValue::Length(clearance),
            &format!("Invented row: pad to NPTH {clearance} mm"),
        ),
    )?;
    Ok(p)
}

fn fabricator(id: &str) -> Fabricator {
    Fabricator {
        id: id.into(),
        name: format!("Fab {}", id.to_uppercase()),
        website: String::new(),
        source: "Invented for a test".into(),
    }
}

/// The demo board, R1 at (10, 10) and R2 at (25, 10), under `board`, with
/// the shared footprint given an unplated locating hole [`GAP`] left of
/// pad 1's copper.
fn connector_board(board: RuleProfile) -> Fallible<Project> {
    let engine = Engine::demo()?;
    let mut project = engine.project().clone();
    for (designator, x) in [("R1", 10.0), ("R2", 25.0)] {
        crate::layout::apply(
            &mut project,
            LayoutCommand::Move {
                instance: by_designator(&engine, designator)?,
                position: [x, 10.0],
                rotation_deg: 0.0,
                layer: Layer::Top,
            },
        )?;
    }
    let r1 = by_designator(&engine, "R1")?;
    let pad1 = placed_pads(&project)
        .into_iter()
        .find(|p| p.instance == r1 && p.pad.number == "1")
        .ok_or("R1 pad 1")?;
    let bounds = crate::layout::CopperRing::of_placed_pad(&pad1)
        .bounds
        .ok_or("pad 1 has copper")?;
    // In R1's footprint frame (R1 sits unrotated at (10, 10)).
    let centre: Point = [
        bounds.min[0] - GAP - HOLE / 2.0 - 10.0,
        (bounds.min[1] + bounds.max[1]) / 2.0 - 10.0,
    ];
    let footprint = project
        .library
        .footprints
        .iter_mut()
        .find(|f| f.definition.pads.iter().any(|p| p.id == pad1.pad.id))
        .ok_or("R1's footprint")?;
    let id = footprint.definition.next_local;
    footprint.definition.next_local += 1;
    footprint.definition.pads.push(Pad {
        id: crate::ids::PadId(id),
        kind: PadKind::NonPlatedHole,
        terminal: None,
        number: String::new(),
        position: pt(centre),
        rotation_mdeg: Default::default(),
        side: Default::default(),
        shape: PadShape::Circle {
            diameter_nm: nm(HOLE),
        },
        hole: Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: nm(HOLE),
            },
            plated: false,
            offset: pt([0.0, 0.0]),
        }),
        mask: MaskRule::default(),
        paste: PasteRule::default(),
        fiducial_role: None,
        vias: Vec::new(),
    });
    project.rules.profile = Some(board);
    Ok(project)
}

fn suggestions(project: &Project, candidates: &[RuleProfile]) -> Fallible<ProfileSuggestions> {
    Ok(suggest(
        project,
        &Default::default(),
        &Default::default(),
        None,
        candidates,
        &[fabricator("a"), fabricator("b")],
    )?)
}

fn report(project: &Project) -> Fallible<PreflightReport> {
    Ok(crate::fabrication::preflight::report(
        project,
        &Default::default(),
        None,
        &Default::default(),
    )?)
}

/// Under profile A (0.2 mm) the locating holes are profile-limited
/// findings that state what is needed, A's limit and its source, and offer
/// no waiver.
#[test]
fn a_part_hole_closer_than_the_profile_allows_states_what_is_needed() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let report = report(&project)?;
    let limited: Vec<_> = report.items.iter().filter(|i| i.needs.is_some()).collect();
    assert_eq!(limited.len(), 2, "one per placed part: {:#?}", report.items);
    for item in &limited {
        assert_eq!(item.code, "board.pad_hole_clearance");
        assert!(
            !item.offers_waiver(),
            "a correctly drawn part is never waived"
        );
        let needs = item.needs.as_ref().ok_or("needs")?;
        let Requirement::AtMost { rule, value } = &needs.requirement else {
            return Err(format!("{needs:?}").into());
        };
        assert_eq!(rule, &key().name());
        assert!((value - GAP).abs() < 2e-3, "{value}");
        assert_eq!(needs.limit.as_deref(), Some("0.2 mm"));
        assert_eq!(needs.source, "Invented row: pad to NPTH 0.2 mm");
        assert_eq!(needs.profile, "Profile a1");
        assert_eq!(
            needs.text,
            format!(
                "Needs {} ≤ {} mm",
                key().label(),
                needs.measured.trim_end_matches(" mm")
            )
        );
        assert!(needs.measured.starts_with("0.17"), "{}", needs.measured);
        assert!(
            item.objects
                .iter()
                .any(|o| matches!(o, ObjectRef::Pad { .. })),
            "{item:?}"
        );
    }
    // Every other finding keeps its waiver as before.
    assert!(
        report
            .items
            .iter()
            .filter(|i| i.needs.is_none())
            .all(|i| i.offers_waiver() == crate::issues::can_waive(&i.code))
    );
    Ok(())
}

/// Another profile of the board's own fabricator comes before another
/// fabricator's, both of which allow the hole; the board's own profile and
/// a stricter one are not suggested.
#[test]
fn the_same_fabricators_profile_is_suggested_first() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let candidates = [
        profile("b1", "b", 0.15)?,
        profile("a1", "a", 0.2)?,
        profile("a2", "a", 0.15)?,
        profile("b2", "b", 0.19)?,
    ];
    let out = suggestions(&project, &candidates)?;
    assert_eq!(out.profile, "Profile a1");
    assert_eq!(out.fabricator.as_deref(), Some("a"));
    assert_eq!(out.needs.len(), 1, "{:#?}", out.needs);
    let ids: Vec<&str> = out.suggestions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["a2", "b1"]);
    assert!(out.suggestions[0].same_fabricator);
    assert!(!out.suggestions[1].same_fabricator);
    assert_eq!(out.suggestions[1].fabricator_name.as_deref(), Some("Fab B"));
    Ok(())
}

/// With only another fabricator's profile allowing it, that one is
/// suggested.
#[test]
fn another_fabricators_profile_when_only_it_allows_the_board() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let out = suggestions(&project, &[profile("b1", "b", 0.15)?])?;
    let ids: Vec<&str> = out.suggestions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["b1"]);
    assert!(!out.suggestions[0].same_fabricator);
    Ok(())
}

/// When no installed profile allows it, only the requirement is stated.
#[test]
fn with_no_profile_allowing_it_only_the_requirement_is_stated() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let out = suggestions(
        &project,
        &[profile("b2", "b", 0.19)?, profile("a3", "a", 0.18)?],
    )?;
    assert!(out.suggestions.is_empty(), "{:#?}", out.suggestions);
    assert_eq!(out.needs.len(), 1);
    assert!(
        out.needs[0]
            .text
            .starts_with(&format!("Needs {} ≤ 0.17", key().label()))
    );
    Ok(())
}

/// A board its profile allows needs nothing and is offered nothing.
#[test]
fn a_board_the_profile_allows_needs_nothing() -> Fallible<()> {
    let project = connector_board(profile("b1", "b", 0.15)?)?;
    let out = suggestions(&project, &[profile("a2", "a", 0.15)?])?;
    assert!(out.needs.is_empty());
    assert!(out.suggestions.is_empty());
    Ok(())
}

/// The engine's query answers with the same suggestions.
#[test]
fn the_query_answers_as_the_function_does() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let mut file = Engine::demo()?.to_file();
    file.project = project.clone();
    let engine = Engine::from_file(file)?;
    let candidates = vec![profile("b1", "b", 0.15)?, profile("a2", "a", 0.15)?];
    let QueryResult::ProfileSuggestions(answer) = engine.query(Query::ProfileSuggestions {
        profile: None,
        candidates: candidates.clone(),
        fabricators: vec![fabricator("a"), fabricator("b")],
    })?
    else {
        return Err("not suggestions".into());
    };
    assert_eq!(*answer, suggestions(&project, &candidates)?);
    Ok(())
}

/// The engine refuses a waiver of a finding the board needs more of its
/// profile for, naming the need, and changes nothing.
#[test]
fn the_engine_never_waives_a_profile_limited_finding() -> Fallible<()> {
    let project = connector_board(profile("a1", "a", 0.2)?)?;
    let mut file = Engine::demo()?.to_file();
    file.project = project.clone();
    let mut engine = Engine::from_file(file)?;
    let item = report(&project)?
        .items
        .into_iter()
        .find(|i| i.needs.is_some())
        .ok_or("a profile-limited finding")?;
    let before = engine.revision();
    let error = engine
        .apply(
            before,
            crate::Operation::Fabrication(crate::fabrication::FabricationCommand::Waive {
                keys: vec![item.key],
                reason: "Drawn to the datasheet".into(),
            }),
        )
        .err()
        .ok_or("the waiver is refused")?;
    assert_eq!(error.rule(), Some("fabrication.not_waivable"), "{error}");
    let refused = error.to_string();
    assert!(
        refused.contains(&format!("Needs {}", key().label())),
        "{refused}"
    );
    assert_eq!(engine.revision(), before);
    assert!(engine.project().production.waivers.is_empty());
    Ok(())
}
