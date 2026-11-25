//! The import on Rust-built synthetic boards, in CI (no KiCad needed).
//! The expected values follow the semantics `tests/oracle.rs` confirmed
//! against `kicad-cli` plots: a pad's `at` is in its footprint's frame and
//! its angle is its board orientation; back-side footprints are stored as
//! seen from the top; a drill offset moves the copper, not the hole; mask
//! and paste margins resolve pad, then footprint, then board (a stored 0
//! is unset); a relative paste margin scales the pad size.
mod support;

use sparksmith_core::geometry::{Transform2, polygon_contains};
use sparksmith_core::layout::{PlacedPad, placed_pads};
use sparksmith_core::library::{HoleShape, MaskRule, PadKind, PadShape, PasteRule, ShapeNm};
use sparksmith_core::{Layer, Project};
use sparksmith_kicad::ImportError;
use sparksmith_kicad::board::BoardError;
use sparksmith_kicad::report::Case;
use support::{Board, Footprint, Pad, import};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// KiCad's board point of a footprint-local point `p` (y down) of a
/// footprint at `fp` (x, y, angle), in our frame about `origin` (y up).
fn kicad_world(origin: [f64; 2], fp: [f64; 3], p: [f64; 2]) -> [f64; 2] {
    let (s, c) = fp[2].to_radians().sin_cos();
    // A counter-clockwise turn on screen in a y-down frame.
    let x = fp[0] + p[0] * c + p[1] * s;
    let y = fp[1] - p[0] * s + p[1] * c;
    [x - origin[0], -(y - origin[1])]
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6
}

/// The placed pad of footprint `k` (board order) with `number`.
fn pad<'a>(
    project: &Project,
    pads: &'a [PlacedPad],
    k: usize,
    number: &str,
) -> Option<&'a PlacedPad> {
    let instance = project.placements.get(k)?.instance;
    pads.iter()
        .find(|p| p.instance == instance && p.pad.number == number)
}

#[test]
fn the_showcase_board_imports_through_the_engine() -> Fallible<()> {
    let board = support::showcase();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let counts = &imported.report.counts;
    assert_eq!((counts.footprints, counts.footprints_back), (11, 5));
    // Every angle of one footprint shares a definition, on either side.
    assert_eq!(counts.definitions, 3);
    assert_eq!(counts.pads, counts.pads_imported);
    assert_eq!(project.placements.len(), 11);
    assert_eq!(project.stackup.layers().len(), 4);
    let origin = board.origin.ok_or("origin")?;
    for (placement, fp) in project.placements.iter().zip(&board.footprints) {
        let at = kicad_world(origin, fp.at, [0.0, 0.0]);
        assert!(
            near(placement.position, at),
            "{:?} {at:?}",
            placement.position
        );
        assert_eq!(placement.layer == Layer::Bottom, fp.back);
    }
    // Each component and footprint takes its KiCad footprint's name (the
    // first footprint of its shape), never a numbered placeholder.
    let names: Vec<&str> = project
        .library
        .components
        .iter()
        .map(|c| c.meta.name.as_str())
        .chain(
            project
                .library
                .footprints
                .iter()
                .map(|f| f.meta.name.as_str()),
        )
        .collect();
    assert_eq!(names.len(), 6, "{names:?}");
    assert!(
        names
            .iter()
            .all(|n| !n.contains(':') && !n.contains("KiCad footprint")),
        "{names:?}"
    );
    // The import is a document to start from, as an opened file: there is
    // nothing to undo into half an import.
    assert!(!imported.engine.can_undo() && !imported.engine.can_redo());
    Ok(())
}

#[test]
fn pads_land_where_kicad_draws_them_on_both_sides_and_every_angle() -> Fallible<()> {
    let board = support::showcase();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let pads = placed_pads(project);
    let origin = board.origin.ok_or("origin")?;
    for (k, fp) in board.footprints.iter().enumerate().take(10) {
        for kpad in &fp.pads {
            if kpad.number.is_empty() {
                continue;
            }
            let ours = pad(project, &pads, k, kpad.number).ok_or("pad")?;
            let t = ours.pad_transform();
            // The hole (or the pad origin without one) is at `at`.
            let hole = ours.pad.hole.map_or([0.0, 0.0], |h| h.offset());
            let expect = kicad_world(origin, fp.at, [kpad.at[0], kpad.at[1]]);
            assert!(
                near(t.apply(hole), expect),
                "pad {} of footprint {k}: {:?} against {expect:?}",
                kpad.number,
                t.apply(hole)
            );
            // Copper on the footprint's side unless placed on the other.
            let side = if fp.back { Layer::Bottom } else { Layer::Top };
            if ours.pad.kind == PadKind::Smd {
                assert_eq!(ours.layer, side);
            }
        }
    }
    Ok(())
}

