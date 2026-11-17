//! Original asymmetric STEP and board fixtures. Expected poses are derived
//! explicitly below, independently of the importer's placement helpers.
use sparksmith_solidcraft::material::BodyKind;
use sparksmith_solidcraft::models::imported::SourceCache;
use sparksmith_solidcraft::native::{BoxBody, Expr, Native};
use sparksmith_solidcraft::project::{self, Flat, NativeModel};
use sparksmith_solidcraft::scene::Geometry;
use std::sync::Arc;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Bounds = [[f64; 3]; 2];
const TOLERANCE_MM: f64 = 0.001;

fn asymmetric_step() -> Fallible<Vec<u8>> {
    let mut native = Native::new("original asymmetric pose fixture")?;
    native.solid_box(BoxBody {
        name: "asymmetric_body",
        corner: [0, 0, 0].map(Expr::from),
        size: [1, 2, 3].map(Expr::from),
    })?;
    Ok(native.step_bytes(&["asymmetric_body".into()], "original asymmetric box")?)
}

fn assert_bounds(label: &str, actual: Bounds, expected: Bounds) {
    for end in 0..2 {
        for axis in 0..3 {
            assert!(
                actual[end][axis].is_finite()
                    && (actual[end][axis] - expected[end][axis]).abs() < TOLERANCE_MM,
                "{label}: bounds {actual:?}, expected {expected:?}"
            );
        }
    }
}

fn mesh_bounds(geometry: &Geometry) -> Bounds {
    let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
    for point in &geometry.world_mesh().positions {
        for (axis, value) in [point.x, point.y, point.z].into_iter().enumerate() {
            bounds[0][axis] = bounds[0][axis].min(value);
            bounds[1][axis] = bounds[1][axis].max(value);
        }
    }
    bounds
}

fn instance_geometry(model: &NativeModel, instance: sparksmith_core::Id) -> Fallible<&Geometry> {
    let matching: Vec<_> = model
        .geometry
        .iter()
        .filter(|geometry| {
            geometry.kind == BodyKind::ModelBody
                && model.sources.iter().any(|source| {
                    geometry.item == Some(source.pick_id)
                        && source.instance == Some(instance)
                        && source.pad.is_none()
                })
        })
        .collect();
    match matching.as_slice() {
        [one] => Ok(*one),
        _ => Err(format!(
            "instance {instance}: expected one model body, got {}",
            matching.len()
        )
        .into()),
    }
}

