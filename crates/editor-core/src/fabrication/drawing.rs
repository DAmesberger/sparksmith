//! What the fabrication and assembly drawings print, in board millimetres,
//! curves already flattened: `sparksmith-io` lays it out as PDF pages and
//! interprets nothing.
use super::drill::{DrillHole, DrillTable};
use crate::diagnostics::Result;
use crate::geometry::{Bounds, Point, Shape, distance};
use crate::ids::Id;
use crate::layout::{self, BoardDrawing, PlacedItem};
use crate::library::{Outline, PAD_ARC_TOLERANCE_MM, PadShape, Segment};
use crate::model::DielectricKind;
use crate::model::board::{CutoutKind, arc_geometry};
use crate::model::{Layer, MaskColour, PadFinish, Project, SilkscreenColour};
use crate::rules::CopperObject;
use serde::Serialize;
use std::collections::BTreeMap;

/// Chord tolerance of the drawn outlines, mm.
pub const DRAWING_CHORD_MM: f64 = 0.01;

/// One row of the stackup table, top to bottom.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct StackupRow {
    /// "Top mask", "Top copper", "Core 1", …
    pub name: String,
    /// "copper", "core", "prepreg", "mask", "legend".
    pub kind: String,
    /// "FR-4 7628", "35 µm Cu", …
    pub material: String,
    pub thickness_mm: f64,
}

/// One placed part on an assembly drawing.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AssemblyPart {
    pub instance: Id,
    pub designator: String,
    pub value: String,
    /// The placement origin.
    pub at: Point,
    pub rotation_deg: f64,
    /// Body outline loops (assembly-layer drawing, else courtyard, else
    /// the pads' box), closed.
    pub body: Vec<Vec<Point>>,
    /// Pad outlines on this side, closed.
    pub pads: Vec<Vec<Point>>,
    /// Where the polarity (pin 1) mark goes, when the footprint has one.
    pub polarity: Option<Point>,
    /// Fitted under the active assembly variant; a part that is not is
    /// drawn crossed out.
    pub fitted: bool,
    /// Its active assembly instructions include a fabricated trace to cut.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub cut_trace: bool,
}

/// One physical trace to cut for this variant, at the bridge on its own side.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TraceCut {
    pub instance: Id,
    pub group: crate::ids::TieId,
    pub bridge: crate::ids::BridgeId,
    pub at: Point,
    pub label: String,
}

/// One side of the assembly drawing.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AssemblySide {
    pub side: Layer,
    pub parts: Vec<AssemblyPart>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cuts: Vec<TraceCut>,
}

/// Everything both drawings print.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DrawingData {
    pub title: String,
    /// The release the drawings belong to, as every file of the set states
    /// it ([`super::Identity`]): its revision and the project's identity.
    pub release: String,
    pub project_id: String,
    /// The board outline's exact bounds.
    pub bounds: Bounds,
    /// The outer outline (counter-clockwise), closed, flattened.
    pub outline: Vec<Point>,
    /// Cutouts and non-plated slots, closed, flattened.
    pub cutouts: Vec<Vec<Point>>,
    pub holes: Vec<DrillHole>,
    pub drills: DrillTable,
    pub stackup: Vec<StackupRow>,
    pub thickness_mm: f64,
    pub copper_layers: usize,
    /// Surface finish, mask and legend colours as words.
    pub finish: String,
    pub mask: String,
    pub legend: String,
    /// The fab profile's name and provenance line.
    pub profile: String,
    /// Fabrication notes from the effective rules (minimum track and gap,
    /// drill, annular ring, mask web, …), one sentence each.
    pub notes: Vec<String>,
    pub sides: Vec<AssemblySide>,
}

fn mm(v: f64) -> String {
    crate::layout::limits::mm(v)
}

fn finish_word(f: PadFinish) -> &'static str {
    match f {
        PadFinish::Enig => "ENIG",
        PadFinish::Hasl => "HASL (tin-lead)",
        PadFinish::LeadFreeHasl => "Lead-free HASL",
        PadFinish::Osp => "OSP",
        PadFinish::ImmersionSilver => "Immersion silver",
    }
}