#[test]
fn back_side_custom_pads_keep_their_board_shape() -> Fallible<()> {
    let board = support::showcase();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let pads = placed_pads(project);
    let origin = board.origin.ok_or("origin")?;
    // Footprint 9 is the back side at 30°; pad 5 is the custom pad at
    // (-2.5, 0.5) with a polygon vertex at (-0.3, 0.6) of its own frame.
    let fp = &board.footprints[9];
    assert!(fp.back && fp.at[2] == 30.0);
    let ours = pad(project, &pads, 9, "5").ok_or("pad 5")?;
    let PadShape::Custom { outline } = &ours.pad.shape else {
        return Err("pad 5 is not custom".into());
    };
    let t = ours.pad_transform();
    let ring: Vec<[f64; 2]> = std::iter::once(outline.start)
        .chain(outline.segments.iter().map(|s| s.to()))
        .map(|p| t.apply(p))
        .collect();
    // KiCad draws pad-local points turned by the pad's board angle, with
    // no reflection: the stored data is already as seen from the top.
    let local = |q: [f64; 2]| {
        let (s, c) = fp.at[2].to_radians().sin_cos();
        [-2.5 + q[0] * c + q[1] * s, 0.5 - q[0] * s + q[1] * c]
    };
    let inside = kicad_world(origin, fp.at, local([-0.3, 0.6]));
    let mirrored = kicad_world(origin, fp.at, local([-0.3, -0.6]));
    let shrink = |p: [f64; 2], toward: [f64; 2]| {
        [
            p[0] + (toward[0] - p[0]) * 1e-3,
            p[1] + (toward[1] - p[1]) * 1e-3,
        ]
    };
    let center = t.apply([0.0, 0.0]);
    assert!(polygon_contains(&ring, shrink(inside, center)));
    assert!(!polygon_contains(&ring, mirrored));
    Ok(())
}

#[test]
fn a_drill_offset_moves_the_copper_not_the_hole() -> Fallible<()> {
    let board = support::showcase();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let pads = placed_pads(project);
    let origin = board.origin.ok_or("origin")?;
    for k in [0, 2, 8] {
        let fp = &board.footprints[k];
        let ours = pad(project, &pads, k, "10").ok_or("pad 10")?;
        let hole = ours.pad.hole.ok_or("no hole")?;
        assert!(
            matches!(hole.shape, HoleShape::Slot { width_nm, length_nm, .. }
                if width_nm.get() == 600_000 && length_nm.get() == 1_200_000)
        );
        let t = ours.pad_transform();
        // The hole at `at` (1.5, 2.5); the copper 0.2 along the pad's x.
        assert!(near(
            t.apply(hole.offset()),
            kicad_world(origin, fp.at, [1.5, 2.5])
        ));
        assert!(near(
            t.apply([0.0, 0.0]),
            kicad_world(origin, fp.at, [1.7, 2.5])
        ));
    }
    Ok(())
}

