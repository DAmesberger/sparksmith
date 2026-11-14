//! Slice B2a: generated tabbed gull-wing models (SOT-23, SOT-223, DPAK). A
//! moulded body on its standoff, gull-wing lead rows on −y and +y with their
//! own counts and spacing (lead step × pitch), and an optional tab: a wide
//! gull-wing tab lead (SOT-223) or a flat flange on the seating plane under
//! the body (DPAK). The body is cut around the leads, so leads and moulding
//! stay disjoint. Leads are numbered in the core's lead order: the −y row
//! in +x, the +y row in −x, then the tab.
use super::profile::bent_strip;
use super::{Build, check_count, check_positive, require, top_edges, upright_edges};
use crate::Result;
use crate::native::{Expr, Pattern, Plane, ProfileBody};
use sparksmith_core::library::BodyRole;
use sparksmith_core::library::package_form::TabbedForm;

/// The builder revision (`PackageModel::Generated { family_rev }`).
pub(super) const FAMILY_REV: Option<u16> = Some(1);

/// How far a lead root reaches into the moulding (at most a third of the
/// half body), and the shoulder's share of the bend room (as gull-wing).
const ROOT_MM: f64 = 0.25;
const SHOULDER_SHARE: f64 = 0.3;
/// Rounds of the moulding's upright corners and top edges (est.: transfer
/// moulded bodies leave the mould with 0.1–0.3 mm radii), capped below.
const CORNER_MM: f64 = 0.2;
const TOP_EDGE_MM: f64 = 0.12;

pub(super) fn check(f: &TabbedForm) -> Result<()> {
    check_positive(&[
        ("pitch", f.pitch()),
        ("body length", f.body_length()),
        ("body width", f.body_width()),
        ("body height", f.body_height()),
        ("span", f.span()),
        ("lead width", f.lead_width()),
        ("lead thickness", f.lead_thickness()),
        ("foot length", f.foot_length()),
    ])?;
    check_count("leads on the −y side", f.leads_per_side[0])?;
    if f.leads_per_side[1] > 0 {
        check_count("leads on the +y side", f.leads_per_side[1])?;
    }
    require(
        f.leads_per_side[1] > 0 || f.tab.is_some(),
        "the +y side needs a lead or a tab",
    )?;
    if let Some(tab) = f.tab {
        check_positive(&[("tab width", tab.width()), ("tab length", tab.length())])?;
    }
    require(
        f.bend_room() > f.lead_thickness(),
        "the span must leave room outside the body for the lead to bend down to its foot",
    )?;
    Ok(())
}

/// The bent side profile of a lead whose tip is `half_span` from the
/// centre and whose foot is `foot` long, in (distance across y, z).
fn profile(f: &TabbedForm, half_span: f64, foot: f64) -> Result<crate::pad_outline::Contour> {
    let t = f.lead_thickness();
    let exit = f.lead_exit();
    let half_body = f.body_width() / 2.0;
    let root = ROOT_MM.min(half_body / 3.0);
    let foot_in = half_span - foot;
    let shoulder = half_body + SHOULDER_SHARE * (foot_in - half_body);
    bent_strip(
        &[
            [half_body - root, exit],
            [shoulder, exit],
            [foot_in, t / 2.0],
            [half_span, t / 2.0],
        ],
        t,
        t,
    )
}

