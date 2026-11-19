//! No two bodies of a built board draw a face on the same plane, facing
//! the same way, over the same area. Such a pair z-fights: the depth
//! buffer cannot tell the two faces apart, and the view shows jagged
//! streaks of one through the other that change as the view moves (the
//! WROOM-3 module's pads seen from below once showed copper streaks
//! through their gold finish).
//!
//! Faces turned toward each other on one plane are two solids touching
//! (a part on its land, the mask on the laminate); each hides the other,
//! so they are allowed.
//!
//! The check covers where parts meet the board: every part body, lead,
//! termination, fillet and marking, and the lands, vias and finish under
//! them. Copper meeting copper is one conductor in one colour (tracks
//! entering via rings and pads, by design). Copper lying on the laminate
//! under the mask is checked elsewhere by the board's own tests.
use sparksmith_solidcraft::geom::{Vec3, prelude::*};
use sparksmith_solidcraft::material::BodyKind;
use sparksmith_solidcraft::scene::Geometry;

/// The bodies of a part or of what it stands on.
fn at_the_seat(kind: BodyKind) -> bool {
    part(kind)
        || matches!(
            kind,
            BodyKind::PadCopper | BodyKind::PadFinish | BodyKind::ViaCopper | BodyKind::ViaBore
        )
}

fn part(kind: BodyKind) -> bool {
    matches!(
        kind,
        BodyKind::PackageBody
            | BodyKind::ChipResistor
            | BodyKind::ChipCapacitor
            | BodyKind::Termination
            | BodyKind::Overcoat
            | BodyKind::Solder
            | BodyKind::PartBody
            | BodyKind::ModelBody
            | BodyKind::ModelLead
            | BodyKind::Marking
    )
}

fn copper(kind: BodyKind) -> bool {
    matches!(kind, BodyKind::PadCopper | BodyKind::ViaCopper)
}

/// Whether two bodies may not share a face: a part and anything at its
/// seat, or a land's finish and copper; never copper and copper.
fn checked(a: BodyKind, b: BodyKind) -> bool {
    if copper(a) && copper(b) {
        return false;
    }
    part(a)
        || part(b)
        || matches!(
            (a, b),
            (BodyKind::PadCopper, BodyKind::PadFinish) | (BodyKind::PadFinish, BodyKind::PadCopper)
        )
}

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Planes closer than this (mm) are one plane to the depth buffer at any
/// zoom the view allows.
const PLANE_MM: f64 = 1e-5;
/// Overlaps smaller than this (mm²) are a shared edge's rounding.
const AREA_MM2: f64 = 1e-6;

struct Face {
    body: usize,
    normal: Vec3,
    offset: f64,
    corners: [Vec3; 3],
}

fn faces(geometry: &[Geometry]) -> Vec<Face> {
    let mut out = Vec::new();
    for (body, g) in geometry.iter().enumerate() {
        if !at_the_seat(g.kind) {
            continue;
        }
        let mesh = g.world_mesh();
        for t in &mesh.triangles {
            let Some(p) = mesh.tri(t) else {
                continue;
            };
            let cross = (p[1] - p[0]).cross(p[2] - p[0]);
            let Some(normal) = cross.normalized() else {
                continue;
            };
            // Only faces of real area: slivers carry no visible fight.
            if cross.len() / 2.0 < AREA_MM2 {
                continue;
            }
            out.push(Face {
                body,
                normal,
                offset: normal.dot(p[0]),
                corners: p,
            });
        }
    }
    out
}

/// The 2D area two triangles share in their common plane.
fn shared_area(a: &Face, b: &Face) -> f64 {
    let n = a.normal;
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let Some(u) = n.cross(helper).normalized() else {
        return 0.0;
    };
    let v = n.cross(u);
    let flat = |p: Vec3| [p.dot(u), p.dot(v)];
    let mut poly: Vec<[f64; 2]> = a.corners.iter().map(|p| flat(*p)).collect();
    let mut clip: Vec<[f64; 2]> = b.corners.iter().map(|p| flat(*p)).collect();
    let signed = |pts: &[[f64; 2]]| -> f64 {
        (0..pts.len())
            .map(|i| {
                let (p, q) = (pts[i], pts[(i + 1) % pts.len()]);
                p[0] * q[1] - q[0] * p[1]
            })
            .sum::<f64>()
            / 2.0
    };
    if signed(&clip) < 0.0 {
        clip.reverse();
    }
    for i in 0..3 {
        let (e0, e1) = (clip[i], clip[(i + 1) % 3]);
        let inside = |p: [f64; 2]| {
            (e1[0] - e0[0]) * (p[1] - e0[1]) - (e1[1] - e0[1]) * (p[0] - e0[0]) >= 0.0
        };
        let cut = |p: [f64; 2], q: [f64; 2]| {
            let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
            let (ex, ey) = (e1[0] - e0[0], e1[1] - e0[1]);
            let den = ex * dy - ey * dx;
            let t = if den.abs() < 1e-30 {
                0.0
            } else {
                (ex * (e0[1] - p[1]) - ey * (e0[0] - p[0])) / den
            };
            [p[0] + t * dx, p[1] + t * dy]
        };
        let input = std::mem::take(&mut poly);
        for k in 0..input.len() {
            let (p, q) = (input[k], input[(k + 1) % input.len()]);
            match (inside(p), inside(q)) {
                (true, true) => poly.push(q),
                (true, false) => poly.push(cut(p, q)),
                (false, true) => {
                    poly.push(cut(p, q));
                    poly.push(q);
                }
                (false, false) => {}
            }
        }
        if poly.len() < 3 {
            return 0.0;
        }
    }
    signed(&poly).abs()
}

