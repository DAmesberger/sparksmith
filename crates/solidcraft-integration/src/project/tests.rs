use super::*;
#[allow(unused_imports)]
use crate::models::tests::{nm, pt};
use crate::test_units::deg;
use sparksmith_core::geometry::cos_sin;
use sparksmith_core::library::{PackageDefinition, PackageJoints, PackageModel};

/// A chip body of the tests: a resistor's or a capacitor's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChipKind {
    Resistor,
    Capacitor,
}

/// Give `package` a chip's terminations of `kind` over its envelope, or a
/// moulded body, with an envelope model: these tests exercise the envelope
/// path.
pub(super) fn set_body(package: &mut PackageDefinition, kind: Option<ChipKind>) {
    package.joints = kind.map(|kind| PackageJoints::Chip {
        resistor: kind == ChipKind::Resistor,
    });
    package.model = PackageModel::default();
}

/// The demo with envelope models instead of its generated chip models,
/// lifted 0.05 mm off the lands as the samples lift an envelope.
pub(super) fn envelope_demo() -> Result<Project> {
    let mut project = Engine::demo()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    for package in &mut project.library.packages {
        package.definition.model = PackageModel::default();
        package.definition.placement.translation_nm = crate::test_units::nm3([0.0, 0.0, 0.05]);
    }
    Ok(project)
}
use crate::geom::Vec3;
use sparksmith_core::library::PadShape;
use sparksmith_core::{Endpoint, Engine, TerminalId, Track};

/// The demo's two instances, by designator.
pub(super) fn instances(project: &Project) -> Result<(Id, Id)> {
    let id = |designator| {
        project
            .instance_by_designator(designator)
            .map(|i| i.id)
            .ok_or_else(|| Error::Verification(format!("demo has no {designator}")))
    };
    Ok((id("R1")?, id("R2")?))
}

/// The two-layer fixture board's laminate: a 1.6 mm finished board less
/// its two 35 µm foils.
const T: f64 = 1.53;

fn near(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

/// The default pad finish's (ENIG's) thickness, mm.
#[allow(clippy::expect_used)]
fn enig() -> f64 {
    let looks = crate::looks::get().expect("the repository's library/materials loads");
    crate::stack::finish_um(looks, sparksmith_core::PadFinish::Enig) / 1000.0
}

fn fixture() -> Result<Project> {
    let mut project = envelope_demo()?;
    // Asymmetric offsets exercise the rigid bottom flip independently of the
    // symmetric body/pad shapes. This synthetic test replaces a snapshot to
    // exercise the adapter; production changes publish a new immutable revision.
    project.library.packages[0]
        .definition
        .placement
        .translation_nm = crate::test_units::nm3([0.25, -0.4, 0.2]);
    project.library.footprints[0].definition.datum.offset = pt([0.3, -0.4]);
    for (placement, (position, layer)) in project
        .placements
        .iter_mut()
        .zip([([10.0, 12.0], Layer::Top), ([30.0, 12.0], Layer::Bottom)])
    {
        placement.position = position;
        placement.rotation_deg = 90.0;
        placement.layer = layer;
    }
    let shared = project
        .net_of(Endpoint {
            instance: instances(&project)?.0,
            terminal: TerminalId(2),
        })
        .map(|net| net.id)
        .ok_or_else(|| Error::Verification("demo has no shared net".into()))?;
    for (y, layer) in [(25.0, Layer::Top), (28.0, Layer::Bottom)] {
        let id = project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        project.tracks.push(Track {
            id,
            net: shared,
            start: [5.0, y],
            end: [15.0, y],
            width: 0.4,
            layer: layer.into(),
            locked: false,
        });
    }
    Ok(project)
}

/// The one body of `kind` that belongs to the matching source.
fn source_geometry(
    model: &NativeModel,
    kind: BodyKind,
    matches: impl Fn(&Source) -> bool,
) -> Result<&Geometry> {
    let source = model
        .sources
        .iter()
        .find(|source| matches(source))
        .ok_or_else(|| Error::Verification("missing source identity".into()))?;
    let mut found = model
        .geometry
        .iter()
        .filter(|g| g.item == Some(source.pick_id) && g.kind == kind);
    let geometry = found
        .next()
        .ok_or_else(|| Error::Verification(format!("source has no {kind:?} geometry")))?;
    if found.next().is_some() {
        return Err(Error::Verification(format!(
            "source has several {kind:?} bodies"
        )));
    }
    Ok(geometry)
}

fn native(model: &NativeModel) -> impl Iterator<Item = &Geometry> {
    model.geometry.iter().filter(|g| !g.kind.is_appearance())
}

fn count(model: &NativeModel, kind: BodyKind) -> usize {
    model.geometry.iter().filter(|g| g.kind == kind).count()
}

fn bounds(geometry: &Geometry, min: [f64; 3], max: [f64; 3]) {
    for axis in 0..3 {
        near(geometry.min[axis], min[axis]);
        near(geometry.max[axis], max[axis]);
    }
}

/// A track's copper outline in plan: its box (min, max) and area. Round
/// end caps of `TRACK_CAP_SEGMENTS` chords each.
fn track_plan(start: Point, end: Point, width: f64) -> ([f64; 2], [f64; 2], f64) {
    let outline = track_outline(start, end, width);
    let n = outline.len();
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in &outline {
        for k in 0..2 {
            min[k] = min[k].min(p[k]);
            max[k] = max[k].max(p[k]);
        }
    }
    let area = (0..n)
        .map(|i| {
            let (a, b) = (outline[i], outline[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f64>()
        / 2.0;
    (min, max, area)
}

#[test]
fn track_outlines_are_stadiums_with_round_caps() {
    // Axis-aligned: the caps reach half the width past each end.
    let (min, max, area) = track_plan([5.0, 10.0], [15.0, 10.0], 0.4);
    for (a, b) in min.iter().zip([4.8, 9.8]) {
        near(*a, b);
    }
    for (a, b) in max.iter().zip([15.2, 10.2]) {
        near(*a, b);
    }
    // The rectangle and a 16-gon inscribed in the cap circle.
    let disc = 8.0 * 0.2 * 0.2 * (std::f64::consts::PI / 8.0).sin();
    near(area, 10.0 * 0.4 + disc);
}

#[test]
fn native_project_has_analytic_board_copper_and_rigid_top_bottom_placements() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = fixture()?;
    let (r1, r2) = instances(&project)?;
    let model = build(&project)?;
    // Native: the substrate, per placement a chip-resistor envelope, per
    // pad its copper and per trace its copper. Display: two mask slabs, per
    // chip two wrapped terminations and an overcoat, per pad its finish and
    // per free trace one mask cap, and at the routed edge a mask lip per
    // side over a line per copper foil and the core's band.
    assert_eq!(native(&model).count(), 9);
    for (kind, n) in [
        (BodyKind::Substrate, 1),
        (BodyKind::MaskTop, 1),
        (BodyKind::MaskBottom, 1),
        (BodyKind::ChipResistor, 2),
        (BodyKind::PackageBody, 0),
        (BodyKind::Termination, 4),
        (BodyKind::Overcoat, 2),
        (BodyKind::PadCopper, 4),
        (BodyKind::PadFinish, 4),
        (BodyKind::CopperTrack, 2),
        (BodyKind::MaskCap, 2 + 2),
        (BodyKind::EdgeCopper, 2),
        (BodyKind::EdgeDielectric, 1),
    ] {
        assert_eq!(count(&model, kind), n, "{kind:?}");
    }
    assert_eq!(model.sources.len(), 8);
    let substrate = model
        .geometry
        .iter()
        .find(|geometry| geometry.kind == BodyKind::Substrate)
        .ok_or_else(|| Error::Verification("missing substrate".into()))?;
    assert_eq!(substrate.item, None);
    bounds(substrate, [0.0, 0.0, 0.0], [60.0, 40.0, T]);
    near(substrate.volume(), 60.0 * 40.0 * T);
    let top = source_geometry(&model, BodyKind::ChipResistor, |s| {
        s.instance == Some(r1) && s.pad.is_none()
    })?;
    bounds(top, [9.8, 11.25, T + 0.235], [11.0, 13.25, T + 0.835]);
    near(top.volume(), 2.0 * 1.2 * 0.6);
    let bottom = source_geometry(&model, BodyKind::ChipResistor, |s| {
        s.instance == Some(r2) && s.pad.is_none()
    })?;
    bounds(bottom, [29.0, 11.25, -0.835], [30.2, 13.25, -0.235]);
    for (instance, center, z) in [
        (r1, [9.6, 10.9], [T, T + 0.035]),
        (r2, [30.4, 10.9], [-0.035, 0.0]),
    ] {
        let pad = source_geometry(&model, BodyKind::PadCopper, |s| {
            s.instance == Some(instance) && s.pad == Some(PadId(1))
        })?;
        // A 1 × 1.4 mm land rotated 90° has a 1.4 × 1 mm world footprint.
        bounds(
            pad,
            [center[0] - 0.7, center[1] - 0.5, z[0]],
            [center[0] + 0.7, center[1] + 0.5, z[1]],
        );
        near(pad.volume(), 1.0 * 1.4 * 0.035);
    }
    for track in &project.tracks {
        let geometry =
            source_geometry(&model, BodyKind::CopperTrack, |s| s.track == Some(track.id))?;
        let z = if track.layer == Layer::Top {
            [T, T + 0.035]
        } else {
            [-0.035, 0.0]
        };
        // Round caps reach half the width past both ends.
        bounds(
            geometry,
            [4.8, track.start[1] - 0.2, z[0]],
            [15.2, track.start[1] + 0.2, z[1]],
        );
        let (_, _, area) = track_plan(track.start, track.end, track.width);
        near(geometry.volume(), area * 0.035);
        let source = model
            .sources
            .iter()
            .find(|s| s.track == Some(track.id))
            .ok_or_else(|| Error::Verification("missing trace source".into()))?;
        assert_eq!(source.net, Some(track.net));
        assert_eq!(source.layer, Some(track.layer));
    }
    Ok(())
}

/// Geometry handed on (to the screen) holds none of the model's evaluated
/// bodies: its unmeasured volumes reach them through the model, so they
/// read NaN once the model is gone. (On the ESP32 board the bodies earlier
/// models' geometry held were 375 MB of live heap after one width edit:
/// `memory_tests`.)
#[test]
fn geometry_holds_no_evaluated_body_once_its_model_is_gone() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = fixture()?;
    let model = build(&project)?;
    let world = std::sync::Arc::downgrade(
        model
            .world
            .as_ref()
            .ok_or_else(|| Error::Verification("no native design".into()))?,
    );
    let unmeasured = model
        .geometry
        .iter()
        .find(|g| g.kind == BodyKind::Substrate)
        .cloned()
        .ok_or_else(|| Error::Verification("missing substrate".into()))?;
    drop(model);
    assert!(world.upgrade().is_none(), "the model's bodies are kept");
    assert!(
        unmeasured.volume().is_nan(),
        "the shown geometry kept its body"
    );
    Ok(())
}

#[test]
fn diagonal_traces_have_round_caps_on_both_copper_layers() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = fixture()?;
    for track in &mut project.tracks {
        track.start = [5.0, 25.0];
        track.end = [9.0, 28.0];
    }
    let model = build(&project)?;
    for track in &project.tracks {
        let geometry = source_geometry(&model, BodyKind::CopperTrack, |source| {
            source.track == Some(track.id)
        })?;
        // The 4:3 direction has length 5; the round caps' chords reach
        // within 2 % of half the width past each end, in every direction.
        let z = if track.layer == Layer::Top {
            [T, T + 0.035]
        } else {
            [-0.035, 0.0]
        };
        let (min, max, area) = track_plan(track.start, track.end, track.width);
        for k in 0..2 {
            assert!(min[k] < [5.0, 25.0][k] - 0.19 && max[k] > [9.0, 28.0][k] + 0.19);
        }
        bounds(geometry, [min[0], min[1], z[0]], [max[0], max[1], z[1]]);
        near(geometry.volume(), area * 0.035);
    }
    Ok(())
}

#[test]
fn native_reopen_retains_sources_and_parameter_driven_board_parts_and_copper() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = fixture()?;
    let (r1, r2) = instances(&project)?;
    let original = build(&project)?;
    let temporary = tempfile::tempdir()?;
    let path = temporary
        .path()
        .join("project.solidcraft")
        .to_string_lossy()
        .into_owned();
    original.save(crate::native::files::test_file(&path))?;
    let reopened = Native::open(&path)?;
    let retained = saved_sources(&reopened)?;
    assert_eq!(
        serde_json::to_value(&retained)?,
        serde_json::to_value(&original.sources)?
    );
    let before = finish(reopened, retained)?;
    // The file holds the native bodies only; display layers are not saved.
    assert_eq!(before.geometry.len(), native(&original).count());
    assert!(before.geometry.iter().all(|g| !g.kind.is_appearance()));
    for previous in native(&original) {
        let mut same = before
            .geometry
            .iter()
            .filter(|g| g.item == previous.item && g.kind == previous.kind);
        let restored = same
            .next()
            .ok_or_else(|| Error::Verification("reopen lost source geometry".into()))?;
        assert!(same.next().is_none(), "{:?} is ambiguous", previous.kind);
        bounds(restored, previous.min, previous.max);
        near(restored.volume(), previous.volume());
        assert_eq!(restored.surface, previous.surface);
    }
    looks_match_the_native_file(&before)?;
    let mut reopened = before.native.ok_or_else(crate::project::no_design)?;
    let mut changes = json!({"board_thickness": "2.4 mm", "copper_thickness": "0.07 mm"});
    for (name, value) in [
        (format!("instance_{r1}_x"), "15 mm"),
        (format!("instance_{r1}_y"), "18 mm"),
        (format!("instance_{r1}_angle"), "0 deg"),
        (format!("instance_{r1}_length"), "4 mm"),
        (format!("instance_{r2}_angle"), "180 deg"),
    ] {
        changes[name] = json!(value);
    }
    reopened.change_parameters(changes)?;
    let changed = finish(reopened, before.sources)?;
    let top = source_geometry(&changed, BodyKind::ChipResistor, |s| {
        s.instance == Some(r1) && s.pad.is_none()
    })?;
    bounds(top, [13.25, 17.0, 2.67], [17.25, 18.2, 3.27]);
    near(top.volume(), 4.0 * 1.2 * 0.6);
    let bottom = source_geometry(&changed, BodyKind::ChipResistor, |s| {
        s.instance == Some(r2) && s.pad.is_none()
    })?;
    bounds(bottom, [28.75, 11.0, -0.87], [30.75, 12.2, -0.27]);
    let pad1 = Some(PadId(1));
    let top_pad = source_geometry(&changed, BodyKind::PadCopper, |s| {
        s.instance == Some(r1) && s.pad == pad1
    })?;
    bounds(top_pad, [13.4, 17.7, 2.4], [14.4, 19.1, 2.47]);
    let bottom_pad = source_geometry(&changed, BodyKind::PadCopper, |s| {
        s.instance == Some(r2) && s.pad == pad1
    })?;
    bounds(bottom_pad, [30.6, 11.7, -0.07], [31.6, 13.1, 0.0]);
    for source in changed.sources.iter().filter(|s| s.track.is_some()) {
        let geometry = source_geometry(&changed, BodyKind::CopperTrack, |s| {
            s.pick_id == source.pick_id
        })?;
        let (_, _, area) = track_plan([5.0, 0.0], [15.0, 0.0], 0.4);
        near(geometry.volume(), area * 0.07);
        near(
            geometry.min[2],
            if source.layer == Some(CopperLayer::Top) {
                2.4
            } else {
                -0.07
            },
        );
    }
    Ok(())
}

#[test]
fn authoring_preview_uses_package_dimensions_and_model_offsets() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut package = Engine::demo()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .library
        .packages[0]
        .definition
        .clone();
    package.envelope_nm = [nm(4.0), nm(3.0), nm(2.0)];
    package.placement.translation_nm = crate::test_units::nm3([1.5, -2.0, 0.4]);
    set_body(&mut package, None);
    let model = preview(&package)?;
    assert_eq!(model.geometry.len(), 1);
    assert_eq!(model.geometry[0].kind, BodyKind::PartBody);
    looks_match_the_native_file(&model)?;
    bounds(&model.geometry[0], [-0.5, -3.5, 0.4], [3.5, -0.5, 2.4]);
    near(model.geometry[0].volume(), 24.0);
    // A package declaring a chip-resistor body previews as one, with the
    // wrapped terminations and overcoat it shows on the board.
    package.envelope_nm = [nm(2.0), nm(1.25), nm(0.5)];
    package.placement.translation_nm = crate::test_units::nm3([0.0; 3]);
    set_body(&mut package, Some(ChipKind::Resistor));
    let chip = preview(&package)?;
    assert_eq!(chip.geometry[0].kind, BodyKind::ChipResistor);
    assert_eq!(count(&chip, BodyKind::Termination), 2);
    assert_eq!(count(&chip, BodyKind::Overcoat), 1);
    looks_match_the_native_file(&chip)?;
    for end in chip
        .geometry
        .iter()
        .filter(|g| g.kind == BodyKind::Termination)
    {
        // 0.4 mm bands (a fifth of 2 mm), 10 µm proud of the body.
        near(end.max[0] - end.min[0], 0.41);
        bounds(end, [end.min[0], -0.635, 0.0], [end.max[0], 0.635, 0.51]);
        assert!(end.min[0] <= -1.01 + 1e-9 || end.max[0] >= 1.01 - 1e-9);
    }
    // A chip-capacitor body: MLCC ceramic, terminations, no overcoat.
    set_body(&mut package, Some(ChipKind::Capacitor));
    let ceramic = preview(&package)?;
    assert_eq!(ceramic.geometry[0].kind, BodyKind::ChipCapacitor);
    assert_eq!(count(&ceramic, BodyKind::Termination), 2);
    assert_eq!(count(&ceramic, BodyKind::Overcoat), 0);
    looks_match_the_native_file(&ceramic)?;
    package.envelope_nm[0] = nm(0.0);
    assert!(matches!(preview(&package), Err(Error::Invalid(_))));
    Ok(())
}