#[test]
fn margins_resolve_pad_then_footprint_then_board() -> Fallible<()> {
    let board = support::showcase();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let pads = placed_pads(project);
    let get = |k: usize, n: &str| pad(project, &pads, k, n).map(|p| p.pad.clone());
    let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
    let expansion = |p: &sparksmith_core::library::Pad| match p.mask {
        MaskRule::Expansion(e) => e.to_mm_f64(),
        _ => f64::NAN,
    };
    let reduction = |p: &sparksmith_core::library::Pad| match p.paste {
        PasteRule::Reduction(r) => r.to_mm_f64(),
        _ => f64::NAN,
    };
    // The board's 0.05 mask and −0.02 paste margins.
    let p1 = get(0, "1").ok_or("pad 1")?;
    assert!(close(expansion(&p1), 0.05) && close(reduction(&p1), 0.02));
    // The pad's own 0.1 and −0.05.
    let p7 = get(0, "7").ok_or("pad 7")?;
    assert!(close(expansion(&p7), 0.1) && close(reduction(&p7), 0.05));
    // The footprint's 0.07; a stored 0 on the pad is unset.
    let u99 = |n: &str| get(10, n).ok_or("U99 pad");
    assert!(close(expansion(&u99("1")?), 0.07));
    assert!(close(expansion(&u99("5")?), 0.07));
    // A relative margin of −0.1 of a 1 mm pad, with the board's −0.02.
    assert!(close(reduction(&u99("3")?), 0.12));
    // A plated pad on every side, mask open, no paste.
    let p9 = get(0, "9").ok_or("pad 9")?;
    assert_eq!(p9.kind, PadKind::PlatedThrough);
    assert_eq!(p9.paste, PasteRule::None);
    // The non-square round rect with a relative margin: KiCad takes the
    // margin per axis (−0.03 − 0.1 · 1.2 and −0.03 − 0.1 · 0.6) and rebuilds
    // the radius from the ratio on the new size, a paste window here.
    assert_eq!(imported.report.count(Case::PasteRatioWindow), 1);
    assert_eq!(imported.report.count(Case::PasteRatio), 0);
    let p4 = u99("4")?;
    let PasteRule::Windows(windows) = &p4.paste else {
        return Err("pad 4 has no paste window".into());
    };
    let [window] = &windows[..] else {
        return Err("one window expected".into());
    };
    let ShapeNm::Rect { a, b, radius_nm } = &window.shape else {
        return Err("a rectangular window expected".into());
    };
    let (w, h) = (
        (b.x_nm.get() - a.x_nm.get()) as f64 / 1e6,
        (b.y_nm.get() - a.y_nm.get()) as f64 / 1e6,
    );
    assert!(close(w, 0.9) && close(h, 0.42), "{w} × {h}");
    assert!(close(radius_nm.to_mm_f64(), 0.105));
    // A round rect's radius is a ratio of its short side.
    let p2 = get(0, "2").ok_or("pad 2")?;
    assert!(matches!(p2.shape, PadShape::RoundRect { radius_ratio, .. } if radius_ratio == 0.25));
    Ok(())
}

fn one_footprint(pads: Vec<Pad>) -> Board {
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![Footprint::new("U1", [10.0, 10.0, 0.0], false, pads)],
        items: String::new(),
    }
}

#[test]
fn paste_only_pads_become_windows_of_the_pad_under_them() -> Fallible<()> {
    let mut pads = vec![
        Pad::smd("9", "rect", [0.0, 0.0, 0.0], [3.0, 3.0]).layers("\"F.Cu\" \"F.Mask\""),
        Pad::smd("1", "rect", [-2.5, 0.0, 0.0], [0.6, 0.3]),
    ];
    for (x, y) in [(-0.7, -0.7), (0.7, -0.7), (-0.7, 0.7), (0.7, 0.7)] {
        pads.push(Pad::smd("", "rect", [x, y, 0.0], [1.0, 1.0]).layers("\"F.Paste\""));
    }
    let imported = import(one_footprint(pads).text().as_bytes())?;
    let placed = placed_pads(imported.engine.project());
    let ep = placed.iter().find(|p| p.pad.number == "9").ok_or("EP")?;
    let PasteRule::Windows(windows) = &ep.pad.paste else {
        return Err("no windows".into());
    };
    assert_eq!(windows.len(), 4);
    assert_eq!(imported.report.count(Case::PasteWindow), 4);
    // Nothing else became a pad.
    assert_eq!(placed.len(), 2);
    Ok(())
}

/// A filled footprint polygon on `layer` through `points` (footprint
/// frame, KiCad's axes).
fn poly(layer: &str, points: &[[f64; 2]]) -> String {
    let pts: Vec<String> = points
        .iter()
        .map(|p| format!("(xy {} {})", p[0], p[1]))
        .collect();
    format!(
        " (fp_poly (pts {}) (stroke (width 0.01) (type solid)) (fill solid) (layer \"{layer}\"))",
        pts.join(" ")
    )
}

/// A box about `c` of half-size `h`, corners sharp.
fn square(c: [f64; 2], h: [f64; 2]) -> Vec<[f64; 2]> {
    vec![
        [c[0] - h[0], c[1] - h[1]],
        [c[0] + h[0], c[1] - h[1]],
        [c[0] + h[0], c[1] + h[1]],
        [c[0] - h[0], c[1] + h[1]],
    ]
}

