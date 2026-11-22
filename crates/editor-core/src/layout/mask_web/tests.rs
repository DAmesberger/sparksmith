//! Mask webs of the assembled board, hand-derived from the demo board's
//! pads: R1 and R2 share one two-pad footprint; each case places them so
//! the copper gap between R1's right pad and R2's left pad, or between
//! R1's own pads, is a chosen value, and the web is that gap less both
//! openings' expansions.
use super::*;
use crate::diagnostics::Result;
use crate::layout::{LayoutCommand, apply};
use crate::rules::{RuleValue, RulesCommand};
use crate::tests::support::by_designator;
use crate::{Engine, Error};

fn rule(project: &mut Project, key: RuleKey, mm: f64) -> Result<()> {
    crate::rules::apply(
        &mut project.rules,
        RulesCommand::SetOverride {
            key,
            value: RuleValue::Length(mm),
        },
    )?;
    Ok(())
}

fn place(project: &mut Project, instance: Id, x: f64) -> Result<()> {
    apply(
        project,
        LayoutCommand::Move {
            instance,
            position: [x, 10.0],
            rotation_deg: 0.0,
            layer: Layer::Top,
        },
    )
}

/// The x extent of `instance`'s pads: (leftmost pad, its left edge),
/// (rightmost pad, its right edge).
fn extent(project: &Project, instance: Id) -> Result<((ObjectRef, f64), (ObjectRef, f64))> {
    let mut left: Option<(ObjectRef, f64)> = None;
    let mut right: Option<(ObjectRef, f64)> = None;
    for p in placed_pads(project)
        .iter()
        .filter(|p| p.instance == instance)
    {
        let b = CopperRing::of_placed_pad(p)
            .bounds
            .ok_or_else(|| Error::Invalid("a pad without copper".into()))?;
        let me = ObjectRef::Pad {
            instance,
            pad: p.pad.id,
        };
        if left.is_none_or(|(_, x)| b.min[0] < x) {
            left = Some((me, b.min[0]));
        }
        if right.is_none_or(|(_, x)| b.max[0] > x) {
            right = Some((me, b.max[0]));
        }
    }
    left.zip(right)
        .ok_or_else(|| Error::NotFound("the part's pads".into()))
}

fn webs(project: &Project) -> Vec<Finding> {
    findings(project)
}

/// Two parts whose neighbouring pads are 0.15 mm apart in copper, each
/// opening 0.05 mm wider: a 0.05 mm web under a 0.1 mm minimum, naming
/// both pads. 0.2 mm apart, the web is the minimum and passes.
#[test]
fn a_web_between_two_placed_parts_is_measured() -> Result<()> {
    let engine = Engine::demo()?;
    let (r1, r2) = (by_designator(&engine, "R1")?, by_designator(&engine, "R2")?);
    let mut project = engine.project().clone();
    rule(&mut project, RuleKey::MaskExpansion, 0.05)?;
    rule(&mut project, RuleKey::MaskWebMin, 0.1)?;
    place(&mut project, r1, 10.0)?;
    place(&mut project, r2, 30.0)?;
    let (_, (r1_right, right_edge)) = extent(&project, r1)?;
    let ((r2_left, left_edge), _) = extent(&project, r2)?;
    let offset = left_edge - 30.0;
    for (gap, expect) in [(0.15, Some("0.05 mm")), (0.2, None)] {
        place(&mut project, r2, right_edge + gap - offset)?;
        let found = webs(&project);
        match expect {
            Some(words) => {
                let [hit] = found.as_slice() else {
                    return Err(Error::Invalid(format!("gap {gap}: {found:#?}")));
                };
                assert_eq!(hit.objects, vec![r1_right, r2_left]);
                assert!(
                    hit.message.contains(&format!("on the top side is {words}"))
                        && hit.message.contains("needs 0.1 mm"),
                    "{}",
                    hit.message
                );
            }
            None => assert!(found.is_empty(), "gap {gap}: {found:#?}"),
        }
    }
    Ok(())
}

/// Within one part the footprint lint measures its own defaults; a board
/// expansion rule that narrows the part's own web is the board's finding.
#[test]
fn a_board_expansion_that_narrows_a_parts_own_web_is_found() -> Result<()> {
    let engine = Engine::demo()?;
    let r1 = by_designator(&engine, "R1")?;
    let mut project = engine.project().clone();
    rule(&mut project, RuleKey::MaskWebMin, 0.1)?;
    let pads: Vec<_> = placed_pads(&project)
        .into_iter()
        .filter(|p| p.instance == r1)
        .collect();
    let [a, b] = pads.as_slice() else {
        return Err(Error::Invalid(format!("{} pads", pads.len())));
    };
    let gap = CopperRing::of_placed_pad(a).gap(&CopperRing::of_placed_pad(b), 100.0);
    // The footprint's own web passes; the board's opening, grown to leave
    // a 0.05 mm web, does not.
    rule(&mut project, RuleKey::MaskExpansion, (gap - 0.05) / 2.0)?;
    let found: Vec<Finding> = webs(&project)
        .into_iter()
        .filter(|f| {
            f.objects
                .iter()
                .all(|o| matches!(o, ObjectRef::Pad { instance, .. } if *instance == r1))
        })
        .collect();
    let [hit] = found.as_slice() else {
        return Err(Error::Invalid(format!("{found:#?}")));
    };
    assert!(hit.message.contains("is 0.05 mm"), "{}", hit.message);
    // The footprint's default expansion: no board finding.
    rule(&mut project, RuleKey::MaskExpansion, 0.05)?;
    assert!(
        webs(&project).iter().all(|f| !f
            .objects
            .iter()
            .all(|o| matches!(o, ObjectRef::Pad { instance, .. } if *instance == r1))),
        "{:#?}",
        webs(&project)
    );
    Ok(())
}