/// The body look comes from the package's declared body, never from the
/// designator: renaming R1 to C1 keeps its chip-resistor body, and giving
/// the shared package a chip-capacitor body turns both into capacitors
/// while they keep their R designators.
#[test]
fn placed_body_follows_the_package_body_not_the_designator() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = fixture()?;
    let (r1, _) = instances(&project)?;
    for instance in &mut project.instances {
        if instance.id == r1 {
            instance.designator = "C1".into();
        }
    }
    let model = build(&project)?;
    assert_eq!(count(&model, BodyKind::ChipResistor), 2);
    assert_eq!(count(&model, BodyKind::ChipCapacitor), 0);
    assert_eq!(count(&model, BodyKind::Overcoat), 2);
    let mut project = fixture()?;
    set_body(
        &mut project.library.packages[0].definition,
        Some(ChipKind::Capacitor),
    );
    let model = build(&project)?;
    assert!(project.instance_by_designator("R1").is_some());
    assert_eq!(count(&model, BodyKind::ChipCapacitor), 2);
    assert_eq!(count(&model, BodyKind::ChipResistor), 0);
    assert_eq!(count(&model, BodyKind::Overcoat), 0);
    assert_eq!(count(&model, BodyKind::Termination), 4);
    set_body(&mut project.library.packages[0].definition, None);
    let model = build(&project)?;
    assert_eq!(count(&model, BodyKind::PackageBody), 2);
    assert_eq!(count(&model, BodyKind::ChipCapacitor), 0);
    assert_eq!(count(&model, BodyKind::Overcoat), 0);
    Ok(())
}

fn native_scale_context() -> Result<sparksmith_core::library::standards::Scope> {
    Ok(crate::looks::test_context()?.enter())
}

#[test]
fn native_scale_fixture_has_shipped_materials_and_a_four_layer_stack() -> Result<()> {
    let _scope = native_scale_context()?;
    crate::looks::get()?;
    let board = via_board(1)?;
    assert_eq!(board.stackup.layers().len(), 4);
    Ok(())
}

/// Build timings for docs/editor-evidence.md:
/// `cargo test --profile ci -p sparksmith-solidcraft build_timings -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark"]
fn build_timings() -> Result<()> {
    let _scope = native_scale_context()?;
    let demo = Engine::demo()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    let mut tracks = demo.clone();
    let net = tracks.nets[0].id;
    for k in 0..400 {
        let id = tracks
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        let y = 12.0 + f64::from(k % 100) * 0.27;
        let x = 2.0 + f64::from(k / 100) * 14.0;
        tracks.tracks.push(Track {
            id,
            net,
            start: [x, y],
            end: [x + 12.0, y],
            width: 0.15,
            layer: if k % 2 == 0 {
                CopperLayer::Top
            } else {
                CopperLayer::Bottom
            },
            locked: false,
        });
    }
    let showcase = Engine::showcase()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    // The demo on 4 layers with 50 through vias and 25 inner tracks.
    let mut vias = demo.clone();
    vias.stackup = Stackup::preset(4).map_err(|error| Error::Verification(error.to_string()))?;
    for k in 0..50 {
        let id = vias
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        let position = [
            4.0 + f64::from(k % 20) * 2.5,
            22.0 + f64::from(k / 20) * 1.5,
        ];
        vias.vias.push(sparksmith_core::Via {
            id,
            net,
            position,
            drill: 0.3,
            diameter: 0.6,
            span: sparksmith_core::ViaSpan::Through,
            tented: k % 2 == 0,
            locked: false,
        });
        if k % 2 == 0 {
            let id = vias
                .allocate()
                .map_err(|error| Error::Verification(error.to_string()))?;
            vias.tracks.push(Track {
                id,
                net,
                start: position,
                end: [position[0] + 2.5, position[1]],
                width: 0.15,
                layer: CopperLayer::Inner(1 + u8::from(k % 4 == 0)),
                locked: false,
            });
        }
    }
    // The showcase on 4 layers with 1000, 4096 and 10000 vias and about
    // 250 inner and outer tracks (`via_board`).
    let (v1000, v4096, v10000) = (via_board(1000)?, via_board(4096)?, via_board(10_000)?);
    // The 10,000-via board at the layout's track limit.
    let v10000_tracks = with_tracks(10_000, 2_000)?;
    for (name, project, runs) in [
        ("demo", &demo, 15),
        ("400 tracks", &tracks, 3),
        ("showcase", &showcase, 5),
        ("4 layers, 50 vias, 25 inner tracks", &vias, 3),
        ("showcase, 4 layers, 1000 vias", &v1000, 3),
        ("showcase, 4 layers, 4096 vias", &v4096, 1),
        ("showcase, 4 layers, 10000 vias", &v10000, 3),
        (
            "showcase, 4 layers, 10000 vias, 2000 tracks",
            &v10000_tracks,
            3,
        ),
    ] {
        let mut times = vec![];
        let mut bodies = (0, 0);
        for _ in 0..runs {
            let start = std::time::Instant::now();
            let model = build(project)?;
            times.push(start.elapsed().as_secs_f64() * 1e3);
            bodies = (native(&model).count(), model.geometry.len());
        }
        times.sort_by(f64::total_cmp);
        let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
        println!(
            "{name}: {} native bodies, {} drawn, build best {:.0} ms, p50 {:.0} ms, p95 {:.0} ms ({runs} runs)",
            bodies.0,
            bodies.1,
            at(0.0),
            at(0.5),
            at(0.95)
        );
        // The founder's bar: 10,000 vias in a few seconds; the target is
        // 3 s, best of the runs, for the benchmark board (about 260
        // tracks). At the layout's 2,000 tracks it is not met yet
        // (docs/pcb-realism.md, Build cost): that case is measured only.
        if name == "showcase, 4 layers, 10000 vias" {
            assert!(at(0.0) < 3000.0, "{name}: best {:.0} ms", at(0.0));
        }
    }
    Ok(())
}

/// [`via_board`]`(count)` with tracks added on the outer layers, between
/// its via rows, until it has `total` tracks.
fn with_tracks(count: usize, total: usize) -> Result<Project> {
    let mut project = via_board(count)?;
    let net = project.nets[0].id;
    // The grid's first via (the showcase has vias of its own before it).
    let base = project
        .vias
        .len()
        .checked_sub(count)
        .and_then(|first| project.vias.get(first))
        .map(|v| v.position)
        .ok_or_else(|| Error::Verification("a via board has vias".into()))?;
    let mut k = 0usize;
    while project.tracks.len() < total {
        let id = project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        let start = [
            base[0] + (k % 63) as f64 * 1.27 + 0.135,
            base[1] + (k / 63) as f64 * 1.27 + 0.635,
        ];
        project.tracks.push(Track {
            id,
            net,
            start,
            end: [start[0] + 1.0, start[1]],
            width: 0.15,
            layer: if k.is_multiple_of(2) {
                CopperLayer::Top
            } else {
                CopperLayer::Bottom
            },
            locked: false,
        });
        k += 1;
    }
    Ok(project)
}