fn mask_word(c: MaskColour) -> &'static str {
    match c {
        MaskColour::Green => "green",
        MaskColour::Blue => "blue",
        MaskColour::Red => "red",
        MaskColour::Purple => "purple",
        MaskColour::Yellow => "yellow",
        MaskColour::Black => "black",
        MaskColour::White => "white",
        MaskColour::Clear => "clear",
    }
}

fn legend_word(c: SilkscreenColour) -> &'static str {
    match c {
        SilkscreenColour::White => "white",
        SilkscreenColour::Black => "black",
        SilkscreenColour::Yellow => "yellow",
    }
}

/// The geometry flattener bounds work per curve. Check its actual chord
/// count against the analytic sagitta, including the closing circle chord.
fn checked_primitive(shape: &Shape, tol: f64, name: &str) -> Result<Vec<Point>> {
    let points = shape.flatten(tol);
    let curve = match *shape {
        Shape::Arc {
            radius, sweep_deg, ..
        } => Some((
            radius,
            sweep_deg.abs().to_radians(),
            points.len().saturating_sub(1),
        )),
        Shape::Circle { radius, .. } => Some((radius, std::f64::consts::TAU, points.len())),
        _ => None,
    };
    if let Some((radius, sweep, count)) = curve {
        // r * (1 - cos(a/2)), evaluated without subtracting near-equal numbers.
        let sagitta = 2.0 * radius * (sweep / (4.0 * count as f64)).sin().powi(2);
        if !sagitta.is_finite() || sagitta > tol {
            let step = 4.0 * (tol / (2.0 * radius)).sqrt().min(1.0).asin();
            let needed = (sweep / step).ceil();
            return Err(crate::Error::Limit(format!(
                "The {name} needs {needed:.0} chords on one curve to meet {tol} mm. The drawing could produce {count}."
            )));
        }
    }
    if !points.iter().flatten().all(|v| v.is_finite()) {
        return Err(crate::Error::Invalid(format!(
            "The {name} has a non-finite drawing coordinate."
        )));
    }
    Ok(points)
}

fn close(points: &mut Vec<Point>) {
    if let Some(&first) = points.first()
        && points.last() != Some(&first)
    {
        points.push(first);
    }
}

fn drawing_shape(shape: &Shape, tol: f64, name: &str) -> Result<Vec<Point>> {
    let mut points = if let Some(parts) = shape.primitives() {
        let mut points = Vec::new();
        for part in &parts {
            for p in checked_primitive(part, tol, name)? {
                if points.last().is_none_or(|q| distance(*q, p) > 1e-12) {
                    points.push(p);
                }
            }
        }
        points
    } else {
        checked_primitive(shape, tol, name)?
    };
    if shape.is_closed() {
        close(&mut points);
    }
    Ok(points)
}

/// Read the stored bulges before tessellation: board edges and custom lands
/// otherwise arrive as already bounded polylines and their error is lost.
fn drawing_outline(outline: &Outline, tol: f64, name: &str) -> Result<Vec<Point>> {
    if !outline.closed() {
        return Err(crate::Error::Invalid(format!("The {name} is open.")));
    }
    let mut points = vec![outline.start];
    let mut from = outline.start;
    for segment in &outline.segments {
        let to = segment.to();
        if let Segment::Arc { bulge, .. } = segment
            && distance(from, to) * bulge.abs() * 0.5 > tol
        {
            // The bulge is the signed sagitta divided by the half-chord.
            // A shallower arc already meets tolerance as its endpoint chord.
            let arc = arc_geometry(from, to, *bulge).ok_or_else(|| {
                crate::Error::Invalid(format!("The {name} has an unrepresentable arc."))
            })?;
            let span = Shape::Arc {
                center: arc.center,
                radius: arc.radius,
                start_deg: arc.start.to_degrees(),
                sweep_deg: arc.sweep.to_degrees(),
            };
            let arc_points = checked_primitive(&span, tol, name)?;
            points.extend(
                arc_points
                    .iter()
                    .skip(1)
                    .take(arc_points.len().saturating_sub(2)),
            );
        }
        points.push(to);
        from = to;
    }
    close(&mut points);
    Ok(points)
}