/// Pairs of bodies (kinds, by index) whose faces lie on one plane facing
/// the same way over a shared area, with the plane and that area.
pub(crate) fn coincident(geometry: &[Geometry]) -> Vec<String> {
    let mut faces = faces(geometry);
    // Group by direction, then sweep each direction's faces by offset.
    let key = |f: &Face| [f.normal.x, f.normal.y, f.normal.z].map(|c| (c * 1e4).round() as i64);
    faces.sort_by(|a, b| key(a).cmp(&key(b)).then(a.offset.total_cmp(&b.offset)));
    let mut found = std::collections::BTreeSet::new();
    for (i, a) in faces.iter().enumerate() {
        for b in faces.iter().skip(i + 1) {
            if key(b) != key(a) || b.offset - a.offset > PLANE_MM {
                break;
            }
            if a.body == b.body || !checked(geometry[a.body].kind, geometry[b.body].kind) {
                continue;
            }
            let pair = (a.body.min(b.body), a.body.max(b.body));
            if found.iter().any(|(p, _)| *p == pair) {
                continue;
            }
            let apart = (0..3).any(|k| {
                let get = |v: Vec3| [v.x, v.y, v.z][k];
                let lo = |f: &Face| f.corners.iter().map(|p| get(*p)).fold(f64::MAX, f64::min);
                let hi = |f: &Face| f.corners.iter().map(|p| get(*p)).fold(f64::MIN, f64::max);
                lo(a) > hi(b) + PLANE_MM || lo(b) > hi(a) + PLANE_MM
            });
            if apart {
                continue;
            }
            let area = shared_area(a, b);
            if area > AREA_MM2 {
                found.insert((
                    pair,
                    format!("plane n·p = {:.4} mm, {area:.2e} mm²", a.offset),
                ));
            }
        }
    }
    found
        .into_iter()
        .map(|((x, y), why)| {
            format!(
                "{:?} #{x} {:?}..{:?} and {:?} #{y} {:?}..{:?}: {why}",
                geometry[x].kind,
                geometry[x].min,
                geometry[x].max,
                geometry[y].kind,
                geometry[y].min,
                geometry[y].max
            )
        })
        .collect()
}

/// The WROOM-3 module board, the showcase (SOIC, QFN with its exposed pad
/// and a via in it, header, chip resistors and capacitors, tantalum) and
/// the demo board: no part fights its seat for a face.
#[test]
fn no_two_bodies_of_a_built_board_share_a_face() -> Fallible<()> {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let boards = [
        ("WROOM-3", super::fixtures::wroom_board()?.project().clone()),
        (
            "showcase",
            sparksmith_core::Engine::showcase()?.project().clone(),
        ),
        ("demo", crate::model_scene::tests::demo_project()?),
    ];
    for (name, project) in boards {
        let model = sparksmith_solidcraft::project::build(&project)?;
        let fights = coincident(&model.geometry);
        assert!(
            fights.is_empty(),
            "{name}: {} pairs, first {:#?}",
            fights.len(),
            fights.iter().take(12).collect::<Vec<_>>()
        );
    }
    Ok(())
}

/// The check finds what it looks for: a finish slab laid on its copper's
/// bottom face, as the finish once was.
#[test]
fn the_check_finds_a_face_shared_by_two_bodies() -> Fallible<()> {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    use sparksmith_solidcraft::geom::Mesh;
    let slab = |z0: f64, z1: f64| -> Fallible<Geometry> {
        let project = crate::model_scene::tests::demo_project()?;
        let model = sparksmith_solidcraft::project::build(&project)?;
        let mut g = model.geometry.first().cloned().ok_or("a body")?;
        let mut mesh = Mesh::default();
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for z in [z0, z1] {
            for c in corners {
                mesh.positions.push(Vec3::new(c[0], c[1], z));
                mesh.normals.push(Vec3::Z);
            }
        }
        // Bottom (facing down) and top (facing up).
        mesh.triangles
            .extend([[0, 2, 1], [0, 3, 2], [4, 5, 6], [4, 6, 7]]);
        g.mesh = std::sync::Arc::new(mesh);
        g.place = None;
        Ok(g)
    };
    let kind = |mut g: Geometry, kind| {
        g.kind = kind;
        g
    };
    let copper = kind(slab(0.0, 0.035)?, BodyKind::PadCopper);
    let finish = kind(slab(0.0, 0.040)?, BodyKind::PadFinish);
    assert_eq!(coincident(&[copper.clone(), finish]).len(), 1);
    let lifted = kind(slab(0.005, 0.040)?, BodyKind::PadFinish);
    assert!(coincident(&[copper.clone(), lifted]).is_empty());
    // A lead standing on the copper touches it face to face: allowed.
    let lead = kind(slab(0.035, 0.07)?, BodyKind::ModelLead);
    assert!(coincident(&[copper, lead]).is_empty());
    Ok(())
}