/// One via board's build, for profiling (`SPARKSMITH_TRACKS` tracks in all,
/// [`with_tracks`]): `SPARKSMITH_VIAS=10000 cargo test
/// --profile ci -p sparksmith-solidcraft via_build_profile -- --ignored --nocapture`.
/// With `SPARKSMITH_BODY_TABLE=FILE` the last build's native bodies are
/// written there, one line each (kind, layer, box, volume and triangles, in
/// the model's order), to compare the model two Solidcraft revisions build.
#[test]
#[ignore = "benchmark"]
fn via_build_profile() -> Result<()> {
    let _scope = native_scale_context()?;
    let count = std::env::var("SPARKSMITH_VIAS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10_000);
    let runs = std::env::var("SPARKSMITH_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let tracks = std::env::var("SPARKSMITH_TRACKS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let project = with_tracks(count, tracks)?;
    let mut last = None;
    for _ in 0..runs {
        let start = std::time::Instant::now();
        let model = build(&project)?;
        println!(
            "{count} vias, {} tracks: {} native bodies, build {:.0} ms",
            project.tracks.len(),
            native(&model).count(),
            start.elapsed().as_secs_f64() * 1e3
        );
        last = Some(model);
    }
    if let (Ok(path), Some(model)) = (std::env::var("SPARKSMITH_BODY_TABLE"), last) {
        use std::fmt::Write;
        let mut table = String::new();
        for g in native(&model) {
            let _ = writeln!(
                table,
                "{:?} {:?} {:?} {:?} {:e} {}",
                g.kind,
                g.layer,
                g.min,
                g.max,
                g.volume(),
                g.mesh.triangles.len()
            );
        }
        std::fs::write(&path, table).map_err(|e| Error::Verification(e.to_string()))?;
    }
    Ok(())
}

/// The exact build of the four-layer via boards by stage, and how it grows
/// with the vias: each laminate slab on its own (its sketch, its extrusion
/// and their evaluation, in a design that holds the laminate alone), then
/// the whole build. `SPARKSMITH_VIAS=100,1000,10000` picks the added vias
/// (default 100, 300 and 1,000). Each size prints as soon as it is
/// measured, so a run stopped by a time limit keeps the sizes it finished:
/// `cargo test --locked --profile ci -p sparksmith-solidcraft --lib
/// four_layer_exact_slab_profile -- --ignored --nocapture`.
#[test]
#[ignore = "benchmark"]
fn four_layer_exact_slab_profile() -> Result<()> {
    let _scope = native_scale_context()?;
    let sizes: Vec<usize> = std::env::var("SPARKSMITH_VIAS")
        .ok()
        .map(|list| {
            list.split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect()
        })
        .filter(|sizes: &Vec<usize>| !sizes.is_empty())
        .unwrap_or_else(|| vec![100, 300, 1000]);
    // Each size's vias, laminate seconds and whole-build seconds.
    let mut measured: Vec<(usize, f64, f64)> = vec![];
    for added in sizes {
        let project = via_board(added)?;
        let (vias, tracks) = (project.vias.len(), project.tracks.len());
        let board = plan::of(&project)?;
        assert_eq!(board.laminate.len(), 5);

        let mut builder = Builder::new("Laminate timing", &SourceCache::default())?;
        builder.native.defer_evaluation()?;
        stackup_params(&mut builder, &project)?;
        builder.native.component("PCB substrate", true)?;
        let mut laminate_s = 0.0;
        for slab in &board.laminate {
            // The slab's own evaluation ends with it: the next slab's
            // commands would otherwise run it.
            let start = std::time::Instant::now();
            laminate(&mut builder, &project, std::slice::from_ref(slab))?;
            builder.native.evaluate_deferred()?;
            let seconds = start.elapsed().as_secs_f64();
            builder.native.defer_evaluation()?;
            laminate_s += seconds;
            println!(
                "{vias} vias, {tracks} tracks: {} ({} loops) {seconds:.2} s",
                slab.name,
                slab.loops.len()
            );
            assert_eq!(builder.bodies.get(&slab.name), Some(&BodyKind::Substrate));
        }
        drop(builder);

        let start = std::time::Instant::now();
        let model = build(&project)?;
        let build_s = start.elapsed().as_secs_f64();
        println!(
            "{vias} vias, {tracks} tracks: laminate alone {laminate_s:.2} s, whole build {build_s:.2} s \
             ({} native bodies)",
            native(&model).count()
        );
        if let Some(&(before, laminate_before, build_before)) = measured.last() {
            // Time grows as vias to this power between the two sizes.
            let power = |now: f64, was: f64| (now / was).ln() / (vias as f64 / before as f64).ln();
            println!(
                "{before} to {vias} vias: laminate grows as vias^{:.2}, whole build as vias^{:.2}",
                power(laminate_s, laminate_before),
                power(build_s, build_before)
            );
        }
        measured.push((vias, laminate_s, build_s));
    }
    Ok(())
}

/// Every body's surface is exactly the look the native file stores, as
/// Solidcraft's own product-scene extraction reads it.
fn looks_match_the_native_file(model: &NativeModel) -> Result<()> {
    let scene = solidcraft_engine::product_scene::extract(model.exact()?.session())
        .map_err(|e| Error::Verification(e.to_string()))?;
    assert_eq!(scene.meshes.len(), native(model).count());
    for (mesh, geometry) in scene.meshes.iter().zip(native(model)) {
        assert_eq!(mesh.surface, Some(geometry.surface), "{}", mesh.name);
        let expected = material::material(
            geometry.kind,
            &Fabrication::default(),
            &crate::stack::Stack::standard()?,
        );
        assert_eq!(mesh.color, expected.srgb8(), "{}", mesh.name);
    }
    Ok(())
}

fn top_z(model: &NativeModel, kind: BodyKind) -> Result<f64> {
    model
        .geometry
        .iter()
        .find(|g| g.kind == kind)
        .map(|g| g.max[2])
        .ok_or_else(|| Error::Verification(format!("no {kind:?}")))
}

/// The first board-level or copper body a vertical ray meets (parts and
/// their solder are left out: they may cover a land).
fn first_hit(model: &NativeModel, at: [f64; 2], down: bool) -> Option<&Geometry> {
    let (z, dz) = if down { (10.0, -1.0) } else { (-10.0, 1.0) };
    model
        .geometry
        .iter()
        .filter(|g| {
            g.kind.is_board()
                || matches!(
                    g.kind,
                    BodyKind::CopperTrack | BodyKind::PadCopper | BodyKind::PadFinish
                )
        })
        .filter_map(|g| {
            g.raycast(Vec3::new(at[0], at[1], z), Vec3::new(0.0, 0.0, dz))
                .map(|(t, _)| (t, g))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, g)| g)
}

#[test]
fn bodies_carry_kinds_and_the_looks_stored_in_the_native_file() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let model = build(&fixture()?)?;
    looks_match_the_native_file(&model)?;
    for geometry in &model.geometry {
        // Board-level bodies and the edge's stackup bands have no pick
        // identity; neither do the mask lips at the edge (mask caps
        // standing proud of the routed outline). Everything else does.
        let lip = geometry.kind == BodyKind::MaskCap && geometry.min[0] < 0.0;
        let board = (geometry.kind.is_board() && geometry.kind != BodyKind::MaskCap)
            || matches!(
                geometry.kind,
                BodyKind::EdgeDielectric | BodyKind::EdgeCopper
            )
            || lip;
        assert_eq!(geometry.item.is_none(), board, "{:?}", geometry.kind);
    }
    let copper = material::material(
        BodyKind::PadCopper,
        &Fabrication::default(),
        &crate::stack::Stack::standard()?,
    )
    .surface()?;
    for source in model.sources.iter().filter(|s| s.pad.is_some()) {
        let pad = source_geometry(&model, BodyKind::PadCopper, |s| s.pick_id == source.pick_id)?;
        let finish = source_geometry(&model, BodyKind::PadFinish, |s| s.pick_id == source.pick_id)?;
        assert_eq!(pad.surface.metallic(), copper.metallic());
        near(pad.volume(), 1.0 * 1.4 * 0.035);
        for axis in 0..2 {
            near(finish.min[axis], pad.min[axis] - enig());
            near(finish.max[axis], pad.max[axis] + enig());
        }
        // The finish plates the land's top and sides, from a finish
        // thickness off the laminate: never on the copper's bottom face.
        if source.layer == Some(CopperLayer::Top) {
            near(finish.min[2], pad.min[2] + enig());
            near(finish.max[2], pad.max[2] + enig());
        } else {
            near(finish.min[2], pad.min[2] - enig());
            near(finish.max[2], pad.max[2] - enig());
        }
    }
    for source in model.sources.iter().filter(|s| s.track.is_some()) {
        let cap = source_geometry(&model, BodyKind::MaskCap, |s| s.pick_id == source.pick_id)?;
        let track = source_geometry(&model, BodyKind::CopperTrack, |s| {
            s.pick_id == source.pick_id
        })?;
        // Free ends are round, like the copper's: the cap overhangs the
        // copper by its margin all round.
        near(cap.min[0], track.min[0] - 0.01);
        near(cap.max[0], track.max[0] + 0.01);
        near(cap.min[1], track.min[1] - 0.01);
        near(cap.max[1], track.max[1] + 0.01);
    }
    Ok(())
}

#[test]
fn exposed_pads_stand_above_the_mask_and_tracks_are_capped() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = fixture()?;
    let model = build(&project)?;
    let mask_top = top_z(&model, BodyKind::MaskTop)?;
    let mask_bottom = model
        .geometry
        .iter()
        .find(|g| g.kind == BodyKind::MaskBottom)
        .map(|g| g.min[2])
        .ok_or_else(|| Error::Verification("no bottom mask".into()))?;
    near(mask_top, T + 0.02);
    near(mask_bottom, -0.02);
    for source in model.sources.iter().filter(|s| s.pad.is_some()) {
        let pad = source_geometry(&model, BodyKind::PadCopper, |s| s.pick_id == source.pick_id)?;
        let center = [
            (pad.min[0] + pad.max[0]) / 2.0,
            (pad.min[1] + pad.max[1]) / 2.0,
        ];
        let top = source.layer == Some(CopperLayer::Top);
        let hit = first_hit(&model, center, top)
            .ok_or_else(|| Error::Verification("ray missed the pad".into()))?;
        assert_eq!(
            (hit.kind, hit.item),
            (BodyKind::PadFinish, Some(source.pick_id))
        );
        if top {
            assert!(hit.max[2] - mask_top >= 0.010 - 1e-9, "{}", hit.max[2]);
        } else {
            assert!(mask_bottom - hit.min[2] >= 0.010 - 1e-9, "{}", hit.min[2]);
        }
    }
    for track in &project.tracks {
        let source = model
            .sources
            .iter()
            .find(|s| s.track == Some(track.id))
            .ok_or_else(|| Error::Verification("missing trace source".into()))?;
        let middle = [
            (track.start[0] + track.end[0]) / 2.0,
            (track.start[1] + track.end[1]) / 2.0,
        ];
        let hit = first_hit(&model, middle, track.layer == Layer::Top)
            .ok_or_else(|| Error::Verification("ray missed the track".into()))?;
        assert_eq!(
            (hit.kind, hit.item),
            (BodyKind::MaskCap, Some(source.pick_id))
        );
    }
    Ok(())
}

/// The top-side pad finishes (not the finish over track copper).
fn pad_finishes(model: &NativeModel) -> Vec<&Geometry> {
    model
        .geometry
        .iter()
        .filter(|g| g.kind == BodyKind::PadFinish && g.max[2] > 1.0)
        .filter(|g| {
            model
                .sources
                .iter()
                .any(|s| Some(s.pick_id) == g.item && s.pad.is_some())
        })
        .collect()
}

/// A track's display pieces of `kind`.
fn pieces(model: &NativeModel, track: Id, kind: BodyKind) -> Vec<&Geometry> {
    let Some(source) = model.sources.iter().find(|s| s.track == Some(track)) else {
        return vec![];
    };
    model
        .geometry
        .iter()
        .filter(|g| g.item == Some(source.pick_id) && g.kind == kind)
        .collect()
}

fn no_cap_over_a_pad(model: &NativeModel, caps: &[&Geometry]) {
    for cap in caps {
        for p in &cap.world_mesh().positions {
            for f in pad_finishes(model) {
                let over = p.x > f.min[0] + 1e-6
                    && p.x < f.max[0] - 1e-6
                    && p.y > f.min[1] + 1e-6
                    && p.y < f.max[1] - 1e-6;
                assert!(!over, "cap vertex {p:?} over a pad finish");
            }
        }
    }
}

#[test]
fn mask_caps_stop_at_mask_openings_and_exposed_track_copper_is_plated() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = fixture()?;
    let (r1, _) = instances(&project)?;
    let net = project.tracks[0].net;
    let pad2 = layout::pads(&project)
        .into_iter()
        .find(|p| p.instance == r1 && p.pad == PadId(2))
        .ok_or_else(|| Error::Verification("missing R1 pad 2".into()))?;
    let c = pad2.position;
    let mut add = |start: Point, end: Point| -> Result<Id> {
        let id = project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        project.tracks.push(Track {
            id,
            net,
            start,
            end,
            width: 0.3,
            layer: CopperLayer::Top,
            locked: false,
        });
        Ok(id)
    };
    let straight = add(c, [c[0], c[1] + 5.0])?;
    let diagonal = add(c, [c[0] - 4.0, c[1] + 3.0])?;
    let inside = add([c[0] - 0.2, c[1]], [c[0] + 0.2, c[1]])?;
    // Across the land's 1.4 mm world width, clear of the part's other land.
    let through = add([c[0] - 4.0, c[1] + 0.1], [c[0] + 4.0, c[1] + 0.1])?;
    let model = build(&project)?;
    assert!(
        pieces(&model, inside, BodyKind::MaskCap).is_empty(),
        "a track inside its pad has no cap"
    );
    for track in [straight, diagonal] {
        let caps = pieces(&model, track, BodyKind::MaskCap);
        assert_eq!(caps.len(), 1);
        no_cap_over_a_pad(&model, &caps);
    }
    // Rotated 90°, the 1 × 1.4 mm land is 1 mm deep along +Y. The straight
    // cap starts at the mask opening, 0.5 + 0.05 mm from the centre, and the
    // copper between the land and the opening is plated like the land.
    let straight_cap = pieces(&model, straight, BodyKind::MaskCap);
    near(straight_cap[0].min[1], c[1] + 0.55);
    let strip = pieces(&model, straight, BodyKind::PadFinish);
    assert_eq!(strip.len(), 1);
    near(strip[0].min[1], c[1]);
    near(strip[0].max[1], c[1] + 0.55);
    // The strip stands 0.8 of the finish's thickness on the copper.
    near(strip[0].max[2], T + 0.035 + 0.8 * enig());
    // A track passing through the land: capped on both sides, plated across.
    let caps = pieces(&model, through, BodyKind::MaskCap);
    assert_eq!(caps.len(), 2, "pass-through cap is split at the opening");
    no_cap_over_a_pad(&model, &caps);
    let mut ends: Vec<f64> = caps.iter().flat_map(|g| [g.min[0], g.max[0]]).collect();
    ends.sort_by(f64::total_cmp);
    near(ends[1], c[0] - 0.75);
    near(ends[2], c[0] + 0.75);
    let strip = pieces(&model, through, BodyKind::PadFinish);
    assert_eq!(strip.len(), 1);
    near(strip[0].min[0], c[0] - 0.75);
    near(strip[0].max[0], c[0] + 0.75);
    // Every finish stands above the mask and below any cap top.
    for f in pad_finishes(&model) {
        near(f.max[2], T + 0.035 + enig());
    }
    Ok(())
}

#[test]
fn native_bodies_are_disjoint_except_where_copper_joins_copper() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = fixture()?;
    let (r1, _) = instances(&project)?;
    let pad = layout::pads(&project)
        .into_iter()
        .find(|p| p.instance == r1)
        .ok_or_else(|| Error::Verification("missing R1 pad".into()))?;
    let id = project
        .allocate()
        .map_err(|error| Error::Verification(error.to_string()))?;
    project.tracks.push(Track {
        id,
        net: pad.net,
        start: pad.position,
        end: [pad.position[0] + 6.0, pad.position[1]],
        width: 0.25,
        layer: CopperLayer::Top,
        locked: false,
    });
    let model = build(&project)?;
    let bodies: Vec<&Geometry> = native(&model).collect();
    let copper = |g: &Geometry| matches!(g.kind, BodyKind::CopperTrack | BodyKind::PadCopper);
    for (i, a) in bodies.iter().enumerate() {
        for b in &bodies[i + 1..] {
            if copper(a) && copper(b) {
                continue;
            }
            // Every body here is a box lying in its own z band, so a shared
            // interior needs overlapping axis-aligned boxes.
            let overlap: f64 = (0..3)
                .map(|k| (a.max[k].min(b.max[k]) - a.min[k].max(b.min[k])).max(0.0))
                .product();
            assert!(
                overlap < 1e-9,
                "{:?} and {:?} overlap by {overlap} mm³",
                a.kind,
                b.kind
            );
        }
    }
    // The saved model's volume is the laminate, copper and envelopes only.
    let volume: f64 = bodies.iter().map(|g| g.volume()).sum();
    let expected = 60.0 * 40.0 * T
        + 2.0 * (2.0 * 1.2 * 0.6)
        + 4.0 * (1.0 * 1.4 * 0.035)
        + (2.0 * track_plan([0.0, 0.0], [10.0, 0.0], 0.4).2
            + track_plan([0.0, 0.0], [6.0, 0.0], 0.25).2)
            * 0.035;
    near(volume, expected);
    Ok(())
}

#[test]
fn every_footprint_pad_is_enumerated_and_unconnected_ones_have_no_net() -> Result<()> {
    let mut project = fixture()?;
    let connected_before = layout::pads(&project).len();
    // Synthetic: a mechanical land whose package terminal carries no
    // electrical terminal, so it has no net on either placement.
    let unmapped = sparksmith_core::PackageTerminalId(90);
    let package = &mut project.library.packages[0].definition;
    package
        .terminals
        .push(sparksmith_core::library::PackageTerminal {
            id: unmapped,
            designation: "MP".into(),
        });
    package.next_local = 91;
    let footprint = &mut project.library.footprints[0].definition;
    footprint.pads.push(sparksmith_core::library::Pad::smd_rect(
        PadId(90),
        unmapped,
        "MP",
        pt([0.0, 3.0]),
        pt([0.8, 0.8]),
    ));
    footprint.next_local = 91;
    // Library validation refuses such a definition today, so the
    // enumeration is checked directly; the core's pad list is unchanged.
    let connected = layout::pads(&project);
    assert_eq!(connected.len(), connected_before);
    let pads = placed_pads(&project)?;
    assert_eq!(pads.len(), 6);
    for pad in &pads {
        let expected = connected
            .iter()
            .find(|p| p.instance == pad.placed.instance && p.pad == pad.pad().id)
            .map(|p| (p.net, p.position));
        assert_eq!(
            pad.placed.net.is_none(),
            pad.pad().terminal == Some(unmapped)
        );
        if let Some((net, position)) = expected {
            assert_eq!(pad.placed.net, Some(net));
            near(pad.origin[0], position[0]);
            near(pad.origin[1], position[1]);
        }
    }
    Ok(())
}