/// A pad with copper only (no mask or paste layer) whose openings the
/// footprint draws as polygons on the mask and paste layers (a footprint
/// generator's habit): each polygon that grows the pad's land evenly
/// becomes the pad's mask expansion or paste rule, on either side and at
/// any angle, instead of a tented pad under a refused drawing. A polygon
/// that is no pad's opening stays refused.
#[test]
fn footprint_mask_and_paste_polygons_become_their_pads_openings() -> Fallible<()> {
    let front = Footprint::new(
        "J1",
        [10.0, 10.0, 30.0],
        false,
        vec![Pad::smd("1", "rect", [1.0, 0.5, 30.0], [0.6, 1.2]).layers("\"F.Cu\"")],
    );
    let back = Footprint::new(
        "J2",
        [20.0, 10.0, 90.0],
        true,
        vec![Pad::smd("1", "rect", [-1.0, 0.0, 90.0], [0.8, 0.8]).layers("\"B.Cu\"")],
    );
    let mut board = one_footprint(vec![]);
    board.footprints = vec![front, back];
    board.footprints[0].extra = [
        poly("F.Mask", &square([1.0, 0.5], [0.4, 0.7])),
        poly("F.Paste", &square([1.0, 0.5], [0.3, 0.6])),
        // Nobody's opening: a window between pads.
        poly("F.Mask", &square([3.0, 3.0], [0.2, 0.2])),
    ]
    .concat();
    board.footprints[1].extra = poly("B.Mask", &square([-1.0, 0.0], [0.45, 0.45]));
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let placed = placed_pads(project);
    let of = |designator: &str| {
        let instance = project.instance_by_designator(designator)?.id;
        placed.iter().find(|p| p.instance == instance)
    };
    let j1 = of("J1").ok_or("J1")?;
    let MaskRule::Expansion(e) = j1.pad.mask else {
        return Err(format!("J1 mask {:?}", j1.pad.mask).into());
    };
    assert!((e.to_mm_f64() - 0.1).abs() < 0.002, "{e:?}");
    let PasteRule::Reduction(r) = j1.pad.paste else {
        return Err(format!("J1 paste {:?}", j1.pad.paste).into());
    };
    assert!(r.to_mm_f64().abs() < 0.002, "{r:?}");
    let j2 = of("J2").ok_or("J2")?;
    let MaskRule::Expansion(e) = j2.pad.mask else {
        return Err(format!("J2 mask {:?}", j2.pad.mask).into());
    };
    assert!((e.to_mm_f64() - 0.05).abs() < 0.002, "{e:?}");
    assert_eq!(imported.report.count(Case::FootprintOpenings), 3);
    assert_eq!(imported.report.count(Case::FootprintFabGraphics), 1);
    Ok(())
}

#[test]
fn a_fiducial_is_a_part_without_terminals() -> Fallible<()> {
    let fiducial = Pad::smd("", "circle", [0.0, 0.0, 0.0], [1.0, 1.0])
        .layers("\"F.Cu\" \"F.Mask\"")
        .with("(property pad_prop_fiducial_loc) (solder_mask_margin 0.5)");
    let imported = import(one_footprint(vec![fiducial]).text().as_bytes())?;
    let project = imported.engine.project();
    let placed = placed_pads(project);
    assert_eq!(placed.len(), 1);
    // No made-up terminal: the pad stays a fiducial and lands none.
    assert_eq!(placed[0].pad.kind, PadKind::Fiducial);
    assert_eq!(
        placed[0].pad.fiducial_role,
        Some(sparksmith_core::library::FiducialRole::Local)
    );
    assert_eq!(placed[0].terminal, None);
    let instance = project.instance_by_designator("U1").ok_or("U1")?;
    let component = project
        .library
        .component(instance.component)
        .ok_or("component")?;
    assert!(component.definition.terminals.is_empty());
    assert_eq!(
        component.definition.assembly,
        sparksmith_core::library::tie::Assembly::CopperOnly
    );
    assert_eq!(imported.report.count(Case::NoTerminals), 1);
    // On the sheet like any part KiCad has in its schematic.
    assert!(!instance.board_only);
    assert_eq!(project.occurrences_of(instance.id).count(), 1);
    Ok(())
}

#[test]
fn a_part_not_in_the_schematic_is_board_only() -> Fallible<()> {
    let hole = Pad::npth([0.0, 0.0, 0.0], 3.2);
    let mut board = one_footprint(vec![hole]);
    board.footprints[0].extra = " (attr board_only exclude_from_bom)".into();
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let instance = project.instance_by_designator("U1").ok_or("U1")?;
    assert!(instance.board_only);
    assert_eq!(project.occurrences_of(instance.id).count(), 0);
    assert!(project.placement_of(instance.id).is_some());
    assert_eq!(imported.report.count(Case::BoardOnly), 1);
    Ok(())
}

