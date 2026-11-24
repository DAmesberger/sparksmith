//! Read the physical paths back from both exported PDFs. Each input edge is a
//! semicircle on a known diameter, so its inward chord error can be measured
//! directly without the board flattener.
use sparksmith_core::Engine;
use sparksmith_core::fabrication::{ProductionSnapshot, drawing};
use sparksmith_core::ids::GraphicId;
use sparksmith_core::library::footprint_source::SketchCurveId;
use sparksmith_core::library::{
    DrawingLayer, Fill, FootprintGraphic, Item, Outline, PadShape, Segment, ShapeNm, Side,
    standards::Context,
};
use sparksmith_core::model::board::{
    BoardFeatureId, BoardShape, BoardSketch, CutoutKind, CutoutRole, SolvedCutout,
};
use sparksmith_core::rules::{RuleKey, RuleValue};
use sparksmith_core::units::{Nm, PointNm};
use sparksmith_io::fab_drawings::{assembly_pdf, fabrication_pdf};
use std::f64::consts::TAU;

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;
type Point = [f64; 2];
const EDGES: usize = 400;

fn end(i: usize, radius: f64, sign: f64) -> Point {
    let a = sign * TAU * (i % EDGES) as f64 / EDGES as f64;
    [radius * a.cos(), radius * a.sin()]
}

fn scallops(radius: f64, sign: f64) -> Outline {
    let at = |i| end(i, radius, sign).map(|v| v + 200.0);
    Outline {
        start: at(0),
        segments: (1..=EDGES)
            .map(|i| Segment::Arc {
                to: at(i),
                bulge: sign,
            })
            .collect(),
    }
}

// Sparksmith writes uncompressed PDF content streams. Select the two long
// manufacturing contours, ignoring the title block, table and drill symbols.
fn rings(pdf: &[u8], min_points: usize) -> Fallible<Vec<Vec<Point>>> {
    let mut rings = Vec::new();
    let mut path = Vec::new();
    for line in String::from_utf8_lossy(pdf).lines() {
        let mut fields = line.split_ascii_whitespace();
        match (fields.next(), fields.next(), fields.next(), fields.next()) {
            (Some(x), Some(y), Some(op @ ("m" | "l")), None) => {
                if op == "m" {
                    path.clear();
                }
                path.push([x.parse()?, y.parse()?]);
            }
            (Some("h" | "S"), None, None, None) if path.len() > min_points => {
                rings.push(std::mem::take(&mut path));
            }
            _ => path.clear(),
        }
    }
    Ok(rings)
}