#[test]
fn rotated_offset_models_have_analytic_top_bottom_poses_in_live_and_saved_exact_models() -> Fallible
{
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("asymmetric.step"), asymmetric_step()?)?;
    // Board origin is its lower-left corner (0,30), so both footprints'
    // y=10 becomes world y=20. Both footprint angles are +90 degrees.
    let mut board = String::from(
        r#"(kicad_pcb (version 20240108)
      (generator "original synthetic pose fixture") (general (thickness 1.6))
      (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
      (gr_rect (start 0 0) (end 40 30) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
    );
    for (designator, side, x) in [("U1", "F", 10), ("U2", "B", 25)] {
        board.push_str(&format!(
            r#"(footprint "Original:AsymmetricPose"
          (layer "{side}.Cu") (at {x} 10 90) (property "Reference" "{designator}")
          (pad "1" smd rect (at 0 0 90) (size 1 1)
            (layers "{side}.Cu" "{side}.Mask" "{side}.Paste"))
          (model "${{KIPRJMOD}}/asymmetric.step" (offset (xyz 2 3 0.5))
            (scale (xyz 1 1 1)) (rotate (xyz 0 0 90))))"#
        ));
    }
    board.push(')');
    let path = dir.path().join("original-pose.kicad_pcb");
    std::fs::write(&path, board)?;
    let imported = crate::kicad_import::import(&path, None)?;
    let project = imported.engine.project();
    assert_eq!(
        imported
            .report
            .count(sparksmith_kicad::report::Case::VendorModel),
        2
    );
    assert_eq!(
        project.library.models.len(),
        1,
        "one shared vendor model revision"
    );
    assert_eq!(project.library.assets.len(), 1, "one packed vendor asset");
    let top = project
        .instance_by_designator("U1")
        .ok_or("top instance missing")?
        .id;
    let bottom = project
        .instance_by_designator("U2")
        .ok_or("bottom instance missing")?
        .id;

    // Source box: (u,v,w) in [0,1]×[0,2]×[0,3]. Model clockwise Z90
    // then offset gives (2+v,3-u,0.5+w). Footprint +90 transforms this
    // to (-3+u,2+v,0.5+w). Bottom mounting reverses local y and z first,
    // yielding (3-u,2+v,-0.5-w). Board translation supplies x=10/25,y=20.
    // Bodies sit on the OUTSIDE of their copper foil, not on the dielectric.
    let dielectric: f64 = project
        .stackup
        .dielectrics
        .iter()
        .map(|d| d.thickness_mm)
        .sum();
    let top_copper = project
        .stackup
        .copper
        .first()
        .ok_or("top foil missing")?
        .thickness_um
        / 1000.0;
    let bottom_copper = project
        .stackup
        .copper
        .last()
        .ok_or("bottom foil missing")?
        .thickness_um
        / 1000.0;
    let top_bounds = [
        [7.0, 22.0, dielectric + top_copper + 0.5],
        [8.0, 24.0, dielectric + top_copper + 3.5],
    ];
    let bottom_bounds = [
        [27.0, 22.0, -bottom_copper - 3.5],
        [28.0, 24.0, -bottom_copper - 0.5],
    ];
    let live = project::build_flat(project, &SourceCache::default(), None, Flat::Meshed)?;
    assert_eq!(
        live.part_designs(),
        1,
        "instances reuse one evaluated part design"
    );
    let live_top = instance_geometry(&live, top)?;
    let live_bottom = instance_geometry(&live, bottom)?;
    assert!(
        Arc::ptr_eq(&live_top.mesh, &live_bottom.mesh),
        "placements share the source mesh"
    );
    assert_bounds("live top", mesh_bounds(live_top), top_bounds);
    assert_bounds("live bottom", mesh_bounds(live_bottom), bottom_bounds);

    let exact = project::build_flat(project, &SourceCache::default(), None, Flat::Native)?;
    assert_bounds(
        "exact top mesh",
        mesh_bounds(instance_geometry(&exact, top)?),
        top_bounds,
    );
    assert_bounds(
        "exact bottom mesh",
        mesh_bounds(instance_geometry(&exact, bottom)?),
        bottom_bounds,
    );
    let parts = exact.part_bodies(project)?;
    assert_eq!(
        parts.len(),
        2,
        "the native export contains both part occurrences"
    );
    let destination = dir.path().join("pose-proof.solidcraft");
    exact.save(crate::native_files::new_file(
        destination.to_str().ok_or("native path is not UTF-8")?,
    ))?;
    // The native export must remain self-contained after the STEP source is gone.
    std::fs::remove_file(dir.path().join("asymmetric.step"))?;
    let reopened = Native::open(destination.to_str().ok_or("native path is not UTF-8")?)?;
    assert!(reopened.evaluation_error().is_none());
    let world = reopened.world_state()?;
    for part in &parts {
        let expected = match part.designator.as_str() {
            "U1" => top_bounds,
            "U2" => bottom_bounds,
            other => return Err(format!("unexpected exported part {other}").into()),
        };
        let body = world.body(&part.body).ok_or("saved part body missing")?;
        let bounds = body.outline_bounds();
        assert_bounds(
            &format!("saved exact {}", part.designator),
            [
                [bounds.min.x, bounds.min.y, bounds.min.z],
                [bounds.max.x, bounds.max.y, bounds.max.z],
            ],
            expected,
        );
    }
    Ok(())
}