#[test]
fn chip_terminations_wrap_the_ends_and_solder_wets_the_lands_on_both_sides() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    // The demo's centred chips get a fillet at every land; the fixture's
    // offset bodies leave one end hanging off its land, which gets none.
    let demo = build(&envelope_demo()?)?;
    assert_eq!(count(&demo, BodyKind::Solder), 4);
    assert!(count(&build(&fixture()?)?, BodyKind::Solder) >= 1);
    check_chip_parts(&demo)?;
    check_chip_parts(&build(&fixture()?)?)?;
    Ok(())
}

/// The showcase's chips (part files with chip terminations): a
/// display-only fillet at each of R1's lands, each on its land and against
/// the end face of a real termination body.
#[test]
fn part_file_chips_get_solder_fillets_against_their_terminations() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let demo = build(&showcase()?)?;
    let r1 = demo
        .sources
        .iter()
        .find(|s| s.pad.is_none() && s.label == "R1 model")
        .ok_or_else(|| Error::Verification("no R1".into()))?;
    let fillets: Vec<&Geometry> = demo
        .geometry
        .iter()
        .filter(|g| g.kind == BodyKind::Solder && g.item == Some(r1.pick_id))
        .collect();
    assert_eq!(fillets.len(), 2);
    for solder in fillets {
        let land = demo.sources.iter().filter(|s| s.pad.is_some()).any(|s| {
            source_geometry(&demo, BodyKind::PadCopper, |x| x.pick_id == s.pick_id).is_ok_and(
                |pad| {
                    (0..2).all(|a| {
                        solder.min[a] >= pad.min[a] - 1e-6 && solder.max[a] <= pad.max[a] + 1e-6
                    })
                },
            )
        });
        assert!(land, "fillet off its land");
        let ends: Vec<&Geometry> = demo
            .geometry
            .iter()
            .filter(|g| g.item == solder.item && g.kind == BodyKind::ModelLead)
            .collect();
        assert_eq!(ends.len(), 2, "the model's two terminations");
        let touches = ends.iter().any(|end| {
            (0..2).any(|a| {
                (solder.min[a] - end.max[a]).abs() < 1e-6
                    || (solder.max[a] - end.min[a]).abs() < 1e-6
            })
        });
        assert!(touches, "fillet does not meet its termination");
        assert!(solder.volume() > 0.0);
    }
    Ok(())
}

/// The showcase's generated SOIC-8 gets a solder fillet at the toe of
/// every foot, each on that foot's land; its generated pin header gets a
/// solder cone round every pin on the far side, centred on the pin's land.
#[test]
fn generated_feet_and_header_pins_get_solder_joints() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = showcase()?;
    let model = build(&project)?;
    let part = |designator: &str| {
        model
            .sources
            .iter()
            .find(|s| s.pad.is_none() && s.label == format!("{designator} model"))
            .ok_or_else(|| Error::Verification(format!("no {designator}")))
    };
    let lands = |part: &Source| -> Vec<&Geometry> {
        model
            .sources
            .iter()
            .filter(|s| s.instance == part.instance && s.pad.is_some())
            .filter_map(|s| {
                // A plated-through land has copper on every layer: any.
                model
                    .geometry
                    .iter()
                    .find(|g| g.item == Some(s.pick_id) && g.kind == BodyKind::PadCopper)
            })
            .collect()
    };
    let solder = |part: &Source| -> Vec<&Geometry> {
        model
            .geometry
            .iter()
            .filter(|g| g.kind == BodyKind::Solder && g.item == Some(part.pick_id))
            .collect()
    };
    let soic = part("U1")?;
    let soic_lands = lands(soic);
    assert_eq!(soic_lands.len(), 8);
    let fillets = solder(soic);
    assert!(
        fillets.len() >= 8,
        "a toe fillet per foot: {}",
        fillets.len()
    );
    for land in &soic_lands {
        let on = fillets.iter().filter(|s| {
            (0..2).all(|a| s.min[a] >= land.min[a] - 1e-6 && s.max[a] <= land.max[a] + 1e-6)
        });
        assert!(on.count() >= 1, "a land of U1 without solder");
    }
    for fillet in &fillets {
        assert!(fillet.volume() > 0.0);
        assert!(fillet.min[2] > land_top(&soic_lands) - 1e-3, "on the land");
    }
    let header = part("J1")?;
    let header_lands = lands(header);
    let cones = solder(header);
    assert_eq!(cones.len(), 10, "a cone per pin");
    let top = header.layer == Some(CopperLayer::Top);
    for cone in &cones {
        assert!(cone.volume() > 0.0);
        // On the far side of the board.
        assert!(if top {
            cone.max[2] < 0.0
        } else {
            cone.min[2] > T
        });
        let mid = [
            (cone.min[0] + cone.max[0]) / 2.0,
            (cone.min[1] + cone.max[1]) / 2.0,
        ];
        let centred = header_lands
            .iter()
            .any(|land| (0..2).all(|a| (mid[a] - (land.min[a] + land.max[a]) / 2.0).abs() < 1e-6));
        let nearest = header_lands
            .iter()
            .map(|land| {
                (0..2)
                    .map(|a| (mid[a] - (land.min[a] + land.max[a]) / 2.0).abs())
                    .fold(0.0, f64::max)
            })
            .fold(f64::MAX, f64::min);
        assert!(centred, "a cone off its pin's land by {nearest} at {mid:?}");
    }
    Ok(())
}

/// The highest land top among `lands`.
fn land_top(lands: &[&Geometry]) -> f64 {
    lands.iter().map(|l| l.max[2]).fold(f64::MIN, f64::max)
}

fn check_chip_parts(model: &NativeModel) -> Result<()> {
    let envelopes = model
        .sources
        .iter()
        .filter(|s| s.pad.is_none() && s.instance.is_some());
    for source in envelopes {
        let body = source_geometry(model, BodyKind::ChipResistor, |s| {
            s.pick_id == source.pick_id
        })?;
        let of = |kind| {
            model
                .geometry
                .iter()
                .filter(move |g| g.item == Some(source.pick_id) && g.kind == kind)
                .collect::<Vec<_>>()
        };
        let ends = of(BodyKind::Termination);
        assert_eq!(ends.len(), 2, "{}", source.label);
        let top = source.layer == Some(CopperLayer::Top);
        for end in &ends {
            // Each cap wraps 10 µm past the body's top and underside (the
            // fixture lifts the body 0.2 mm), its sides and one end face.
            near(end.max[2], body.max[2] + 0.01);
            near(end.min[2], body.min[2] - 0.01);
            let outside = (0..2).any(|a| {
                (end.min[a] - (body.min[a] - 0.01)).abs() < 1e-9
                    || (end.max[a] - (body.max[a] + 0.01)).abs() < 1e-9
            });
            assert!(outside, "{} termination misses an end face", source.label);
        }
        let overcoat = source_geometry(model, BodyKind::Overcoat, |s| s.pick_id == source.pick_id)?;
        assert!(overcoat.max[2] > body.max[2] || overcoat.min[2] < body.min[2]);
        let lands: Vec<&Geometry> = model
            .sources
            .iter()
            .filter(|s| s.instance == source.instance && s.pad.is_some())
            .filter_map(|s| {
                source_geometry(model, BodyKind::PadCopper, |x| x.pick_id == s.pick_id).ok()
            })
            .collect();
        for solder in of(BodyKind::Solder) {
            let on_land = lands.iter().any(|pad| {
                (0..2).all(|a| {
                    solder.min[a] >= pad.min[a] - 1e-6 && solder.max[a] <= pad.max[a] + 1e-6
                })
            });
            assert!(on_land, "{} fillet off its land", source.label);
            let touches = ends.iter().any(|end| {
                (0..2).any(|a| {
                    (solder.min[a] - end.max[a]).abs() < 1e-6
                        || (solder.max[a] - end.min[a]).abs() < 1e-6
                })
            });
            assert!(
                touches,
                "{} fillet does not meet its termination",
                source.label
            );
            if top {
                // On the finish, its toe lifted 2 µm off it.
                near(solder.min[2], T + 0.035 + enig() + 0.002);
            } else {
                near(solder.max[2], -0.035 - enig() - 0.002);
            }
            assert!(solder.volume() > 0.0);
        }
    }
    Ok(())
}

/// The project's fabrication attributes set the drawn mask and finish looks,
/// while the saved native bodies, their stored looks and every shape stay as
/// they are for the default fabrication.
#[test]
fn fabrication_restyles_display_layers_and_leaves_the_native_model_alone() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    use sparksmith_core::{MaskColour, MaskFinish, PadFinish};
    let plain = build(&fixture()?)?;
    let mut project = fixture()?;
    project.fabrication = Fabrication {
        mask_colour: MaskColour::Red,
        mask_finish: MaskFinish::Matte,
        pad_finish: PadFinish::ImmersionSilver,
        ..Fabrication::default()
    };
    let styled = build(&project)?;
    looks_match_the_native_file(&styled)?;
    assert_eq!(plain.geometry.len(), styled.geometry.len());
    let mut restyled = 0;
    let silver = crate::stack::finish_um(crate::looks::get()?, PadFinish::ImmersionSilver) / 1000.0;
    for (a, b) in plain.geometry.iter().zip(&styled.geometry) {
        assert_eq!((a.kind, a.item), (b.kind, b.item));
        // The finish is as thick as the pad finish is (ENIG to immersion
        // silver here), and the solder and bore discs stand on it; every
        // other body keeps its place.
        if b.kind == BodyKind::PadFinish && b.max[2] > T + 0.035 {
            near(a.max[2] - b.max[2], enig() - silver);
        } else if !matches!(
            b.kind,
            BodyKind::PadFinish | BodyKind::Solder | BodyKind::ViaBore
        ) {
            assert_eq!((a.min, a.max), (b.min, b.max), "{:?}", a.kind);
        }
        if b.kind.is_appearance() {
            // Drawn directly from the table; native bodies go through the
            // file's 8-bit colour, which looks_match_the_native_file checks.
            let expected = material::material(
                b.kind,
                &project.fabrication,
                &crate::stack::Stack::of(&project.stackup, project.fabrication.pad_finish)?,
            )
            .surface()?;
            assert_eq!(b.surface, expected, "{:?}", b.kind);
        }
        if matches!(
            b.kind,
            BodyKind::MaskTop | BodyKind::MaskBottom | BodyKind::MaskCap | BodyKind::PadFinish
        ) {
            assert_ne!(a.surface, b.surface, "{:?}", b.kind);
            restyled += 1;
        } else {
            assert_eq!(a.surface, b.surface, "{:?}", b.kind);
        }
    }
    assert!(restyled >= 4, "{restyled}");
    Ok(())
}

/// The fixture with every kind of land on its one footprint (synthetic,
/// original): an L-shaped custom pad turned 30°, a plated-through stadium
/// turned 45° with an offset drill, a bare hole and a tented mechanical
/// land on the far side; the datum is turned a quarter turn.
pub(super) fn shaped() -> Result<Project> {
    use sparksmith_core::library::{Hole, HoleShape, MaskRule, Outline, PadKind, PadSide, Segment};
    let mut project = fixture()?;
    let footprint = &mut project.library.footprints[0].definition;
    footprint.datum.rotation_mdeg = deg(90.0);
    let [first, second] = &mut footprint.pads[..] else {
        return Err(Error::Verification("demo footprint has two pads".into()));
    };
    let l = |x: f64, y: f64| Segment::Line { to: [x, y] };
    first.shape = PadShape::Custom {
        outline: Outline {
            start: [-0.5, -0.4],
            segments: vec![
                l(0.5, -0.4),
                l(0.5, 0.0),
                l(0.0, 0.0),
                l(0.0, 0.4),
                l(-0.5, 0.4),
                l(-0.5, -0.4),
            ],
        },
    };
    first.rotation_mdeg = deg(30.0);
    second.kind = PadKind::PlatedThrough;
    second.shape = PadShape::Oblong {
        size: pt([1.6, 1.0]),
    };
    second.rotation_mdeg = deg(45.0);
    second.hole = Some(Hole {
        shape: HoleShape::Round {
            diameter_nm: nm(0.6),
        },
        plated: true,
        offset: pt([0.2, 0.0]),
    });
    let mut bare = first.clone();
    bare.id = PadId(90);
    bare.kind = PadKind::NonPlatedHole;
    bare.terminal = None;
    bare.number = "MH".into();
    bare.position = pt([0.0, 2.5]);
    bare.rotation_mdeg = deg(0.0);
    bare.shape = PadShape::Circle {
        diameter_nm: nm(1.2),
    };
    bare.hole = Some(Hole {
        shape: HoleShape::Round {
            diameter_nm: nm(1.0),
        },
        plated: false,
        offset: pt([0.0, 0.0]),
    });
    let mut tab = bare.clone();
    tab.id = PadId(91);
    tab.kind = PadKind::Mechanical;
    tab.number = "MP".into();
    tab.position = pt([0.0, -2.5]);
    tab.side = PadSide::Bottom;
    tab.shape = PadShape::RoundRect {
        size: pt([0.8, 0.5]),
        radius_ratio: 0.25,
        max_radius_nm: None,
        corners: sparksmith_core::library::CornerMask::ALL,
    };
    tab.hole = None;
    tab.mask = MaskRule::Tented;
    footprint.pads.extend([bare, tab]);
    footprint.next_local = 92;
    Ok(project)
}

/// A pad-frame point of a placed pad on the board.
fn on_board(pad: &layout::PlacedPad, q: Point) -> Point {
    let (c, s) = cos_sin(pad.pad.rotation_deg());
    let p = pad.pad.position();
    pad.transform
        .apply([p[0] + c * q[0] - s * q[1], p[1] + s * q[0] + c * q[1]])
}

/// Mid-height of the copper on a layer's face.
fn copper_z(layer: Layer) -> f64 {
    match layer {
        Layer::Top => T + 0.0175,
        Layer::Bottom => -0.0175,
    }
}

fn inside(geometry: &[&Geometry], p: Point, z: f64) -> bool {
    geometry
        .iter()
        .any(|g| g.world_mesh().contains(Vec3::new(p[0], p[1], z)))
}

fn pad_bodies(model: &NativeModel, instance: Id, pad: PadId) -> Vec<&Geometry> {
    let Some(source) = model
        .sources
        .iter()
        .find(|s| s.instance == Some(instance) && s.pad == Some(pad))
    else {
        return vec![];
    };
    native(model)
        .filter(|g| g.item == Some(source.pick_id) && g.kind == BodyKind::PadCopper)
        .collect()
}