#[test]
fn unsupported_versions_tokens_and_malformed_files_are_refused() -> Fallible<()> {
    let text = support::showcase().text();
    let newer = text.replacen("(version 20240108)", "(version 20250114)", 1);
    assert!(matches!(
        import(newer.as_bytes()),
        Err(ImportError::Board(BoardError::Version(20250114)))
    ));
    let older = text.replacen("(version 20240108)", "(version 20211014)", 1);
    assert!(matches!(
        import(older.as_bytes()),
        Err(ImportError::Board(BoardError::Version(_)))
    ));
    assert!(matches!(
        import(&text.as_bytes()[..text.len() - 3]),
        Err(ImportError::Board(BoardError::Syntax(_)))
    ));
    assert!(matches!(
        import(b"(kicad_sch (version 20231120))"),
        Err(ImportError::Board(BoardError::NotABoard))
    ));
    // A pad with a token we do not know is refused, not guessed.
    let pads = vec![
        Pad::smd("1", "rect", [0.0, 0.0, 0.0], [1.0, 1.0]),
        Pad::smd("2", "rect", [2.0, 0.0, 0.0], [1.0, 1.0]).with("(future_shape_token 1)"),
    ];
    let imported = import(one_footprint(pads).text().as_bytes())?;
    assert_eq!(imported.report.count(Case::PadUnknownToken), 1);
    assert_eq!(placed_pads(imported.engine.project()).len(), 1);
    Ok(())
}

#[test]
fn a_definition_frame_is_independent_of_placement() -> Fallible<()> {
    // The same footprint at 0° and 90° gives one definition whose pads sit
    // where an unturned footprint has them.
    let pads = |a: f64| {
        vec![
            Pad::smd("1", "rect", [1.0, 0.5, a], [0.8, 0.4]),
            Pad::smd("2", "rect", [-1.0, 0.5, a], [0.8, 0.4]),
        ]
    };
    let board = Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: vec![
            Footprint::new("R1", [10.0, 10.0, 0.0], false, pads(0.0)),
            Footprint::new("R2", [20.0, 10.0, 90.0], false, pads(90.0)),
        ],
        items: String::new(),
    };
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.counts.definitions, 1);
    let placed = placed_pads(imported.engine.project());
    let def = &placed[0].pad;
    // KiCad's (1, 0.5) with y down is (1, −0.5) with y up.
    assert_eq!(def.position(), [1.0, -0.5]);
    assert_eq!(def.rotation_deg(), 0.0);
    // Without an outline the board's corner goes where everything lies on
    // it: R1's origin, 10 mm left of R2's.
    let first = placed[0].pad_transform().apply([0.0, 0.0]);
    let origin = [first[0] - 1.0, first[1] + 0.5];
    assert!(origin[0] > 0.0 && origin[1] > 0.0, "{origin:?}");
    let turned = Transform2 {
        translate: [origin[0] + 10.0, origin[1]],
        rotation_deg: 90.0,
        mirror: false,
    };
    let second = placed
        .iter()
        .find(|p| p.instance != placed[0].instance)
        .ok_or("R2")?;
    assert!(near(
        second.pad_transform().apply([0.0, 0.0]),
        turned.apply(def.position())
    ));
    Ok(())
}

/// Run 7, finding 1: a resistor, an LED and a capacitor whose KiCad
/// footprints have equal pads but different names stay three components,
/// each under its own footprint's name, and three BOM lines; two copies
/// of one footprint still share one.
#[test]
fn distinct_footprints_with_equal_pads_stay_distinct() -> Fallible<()> {
    let pads = || {
        vec![
            Pad::smd("1", "roundrect", [-0.95, 0.0, 0.0], [0.9, 1.3])
                .with("(roundrect_rratio 0.25)"),
            Pad::smd("2", "roundrect", [0.95, 0.0, 0.0], [0.9, 1.3])
                .with("(roundrect_rratio 0.25)"),
        ]
    };
    let part = |name: &str, reference: &str, x: f64| {
        let mut fp = Footprint::new(reference, [x, 10.0, 0.0], false, pads());
        fp.name = name.to_owned();
        fp
    };
    let board = Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: vec![
            part("Lib:R_0805", "R1", 10.0),
            part("Lib:LED_0805", "D1", 15.0),
            part("Lib:C_0805", "C1", 20.0),
            part("Lib:R_0805", "R2", 25.0),
            part("Other:R_0805", "R3", 30.0),
        ],
        items: String::new(),
    };
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.counts.definitions, 4);
    let project = imported.engine.project();
    let component = |designator: &str| -> Fallible<String> {
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator.to_owned())?;
        Ok(project
            .library
            .component(instance.component)
            .ok_or("component")?
            .meta
            .name
            .clone())
    };
    // The visible name is the footprint's own; the library nickname and
    // full id stay in the provenance. Another library's footprint of the
    // same name is numbered apart, and the report counts both.
    assert_eq!(component("R1")?, "R_0805");
    assert_eq!(component("D1")?, "LED_0805");
    assert_eq!(component("C1")?, "C_0805");
    assert_eq!(component("R2")?, "R_0805");
    assert_eq!(component("R3")?, "R_0805, 2");
    assert_eq!(imported.report.count(Case::NameShortened), 4);
    assert_eq!(imported.report.count(Case::NameNumbered), 1);
    let r3 = project.instance_by_designator("R3").ok_or("R3")?;
    let provenance = &project
        .library
        .component(r3.component)
        .ok_or("component")?
        .meta
        .provenance;
    assert!(
        provenance.contains("KiCad footprint: Other:R_0805"),
        "{provenance}"
    );
    let bom = sparksmith_core::completeness::bom(project)?;
    let lines: Vec<Vec<&str>> = bom
        .lines
        .iter()
        .map(|l| l.designators.iter().map(String::as_str).collect())
        .collect();
    assert_eq!(
        lines,
        [vec!["C1"], vec!["D1"], vec!["R1", "R2"], vec!["R3"]]
    );
    Ok(())
}