/// The stackup table rows, top to bottom: copper and dielectrics, the mask
/// at the stackup's mask thickness and the legend by side without a
/// thickness, which the stackup does not hold. Neither counts towards the
/// nominal board thickness.
fn stackup_rows(project: &Project) -> Vec<StackupRow> {
    let s = &project.stackup;
    let mut rows = Vec::new();
    if s.silkscreen_top {
        rows.push(StackupRow {
            name: "Top legend".into(),
            kind: "legend".into(),
            material: format!("{} ink", legend_word(project.fabrication.silkscreen_colour)),
            thickness_mm: 0.0,
        });
    }
    if s.mask_top {
        rows.push(StackupRow {
            name: "Top mask".into(),
            kind: "mask".into(),
            material: format!("{} solder mask", mask_word(project.fabrication.mask_colour)),
            thickness_mm: s.mask_um / 1000.0,
        });
    }
    let layers = s.layers();
    let (mut cores, mut prepregs) = (0, 0);
    for (i, foil) in s.copper.iter().enumerate() {
        let name = layers
            .get(i)
            .map(|l| l.name())
            .unwrap_or_else(|| format!("Copper {}", i + 1));
        rows.push(StackupRow {
            name: format!("{name} copper"),
            kind: "copper".into(),
            material: format!("{:.0} µm Cu", foil.thickness_um),
            thickness_mm: foil.thickness_um / 1000.0,
        });
        if let Some(d) = s.dielectrics.get(i) {
            let (kind, n) = match d.kind {
                DielectricKind::Core => {
                    cores += 1;
                    ("core", cores)
                }
                DielectricKind::Prepreg => {
                    prepregs += 1;
                    ("prepreg", prepregs)
                }
            };
            let glass = match d.glass {
                crate::model::GlassStyle::Style106 => "106",
                crate::model::GlassStyle::Style1080 => "1080",
                crate::model::GlassStyle::Style2116 => "2116",
                crate::model::GlassStyle::Style7628 => "7628",
            };
            let title = if kind == "core" { "Core" } else { "Prepreg" };
            rows.push(StackupRow {
                name: format!("{title} {n}"),
                kind: kind.into(),
                material: format!("FR-4 {glass}"),
                thickness_mm: d.thickness_mm,
            });
        }
    }
    if s.mask_bottom {
        rows.push(StackupRow {
            name: "Bottom mask".into(),
            kind: "mask".into(),
            material: format!("{} solder mask", mask_word(project.fabrication.mask_colour)),
            thickness_mm: s.mask_um / 1000.0,
        });
    }
    if s.silkscreen_bottom {
        rows.push(StackupRow {
            name: "Bottom legend".into(),
            kind: "legend".into(),
            material: format!("{} ink", legend_word(project.fabrication.silkscreen_colour)),
            thickness_mm: 0.0,
        });
    }
    rows
}