fn kind(model: &NativeModel, kind: BodyKind) -> Vec<&Geometry> {
    model.geometry.iter().filter(|g| g.kind == kind).collect()
}

#[test]
fn every_pad_shape_is_exact_native_copper_in_its_placed_frame() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = shaped()?;
    let model = build(&project)?;
    let substrate = kind(&model, BodyKind::Substrate);
    let placed = layout::placed_pads(&project);
    assert_eq!(placed.len(), 8);
    for pad in &placed {
        let bodies = pad_bodies(&model, pad.instance, pad.pad.id);
        let other = match pad.layer {
            Layer::Top => Layer::Bottom,
            Layer::Bottom => Layer::Top,
        };
        match pad.pad.id.0 {
            // The L: both arms in, the notch out. A mirror or turn error
            // swaps them.
            1 => {
                assert_eq!(bodies.len(), 1);
                let z = copper_z(pad.layer);
                for (q, expected) in [
                    ([0.3, -0.2], true),
                    ([-0.3, 0.2], true),
                    ([0.3, 0.2], false),
                    ([-0.3, -0.35], true),
                    ([0.45, -0.35], true),
                    ([0.6, -0.2], false),
                ] {
                    let p = on_board(pad, q);
                    assert_eq!(
                        inside(&bodies, p, z),
                        expected,
                        "pad 1 {q:?} on {:?}",
                        pad.layer
                    );
                }
            }
            // Plated through: a land on each face, a barrel between them,
            // the drill through the laminate.
            2 => {
                assert_eq!(bodies.len(), 3);
                for layer in [pad.layer, other] {
                    let z = copper_z(layer);
                    assert!(inside(&bodies, on_board(pad, [0.7, 0.0]), z));
                    assert!(inside(&bodies, on_board(pad, [-0.75, 0.0]), z));
                    assert!(
                        !inside(&bodies, on_board(pad, [0.75, 0.45]), z),
                        "stadium end"
                    );
                    assert!(!inside(&bodies, on_board(pad, [0.2, 0.0]), z), "bore");
                }
                // The hole's 0.6 mm is its finished size: open to it, with
                // the barrel outside it in a hole drilled wider.
                let wall = on_board(pad, [0.2 + 0.3 + 0.025 / 2.0, 0.0]);
                assert!(inside(&bodies, wall, 0.8), "barrel");
                let finished = on_board(pad, [0.2 + 0.3 - 0.005, 0.0]);
                assert!(
                    !inside(&bodies, finished, 0.8),
                    "the bore is the finished size"
                );
                assert!(!inside(&substrate, wall, 0.8), "barrel inside the laminate");
                assert!(!inside(&bodies, on_board(pad, [0.2, 0.0]), 0.8));
                assert!(!inside(&substrate, on_board(pad, [0.2, 0.0]), 0.8));
                assert!(inside(&substrate, on_board(pad, [0.2 + 0.35, 0.0]), 0.8));
            }
            // A bare hole: no copper and no pick, the laminate drilled.
            90 => {
                assert!(bodies.is_empty());
                assert!(
                    !model
                        .sources
                        .iter()
                        .any(|s| s.instance == Some(pad.instance) && s.pad == Some(pad.pad.id))
                );
                assert!(!inside(&substrate, on_board(pad, [0.45, 0.0]), 0.8));
                assert!(inside(&substrate, on_board(pad, [0.55, 0.0]), 0.8));
            }
            // A far-side land: copper on the opposite face only.
            91 => {
                assert_eq!(bodies.len(), 1);
                assert_ne!(
                    Some(pad.layer),
                    project
                        .placements
                        .iter()
                        .find(|p| p.instance == pad.instance)
                        .map(|p| p.layer)
                );
                assert!(inside(
                    &bodies,
                    on_board(pad, [0.35, 0.2]),
                    copper_z(pad.layer)
                ));
                assert!(
                    !inside(&bodies, on_board(pad, [0.39, 0.24]), copper_z(pad.layer)),
                    "rounded corner"
                );
                assert!(!inside(&bodies, on_board(pad, [0.0, 0.0]), copper_z(other)));
            }
            id => return Err(Error::Verification(format!("unexpected pad {id}"))),
        }
    }
    // The laminate lost two drills per placement.
    let full = 60.0 * 40.0 * T;
    // The plated hole is drilled wider than its finished 0.6 mm by the
    // barrel's wall; the bare hole at its size.
    let plated: f64 = 0.3 + 0.025;
    let drilled = 2.0 * std::f64::consts::PI * (plated.powi(2) + 0.5f64.powi(2)) * T;
    let volume: f64 = substrate.iter().map(|g| g.volume()).sum();
    assert!((volume - (full - drilled)).abs() < 2e-3, "{volume}");
    Ok(())
}

#[test]
fn mask_openings_and_finish_follow_the_shapes_and_tented_lands_stay_covered() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = shaped()?;
    let model = build(&project)?;
    let mask = |layer: Layer| {
        kind(
            &model,
            match layer {
                Layer::Top => BodyKind::MaskTop,
                Layer::Bottom => BodyKind::MaskBottom,
            },
        )
    };
    let mask_z = |layer: Layer| match layer {
        Layer::Top => T + 0.01,
        Layer::Bottom => -0.01,
    };
    let finish = kind(&model, BodyKind::PadFinish);
    for pad in layout::placed_pads(&project) {
        let layer = pad.layer;
        let other = match layer {
            Layer::Top => Layer::Bottom,
            Layer::Bottom => Layer::Top,
        };
        match pad.pad.id.0 {
            2 => {
                for side in [layer, other] {
                    // The 50 µm default opening around the stadium's tip.
                    let ring = on_board(&pad, [0.8 + 0.025, 0.0]);
                    let beyond = on_board(&pad, [0.8 + 0.08, 0.0]);
                    assert!(!inside(&mask(side), ring, mask_z(side)), "ring on {side:?}");
                    assert!(
                        inside(&mask(side), beyond, mask_z(side)),
                        "beyond on {side:?}"
                    );
                    let z = copper_z(side) + if side == Layer::Top { 0.02 } else { -0.02 };
                    assert!(inside(&finish, on_board(&pad, [0.7, 0.0]), z));
                    assert!(
                        !inside(&finish, on_board(&pad, [0.2, 0.0]), z),
                        "finish over the bore"
                    );
                }
            }
            90 => {
                for side in [Layer::Top, Layer::Bottom] {
                    assert!(!inside(
                        &mask(side),
                        on_board(&pad, [0.0, 0.0]),
                        mask_z(side)
                    ));
                }
            }
            91 => {
                assert!(inside(
                    &mask(layer),
                    on_board(&pad, [0.0, 0.0]),
                    mask_z(layer)
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

#[test]
fn shaped_copper_survives_reopening_and_follows_the_board_thickness() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = shaped()?;
    let (r1, _) = instances(&project)?;
    let original = build(&project)?;
    let temporary = tempfile::tempdir()?;
    let path = temporary
        .path()
        .join("shaped.solidcraft")
        .to_string_lossy()
        .into_owned();
    original.save(crate::native::files::test_file(&path))?;
    let reopened = Native::open(&path)?;
    let before = finish(reopened, original.sources.clone())?;
    assert_eq!(before.geometry.len(), native(&original).count());
    let total = |m: &NativeModel| -> f64 { native(m).map(|g| g.volume()).sum() };
    near(total(&before), total(&original));
    let mut native_model = before.native.ok_or_else(crate::project::no_design)?;
    native_model.change_parameters(json!({"board_thickness": "2.4 mm"}))?;
    let changed = finish(native_model, before.sources)?;
    let through = pad_bodies(&changed, r1, PadId(2));
    assert_eq!(through.len(), 3);
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for g in &through {
        lo = lo.min(g.min[2]);
        hi = hi.max(g.max[2]);
    }
    near(lo, -0.035);
    near(hi, 2.4 + 0.035);
    Ok(())
}

#[test]
fn an_envelope_follows_its_turned_placement_and_datum_like_its_2d_outline() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = envelope_demo()?;
    let package = &mut project.library.packages[0].definition;
    set_body(package, None);
    package.placement = crate::test_units::placement([0.25, -0.4, 0.0], [90.0, 0.0, 90.0]);
    let package = package.clone();
    let datum = sparksmith_core::library::Datum {
        offset: pt([0.3, -0.4]),
        rotation_mdeg: deg(90.0),
    };
    project.library.footprints[0].definition.datum = datum;
    for placement in &mut project.placements {
        placement.rotation_deg = 30.0;
    }
    let model = build(&project)?;
    for placement in &project.placements {
        let body = source_geometry(&model, BodyKind::PackageBody, |s| {
            s.instance == Some(placement.instance) && s.pad.is_none()
        })?;
        // Expected: each envelope corner through the model frame into the
        // footprint frame, then onto the board as the pads are placed.
        let frame = sparksmith_core::library::model_frame(&datum, &package.placement);
        let board = sparksmith_core::layout::placement_transform(placement, &datum);
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for corner in package.envelope_corners() {
            let f = frame.apply(corner);
            let [x, y] = board.apply([f[0], f[1]]);
            let z = match placement.layer {
                Layer::Top => T + 0.035 + f[2],
                Layer::Bottom => -0.035 - f[2],
            };
            for (axis, v) in [x, y, z].into_iter().enumerate() {
                lo[axis] = lo[axis].min(v);
                hi[axis] = hi[axis].max(v);
            }
        }
        bounds(body, lo, hi);
    }
    // The preview turns the box the same way (here in the datum frame).
    let preview = preview(&package)?;
    let body = preview
        .geometry
        .iter()
        .find(|g| g.kind == BodyKind::PartBody)
        .ok_or_else(|| Error::Verification("no preview body".into()))?;
    let t = package.placement.transform();
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for corner in package.envelope_corners() {
        let p = t.apply(corner);
        for axis in 0..3 {
            lo[axis] = lo[axis].min(p[axis]);
            hi[axis] = hi[axis].max(p[axis]);
        }
    }
    bounds(body, lo, hi);
    Ok(())
}

/// The showcase on 4 layers with `count` through vias on a 1.27 mm grid
/// beside its parts, and 200 inner tracks from vias spread over the grid
/// (a few hundred pads and tracks with the showcase's own).
pub(super) fn via_board(count: usize) -> Result<Project> {
    let mut project = Engine::showcase()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    project.stackup = Stackup::preset(4).map_err(|error| Error::Verification(error.to_string()))?;
    // The showcase's pours were filled for the showcase's own board and
    // stackup; this board's would be stale, so they are left unfilled.
    for fill in &mut project.fills {
        fill.computed = None;
    }
    let net = project.nets[0].id;
    let columns = 64usize;
    let rows = count.div_ceil(columns);
    let stride = (count / 200).max(2);
    let [width, depth] = project.board_size;
    project.board_size = [
        width.max(columns as f64 * 1.27 + 6.0),
        depth + rows as f64 * 1.27 + 4.0,
    ];
    for k in 0..count {
        let position = [
            3.0 + (k % columns) as f64 * 1.27,
            depth + 2.0 + (k / columns) as f64 * 1.27,
        ];
        let id = project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        project.vias.push(sparksmith_core::Via {
            id,
            net,
            position,
            drill: 0.3,
            diameter: 0.6,
            span: sparksmith_core::ViaSpan::Through,
            tented: k % 2 == 0,
            locked: false,
        });
        if k % stride == 0 && k % columns != columns - 1 {
            let id = project
                .allocate()
                .map_err(|error| Error::Verification(error.to_string()))?;
            project.tracks.push(Track {
                id,
                net,
                start: position,
                end: [position[0] + 1.27, position[1]],
                width: 0.15,
                layer: CopperLayer::Inner(1 + u8::from(k % 4 == 0)),
                locked: false,
            });
        }
    }
    Ok(project)
}

/// Hundreds of vias are placements of one definition per size, each with
/// its barrel and four rings where the via is; the laminate is stacked
/// slabs (no Boolean); and a saved file records every via compactly, so
/// reopening it finds each via's copper again.
#[test]
fn many_vias_are_placements_of_one_definition_and_survive_reopening() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = via_board(600)?;
    // A second via size gets a definition of its own.
    if let Some(via) = project.vias.last_mut() {
        via.drill = 0.4;
        via.diameter = 0.8;
    }
    let model = build(&project)?;
    let vias: Vec<&Source> = model.sources.iter().filter(|s| s.via.is_some()).collect();
    assert_eq!(vias.len(), project.vias.len());
    let definitions: BTreeSet<u64> = vias.iter().map(|s| s.component).collect();
    assert_eq!(definitions.len(), 2);
    // Three dielectrics, the two holding an inner foil split at it.
    let slabs = native(&model)
        .filter(|g| g.kind == BodyKind::Substrate)
        .count();
    assert_eq!(slabs, 5);
    let t = project.stackup.laminate_mm();
    let extent = |copper: &[&Geometry]| {
        let low = copper.iter().map(|g| g.min).fold([f64::MAX; 3], |a, b| {
            std::array::from_fn(|i| a[i].min(b[i]))
        });
        let high = copper.iter().map(|g| g.max).fold([f64::MIN; 3], |a, b| {
            std::array::from_fn(|i| a[i].max(b[i]))
        });
        (low, high)
    };
    for (via, source) in project.vias.iter().zip(&vias) {
        assert_eq!(source.via, Some(via.id));
        let copper: Vec<&Geometry> = native(&model)
            .filter(|g| g.item == Some(source.pick_id) && g.kind == BodyKind::ViaCopper)
            .collect();
        assert_eq!(copper.len(), 5, "via {}", via.id);
        let (low, high) = extent(&copper);
        let r = via.diameter / 2.0;
        for (axis, centre) in via.position.iter().enumerate() {
            assert!(
                (low[axis] - (centre - r)).abs() < 1e-6,
                "via {} {low:?}",
                via.id
            );
            assert!(
                (high[axis] - (centre + r)).abs() < 1e-6,
                "via {} {high:?}",
                via.id
            );
        }
        near(low[2], -0.035);
        near(high[2], t + 0.035);
    }
    let temporary = tempfile::tempdir()?;
    let path = temporary
        .path()
        .join("vias.solidcraft")
        .to_string_lossy()
        .into_owned();
    model.save(crate::native::files::test_file(&path))?;
    let reopened = Native::open(&path)?;
    let saved = saved_sources(&reopened)?;
    let key = |s: &&Source| (s.via, s.net, s.component, s.occurrence);
    let mut restored: Vec<_> = saved
        .iter()
        .filter(|s| s.via.is_some())
        .map(|s| key(&s))
        .collect();
    let mut original: Vec<_> = vias.iter().map(key).collect();
    restored.sort_unstable();
    original.sort_unstable();
    assert_eq!(restored, original);
    let again = finish(reopened, saved)?;
    for via in &project.vias {
        let picks: Vec<u64> = again
            .sources
            .iter()
            .filter(|s| s.via == Some(via.id))
            .map(|s| s.pick_id)
            .collect();
        let copper = again
            .geometry
            .iter()
            .filter(|g| g.item.is_some_and(|i| picks.contains(&i)))
            .count();
        assert_eq!(copper, 5, "reopened via {}", via.id);
    }
    Ok(())
}

/// A board build a newer request replaced stops at its next command, and
/// one never cancelled builds as `build` does.
#[test]
fn a_cancelled_build_stops_and_an_uncancelled_one_completes() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    let project = Engine::demo()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    let stopped = build_cancellable(&project, Arc::new(AtomicBool::new(true)));
    assert!(matches!(stopped, Err(Error::Cancelled)));
    let built = build_cancellable(&project, Arc::new(AtomicBool::new(false)))?;
    assert_eq!(native(&built).count(), native(&build(&project)?).count());
    Ok(())
}

/// A board of 10,000 vias and 1,500 tracks saves its identity within
/// Solidcraft's metadata limits, and reopening it finds every pad, track
/// and via's copper again. 1,500, not the document's 2,000 tracks: five
/// parameters a track reach Solidcraft's 10,000 parameters a design first
/// (`identity::tests` records 2,000 tracks and 16,384 vias through a real
/// save). Slow (a full 10,000-via build):
/// `cargo test --profile ci -p sparksmith-solidcraft ten_thousand_vias_and_many_tracks -- --ignored --nocapture`.
#[test]
#[ignore = "builds a 10,000-via board"]
fn ten_thousand_vias_and_many_tracks_survive_reopening() -> Result<()> {
    let _scope = native_scale_context()?;
    let mut project = via_board(10_000)?;
    let net = project.nets[0].id;
    let columns = 64;
    let first = project.vias.len() - 10_000;
    let mut k = 0usize;
    while project.tracks.len() < 1_500 {
        let (Some(a), Some(b)) = (
            project.vias.get(first + k).map(|v| v.position),
            project.vias.get(first + k + columns).map(|v| v.position),
        ) else {
            return Err(Error::Verification("ran out of vias for tracks".into()));
        };
        let id = project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?;
        project.tracks.push(Track {
            id,
            net,
            start: a,
            end: b,
            width: 0.15,
            layer: CopperLayer::Inner(1 + u8::from(k.is_multiple_of(2))),
            locked: false,
        });
        k += 1;
    }
    let start = std::time::Instant::now();
    let model = build(&project)?;
    eprintln!("built in {:?}", start.elapsed());
    let temporary = tempfile::tempdir()?;
    let path = temporary
        .path()
        .join("large.solidcraft")
        .to_string_lossy()
        .into_owned();
    model.save(crate::native::files::test_file(&path))?;
    let reopened = Native::open(&path)?;
    let metadata = serde_json::to_string(&reopened.session().doc.metadata)?;
    eprintln!("metadata {} bytes", metadata.len());
    let saved = saved_sources(&reopened)?;
    assert_eq!(
        serde_json::to_value(&saved)?,
        serde_json::to_value(&model.sources)?
    );
    let start = std::time::Instant::now();
    let again = finish(reopened, saved)?;
    eprintln!("reopened in {:?}", start.elapsed());
    let count = |m: &NativeModel, f: &dyn Fn(&Source) -> bool| {
        let picks: BTreeSet<u64> = m
            .sources
            .iter()
            .filter(|s| f(s))
            .map(|s| s.pick_id)
            .collect();
        let copper = m
            .geometry
            .iter()
            .filter(|g| !g.kind.is_appearance() && g.item.is_some_and(|i| picks.contains(&i)))
            .count();
        (picks.len(), copper)
    };
    for (what, f) in [
        (
            "vias",
            &(|s: &Source| s.via.is_some()) as &dyn Fn(&Source) -> bool,
        ),
        ("tracks", &|s: &Source| s.track.is_some()),
        ("pads", &|s: &Source| s.pad.is_some()),
    ] {
        let (was, now) = (count(&model, f), count(&again, f));
        assert_eq!(was, now, "{what}");
        assert!(was.0 > 0 && was.1 >= was.0, "{what}: {was:?}");
    }
    assert_eq!(count(&again, &|s| s.via.is_some()).0, project.vias.len());
    assert_eq!(count(&again, &|s| s.track.is_some()).0, 1_500);
    Ok(())
}

/// The demo with its chip package on the original glTF fixture model, its
/// first terminal on the model's lead.
pub(super) fn library_demo() -> Result<Project> {
    use crate::models::imported::tests::{fixture_glb, lead_key, model, use_lead};
    use sparksmith_core::library::bundle::{AssetBytes, PackedAsset};
    use sparksmith_core::library::model3d::{LeadTag, ModelFormat};
    use sparksmith_core::library::{DefinitionMeta, Published};
    use sparksmith_core::{DefinitionId, ModelRef, Revision};
    let bytes = fixture_glb();
    let seen = crate::models::inspect::inspect(&bytes, ModelFormat::Gltf)?;
    let mut definition = model(ModelFormat::Gltf, &bytes, &seen);
    definition.leads = vec![LeadTag {
        key: lead_key(&seen)?,
        designation: "1".into(),
        contact_nm: crate::test_units::nm3([0.0; 3]),
    }];
    definition.leads[0].contact_nm =
        crate::test_units::nm3(models::imported::lead_contacts(&definition, &bytes)?[0]);
    definition.provenance.record.licence = Some("CC0-1.0".into());
    let mut project = Engine::demo()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone();
    let id = DefinitionId(
        project
            .allocate()
            .map_err(|error| Error::Verification(error.to_string()))?,
    );
    let reference = ModelRef::new(id, Revision(1));
    project.library.assets.push(PackedAsset {
        hash: definition.asset.hash,
        media_type: definition.asset.media_type.clone(),
        data: AssetBytes::new(bytes.clone()),
    });
    project.library.models.push(Published {
        id,
        revision: Revision(1),
        meta: DefinitionMeta {
            name: "Original fixture".into(),
            ..DefinitionMeta::default()
        },
        definition,
    });
    let package = &mut project.library.packages[0];
    let terminal = package
        .definition
        .terminals
        .first()
        .map(|t| t.id.0)
        .ok_or_else(|| Error::Verification("demo package has no terminal".into()))?;
    package.definition.model = PackageModel::Library(sparksmith_core::library::LibraryModel {
        model: reference,
        ..use_lead(terminal)
    });
    Ok(project)
}

/// D2's board hook: a package with a library model (an original GLB with
/// its own clear-coated material) builds one shared definition its
/// instances occur in, its bodies carrying their instance's pick, the
/// lead bound to its terminal, and the model material's full look drawn.
#[test]
fn a_library_model_builds_on_the_board_as_one_shared_definition() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = library_demo()?;
    let (r1, r2) = instances(&project)?;
    let built = build(&project)?;
    assert_eq!(built.models.len(), 1, "one definition per package revision");
    for instance in [r1, r2] {
        let pick = built
            .sources
            .iter()
            .find(|s| s.instance == Some(instance) && s.occurrence.is_some())
            .map(|s| s.pick_id)
            .ok_or_else(|| Error::Verification(format!("{instance} has no model")))?;
        let bodies: Vec<&Geometry> = built
            .geometry
            .iter()
            .filter(|g| g.item == Some(pick))
            .collect();
        assert!(bodies.iter().any(|g| g.kind == BodyKind::ModelLead));
        let moulding = bodies
            .iter()
            .find(|g| g.kind == BodyKind::ModelBody)
            .ok_or_else(|| Error::Verification("no model body".into()))?;
        assert!(
            moulding.surface.clear_coat().is_some(),
            "the model material's coat is drawn"
        );
    }
    // The model worker's source cache: a rebuild neither re-hashes nor
    // re-reads the model file, and draws the same; a build that no longer
    // uses it drops it.
    let sources = models::imported::SourceCache::default();
    let start = std::time::Instant::now();
    let cold = build_with(&project, &sources, None)?;
    let cold_time = start.elapsed();
    assert_eq!(sources.counts(), (0, 1));
    let start = std::time::Instant::now();
    let warm = build_with(&project, &sources, None)?;
    let warm_time = start.elapsed();
    assert_eq!(sources.counts(), (1, 1), "the second build reads nothing");
    assert_eq!(sources.len(), 1);
    assert_eq!(warm.geometry.len(), cold.geometry.len());
    for (a, b) in cold.geometry.iter().zip(&warm.geometry) {
        assert_eq!(
            (a.kind, a.item, a.min, a.max),
            (b.kind, b.item, b.min, b.max)
        );
    }
    eprintln!("library-model board build: cold {cold_time:?}, cached {warm_time:?}");
    // A model or asset the project does not hold refuses the build,
    // naming it; nothing falls back to an envelope.
    let mut no_model = project.clone();
    no_model.library.models.clear();
    let refused = build_with(&no_model, &sources, None)
        .err()
        .map(|e| e.to_string());
    assert!(sources.is_empty(), "an unused model file is dropped");
    assert!(
        refused
            .as_deref()
            .is_some_and(|e| e.contains("3D model") && e.contains("missing")),
        "{refused:?}"
    );
    project.library.assets.clear();
    let refused = build(&project).err().map(|e| e.to_string());
    assert!(
        refused
            .as_deref()
            .is_some_and(|e| e.contains("fixture.glb") && e.contains("not packed")),
        "{refused:?}"
    );
    Ok(())
}