fn dist(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

// Check both ends and the midpoint of every chord against the exact circle,
// in order, and require every stored semicircle endpoint to be reached.
fn check_ring(points: &[Point], radius: f64, sign: f64, paper_start: Point, mirror: f64) {
    let points_per_mm = 0.5 * 72.0 / 25.4;
    let board = |p: Point| {
        [
            180.0 + mirror * (p[0] - paper_start[0]) / points_per_mm,
            (p[1] - paper_start[1]) / points_per_mm,
        ]
    };
    // Reading coordinates printed as PDF f32 points on A3 contributes less
    // than 0.0002 board mm at the stated scale.
    let roundoff = 0.0002;
    let mut prev = board(points[0]);
    assert!(dist(prev, end(0, radius, sign)) < roundoff);
    let mut edge = 0;
    for p in points.iter().skip(1).chain(points.first()) {
        let next = board(*p);
        if dist(prev, next) < roundoff {
            continue;
        }
        assert!(edge < EDGES);
        let a = end(edge, radius, sign);
        let b = end(edge + 1, radius, sign);
        let center = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
        let r = dist(a, b) / 2.0;
        let m = [(prev[0] + next[0]) / 2.0, (prev[1] + next[1]) / 2.0];
        assert!((dist(next, center) - r).abs() < roundoff, "edge {edge}");
        assert!(r - dist(m, center) <= 0.01 + roundoff, "edge {edge}");
        let u = [prev[0] - center[0], prev[1] - center[1]];
        let v = [next[0] - center[0], next[1] - center[1]];
        assert!(sign * (u[0] * v[1] - u[1] * v[0]) > 0.0, "edge {edge}");
        if dist(next, b) < roundoff {
            edge += 1;
        }
        prev = next;
    }
    assert_eq!(edge, EDGES);
    assert!(dist(prev, board(points[0])) < roundoff);
}

#[test]
fn the_complete_outer_edge_and_cutout_survive_all_three_pdf_views() -> Fallible<()> {
    let _scope = Context::fixture()?.enter();
    let mut file = Engine::showcase()?.to_file();
    let p = &mut file.project;
    let mut solved = p.board().into_owned();
    solved.outer = scallops(180.0, 1.0);
    solved.outer_curves.clear();
    solved.edges.clear();
    let curves: Vec<_> = (0..EDGES)
        .map(|i| SketchCurveId(format!("arc{i}")))
        .collect();
    solved.cutouts = vec![SolvedCutout {
        id: Some(BoardFeatureId(1)),
        kind: CutoutKind::Cutout,
        outline: scallops(150.0, -1.0),
        curves: curves.clone(),
    }];
    p.board_size = solved.bounds().max;
    let RuleValue::Length(outline_fit) = p.rules.resolve(RuleKey::BoardOutlineFit).value else {
        return Err("missing outline fit".into());
    };
    p.board_shape = BoardShape::Sketch(Box::new(BoardSketch {
        evaluator: Default::default(),
        sketch: Default::default(),
        parameters: vec![],
        cutouts: vec![CutoutRole {
            id: BoardFeatureId(1),
            kind: CutoutKind::Cutout,
            curves,
        }],
        holes: vec![],
        edges: vec![],
        next_feature: 2,
        solve: Default::default(),
        solved,
        current: true,
        outline_fit,
        fault: None,
        idle: vec![],
    }));
    let engine = Engine::from_file(file)?;
    let mut project = engine.project().clone();
    // Use a known circle for each land and placed body, retaining the
    // showcase's placements and two populated sides.
    for published in &mut project.library.footprints {
        let fp = &mut published.definition;
        for pad in &mut fp.pads {
            pad.shape = PadShape::Circle {
                diameter_nm: Nm::from_mm_f64_rounded(2.0)?,
            };
        }
        fp.graphics = vec![FootprintGraphic {
            id: GraphicId(fp.next_local),
            layer: DrawingLayer::Assembly,
            side: Side::Component,
            item: Item::Shape {
                shape: ShapeNm::Circle {
                    center: PointNm::ORIGIN,
                    radius_nm: Nm::from_mm_f64_rounded(3.0)?,
                },
                width_nm: Nm::from_mm_f64_rounded(0.1)?,
                fill: Fill::None,
            },
        }];
        fp.next_local += 1;
    }
    let snapshot = ProductionSnapshot::resolve(&project, None)?;
    let data = drawing::data(&snapshot, &Default::default(), engine.check_settings())?;
    for (pdf, mirrors) in [
        (fabrication_pdf(&data)?, vec![1.0]),
        (assembly_pdf(&data)?, vec![1.0, -1.0]),
    ] {
        assert!(String::from_utf8_lossy(&pdf).contains("Scale 1:2"));
        let board_rings = rings(&pdf, EDGES)?;
        assert_eq!(board_rings.len(), 2 * mirrors.len());
        for (pair, &mirror) in board_rings.chunks_exact(2).zip(&mirrors) {
            let origin = pair[0][0];
            check_ring(&pair[0], 180.0, 1.0, origin, mirror);
            check_ring(&pair[1], 150.0, -1.0, origin, mirror);
        }
        if mirrors.len() == 2 {
            let mut component_rings = rings(&pdf, 8)?
                .into_iter()
                .filter(|path| path.len() <= EDGES);
            for (side, mirror) in data.sides.iter().zip(mirrors) {
                let pad_count = side.parts.iter().map(|p| p.pads.len()).sum::<usize>();
                assert!(pad_count > 0 && !side.parts.is_empty());
                for radius in std::iter::repeat_n(1.0, pad_count)
                    .chain(std::iter::repeat_n(3.0, side.parts.len()))
                {
                    let ring = component_rings.next().ok_or("missing component contour")?;
                    let scale = 0.5 * 72.0 / 25.4;
                    let center = [ring[0][0] - mirror * radius * scale, ring[0][1]];
                    for (a, b) in ring.iter().zip(ring.iter().cycle().skip(1)) {
                        assert!((dist(*a, center) / scale - radius).abs() < 0.0002);
                        let mid = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                        assert!(radius - dist(mid, center) / scale <= 0.01 + 0.0002);
                    }
                }
            }
            assert!(component_rings.next().is_none());
        }
    }
    Ok(())
}