/// Run 7, finding 2: without a generator (this importer alone) each
/// footprint is a project part of its lands, named as an authored part is,
/// by its footprint and value; two copies of one footprint with other
/// values are two parts, and "~" is no value. No import-only field is
/// made (a generated part's value is its typed fields:
/// `kicad_models::tests::values_become_the_generated_parts_typed_fields`).
#[test]
fn kicad_values_become_the_parts_values() -> Fallible<()> {
    let part = |reference: &str, x: f64, value: &str| {
        let mut fp = Footprint::new(
            reference,
            [x, 10.0, 0.0],
            false,
            vec![
                Pad::smd("1", "rect", [-0.95, 0.0, 0.0], [0.9, 1.3]),
                Pad::smd("2", "rect", [0.95, 0.0, 0.0], [0.9, 1.3]),
            ],
        );
        fp.name = "Lib:R_0805".to_owned();
        fp.value = Some(value.to_owned());
        fp
    };
    let board = Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: vec![
            part("R1", 10.0, "330"),
            part("R2", 15.0, "10k"),
            part("R3", 20.0, "~"),
        ],
        items: String::new(),
    };
    let imported = import(board.text().as_bytes())?;
    let project = imported.engine.project();
    let name = |designator: &str| -> Fallible<String> {
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator.to_owned())?;
        Ok(project
            .library
            .component(instance.component)
            .ok_or("component")?
            .meta
            .name
            .clone())
    };
    assert_eq!(name("R1")?, "R_0805 330");
    assert_eq!(name("R2")?, "R_0805 10k");
    assert_eq!(name("R3")?, "R_0805");
    // The part selects the standard part value field, which holds each
    // part's value; every field is a standard one: no import-only field.
    let standard = imported
        .engine
        .context()
        .ok_or("standards")?
        .standards()
        .fields
        .library();
    let names: Vec<&str> = project
        .library
        .fields
        .iter()
        .map(|f| f.meta.name.as_str())
        .collect();
    assert!(names.contains(&"Value"), "{names:?}");
    assert!(
        project.library.fields.iter().all(|f| project
            .library
            .origins
            .iter()
            .any(|o| o.local.id == f.id && o.origin.library == standard)),
        "no import-only field"
    );
    let value = |designator: &str| -> Fallible<Option<String>> {
        let instance = project
            .instance_by_designator(designator)
            .ok_or(designator.to_owned())?;
        Ok(sparksmith_core::instances::value_text(
            &project.library,
            instance,
        )?)
    };
    assert_eq!(value("R1")?.as_deref(), Some("330"));
    assert_eq!(value("R2")?.as_deref(), Some("10k"));
    let bom = sparksmith_core::completeness::bom(project)?;
    assert_eq!(bom.lines.len(), 3);
    Ok(())
}

#[test]
fn a_placeholder_body_is_centred_on_its_pads_not_on_the_footprint_origin() -> Fallible<()> {
    // A 1×10 header whose origin is pin 1, as headers commonly are.
    const NUMBERS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"];
    let pads = NUMBERS
        .iter()
        .enumerate()
        .map(|(k, n)| Pad::tht(n, "oval", [0.0, 2.54 * k as f64, 0.0], [1.7, 1.7], "1.0"))
        .collect();
    let imported = import(one_footprint(pads).text().as_bytes())?;
    let project = imported.engine.project();
    assert_eq!(project.library.packages.len(), 1);
    let package = &project.library.packages[0].definition;
    let [length, width, _] = package.envelope();
    assert!((length - 1.7).abs() < 1e-6 && (width - (9.0 * 2.54 + 1.7)).abs() < 1e-6);
    // KiCad's y points down; ours up: the pads run from 0 to -22.86 mm.
    let [x, y, z] = package.placement.translation();
    assert!(
        x.abs() < 1e-6 && (y + 4.5 * 2.54).abs() < 1e-6 && z == 0.0,
        "{x} {y} {z}"
    );
    Ok(())
}