/// A laminate level drops a hole lying wholly inside another (a via ring
/// under a wide track's copper; one of two alike) and keeps holes that
/// only overlap, so the sketch leaves no island of laminate in a cut.
#[test]
fn nested_holes_are_dropped_and_overlapping_ones_kept() -> Result<()> {
    let circle = |center: Point, radius: f64| Contour::Circle { center, radius };
    let wide = polygon(&track_outline([0.0, 0.0], [3.0, 0.0], 1.0))
        .ok_or_else(|| Error::Verification("no track".into()))?;
    let narrow = polygon(&track_outline([5.0, 0.0], [8.0, 0.0], 0.2))
        .ok_or_else(|| Error::Verification("no track".into()))?;
    let kept = without_nested(vec![
        wide,
        circle([3.0, 0.0], 0.3),
        narrow,
        circle([8.0, 0.0], 0.3),
        circle([20.0, 0.0], 0.3),
        circle([20.0, 0.0], 0.3),
    ]);
    let circles: Vec<Point> = kept
        .iter()
        .filter_map(|c| match c {
            Contour::Circle { center, .. } => Some(*center),
            Contour::Outline(_) => None,
        })
        .collect();
    assert_eq!(
        kept.len(),
        4,
        "both tracks, the overlapping ring, one of the twins"
    );
    assert_eq!(circles, vec![[8.0, 0.0], [20.0, 0.0]]);
    Ok(())
}

/// The showcase with its pours filled: four layers, inner tracks, vias
/// and fills on outer and inner layers.
pub(super) fn showcase() -> Result<Project> {
    Ok(Engine::showcase()
        .map_err(|error| Error::Verification(error.to_string()))?
        .project()
        .clone())
}

/// The exact build makes the board plan's flat bodies and no others
/// ([`plan::parity`]): every kind of land of the shaped fixture, on both
/// sides and turned, and the showcase's tracks, vias, fills and the
/// laminate's foil levels.
#[test]
fn the_build_makes_the_plans_flat_bodies_and_no_others() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    for project in [shaped()?, showcase()?] {
        let plan = plan::of(&project)?;
        assert!(
            plan.laminate.len() >= project.stackup.dielectrics.len(),
            "a slab per dielectric at least"
        );
        let model = build(&project)?;
        let differences = plan::parity(&plan, &model)?;
        assert!(differences.is_empty(), "{differences:#?}");
    }
    Ok(())
}

/// The parity check sees a body the plan lacks, a body it holds that is
/// not built and one built in another shape.
#[test]
fn the_parity_check_finds_a_dropped_added_or_reshaped_body() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = showcase()?;
    let model = build(&project)?;
    let plan = plan::of(&project)?;
    // A track's body dropped from the plan: built, not planned.
    let mut dropped = plan.clone();
    dropped.tracks.pop();
    let differences = plan::parity(&dropped, &model)?;
    assert_eq!(differences.len(), 1, "{differences:#?}");
    assert!(differences[0].starts_with("built, not planned"));
    // A via body planned twice: planned, not built.
    let mut added = plan.clone();
    let extra = added.vias[0].bodies[0].clone();
    added.vias[0].bodies.push(extra);
    let differences = plan::parity(&added, &model)?;
    assert_eq!(differences.len(), 1, "{differences:#?}");
    assert!(differences[0].starts_with("planned, not built"));
    // A slab planned 10 µm thicker: neither matches the other.
    let mut thicker = plan.clone();
    thicker.laminate[0].z[1] += 0.01;
    let differences = plan::parity(&thicker, &model)?;
    assert_eq!(differences.len(), 2, "{differences:#?}");
    // A track planned 0.1 mm wider: neither matches the other.
    let mut wider = plan;
    let track = &project.tracks[0];
    let body = &mut wider.tracks[0].body;
    let length = (track.end[0] - track.start[0]).hypot(track.end[1] - track.start[1]);
    body.loops = vec![
        polygon(&track_outline([0.0, 0.0], [length, 0.0], track.width + 0.1))
            .ok_or_else(|| Error::Verification("no track".into()))?,
    ];
    let differences = plan::parity(&wider, &model)?;
    assert_eq!(differences.len(), 2, "{differences:#?}");
    Ok(())
}

/// The plan's heights are the stackup's: the laminate's slabs stack from
/// z = 0 to the board thickness without gap or overlap, each copper body
/// lies in its layer's foil, and a barrel spans the laminate.
#[test]
fn plan_bodies_lie_in_their_layers() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    for project in [shaped()?, showcase()?] {
        let plan = plan::of(&project)?;
        let stackup = &project.stackup;
        let thickness = stackup.laminate_mm();
        let mut slabs: Vec<[f64; 2]> = plan.laminate.iter().map(|s| s.z).collect();
        slabs.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let (Some(first), Some(last)) = (slabs.first(), slabs.last()) else {
            return Err(Error::Verification("no slab".into()));
        };
        near(first[0], 0.0);
        near(last[1], thickness);
        for pair in slabs.windows(2) {
            near(pair[0][1], pair[1][0]);
        }
        for body in plan.bodies().filter(|b| b.kind != BodyKind::Substrate) {
            match body.layer {
                Some(layer) => {
                    let (low, high) = stackup
                        .copper_span(layer)
                        .ok_or_else(|| Error::Verification("no layer".into()))?;
                    near(body.z[0], low);
                    near(body.z[1], high);
                }
                None => {
                    near(body.z[0], 0.0);
                    near(body.z[1], thickness);
                }
            }
            assert!(body.volume().is_some_and(|v| v > 0.0), "{}", body.name);
        }
    }
    // The shaped fixture's plated-through stadium: a land each side and a
    // barrel, on both placements; on four layers a land on each inner
    // layer too.
    for (project, inner) in [(shaped()?, vec![]), (four_layer_shaped()?, inner_layers())] {
        let plan = plan::of(&project)?;
        for placed in plan.pads.iter().filter(|p| p.pad == PadId(2)) {
            let mut parts: Vec<String> = placed
                .bodies
                .iter()
                .map(|b| match b.source {
                    plan::Origin::Pad { part, .. } => format!("{part:?}"),
                    _ => String::new(),
                })
                .collect();
            parts.sort();
            let mut expected: Vec<String> = [
                plan::PadPart::Land,
                plan::PadPart::Opposite,
                plan::PadPart::Barrel(0),
            ]
            .into_iter()
            .chain(inner.iter().map(|l| plan::PadPart::Inner(*l)))
            .map(|p| format!("{p:?}"))
            .collect();
            expected.sort();
            assert_eq!(parts, expected);
        }
    }
    Ok(())
}