fn notes(project: &Project, settings: &crate::layout::CheckSettings) -> Vec<String> {
    let rules = project.rules.effective();
    let gap = rules
        .copper
        .clearance
        .get(CopperObject::Track, CopperObject::Track);
    let via_hole = rules
        .copper
        .clearance
        .get(CopperObject::ViaHole, CopperObject::Track);
    let pth_hole = rules
        .copper
        .clearance
        .get(CopperObject::PthHole, CopperObject::Track);
    let npth_hole = rules
        .copper
        .clearance
        .get(CopperObject::NpthHole, CopperObject::Track);
    let weights: Vec<String> = project
        .stackup
        .copper
        .iter()
        .map(|c| format!("{:.0} µm", c.thickness_um))
        .collect();
    let s = &project.stackup;
    let masked = s.mask_top || s.mask_bottom;
    let mut out = vec![
        format!(
            "Nominal board thickness {} over the outer copper, solder mask and legend \
             excluded; {} copper layers.",
            mm(s.thickness_mm()),
            s.copper.len()
        ),
        if masked {
            format!(
                "Solder mask {} µm thick over copper and {} µm over laminate on each \
                 masked side.",
                trim_um(s.mask_um),
                trim_um(s.mask_laminate_nm.to_mm_f64() * 1000.0)
            )
        } else {
            "No solder mask.".into()
        },
        format!("Copper thickness top to bottom: {}.", weights.join(", ")),
        format!(
            "Minimum track width {}, minimum copper gap {}.",
            mm(rules.copper.track_width_min),
            mm(gap)
        ),
        format!(
            "Minimum finished drill {}, via hole to copper {}, plated hole to copper {}, \
             unplated hole to copper {}.",
            mm(rules.holes.drill_min),
            mm(via_hole),
            mm(pth_hole),
            mm(npth_hole)
        ),
        format!(
            "Minimum annular ring: vias {}, plated holes {}, copper round unplated holes {}.",
            mm(rules.holes.via_annular_ring_min),
            mm(rules.holes.pth_annular_ring_min),
            mm(rules.holes.npth_annular_ring_min)
        ),
        format!(
            "Solder mask: {} {}, expansion {}, minimum web {}.",
            mask_word(project.fabrication.mask_colour),
            match project.fabrication.mask_finish {
                crate::model::MaskFinish::Gloss => "gloss",
                crate::model::MaskFinish::Satin => "satin",
                crate::model::MaskFinish::Matte => "matte",
            },
            mm(rules.mask.expansion),
            mm(rules.mask.web_min)
        ),
        format!(
            "Legend: {} ink, minimum line {}, minimum text height {}.",
            legend_word(project.fabrication.silkscreen_colour),
            mm(rules.silkscreen.line_width_min),
            mm(rules.silkscreen.text_height_min)
        ),
        format!(
            "Surface finish: {}.",
            finish_word(project.fabrication.pad_finish)
        ),
        format!(
            "Dimensions in millimetres. {}",
            project.rules.profile.as_ref().map_or_else(
                || "Hole sizes are finished sizes; no fab profile says how the fabricator \
                    reads drill files."
                    .to_string(),
                |p| p.drill_files.note()
            )
        ),
    ];
    out.extend(waived(project, settings));
    out.push(
        concat!(
            "Generated by ",
            crate::product_name!(),
            "; the Gerber job file lists every layer."
        )
        .into(),
    );
    out
}

/// One note per waived production finding, asking the fabricator to
/// confirm it in its DFM review: the board check's message (the code when
/// no current board finding matches) and the waiver's reason. A saved
/// waiver of a finding that may not be waived is ignored.
fn waived(project: &Project, settings: &crate::layout::CheckSettings) -> Vec<String> {
    let waivers = &project.production.waivers;
    if waivers.is_empty() {
        return Vec::new();
    }
    let findings = layout::checks(project, settings);
    waivers
        .iter()
        .filter(|w| {
            super::preflight::can_waive(w.key.split_whitespace().next().unwrap_or_default())
        })
        .map(|w| {
            let what = findings
                .iter()
                .find(|f| crate::model::production::waiver_key(&f.code, &f.objects) == w.key)
                .map_or_else(
                    || {
                        w.key
                            .split_whitespace()
                            .next()
                            .unwrap_or_default()
                            .to_owned()
                    },
                    |f| f.message.clone(),
                );
            let reason = if w.reason.is_empty() {
                String::new()
            } else {
                format!(" Reason: {}.", w.reason.trim_end_matches('.'))
            };
            format!("Waived by the designer, please confirm in DFM review: {what}.{reason}")
        })
        .collect()
}

fn profile_line(project: &Project) -> String {
    let Some(p) = &project.rules.profile else {
        return "No fab profile: no manufacturing rule was checked".into();
    };
    let mut source = p.source.publisher.clone();
    if !p.source.document.is_empty() {
        source = format!("{source}, {}", p.source.document);
    }
    format!("{} (from {source}, read {})", p.name, p.source.date)
}