pub(super) fn build(b: &mut Build<'_>, f: &TabbedForm) -> Result<()> {
    let t = f.lead_thickness();
    let [n, m] = f.leads_per_side;
    let pitch = b.length("pitch", f.pitch())?;
    let bl = b.length("body_length", f.body_length())?;
    let bw = b.length("body_width", f.body_width())?;
    let bh = b.length("body_height", f.body_height())?;
    let standoff = b.length("standoff", f.standoff())?;
    b.length("span", f.span())?;
    let lw = b.length("lead_width", f.lead_width())?;
    b.length("lead_thickness", t)?;
    b.length("foot_length", f.foot_length())?;

    let body = b.name("body");
    let material = b.material(BodyRole::Body);
    b.solid(
        &body,
        BodyRole::Body,
        material,
        [
            format!("- {bl} / 2"),
            format!("- {bw} / 2"),
            standoff.clone(),
        ],
        [bl.clone(), bw.clone(), bh.clone()],
    )?;

    let lead_material = b.material(BodyRole::Lead);
    let shape = profile(f, f.span() / 2.0, f.foot_length())?;
    let mut ordered_rows = Vec::new();
    for (row, part, count, plus) in [(0usize, "row_s", n, false), (1, "row_n", m, true)] {
        if count == 0 {
            continue;
        }
        let count_param = b.count(&format!("{part}_leads"), count)?;
        let step = b.count(&format!("{part}_step"), f.lead_step[row])?;
        let contour = if plus {
            shape.clone()
        } else {
            shape.mapped(|[s, z]| [-s, z], true)
        };
        let name = b.name(&format!("{part} lead"));
        let offset = format!("- ({count_param} - 1) / 2 * {step} * {pitch} - {lw} / 2");
        b.native.plane_body(
            Plane::Yz,
            ProfileBody {
                name: &name,
                offset: offset.into(),
                contours: std::slice::from_ref(&contour),
                height: Expr::from(lw.clone()),
            },
        )?;
        b.push(name.clone(), BodyRole::Lead, lead_material);
        let names = if count > 1 {
            let all = b.native.pattern_bodies(
                &b.name(&format!("{part} pattern")),
                &[&name],
                Pattern {
                    dir1: [1.0, 0.0, 0.0],
                    count1: count_param.clone().into(),
                    spacing1: format!("{step} * {pitch}").into(),
                    second: None,
                },
            )?;
            b.push_copies(&all, BodyRole::Lead, lead_material);
            all
        } else {
            vec![name]
        };
        ordered_rows.push((names, 0, !plus));
    }
    let mut leads = b.order(&ordered_rows)?;

    if let Some(tab) = f.tab {
        let tw = b.length("tab_width", tab.width())?;
        let tl = b.length("tab_length", tab.length())?;
        let name = b.name("tab");
        match tab.flange() {
            None => {
                let contour = profile(f, f.span() / 2.0, tab.length())?;
                b.native.plane_body(
                    Plane::Yz,
                    ProfileBody {
                        name: &name,
                        offset: format!("- {tw} / 2").into(),
                        contours: std::slice::from_ref(&contour),
                        height: Expr::from(tw.clone()),
                    },
                )?;
                b.push(name.clone(), BodyRole::Lead, lead_material);
            }
            Some(protrusion) => {
                let p = b.length("flange_protrusion", protrusion)?;
                let thickness = b.p("lead_thickness");
                b.solid(
                    &name,
                    BodyRole::Lead,
                    lead_material,
                    [
                        format!("- {tw} / 2"),
                        format!("{bw} / 2 + {p} - {tl}"),
                        "0 mm".into(),
                    ],
                    [tw.clone(), tl.clone(), thickness],
                )?;
            }
        }
        leads.push(name);
    }
    let tools: Vec<&str> = leads.iter().map(String::as_str).collect();
    b.native.cut_keeping(&b.name("lead roots"), &body, &tools)?;
    b.number(&leads);

    // Rounded upright corners clear of the end leads and the tab, then
    // rounded top edges clear of the lead exit.
    let row_length = |count: u32, step: u32| {
        f64::from(count.saturating_sub(1)) * f64::from(step) * f.pitch() + f.lead_width()
    };
    let widest = row_length(n, f.lead_step[0])
        .max(if m > 0 {
            row_length(m, f.lead_step[1])
        } else {
            0.0
        })
        .max(f.tab.map_or(0.0, |tab| tab.width()));
    let short = f.body_length().min(f.body_width());
    let corner = CORNER_MM
        .min(0.06 * short)
        .min((f.body_length() - widest) / 4.0);
    let top = f.standoff() + f.body_height();
    let edge = TOP_EDGE_MM
        .min(0.1 * f.body_height())
        .min((top - f.lead_exit() - t / 2.0) / 2.0)
        .min(corner.max(0.0));
    let lo = [-f.body_length() / 2.0, -f.body_width() / 2.0, f.standoff()];
    let hi = [f.body_length() / 2.0, f.body_width() / 2.0, top];
    // Pick the upright edges above the flange and lead roots.
    let mut uprights = upright_edges(lo, hi);
    for p in &mut uprights {
        p[2] = (f.lead_exit() + t / 2.0 + top) / 2.0;
    }
    b.round("corner_round", &body, &uprights, corner)?;
    b.round("edge_round", &body, &top_edges(lo, hi), edge)?;
    Ok(())
}