/// The demo board with its first part's model made by `generator` from
/// `preset` (with `params`): the shipped bundle's part file, as the 3D
/// view builds it. Only the first part stays placed: the demo's two
/// resistors share the package 8.25 mm apart, closer than a large part's
/// span, and their leads would overlap (the board, not the part file,
/// once made an LQFP-32's facing rows look drawn twice).
fn demo_with_generated(
    generator: &str,
    preset: &str,
    params: &serde_json::Value,
) -> Fallible<sparksmith_core::Project> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let mut session = sparksmith_automation::Session::new(sparksmith_core::Engine::demo()?);
    session.set_shipped(sparksmith_io::shipped::ShippedLibrary::at(&root)?);
    let revision = session.engine().revision();
    let made = session.dispatch(serde_json::json!({
        "command": "library.generate_part",
        "params": {"generator": generator, "scope": "package", "preset": preset,
                   "params": params, "expected_revision": revision},
    }));
    if made["ok"] != true {
        return Err(format!("{generator} {preset} {params}: not made: {made}").into());
    }
    let mut project = session.engine().project().clone();
    let generated = project
        .library
        .packages
        .last()
        .map(|p| p.definition.clone())
        .ok_or("no package")?;
    let target = &mut project
        .library
        .packages
        .first_mut()
        .ok_or("no demo package")?
        .definition;
    target.envelope_nm = generated.envelope_nm;
    target.model = generated.model;
    project.placements.truncate(1);
    Ok(project)
}

/// Every part file of the IC and discrete generators (one preset each, and
/// the variants a preset does not reach), placed on the demo board: no two of
/// its bodies, nor a body and the board under it, share a face (a pin-1
/// laser dot once lay flush with the package top and fought it, and a land
/// grid's lands lay flush with its underside).
#[test]
fn no_generated_part_shares_a_face() -> Fallible<()> {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let generators = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../generators");
    let variants: Vec<(String, String, serde_json::Value)> = [
        (
            "no-lead",
            "no-lead-qfn-16-4x4-0.65-ti-rgv",
            serde_json::json!({"wettable_flank": true}),
        ),
        (
            "no-lead",
            "no-lead-dfn-8-2x3-0.5",
            serde_json::json!({"wettable_flank": true}),
        ),
        (
            "gull-wing",
            "gull-wing-tssop-14-4.4",
            serde_json::json!({"pin1_bevel": true}),
        ),
    ]
    .into_iter()
    .map(|(g, p, v)| (g.to_owned(), p.to_owned(), v))
    .collect();
    let mut cases = variants;
    for generator in ["gull-wing", "tabbed", "no-lead", "chip", "molded", "grid"] {
        let text = std::fs::read_to_string(generators.join(generator).join("presets.json"))?;
        for preset in serde_json::from_str::<Vec<serde_json::Value>>(&text)? {
            let key = preset["key"].as_str().ok_or("a preset without a key")?;
            cases.push((generator.to_owned(), key.to_owned(), serde_json::json!({})));
        }
    }
    // Built once per part file: the faces each file draws are its
    // construction's, the same at every size the presets ask for.
    let mut seen = std::collections::BTreeSet::new();
    let mut failures = Vec::new();
    for (generator, preset, params) in &cases {
        let project = demo_with_generated(generator, preset, params)?;
        let file = project
            .library
            .packages
            .first()
            .and_then(|p| match &p.definition.model {
                sparksmith_core::library::PackageModel::PartFile(part) => project
                    .library
                    .model(part.model)
                    .map(|m| format!("{:?}", m.definition.asset.hash)),
                _ => None,
            });
        if !seen.insert(file.ok_or_else(|| format!("{preset}: no part file"))?) {
            continue;
        }
        let model = sparksmith_solidcraft::project::build(&project)?;
        let fights = coincident(&model.geometry);
        if let Some(first) = fights.first() {
            failures.push(format!(
                "{preset} {params}: {} pairs, first {first}",
                fights.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
    Ok(())
}