/// `um` without trailing zeros.
fn trim_um(um: f64) -> String {
    let text = format!("{um:.1}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The value printed for each instance: its BOM values, else its MPN.
fn values(project: &Project) -> BTreeMap<Id, String> {
    let mut out = BTreeMap::new();
    if let Ok(bom) = crate::completeness::bom(project) {
        for line in bom.lines {
            let text = if line.values.is_empty() {
                line.mpn.clone().unwrap_or_default()
            } else {
                line.values
                    .iter()
                    .map(|v| v.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            };
            for id in line.instances {
                out.insert(id, text.clone());
            }
        }
    }
    out
}

fn box_loop(b: Bounds) -> Vec<Point> {
    vec![b.min, [b.max[0], b.min[1]], b.max, [b.min[0], b.max[1]]]
}

fn trace_cuts(project: &Project) -> Vec<(Layer, TraceCut)> {
    use crate::library::tie::{JumperBridge, TieForm};
    layout::placed_bridges(project)
        .into_iter()
        .filter_map(|bridge| {
            let instance = project.instance(bridge.instance)?;
            if crate::instances::variants::population(instance, project.variants.active())
                .is_populated()
            {
                return None;
            }
            let component = project.library.component(instance.component)?;
            let group = component
                .definition
                .ties
                .iter()
                .find(|t| t.id == bridge.group)?;
            if !matches!(
                group.form,
                TieForm::SolderJumper {
                    bridge: JumperBridge::Trace,
                    ..
                }
            ) {
                return None;
            }
            let view = crate::instances::resolve(&project.library, instance)
                .ok()?
                .physical?;
            let footprint = &view.footprint.definition;
            let mut label = format!("Cut {}", instance.designator);
            if footprint.pads.len() > 2 {
                let numbers = bridge
                    .pads
                    .iter()
                    .filter_map(|id| footprint.pad(*id).map(|p| p.number.as_str()))
                    .collect::<Vec<_>>();
                label.push_str(" pads ");
                label.push_str(&numbers.join("–"));
            }
            let at = crate::geometry::polygon_marker(&bridge.outline.flatten(DRAWING_CHORD_MM))?;
            Some((
                bridge.layer.side()?,
                TraceCut {
                    instance: instance.id,
                    group: group.id,
                    bridge: bridge.bridge,
                    at,
                    label,
                },
            ))
        })
        .collect()
}

fn assembly_sides(project: &Project) -> Result<Vec<AssemblySide>> {
    let values = values(project);
    let mut assembly: BTreeMap<(Id, bool), Vec<Vec<Point>>> = BTreeMap::new();
    let mut courtyard: BTreeMap<(Id, bool), Vec<Vec<Point>>> = BTreeMap::new();
    let bottom = |l: Layer| l == Layer::Bottom;
    for g in layout::placed_graphics(project) {
        let (map, side) = match g.layer {
            BoardDrawing::Assembly(side) => (&mut assembly, side),
            BoardDrawing::Courtyard(side) => (&mut courtyard, side),
            BoardDrawing::Silkscreen(_) => continue,
        };
        if let PlacedItem::Shape { shape, .. } = &g.item {
            let name = format!("assembly graphic {} of part {}", g.graphic.0, g.instance);
            let ring = drawing_shape(shape, DRAWING_CHORD_MM, &name)?;
            if ring.len() >= 2 {
                map.entry((g.instance, bottom(side)))
                    .or_default()
                    .push(ring);
            }
        }
    }
    let mut pads: BTreeMap<(Id, bool), Vec<Vec<Point>>> = BTreeMap::new();
    let mut polarity: BTreeMap<Id, Point> = BTreeMap::new();
    for placed in layout::placed_pads(project) {
        let name = format!("pad {} of part {}", placed.pad.number, placed.instance);
        let ring = if let PadShape::Custom { outline } = &placed.pad.shape {
            let transform = placed.pad_transform();
            drawing_outline(outline, PAD_ARC_TOLERANCE_MM, &name)?
                .into_iter()
                .map(|p| transform.apply(p))
                .collect()
        } else {
            drawing_shape(&placed.outline(), PAD_ARC_TOLERANCE_MM.max(0.005), &name)?
        };
        if ring.len() < 3 {
            continue;
        }
        let sides: &[Layer] = if placed.pad.kind.through_hole() {
            &[Layer::Top, Layer::Bottom]
        } else {
            std::slice::from_ref(&placed.layer)
        };
        for side in sides {
            pads.entry((placed.instance, bottom(*side)))
                .or_default()
                .push(ring.clone());
        }
        let is_polarity = project
            .instance(placed.instance)
            .and_then(|i| crate::instances::resolve(&project.library, i).ok())
            .and_then(|r| r.physical)
            .and_then(|p| p.footprint.definition.polarity.as_ref().map(|q| q.pad))
            == Some(placed.pad.id);
        if is_polarity {
            polarity.insert(placed.instance, placed.center());
        }
    }
    let cuts = trace_cuts(project);
    let cut_parts: std::collections::BTreeSet<Id> = cuts.iter().map(|(_, c)| c.instance).collect();
    let mut sides = Vec::new();
    for side in [Layer::Top, Layer::Bottom] {
        let mut parts = Vec::new();
        for placement in &project.placements {
            if placement.layer != side {
                continue;
            }
            let Some(instance) = project.instance(placement.instance) else {
                continue;
            };
            let key = (instance.id, bottom(side));
            let part_pads = pads.get(&key).cloned().unwrap_or_default();
            let body = assembly
                .get(&key)
                .cloned()
                .or_else(|| courtyard.get(&key).cloned())
                .or_else(|| {
                    let b = Bounds::of(part_pads.iter().flatten().copied())?;
                    let mut ring = box_loop(b);
                    ring.push(b.min);
                    Some(vec![ring])
                })
                .unwrap_or_default();
            let fitted = crate::instances::variants::fitted(project, instance).populated();
            let cut_trace = cut_parts.contains(&instance.id);
            parts.push(AssemblyPart {
                instance: instance.id,
                designator: instance.designator.clone(),
                value: values.get(&instance.id).cloned().unwrap_or_default(),
                at: placement.position,
                rotation_deg: placement.rotation_deg,
                body,
                pads: part_pads,
                polarity: polarity.get(&instance.id).copied(),
                fitted,
                cut_trace,
            });
        }
        let cuts: Vec<_> = cuts
            .iter()
            .filter(|(s, _)| *s == side)
            .map(|(_, cut)| cut.clone())
            .collect();
        if !parts.is_empty() || !cuts.is_empty() {
            sides.push(AssemblySide { side, parts, cuts });
        }
    }
    Ok(sides)
}

/// The drawing data of `snapshot`, for the release `identity` (the
/// [`super::FabricationSet::identity`] of the same snapshot).
pub fn data(
    snapshot: &super::ProductionSnapshot<'_>,
    identity: &super::Identity,
    settings: &crate::layout::CheckSettings,
) -> Result<DrawingData> {
    let project = snapshot.project();
    let board = project.board();
    let holes = super::drill::holes(project);
    let outline = drawing_outline(&board.outer, DRAWING_CHORD_MM, "board outline")?;
    let mut cutouts = Vec::new();
    for (i, cutout) in board.cutouts.iter().enumerate() {
        if cutout.kind != (CutoutKind::Slot { plated: true }) {
            cutouts.push(drawing_outline(
                &cutout.outline,
                DRAWING_CHORD_MM,
                &format!("board cutout {}", i + 1),
            )?);
        }
    }
    let f = &project.fabrication;
    Ok(DrawingData {
        title: project.name.clone(),
        release: identity.revision.clone(),
        project_id: identity.guid.clone(),
        bounds: board.bounds(),
        outline,
        cutouts,
        drills: super::drill::table(&holes),
        holes,
        stackup: stackup_rows(project),
        thickness_mm: project.stackup.thickness_mm(),
        copper_layers: project.stackup.copper.len(),
        finish: finish_word(f.pad_finish).into(),
        mask: mask_word(f.mask_colour).into(),
        legend: legend_word(f.silkscreen_colour).into(),
        profile: profile_line(project),
        notes: notes(project, settings),
        sides: assembly_sides(project)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;

    #[test]
    fn a_drawing_names_the_contour_when_its_curve_cannot_meet_tolerance() -> Result<()> {
        let circle = crate::geometry::Shape::Circle {
            center: [0.0, 0.0],
            radius: 10_000.0,
        };
        let Err(error) = drawing_shape(&circle, DRAWING_CHORD_MM, "assembly graphic 7") else {
            return Err(crate::Error::Invalid(
                "a full turn needs more than 2,200 chords at 10 µm".into(),
            ));
        };
        let message = error.to_string();
        assert!(message.contains("assembly graphic 7"), "{message}");
        assert!(message.contains("0.01 mm"), "{message}");
        assert!(message.contains("2222"), "{message}");
        Ok(())
    }

    #[test]
    fn a_custom_land_and_an_assembly_path_retain_their_last_edge() -> Result<()> {
        use crate::library::{Outline, Segment};
        // The pad has 400 semicircles of radius 1.5 mm. At 5 µm
        // their ends and all intermediate chords require over 4,096 points.
        let at = |i: usize| {
            let angle = std::f64::consts::TAU * (i % 400) as f64 / 400.0;
            [190.0 * angle.cos(), 190.0 * angle.sin()]
        };
        let outline = Outline {
            start: at(0),
            segments: (1..=400)
                .map(|i| Segment::Arc {
                    to: at(i),
                    bulge: 1.0,
                })
                .collect(),
        };
        let land = drawing_outline(&outline, 0.005, "pad 1")?;
        assert!(land.len() > 4096);
        assert_eq!(land.first(), land.last());
        for i in 0..400 {
            assert!(land.contains(&at(i)), "arc endpoint {i}");
        }
        let path = crate::geometry::Shape::polygon(land[..land.len() - 1].to_vec());
        assert_eq!(drawing_shape(&path, DRAWING_CHORD_MM, "body")?, land);
        Ok(())
    }

    /// Each waiver adds a note asking the fabricator to confirm it: the
    /// board finding's message with the reason, or the code when no
    /// current finding matches.
    #[test]
    fn waived_findings_are_noted_for_the_fabricator() -> Result<()> {
        use crate::model::production::{Waiver, waiver_key};
        let mut project = Engine::showcase()?.project().clone();
        let before = data(
            &super::super::ProductionSnapshot::resolve(&project, None)?,
            &Default::default(),
            &Default::default(),
        )?
        .notes;
        assert!(
            !before.iter().any(|n| n.starts_with("Waived")),
            "{before:?}"
        );
        // A track of one net laid over a track of another: a finding.
        let over = project
            .tracks
            .first()
            .cloned()
            .ok_or(crate::Error::Invalid("no track".into()))?;
        let other = project
            .nets
            .iter()
            .map(|n| n.id)
            .find(|n| *n != over.net)
            .ok_or(crate::Error::Invalid("no net".into()))?;
        let id = project.allocate()?;
        project.tracks.push(crate::model::Track {
            id,
            net: other,
            ..over
        });
        let finding = layout::checks(&project, &Default::default())
            .into_iter()
            .find(|f| f.code == "board.clearance")
            .ok_or(crate::Error::Invalid("no clearance finding".into()))?;
        project.production.waivers = vec![
            Waiver {
                key: "board.clearance []".into(),
                reason: String::new(),
            },
            Waiver {
                key: waiver_key(&finding.code, &finding.objects),
                reason: "Checked with the fab".into(),
            },
        ];
        project.production.waivers.sort_by(|a, b| a.key.cmp(&b.key));
        let notes = data(
            &super::super::ProductionSnapshot::resolve(&project, None)?,
            &Default::default(),
            &Default::default(),
        )?
        .notes;
        let waived: Vec<&String> = notes.iter().filter(|n| n.starts_with("Waived")).collect();
        assert_eq!(waived.len(), 2, "{notes:?}");
        let prefix = "Waived by the designer, please confirm in DFM review: ";
        assert!(
            waived.contains(&&format!("{prefix}board.clearance.")),
            "{waived:?}"
        );
        assert!(
            waived.contains(&&format!(
                "{prefix}{}. Reason: Checked with the fab.",
                finding.message
            )),
            "{waived:?}"
        );
        assert_eq!(
            notes.last().map(String::as_str),
            Some(concat!(
                "Generated by ",
                crate::product_name!(),
                "; the Gerber job file lists every layer."
            ))
        );
        Ok(())
    }

    #[test]
    fn cut_marker_is_on_copper_for_a_bent_bridge() {
        let ring = [[0., 0.], [4., 0.], [4., 1.], [1., 1.], [1., 4.], [0., 4.]];
        assert_eq!(crate::geometry::polygon_marker(&ring), Some([0.5, 2.0]));
    }

    #[test]
    fn the_showcase_drawing_data_matches_the_board() -> Result<()> {
        let engine = Engine::showcase()?;
        let project = engine.project();
        let d = data(
            &super::super::ProductionSnapshot::resolve(project, None)?,
            &Default::default(),
            &Default::default(),
        )?;
        let size = project.board_size;
        assert!((d.bounds.max[0] - d.bounds.min[0] - size[0]).abs() < 1e-9);
        assert!((d.bounds.max[1] - d.bounds.min[1] - size[1]).abs() < 1e-9);
        let ob = Bounds::of(d.outline.iter().copied()).ok_or(crate::Error::Invalid("o".into()))?;
        assert_eq!(ob, d.bounds);
        assert_eq!(d.outline.first(), d.outline.last());
        // The nominal thickness is copper plus dielectrics; the mask rows
        // carry the stackup's mask thickness but are not part of it.
        let total: f64 = d
            .stackup
            .iter()
            .filter(|r| r.kind != "mask")
            .map(|r| r.thickness_mm)
            .sum();
        assert!((total - project.stackup.thickness_mm()).abs() < 1e-9);
        let masks: Vec<f64> = d
            .stackup
            .iter()
            .filter(|r| r.kind == "mask")
            .map(|r| r.thickness_mm)
            .collect();
        assert_eq!(masks.len(), 2);
        assert!(
            masks
                .iter()
                .all(|t| (t - project.stackup.mask_um / 1000.0).abs() < 1e-12)
        );
        assert!(
            d.notes
                .iter()
                .any(|n| n.starts_with("Nominal board thickness")
                    && n.contains("solder mask and legend excluded")),
            "{:?}",
            d.notes
        );
        assert!(
            d.notes.iter().any(|n| n
                == &format!(
                    "Solder mask {} µm thick over copper and {} µm over laminate on each \
                     masked side.",
                    project.stackup.mask_um,
                    project.stackup.mask_laminate_nm.to_mm_f64() * 1000.0
                )),
            "{:?}",
            d.notes
        );
        // The test prototype reads drill files as finished sizes.
        assert!(
            d.notes.iter().any(|n| n
                .contains("Hole sizes are finished sizes; the drill file lists finished sizes.")),
            "{:?}",
            d.notes
        );
        assert_eq!(d.copper_layers, 4);
        assert_eq!(d.stackup.iter().filter(|r| r.kind == "copper").count(), 4);
        // Parts per side: every placement once, on its side.
        let count = |side: Layer| {
            d.sides
                .iter()
                .filter(|s| s.side == side)
                .map(|s| s.parts.len())
                .sum::<usize>()
        };
        let expect = |side: Layer| {
            project
                .placements
                .iter()
                .filter(|p| p.layer == side)
                .count()
        };
        assert_eq!(count(Layer::Top), expect(Layer::Top));
        assert_eq!(count(Layer::Bottom), expect(Layer::Bottom));
        assert!(
            count(Layer::Bottom) >= 1,
            "the showcase has a bottom resistor"
        );
        // Polarised parts carry a polarity mark at their pin-1 pad.
        let polarised: Vec<&AssemblyPart> = d
            .sides
            .iter()
            .flat_map(|s| &s.parts)
            .filter(|p| p.polarity.is_some())
            .collect();
        assert!(
            polarised.len() >= 3,
            "{:?}",
            polarised.iter().map(|p| &p.designator).collect::<Vec<_>>()
        );
        for part in d.sides.iter().flat_map(|s| &s.parts) {
            assert!(!part.body.is_empty(), "{}", part.designator);
            assert!(!part.pads.is_empty(), "{}", part.designator);
            if let Some(p) = part.polarity {
                let near = part
                    .pads
                    .iter()
                    .flatten()
                    .any(|q| (q[0] - p[0]).hypot(q[1] - p[1]) < 5.0);
                assert!(near, "{}", part.designator);
            }
        }
        // Drill table counts every hole.
        let counted: usize = d.drills.tools.iter().map(|t| t.count).sum();
        assert_eq!(counted, d.holes.len());
        assert!(d.notes.iter().any(|n| n.contains("Minimum track width")));
        assert!(!d.profile.is_empty());
        Ok(())
    }
}