/// A declaration with no copper in the gap cannot be fabricated as a tie.
#[test]
fn a_missing_bridge_is_reported_by_designator() -> Fallible<()> {
    let mut tie = Footprint::new(
        "NT1",
        [10.0, 10.0, 0.0],
        false,
        vec![
            Pad::smd("1", "rect", [-0.5, 0.0, 0.0], [0.6, 0.6]).with("(net 1 \"GND\")"),
            Pad::smd("2", "rect", [0.5, 0.0, 0.0], [0.6, 0.6]).with("(net 2 \"AGND\")"),
        ],
    );
    tie.name = "Synthetic:Tie".into();
    tie.extra = " (net_tie_pad_groups \"1, 2\")".into();
    let board = Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![tie],
        items: "  (net 1 \"GND\") (net 2 \"AGND\")
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n"
            .into(),
    };
    let text = board.text();
    let imported = import(text.as_bytes())?;
    let report = &imported.report;
    assert_eq!(report.count(Case::NetTie), 1);
    let detail = report
        .details()
        .iter()
        .find(|d| d.case == Case::NetTie)
        .ok_or("no detail naming the net tie")?;
    assert_eq!(detail.footprint, "NT1");
    let title = Case::NetTie.describe().2;
    assert!(title.contains("not imported"), "{title}");
    let named = report.summary_named();
    assert!(named.contains("NT1"), "{named}");
    assert!(!report.summary().contains("NT1"));
    let drawn=text.replace("(net_tie_pad_groups \"1, 2\")","(net_tie_pad_groups \"1, 2\") (fp_rect (start -0.5 -0.1) (end 0.5 0.1) (stroke (width 0) (type solid)) (fill solid) (layer \"F.Cu\"))");
    let with_copper = import(drawn.as_bytes())?;
    assert_eq!(with_copper.report.count(Case::ImportedTie), 1);
    assert_eq!(with_copper.report.count(Case::NetTie), 0);
    let project = with_copper.engine.project();
    let rows = sparksmith_core::instances::ties::rows(project);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].purpose,
        sparksmith_core::library::tie::TiePurpose::None
    );
    let bridges = sparksmith_core::layout::placed_bridges(project);
    assert_eq!(bridges.len(), 1);
    assert_eq!(bridges[0].nets.len(), 2);
    Ok(())
}

fn two_group_board() -> Board {
    let mut part = Footprint::new(
        "NT1",
        [10., 10., 0.],
        false,
        vec![
            Pad::smd("1", "rect", [-1., 0., 0.], [0.6, 0.6]).with("(net 1 \"A\")"),
            Pad::smd("2", "rect", [1., 0., 0.], [0.6, 0.6]).with("(net 2 \"B\")"),
            Pad::smd("3", "rect", [-1., 2., 0.], [0.6, 0.6]).with("(net 3 \"C\")"),
            Pad::smd("4", "rect", [1., 2., 0.], [0.6, 0.6]).with("(net 4 \"D\")"),
        ],
    );
    part.extra = r#"(net_tie_pad_groups "1,2" "3,4")
      (fp_rect (start -1 -0.1) (end 1 0.1) (stroke (width 0) (type solid)) (fill solid) (layer "F.Cu"))
      (fp_rect (start -1 1.9) (end 1 2.1) (stroke (width 0) (type solid)) (fill solid) (layer "F.Cu"))"#.into();
    Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: vec![part],
        items: r#"(net 1 "A") (net 2 "B") (net 3 "C") (net 4 "D")
    (gr_rect (start 0 0) (end 20 20) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#
            .into(),
    }
}