/// The inner layers of a four-layer stackup.
fn inner_layers() -> Vec<CopperLayer> {
    vec![CopperLayer::Inner(1), CopperLayer::Inner(2)]
}

/// [`shaped`] on a four-layer stackup with a track and a via on each inner
/// layer: plated pads land copper on every layer.
pub(super) fn four_layer_shaped() -> Result<Project> {
    let mut project = shaped()?;
    let four = showcase()?.stackup;
    if four.layers().len() != 4 {
        return Err(Error::Verification("the showcase has four layers".into()));
    }
    project.stackup = four;
    let net = project
        .nets
        .first()
        .map(|n| n.id)
        .ok_or_else(|| Error::Verification("the fixture has a net".into()))?;
    let base = project.next_id;
    project.next_id = base + 3;
    for (k, layer) in inner_layers().into_iter().enumerate() {
        let y = 4.0 + 2.0 * k as f64;
        project.tracks.push(Track {
            id: base + k as u64,
            net,
            start: [4.0, y],
            end: [12.0, y + 1.0],
            width: 0.25,
            layer,
            locked: false,
        });
    }
    project.vias.push(sparksmith_core::Via {
        id: base + 2,
        net,
        position: [16.0, 4.0],
        drill: 0.3,
        diameter: 0.6,
        span: sparksmith_core::ViaSpan::Through,
        tented: false,
        locked: false,
    });
    Ok(project)
}

/// 2D equals 3D, measured: on every layer of every fixture (every pad
/// kind on both sides and turned, plated pads through four layers, the
/// showcase's tracks, vias and fills), the board plan's copper (which
/// Layout draws and the 3D view meshes) covers the core's copper (which
/// the copper graph joins and fabrication plots) within flattening, and
/// the 3D view's meshes are the plan's bodies.
#[test]
fn every_layers_copper_is_the_cores_in_the_plan_and_its_meshes() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    for project in [shaped()?, four_layer_shaped()?, showcase()?] {
        let plan = plan::of(&project)?;
        let copper = plan::copper(&project, &mut plan::Memo::default())?;
        let all: Vec<_> = plan
            .bodies()
            .filter(|b| b.kind != BodyKind::Substrate)
            .collect();
        let only: Vec<_> = copper.bodies().collect();
        assert_eq!(all, only, "the copper plan is the full plan's copper");
        for layer in copper_agree::agreement(&project, &plan)? {
            assert!(layer.plan_mm2 > 0.0, "{layer:?}");
            assert!(layer.agrees(), "{layer:?}");
        }
        let sources = mesh::plan_sources(&plan);
        let (meshed, _) = mesh::bodies(&plan, &sources, &mut mesh::Memo::default())?;
        let differences = mesh::parity(&plan, &meshed)?;
        assert!(differences.is_empty(), "{differences:#?}");
    }
    Ok(())
}

/// `project` on a solved sketch of its rectangle with one 3 mm plated
/// mounting hole and a 6 mm washer, 6 mm in from its top right corner.
fn with_washer(mut project: Project) -> Project {
    use sparksmith_core::model::board::{
        BoardFeatureId, BoardShape, BoardSketch, SolvedBoard, SolvedMountingHole,
    };
    let [w, h] = project.board_size;
    let mut solved = SolvedBoard::rectangle(project.board_size);
    solved.holes.push(SolvedMountingHole {
        id: BoardFeatureId(1),
        at: [w - 6.0, h - 6.0],
        drill: 3.0,
        plated: true,
        pad_diameter: Some(6.0),
    });
    project.board_shape = BoardShape::Sketch(Box::new(BoardSketch {
        evaluator: Default::default(),
        sketch: Default::default(),
        parameters: Vec::new(),
        cutouts: Vec::new(),
        holes: Vec::new(),
        edges: Vec::new(),
        next_feature: 2,
        solve: Default::default(),
        solved,
        outline_fit: 0.01,
        fault: None,
        idle: Vec::new(),
    }));
    project
}

/// The two rings and the barrel all select their mounting hole. Loading the
/// source table from disk must recover the same picks without the PCB project.
#[test]
fn mounting_hole_picks_survive_native_reopening() -> Result<()> {
    use sparksmith_core::model::board::{BoardFeatureId, BoardShape};
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = with_washer(fixture()?);
    let BoardShape::Sketch(sketch) = &mut project.board_shape else {
        return Err(Error::Verification("expected a board sketch".into()));
    };
    let first = sketch.solved.holes[0];
    let mut second = first;
    second.id = BoardFeatureId(17);
    second.at = [6.0, 6.0];
    sketch.solved.holes.push(second);
    sketch.next_feature = 18;
    let plan = plan::of(&project)?;
    let expected = [None, Some(CopperLayer::Top), Some(CopperLayer::Bottom)];
    let sources = mesh::plan_sources(&plan);
    let (meshed, _) = mesh::bodies(&plan, &sources, &mut mesh::Memo::default())?;
    for hole in [first, second] {
        let mut from_plan = Vec::new();
        for body in &meshed {
            if let plan::Origin::Washer { hole: id, layer } = body.source
                && id == hole.id
            {
                let source = sources
                    .iter()
                    .find(|s| Some(s.pick_id) == body.geometry.item);
                assert_eq!(
                    source.and_then(|s| s.hole.as_ref()).map(|h| h.id),
                    Some(hole.id)
                );
                from_plan.push(layer);
            }
        }
        assert_eq!(from_plan, expected);
    }
    let check = |model: &NativeModel| -> Result<Vec<u64>> {
        assert!(
            model
                .geometry
                .iter()
                .filter(|g| g.kind == BodyKind::Substrate)
                .all(|g| g.item.is_none())
        );
        let mut picks = Vec::new();
        for hole in [first, second] {
            let sources: Vec<_> = model
                .sources
                .iter()
                .filter(|s| s.hole.as_ref().is_some_and(|h| h.id == hole.id))
                .collect();
            let [source] = sources.as_slice() else {
                return Err(Error::Verification(format!(
                    "expected one source for hole {:?}",
                    hole.id
                )));
            };
            let mut layers = Vec::new();
            let [x, y] = hole.at;
            for g in model
                .geometry
                .iter()
                .filter(|g| g.item == Some(source.pick_id))
            {
                assert_eq!(g.kind, BodyKind::PadCopper);
                // The circle mesh starts at its eastern edge and is symmetric
                // about Y = y. An odd chord count can miss its western extreme.
                let outer_radius = if g.layer.is_some() { 3.0 } else { 1.525 };
                near(g.max[0] - outer_radius, x);
                near((g.min[1] + g.max[1]) / 2.0, y);
                layers.push(g.layer);
            }
            layers.sort();
            assert_eq!(layers, expected);
            picks.push(source.pick_id);
        }
        assert_ne!(picks[0], picks[1]);
        Ok(picks)
    };
    let live = build_flat(&project, &SourceCache::default(), None, Flat::Meshed)?;
    let picks = check(&live)?;
    let exact = build(&project)?;
    assert_eq!(check(&exact)?, picks);
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("plated-hole.solidcraft");
    let path = path.to_string_lossy();
    exact.save(crate::native::files::test_file(&path))?;
    let reopened = Native::open(&path)?;
    let restored = saved_sources(&reopened)?;
    assert_eq!(check(&finish(reopened, restored)?)?, picks);
    Ok(())
}

/// A plated mounting hole's washer is copper of the board's physical
/// inventory on every layer: the plan holds a ring on each, of area
/// π (3² − 1.5²) mm² on the outer layers (hand-derived from the 6 mm
/// washer round the 3 mm finished hole) and π (3² − 1.525²) mm² on the
/// inner ones (round the barrel, drilled 25 µm of plating wider), and the
/// barrel; the core's copper agrees with it layer by layer, the 3D view
/// meshes it, the exact build builds it, and a plan without it differs on
/// every layer.
#[test]
fn a_plated_mounting_holes_washer_is_copper_on_every_layer() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = with_washer(four_layer_shaped()?);
    let plan = plan::of(&project)?;
    let [washer] = plan.washers.as_slice() else {
        return Err(Error::Verification(format!(
            "{} washers",
            plan.washers.len()
        )));
    };
    let layers = project.stackup.layers();
    assert_eq!(
        washer.bodies.len(),
        layers.len() + 1,
        "a ring a layer and the barrel"
    );
    let pi = std::f64::consts::PI;
    for body in &washer.bodies {
        let volume = body.volume().unwrap_or(0.0);
        let area = volume / (body.z[1] - body.z[0]);
        let want = match body.layer {
            None => pi * (1.525_f64.powi(2) - 1.5_f64.powi(2)),
            Some(l) if l.side().is_some() => pi * (3.0_f64.powi(2) - 1.5_f64.powi(2)),
            Some(_) => pi * (3.0_f64.powi(2) - 1.525_f64.powi(2)),
        };
        assert!((area - want).abs() < 0.01, "{}: {area} mm²", body.name);
    }
    for layer in copper_agree::agreement(&project, &plan)? {
        assert!(layer.agrees(), "{layer:?}");
    }
    let sources = mesh::plan_sources(&plan);
    let (meshed, _) = mesh::bodies(&plan, &sources, &mut mesh::Memo::default())?;
    let differences = mesh::parity(&plan, &meshed)?;
    assert!(differences.is_empty(), "{differences:#?}");
    let model = build(&project)?;
    let differences = plan::parity(&plan, &model)?;
    assert!(differences.is_empty(), "{differences:#?}");
    let mut without = plan.clone();
    without.washers.clear();
    let differing = copper_agree::agreement(&project, &without)?;
    assert!(differing.iter().all(|l| !l.agrees()), "{differing:#?}");
    Ok(())
}

/// Every plated hole the drill inventory lists has a plated wall in the
/// exact model, measured against the inventory rather than the plan (an
/// exact-versus-live parity check passes when both omit a barrel): a
/// copper body that spans the laminate, centred on the hole, whose box is
/// the drilled hole (the finished hole and 25 µm of plating) and whose
/// volume is the wall's, π ((d/2 + 0.025)² − (d/2)²) × the laminate's
/// thickness. The board carries a plated mounting hole with a washer, one
/// without, and a via.
#[test]
fn every_plated_hole_of_the_inventory_has_its_wall_in_the_exact_model() -> Result<()> {
    use sparksmith_core::layout::physical::{self, HoleFeature};
    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut project = with_washer(four_layer_shaped()?);
    if let sparksmith_core::model::BoardShape::Sketch(sketch) = &mut project.board_shape {
        sketch
            .solved
            .holes
            .push(sparksmith_core::model::board::SolvedMountingHole {
                id: sparksmith_core::model::board::BoardFeatureId(2),
                at: [6.0, 6.0],
                drill: 2.2,
                plated: true,
                pad_diameter: None,
            });
        sketch.next_feature = 3;
    }
    let model = build(&project)?;
    let geometry = model.native_geometry()?;
    // The laminate: every dielectric and the inner foils between them.
    let thickness = project.stackup.laminate_mm();
    let plated: Vec<physical::Hole> = physical::holes(&project)
        .into_iter()
        .filter(|h| h.plated && !matches!(h.owner, physical::HoleOwner::Pad { .. }))
        .collect();
    assert!(plated.len() >= 3, "{plated:?}");
    let mut missing = Vec::new();
    for hole in &plated {
        let HoleFeature::Round { at, diameter } = hole.feature else {
            continue;
        };
        let (r, wall) = (diameter / 2.0, diameter / 2.0 + 0.025);
        let volume = std::f64::consts::PI * (wall * wall - r * r) * thickness;
        let found = geometry.iter().any(|g| {
            matches!(g.kind, BodyKind::PadCopper | BodyKind::ViaCopper)
                && (g.min[2] - 0.0).abs() < 1e-3
                && (g.max[2] - thickness).abs() < 1e-3
                && (g.min[0] - (at[0] - wall)).abs() < 2e-3
                && (g.max[0] - (at[0] + wall)).abs() < 2e-3
                && (g.min[1] - (at[1] - wall)).abs() < 2e-3
                && (g.max[1] - (at[1] + wall)).abs() < 2e-3
                && (g.volume.get() - volume).abs() < 0.01 * volume
        });
        if !found {
            missing.push(format!("{} ({diameter} mm at {at:?})", hole.name));
        }
    }
    assert!(missing.is_empty(), "no plated wall: {missing:#?}");
    Ok(())
}

/// The measure finds copper the plan lacks: a pad's land dropped from one
/// layer, and a track drawn wider.
#[test]
fn the_copper_measure_finds_missing_and_reshaped_copper() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let project = four_layer_shaped()?;
    let plan = plan::of(&project)?;
    let mut dropped = plan.clone();
    for pad in &mut dropped.pads {
        pad.bodies.retain(|b| {
            !matches!(
                b.source,
                plan::Origin::Pad {
                    part: plan::PadPart::Inner(CopperLayer::Inner(1)),
                    ..
                }
            )
        });
    }
    let layers = copper_agree::agreement(&project, &dropped)?;
    let bad: Vec<_> = layers.iter().filter(|l| !l.agrees()).collect();
    assert_eq!(bad.len(), 1, "{layers:#?}");
    assert_eq!(bad[0].layer, CopperLayer::Inner(1));
    let mut wider = project.clone();
    for track in &mut wider.tracks {
        track.width += 0.05;
    }
    let differing = copper_agree::agreement(&wider, &plan)?;
    assert!(differing.iter().any(|l| !l.agrees()), "{differing:#?}");
    Ok(())
}

/// The active assembly variant decides the bodies: a part it does not fit
/// places none, in the live view and the export alike (both read
/// `placed_parts`), and a part it fits another Part on places that Part's
/// package (docs/assembly-variants.md).
#[test]
fn the_active_variant_decides_which_bodies_are_placed() -> Result<()> {
    use sparksmith_core::library::marking::{
        Align, LaidElement, MarkingItem, MarkingSpec, Template,
    };
    use sparksmith_core::library::{PartDefinition, Published};
    use sparksmith_core::{
        AssemblyVariant, BindingRef, ComponentRef, DefinitionId, FootprintRef, Id, PackageRef,
        PartRef, Population, VariantOverride,
    };
    let fail = |what: &str| Error::Verification(what.into());
    let mut project = envelope_demo()?;
    let (r1, r2) = instances(&project)?;
    assert_eq!(placed_parts(&project)?.len(), 2);

    // A second package on the same lands: copies of R1's package,
    // footprint, binding and component under new lineages, and a Part.
    let instance = project.instance(r1).ok_or_else(|| fail("R1"))?.clone();
    let own = instances::resolve(&project.library, &instance).map_err(|e| fail(&e.to_string()))?;
    let physical = own.physical.ok_or_else(|| fail("R1's footprint"))?;
    let own_stem = models::definition_name(physical.package.id, physical.package.revision.0);
    let (package, footprint, binding, component) = (
        physical.package.clone(),
        physical.footprint.clone(),
        physical.binding.clone(),
        own.component.clone(),
    );
    let mut new_id = || -> Result<DefinitionId> {
        project
            .allocate()
            .map(DefinitionId)
            .map_err(|e| fail(&e.to_string()))
    };
    let ids = [new_id()?, new_id()?, new_id()?, new_id()?, new_id()?];
    let [pkg, fp, bd, comp, part] = ids;
    let package_ref = PackageRef::new(pkg, package.revision);
    let footprint_ref = FootprintRef::new(fp, footprint.revision);
    let binding_ref = BindingRef::new(bd, binding.revision);
    let component_ref = ComponentRef::new(comp, component.revision);
    let mut other_package = Published { id: pkg, ..package };
    other_package.definition.placement.translation_nm = crate::test_units::nm3([0.0, 0.0, 0.1]);
    let mut other_footprint = Published {
        id: fp,
        ..footprint
    };
    other_footprint.definition.package = package_ref;
    let mut other_binding = Published { id: bd, ..binding };
    other_binding.definition.component = comp;
    other_binding.definition.package = package_ref;
    other_binding.definition.footprints = vec![footprint_ref];
    let mut other_component = Published {
        id: comp,
        ..component
    };
    other_component.definition.implementations = vec![binding_ref];
    project.library.packages.push(other_package);
    project.library.footprints.push(other_footprint);
    project.library.bindings.push(other_binding);
    project.library.components.push(other_component);
    let part_ref = PartRef::new(part, sparksmith_core::Revision::FIRST);
    project.library.parts.push(Published {
        id: part,
        revision: part_ref.revision,
        meta: Default::default(),
        definition: PartDefinition {
            mpn: "EX-ALT-1".into(),
            component: component_ref,
            binding: binding_ref,
            top_mark: Some(Box::new(MarkingSpec {
                items: vec![MarkingItem::Text {
                    template: Template("{mpn}".into()),
                    height_nm: None,
                    align: Align::Centre,
                }],
                ..MarkingSpec::default()
            })),
            ..PartDefinition::default()
        },
    });

    let variant = project.allocate().map_err(|e| fail(&e.to_string()))?;
    project.variants.list.push(AssemblyVariant {
        id: variant,
        name: "Alt".into(),
        overrides: [
            (
                r1,
                VariantOverride {
                    population: None,
                    part: Some(part_ref),
                },
            ),
            (
                r2,
                VariantOverride {
                    population: Some(Population::NotPopulated),
                    part: None,
                },
            ),
        ]
        .into(),
        reviews: Default::default(),
    });
    // The default still places both parts on their own package.
    let default = placed_parts(&project)?;
    assert_eq!(default.len(), 2);
    assert!(default.iter().all(|p| p.stem == own_stem));
    project.variants.active = Some(variant);
    let alt = placed_parts(&project)?;
    assert_eq!(alt.len(), 1, "R2 is not fitted in Alt");
    let r1_part = alt.first().ok_or_else(|| fail("R1 placed"))?;
    assert_eq!(r1_part.instance.id, r1);
    assert_eq!(
        r1_part.stem,
        models::definition_name(pkg, package_ref.revision.0),
        "R1 carries the substitute's package"
    );
    // The substitute is marked from its own data, through the same marking
    // path as any fitted part; R2, not fitted, has no marking.
    let marked = |project: &Project| -> Vec<(Id, Vec<String>)> {
        crate::marking::board::instance_layouts(project)
            .layouts
            .iter()
            .map(|(id, layout)| {
                let texts = layout
                    .elements
                    .iter()
                    .filter_map(|e| match e {
                        LaidElement::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect();
                (*id, texts)
            })
            .collect()
    };
    assert_eq!(marked(&project), [(r1, vec!["EX-ALT-1".to_owned()])]);
    project.variants.active = None;
    assert!(
        marked(&project)
            .iter()
            .all(|(_, texts)| !texts.iter().any(|t| t == "EX-ALT-1")),
        "the default part does not carry the substitute's marking"
    );
    // The default population is typed on the instance as well.
    project.variants.active = None;
    for i in project.instances.iter_mut() {
        if i.id == r1 {
            i.population = Population::NotPopulated;
        }
    }
    let left = placed_parts(&project)?;
    assert_eq!(left.len(), 1);
    assert!(left.iter().all(|p| p.instance.id == r2));
    Ok(())
}

/// A package of no body places nothing: neither the live view's parts nor
/// the export's see it, while the other parts stay.
#[test]
fn a_package_of_no_body_places_nothing() -> Result<()> {
    let mut project = envelope_demo()?;
    let all = placed_parts(&project)?.len();
    assert!(all >= 2, "the demo places parts");
    let (r1, _) = instances(&project)?;
    let instance = project
        .instance(r1)
        .ok_or_else(|| Error::Verification("R1".into()))?;
    let package = instances::resolve(&project.library, instance)
        .ok()
        .and_then(|r| r.physical)
        .map(|p| p.package.id)
        .ok_or_else(|| Error::Verification("R1's package".into()))?;
    for published in &mut project.library.packages {
        if published.id == package {
            published.definition.model = PackageModel::NoBody;
        }
    }
    let left = placed_parts(&project)?;
    assert!(left.len() < all);
    assert!(left.iter().all(|p| p.instance.id != r1));
    assert!(
        left.iter()
            .all(|p| !matches!(p.package.model, PackageModel::NoBody))
    );
    Ok(())
}

/// Both faces of the board use the same authored 0.2 mm strip and 35 µm
/// copper. Its two named nets survive both model paths, including picking.
#[test]
fn bridge_group_and_both_nets_survive_the_live_and_exact_board() -> Result<()> {
    use sparksmith_core::library::tie::{Assembly, Bridge, TieForm, TieSpec};
    use sparksmith_core::library::{FootprintKeepout, PadSide};
    use sparksmith_core::schematic::SchematicCommand;
    use sparksmith_core::{BridgeId, LabelKind, Operation, TieId};

    let _context = crate::looks::test_context().map(|c| c.enter());
    let mut engine = Engine::demo().map_err(|e| Error::Verification(e.to_string()))?;
    let (first, second) = instances(engine.project())?;
    for (instance, terminal, x) in [(first, 1, -24.0), (second, 2, 24.0)] {
        engine
            .apply(
                engine.revision(),
                Operation::Schematic(SchematicCommand::PlaceLabel {
                    sheet: None,
                    name: "RETURN".into(),
                    kind: LabelKind::Local,
                    position: pt([x, 0.0]),
                    rotation: Default::default(),
                    attach: Some(
                        Endpoint {
                            instance,
                            terminal: TerminalId(terminal),
                        }
                        .into(),
                    ),
                }),
            )
            .map_err(|e| Error::Verification(e.to_string()))?;
    }
    let mut project = engine.project().clone();
    let expected: std::collections::BTreeSet<_> = project.nets.iter().map(|n| n.id).collect();
    assert_eq!(expected.len(), 2);
    let component = &mut project.library.components[0].definition;
    let group = TieId(component.next_local);
    component.next_local += 1;
    component.assembly = Assembly::CopperOnly;
    component.ties = vec![TieSpec {
        id: group,
        ends: vec![TerminalId(1), TerminalId(2)],
        form: TieForm::Copper,
    }];
    for instance in &mut project.instances {
        instance.ties.insert(group, Default::default());
    }
    let package = &mut project.library.packages[0].definition;
    package.model = PackageModel::NoBody;
    package.joints = None;
    let footprint = &mut project.library.footprints[0].definition;
    let bridge = BridgeId(footprint.next_local);
    footprint.next_local += 1;
    let a = footprint.pads[0].position()[0];
    let b = footprint.pads[1].position()[0];
    footprint.bridges.push(Bridge {
        id: bridge,
        side: PadSide::Top,
        pads: footprint.pads.iter().map(|p| p.id).collect(),
        outline: FootprintKeepout::rect("", [a.min(b), -0.1], [a.max(b), 0.1]).outline,
    });
    for placement in &mut project.placements {
        placement.rotation_deg = 90.0;
        placement.layer = if placement.instance == first {
            Layer::Top
        } else {
            Layer::Bottom
        };
    }

    let plan = plan::of(&project)?;
    assert_eq!(plan.bridges.len(), 2);
    for layer in copper_agree::agreement(&project, &plan)? {
        assert!(layer.agrees(), "{layer:?}");
    }
    let mut missing = plan.clone();
    missing.bridges.clear();
    for layer in copper_agree::agreement(&project, &missing)? {
        assert!(
            layer.objects_differ_mm2 > 0.0,
            "removed bridge on {:?}",
            layer.layer
        );
    }
    let cache = SourceCache::default();
    let live = build_flat(&project, &cache, None, Flat::Meshed)?;
    let exact = build(&project)?;
    let differences = plan::parity(&plan, &exact)?;
    assert!(differences.is_empty(), "{differences:#?}");
    let extra = plan::check(&missing, &exact, &mut || Ok(()))?.differences;
    assert_eq!(extra.len(), 2);
    for difference in extra {
        let plan::Source::Bridge { instance, .. } = difference.source else {
            return Err(Error::Verification("Expected the omitted bridge.".into()));
        };
        let owner = project
            .instance(instance)
            .ok_or_else(|| Error::Verification("Missing bridge owner.".into()))?;
        let line = crate::export::describe(&project, &difference);
        assert!(
            line.starts_with(&format!(
                "Bridge {bridge} of {} group {group}",
                owner.designator
            )),
            "{line}"
        );
        assert!(line.contains("bridge copper"), "{line}");
    }
    for model in [&live, &exact] {
        assert!(
            part_picks(&model.sources).is_empty(),
            "the copper-only packages have no package body to pick"
        );
        for (instance, layer) in [(first, Layer::Top), (second, Layer::Bottom)] {
            let matches = |s: &Source| s.instance == Some(instance) && s.tie.is_some();
            let source = model
                .sources
                .iter()
                .find(|s| matches(s))
                .ok_or_else(|| Error::Verification("Missing copper bridge.".into()))?;
            let identity = source
                .tie
                .as_ref()
                .ok_or_else(|| Error::Verification("Missing bridge identity.".into()))?;
            assert_eq!((identity.group, identity.bridge), (group, bridge));
            assert_eq!(source.layer, Some(layer.into()));
            assert_eq!(
                identity
                    .nets
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            near(
                source_geometry(model, BodyKind::CopperTrack, matches)?.volume(),
                (a - b).abs() * 0.2 * 0.035,
            );
        }
    }
    Ok(())
}

/// Set a physical wall independently of the tool diameters requested by
/// the fab. The washer, via, pad and laminate all use the same 40 µm wall.
#[test]
fn the_stackup_sets_the_barrel_wall_independently_of_drill_allowance() -> Result<()> {
    let _context = crate::looks::test_context()?.enter();
    let mut project = with_washer(four_layer_shaped()?);
    project.stackup.barrel_plating = Some(sparksmith_core::model::BarrelPlating {
        wall_nm: nm(0.040),
        source: "Original test coupon section".into(),
        assumed: false,
    });
    let profile = project
        .rules
        .profile
        .as_mut()
        .ok_or_else(|| Error::Verification("missing profile".into()))?;
    profile.drill_files = sparksmith_core::rules::profile::DrillFiles::Drilled {
        pth_allowance: 0.1,
        source: "Test drill process".into(),
    };
    let drills = sparksmith_core::fabrication::excellon::files(
        &project,
        &Default::default(),
        &Default::default(),
        "fixture",
    )
    .map_err(|e| Error::Verification(e.to_string()))?;
    let file = drills
        .iter()
        .find(|f| f.name == "fixture-plated.drl")
        .ok_or_else(|| Error::Verification("missing plated drill file".into()))?;
    assert!(String::from_utf8_lossy(&file.bytes).lines().any(|line| {
        line.starts_with('T')
            && line
                .split_once('C')
                .is_some_and(|(_, size)| size == "3.100")
    }));
    let plan = plan::of(&project)?;
    let mut measured = [0; 3];
    for body in plan.bodies() {
        let kind = match body.source {
            plan::Origin::Washer { layer: None, .. } => 0,
            plan::Origin::Via {
                part: plan::ViaPart::Barrel,
                ..
            } => 1,
            plan::Origin::Pad {
                part: plan::PadPart::Barrel(_),
                ..
            } => 2,
            _ => continue,
        };
        let [
            Contour::Circle { radius: wall, .. },
            Contour::Circle { radius: bore, .. },
        ] = body.loops.as_slice()
        else {
            return Err(Error::Verification(format!(
                "expected round barrel on {}",
                body.name
            )));
        };
        near(wall - bore, 0.040);
        measured[kind] += 1;
    }
    assert!(measured.into_iter().all(|n| n > 0));
    // The via is at (16,4), with a 0.30 mm finished bore. Its barrel fills
    // the hole of radius 0.19 in the top dielectric.
    let slab = plan
        .laminate
        .first()
        .ok_or_else(|| Error::Verification("missing slab".into()))?;
    assert!(
        slab.loops
            .iter()
            .any(|c| matches!(c, Contour::Circle { center, radius }
        if *center == [16.0,4.0] && (radius - 0.19).abs() < 1e-9))
    );
    let model = build(&project)?;
    let native = model
        .native()
        .ok_or_else(|| Error::Verification("missing native model".into()))?;
    let metadata = &native.session().doc.metadata.0;
    assert_eq!(
        metadata.get("org.sparksmith.electronics.barrel_plating"),
        Some(&serde_json::to_value(&project.stackup.barrel_plating)?)
    );
    Ok(())
}