#[test]
fn two_pad_groups_on_one_footprint_are_independent_ties() -> Fallible<()> {
    let imported = import(two_group_board().text().as_bytes())?;
    let project = imported.engine.project();
    assert_eq!(project.instances.len(), 1);
    let rows = sparksmith_core::instances::ties::rows(project);
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0].group, rows[1].group);
    let network = sparksmith_core::instances::ties::Network::of(&rows);
    let mut groups = std::collections::BTreeSet::new();
    let graph = sparksmith_core::layout::CopperGraph::of(project);
    for row in rows {
        let expected = if row.ends.contains_key(&sparksmith_core::TerminalId(1)) {
            ["A", "B"]
        } else {
            ["C", "D"]
        };
        let names = row
            .ends
            .values()
            .flatten()
            .map(|n| project.net_name(*n))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(names, expected.into());
        let members = row
            .ends
            .values()
            .flatten()
            .map(|n| network.group(*n))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(members.len(), 1);
        groups.extend(members);
        let join = graph
            .ties()
            .iter()
            .find(|t| t.bridge.group == row.group)
            .ok_or("missing group bridge")?;
        assert_eq!(join.islands.len(), 2);
        assert!(join.foreign.is_empty());
    }
    assert_eq!(groups.len(), 2);
    Ok(())
}

#[test]
fn curved_bridge_follows_the_three_point_arc_and_preserves_the_second_group() -> Fallible<()> {
    let mut board = two_group_board();
    board.footprints[0].extra = r#"(net_tie_pad_groups "1,2" "3,4")
      (fp_arc (start -1 0) (mid 0 -1) (end 1 0) (stroke (width 0.12) (type solid)) (layer "F.Cu"))
      (fp_poly (pts (xy -1 2) (xy 0 1.97) (xy 1 2) (xy 0 2.03))
        (stroke (width 0.12) (type solid)) (fill none) (layer "F.Cu"))"#
        .into();
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::ImportedTie), 1);
    let project = imported.engine.project();
    let owner = project.instances.first().ok_or("missing tie")?;
    let view = sparksmith_core::instances::resolve(&project.library, owner)?
        .physical
        .ok_or("missing tie footprint")?;
    let bridges = &view.footprint.definition.bridges;
    assert_eq!(bridges.len(), 2);
    let arc = bridges
        .iter()
        .find(|b| b.pads.contains(&sparksmith_core::PadId(1)))
        .ok_or("arc missing")?;
    let ring = arc.outline.flatten(0.00025);
    assert!(sparksmith_core::geometry::polygon_contains(&ring, [0., 1.]));
    assert!(!sparksmith_core::geometry::polygon_contains(
        &ring,
        [0., 0.]
    ));
    Ok(())
}

#[test]
fn ambiguous_net_tie_groups_preserve_lands_for_review() -> Fallible<()> {
    let source = two_group_board().text();
    for (name, edited) in [
        (
            "shared pad",
            source.replace(r#""1,2" "3,4""#, r#""1,2" "2,3,4""#),
        ),
        ("unknown pad", source.replace(r#""3,4""#, r#""3,5""#)),
        (
            "dashed stroke",
            source.replace(
                "(fp_rect (start -1 -0.1) (end 1 0.1) (stroke (width 0) (type solid)) (fill solid)",
                "(fp_line (start -1 0) (end 1 0) (stroke (width 0.05) (type dash))",
            ),
        ),
        (
            "visible copper text",
            source.replace(
                "(net_tie_pad_groups",
                r#"(fp_text user "A" (at 0 0) (layer "F.Cu")) (net_tie_pad_groups"#,
            ),
        ),
        ("empty declaration", source.replace(r#""1,2" "3,4""#, "")),
        (
            "cross-group drawing",
            source.replace("(end 1 0.1)", "(end 1 2.1)"),
        ),
        (
            "missing group bridge",
            source.replace("(start -1 1.9)", "(start 0.5 1.9)"),
        ),
    ] {
        assert_ne!(edited, source, "fixture for {name}");
        let imported = import(edited.as_bytes())?;
        assert_eq!(imported.report.count(Case::NetTie), 1, "{name}");
        assert_eq!(imported.report.count(Case::ImportedTie), 0, "{name}");
        let p = imported.engine.project();
        assert_eq!(placed_pads(p).len(), 4, "{name}");
        assert!(
            sparksmith_core::instances::ties::rows(p).is_empty(),
            "{name}"
        );
        assert!(
            sparksmith_core::layout::placed_bridges(p).is_empty(),
            "{name}"
        );
    }
    Ok(())
}

#[test]
fn a_dnp_mark_on_fabricated_ties_keeps_their_bridges_closed() -> Fallible<()> {
    let mut board = two_group_board();
    board.footprints[0].extra.push_str(" (attr dnp)");
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::CopperTieDnp), 1);
    assert_eq!(imported.report.count(Case::NotPopulatedAttribute), 0);
    let rows = sparksmith_core::instances::ties::rows(imported.engine.project());
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r.state == sparksmith_core::library::tie::JumperState::Closed)
    );
    Ok(())
}
