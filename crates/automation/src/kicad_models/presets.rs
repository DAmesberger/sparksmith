//! A per-import cache of shipped generator outputs. Pad matching selects a
//! mechanical approximation only; it never changes terminals, pads or wiring.
use super::{BTreeMap, Choice, Engine, ModelPlacement, ModelRef, PackageModel, Pad, publish_model};
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::PadShape;
use sparksmith_core::library::generated::{self, GeneratorRuntime, expand};
use sparksmith_core::library::model3d::Redistribution;
use sparksmith_core::library::package_model::PartFileModel;
use sparksmith_generators::abi::v1;
use sparksmith_io::generator_bundle::GeneratorBundle;
use sparksmith_io::shipped::ShippedLibrary;
use std::sync::Arc;

const TOLERANCE: f64 = 0.05;
const MAX_PRESETS: usize = 512;
struct Candidate {
    bundle: Arc<GeneratorBundle>,
    generation: v1::Generation,
    pads: Vec<Pad>,
    name: String,
}
#[derive(Default)]
pub(super) struct Presets {
    candidates: Option<Vec<Candidate>>,
    failure: Option<String>,
    chosen: BTreeMap<usize, Choice>,
    matches: BTreeMap<ContentHash, Option<(usize, ModelPlacement)>>,
}
impl Presets {
    fn load(&mut self, deadline: std::time::Instant) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.clone());
        }
        let result = self.build(deadline);
        if let Err(failure) = &result {
            self.failure = Some(failure.clone());
        }
        result
    }
    fn build(&mut self, deadline: std::time::Instant) -> Result<(), String> {
        if self.candidates.is_some() {
            return Ok(());
        }
        super::check_time(deadline)?;
        let library = ShippedLibrary::locate().map_err(|e| e.to_string())?;
        let names = library.generators().map_err(|e| e.to_string())?;
        if names.len() > 64 {
            return Err("3D preset search exceeds 64 generators".into());
        }
        let runtime = sparksmith_generators::worker::Worker::default();
        let mut candidates = Vec::new();
        let mut count = 0;
        for name in names {
            let bundle = Arc::new(library.generator(&name).map_err(|e| e.to_string())?);
            let description = runtime
                .describe(&bundle.module)
                .map_err(|e| e.to_string())?;
            if !description.scopes().contains(&v1::Scope::Package) {
                continue;
            }
            for preset in description.presets() {
                super::check_time(deadline)?;
                count += 1;
                if count > MAX_PRESETS {
                    return Err("3D preset search exceeds 512 presets".into());
                }
                let scope = if description.scopes().contains(&v1::Scope::Component) {
                    v1::Scope::Component
                } else {
                    v1::Scope::Package
                };
                let request = generated::complete(&description, scope, &preset.params)
                    .map_err(|e| e.to_string())?;
                let generation = runtime
                    .generate(&bundle.module, &request)
                    .map_err(|e| e.to_string())?
                    .generation;
                let pads = expand::footprint_geometry(&generation)
                    .map_err(|e| e.to_string())?
                    .pads;
                candidates.push(Candidate {
                    bundle: bundle.clone(),
                    generation,
                    pads,
                    name: format!("{name}/{}", preset.key),
                });
            }
        }
        super::check_time(deadline)?;
        self.candidates = Some(candidates);
        Ok(())
    }
    pub(super) fn match_model(
        &mut self,
        engine: &mut Engine,
        pads: &[Pad],
        prefix: &str,
        models: &mut BTreeMap<ContentHash, ModelRef>,
        deadline: std::time::Instant,
    ) -> Result<Option<Choice>, String> {
        super::check_time(deadline)?;
        if pads.is_empty() {
            return Ok(None);
        }
        self.load(deadline)?;
        let Some(candidates) = &self.candidates else {
            return Ok(None);
        };
        let mut geometry: Vec<_> = pads
            .iter()
            .map(|p| {
                (
                    &p.number,
                    p.kind,
                    &p.hole,
                    p.side,
                    p.position,
                    p.rotation_mdeg,
                    &p.shape,
                )
            })
            .collect();
        geometry.sort_by(|a, b| a.0.cmp(b.0));
        let key = sparksmith_core::library::bundle::asset_hash(
            &serde_json::to_vec(&(geometry, prefix)).map_err(|e| e.to_string())?,
        );
        let found = if let Some(found) = self.matches.get(&key) {
            *found
        } else {
            let mut matching = Vec::new();
            for (index, candidate) in candidates.iter().enumerate() {
                super::check_time(deadline)?;
                if let Some(placement) = matches(pads, &candidate.pads) {
                    matching.push((index, placement));
                }
            }
            let found = if matching.len() == 1 {
                matching.first().copied()
            } else {
                // The generator supplies this optional appearance hint. It never
                // replaces geometry matching or changes electrical definitions.
                let hinted: Vec<_> = matching
                    .into_iter()
                    .filter(|(index, _)| {
                        candidates[*index]
                            .generation
                            .component
                            .as_ref()
                            .is_some_and(|c| c.prefix.eq_ignore_ascii_case(prefix))
                    })
                    .collect();
                (hinted.len() == 1).then(|| hinted[0])
            };
            self.matches.insert(key, found);
            found
        };
        let Some((index, placement)) = found else {
            return Ok(None);
        };
        if let Some(choice) = self.chosen.get(&index) {
            let mut choice = choice.clone();
            choice.placement = placement;
            return Ok(Some(choice));
        }
        let candidate = &candidates[index];
        super::check_time(deadline)?;
        let mut made = crate::session::generated_models::generated_model(
            engine,
            &candidate.generation,
            Some(&candidate.bundle),
        )
        .map_err(|e| e.to_string())?;
        super::check_time(deadline)?;
        made.definition.provenance.redistribution = Redistribution::Forbidden;
        let key = made.definition.asset.hash;
        let provenance = format!(
            "Generated from preset {} (bundle sha256 {}). Mechanical approximation: pad centres and sizes within 50 µm, orientation within 0.1°. An ambiguous land pattern may use the generator’s declared reference prefix. No electrical mapping inferred. Project-only.",
            candidate.name, candidate.bundle.hash
        );
        let reference = if let Some(reference) = models.get(&key) {
            *reference
        } else {
            let bytes = made.data.take().ok_or("Preset model bytes unavailable")?;
            let reference = publish_model(engine, &made.name, made.definition, bytes, &provenance)?;
            models.insert(key, reference);
            reference
        };
        // The model is mechanical only; the import keeps its own terminal IDs.
        // Do not retain the generator's lead-to-terminal bindings.
        made.derived.leads.clear();
        let envelope_nm = super::envelope(made.derived.bounds())?;
        let choice = Choice {
            model: PackageModel::PartFile(Box::new(PartFileModel {
                model: reference,
                values: made.values,
                derived: made.derived,
            })),
            placement,
            envelope_nm,
            provenance,
        };
        super::check_time(deadline)?;
        self.chosen.insert(index, choice.clone());
        Ok(Some(choice))
    }
}

fn shape_class(shape: &PadShape) -> u8 {
    match shape {
        PadShape::Rect { .. } | PadShape::RoundRect { .. } => 0,
        PadShape::Circle { .. } => 1,
        PadShape::Oblong { .. } => 2,
        PadShape::Custom { .. } => 3,
    }
}
/// Match numbered physical lands at quarter turns and a common translation.
/// Numbering only constrains model orientation; electrical meaning is untouched.
fn matches(imported: &[Pad], candidate: &[Pad]) -> Option<ModelPlacement> {
    if imported.len() != candidate.len() || imported.iter().any(|p| p.number.is_empty()) {
        return None;
    }
    let mut by_number = BTreeMap::new();
    for pad in candidate {
        if by_number.insert(pad.number.as_str(), pad).is_some() {
            return None;
        }
    }
    let mut imported_numbers = std::collections::BTreeSet::new();
    let mut pairs = Vec::new();
    for pad in imported {
        if !imported_numbers.insert(pad.number.as_str()) {
            return None;
        }
        let other = by_number.get(pad.number.as_str())?;
        if pad.kind != other.kind
            || pad.hole != other.hole
            || pad.side != other.side
            || shape_class(&pad.shape) != shape_class(&other.shape)
            || shape_class(&pad.shape) == 3
        {
            return None;
        }
        pairs.push((pad, *other));
    }
    pairs.sort_by(|a, b| a.0.number.cmp(&b.0.number));
    for angle in [0.0, 90.0, 180.0, 270.0] {
        let (c, s) = sparksmith_core::geometry::cos_sin(angle);
        let rotate = |p: [f64; 2]| [c * p[0] - s * p[1], s * p[0] + c * p[1]];
        let (first, other) = pairs.first()?;
        let a = first.position.to_mm_f64();
        let b = rotate(other.position.to_mm_f64());
        let offset = [a[0] - b[0], a[1] - b[1]];
        let valid = pairs.iter().all(|(a, b)| {
            let aa = a.position.to_mm_f64();
            let bb = rotate(b.position.to_mm_f64());
            let ae = a.shape.extent();
            let be = b.shape.extent();
            let turn =
                (a.rotation_mdeg.to_degrees_f64() - b.rotation_mdeg.to_degrees_f64() - angle)
                    .rem_euclid(180.0);
            let same = turn.min(180.0 - turn) <= 0.1
                && ae.iter().zip(be).all(|(x, y)| (x - y).abs() <= TOLERANCE);
            let quarter = (turn - 90.0).abs() <= 0.1
                && (ae[0] - be[1]).abs() <= TOLERANCE
                && (ae[1] - be[0]).abs() <= TOLERANCE;
            (same || quarter) && (0..2).all(|i| (aa[i] - bb[i] - offset[i]).abs() <= TOLERANCE)
        });
        if valid {
            return ModelPlacement::from_mm([offset[0], offset[1], 0.0], [0.0, 0.0, angle]).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::units::PointNm;
    use sparksmith_core::{PackageTerminalId, PadId};
    type Fallible = Result<(), Box<dyn std::error::Error>>;
    fn lands() -> Result<Vec<Pad>, sparksmith_core::Error> {
        [(-0.8, "1"), (0.8, "2")]
            .into_iter()
            .enumerate()
            .map(|(n, (x, number))| {
                Ok(Pad::smd_rect(
                    PadId(n as u32 + 1),
                    PackageTerminalId(n as u32 + 1),
                    number,
                    PointNm::from_mm_f64_rounded([x, 0.0])?,
                    PointNm::from_mm_f64_rounded([0.7, 0.8])?,
                ))
            })
            .collect()
    }
    #[test]
    fn exact_land_match_sets_model_pose_and_rejects_near_miss_and_duplicate_numbers() -> Fallible {
        let source = lands()?;
        let mut moved = source.clone();
        for pad in &mut moved {
            let p = pad.position.to_mm_f64();
            pad.position = PointNm::from_mm_f64_rounded([-p[1] + 3.0, p[0] + 4.0])?;
            pad.rotation_mdeg = sparksmith_core::units::Angle::from_degrees_f64_rounded(90.0)?;
        }
        let pose = matches(&moved, &source).ok_or("rotated match missing")?;
        assert_eq!(pose.translation(), [3.0, 4.0, 0.0]);
        assert_eq!(pose.rotation_deg(), [0.0, 0.0, 90.0]);
        let [x, y] = moved[1].position.to_mm_f64();
        moved[1].position = PointNm::from_mm_f64_rounded([x + 0.051, y])?;
        assert!(matches(&moved, &source).is_none());
        let mut duplicate = source.clone();
        duplicate[1].number = "1".into();
        assert!(matches(&duplicate, &source).is_none());
        Ok(())
    }
    #[test]
    fn shipped_chip_sot_soic_qfn_create_real_models_and_refuse_ambiguous_and_near_miss() -> Fallible
    {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        let mut presets = Presets::default();
        presets.load(deadline)?;
        let candidates = presets.candidates.as_ref().ok_or("No presets")?;
        let fixtures: Vec<_> = [
            "chip/resistor-0603",
            "tabbed/tabbed-sot-23-3",
            "gull-wing/gull-wing-soic-8-3.9",
            "no-lead/no-lead-qfn-12-3x3-0.5",
        ]
        .iter()
        .map(|name| {
            let c = candidates
                .iter()
                .find(|c| &c.name == name)
                .ok_or_else(|| format!("No {name} preset"))?;
            Ok((
                c.name.clone(),
                c.pads.clone(),
                c.generation
                    .component
                    .as_ref()
                    .map_or("U", |c| c.prefix.as_str())
                    .to_owned(),
            ))
        })
        .collect::<Result<_, String>>()?;
        let mut engine = Engine::demo()?;
        let mut models = BTreeMap::new();
        let chip = &fixtures[0];
        assert!(
            presets
                .match_model(&mut engine, &chip.1, "?", &mut models, deadline)?
                .is_none(),
            "identical resistor/capacitor lands without a unique hint must refuse"
        );
        for (name, pads, prefix) in fixtures {
            let choice = presets
                .match_model(&mut engine, &pads, &prefix, &mut models, deadline)?
                .ok_or_else(|| format!("No actual generated model for {name}"))?;
            assert!(choice.provenance.contains(&name));
            let PackageModel::PartFile(part) = &choice.model else {
                return Err("not a part file".into());
            };
            assert!(
                part.derived.bodies >= 3,
                "body plus distinct terminals: {name}"
            );
            assert!(part.derived.bounds()[1][2] > 0.0);
            assert!(
                part.derived.leads.is_empty(),
                "importer must not infer electrical bindings"
            );
            let repeated = presets
                .match_model(&mut engine, &pads, &prefix, &mut models, deadline)?
                .ok_or("cached model missing")?;
            assert_eq!(choice.model, repeated.model);
            let mut near = pads.clone();
            let [x, y] = near[1].position.to_mm_f64();
            near[1].position = PointNm::from_mm_f64_rounded([x + 0.051, y])?;
            assert!(
                presets
                    .match_model(&mut engine, &near, &prefix, &mut models, deadline)?
                    .is_none(),
                "51 micron near miss was accepted for {name}"
            );
        }
        assert_eq!(engine.project().library.models.len(), 4);
        assert!(
            engine
                .project()
                .library
                .models
                .iter()
                .all(|m| m.definition.provenance.redistribution == Redistribution::Forbidden)
        );
        Ok(())
    }
    /// Original fixture for a human/VM visual check; writes only when explicitly run.
    #[test]
    #[ignore = "creates the original visual fixture in SPARKSMITH_MODELS_FIXTURE_DIR"]
    fn write_original_models_visual_fixture() -> Fallible {
        use sparksmith_solidcraft::native::{BoxBody, Expr, Look, Native};
        let directory = std::env::var("SPARKSMITH_MODELS_FIXTURE_DIR")?;
        let directory = std::path::Path::new(&directory);
        std::fs::create_dir_all(directory)?;
        let context = sparksmith_core::library::standards::Context::fixture()?;
        let _context = context.enter();
        let mut vendor = Native::new("original two-colour device")?;
        for (name, corner, size, colour, metallic) in [
            (
                "body",
                [-2.0, -1.5, 0.3],
                [4.0, 3.0, 1.5],
                [35, 45, 75],
                0.0,
            ),
            (
                "contact1",
                [-2.8, -1.0, 0.0],
                [0.8, 2.0, 0.5],
                [185, 190, 195],
                0.8,
            ),
            (
                "contact2",
                [2.0, -1.0, 0.0],
                [0.8, 2.0, 0.5],
                [185, 190, 195],
                0.8,
            ),
        ] {
            vendor.solid_box(BoxBody {
                name,
                corner: corner.map(Expr::from),
                size: size.map(Expr::from),
            })?;
            vendor.look(
                &[name],
                &Look {
                    name,
                    srgb8: colour,
                    metallic,
                    roughness: 0.35,
                    ior: 1.5,
                },
            )?;
        }
        std::fs::write(
            directory.join("original-device.step"),
            vendor.step_bytes(
                &["body".into(), "contact1".into(), "contact2".into()],
                "original device",
            )?,
        )?;
        let mut presets = Presets::default();
        presets.load(std::time::Instant::now() + std::time::Duration::from_secs(120))?;
        let candidates = presets.candidates.as_ref().ok_or("No candidates")?;
        let mut board = String::from(
            r#"(kicad_pcb (version 20240108) (generator "original synthetic model check")
            (general (thickness 1.6)) (layers (0 "F.Cu" signal) (31 "B.Cu" signal))
            (gr_rect (start 0 0) (end 70 40) (stroke (width 0.1) (type default)) (layer "Edge.Cuts"))"#,
        );
        for (index, (name, reference)) in [
            ("chip/resistor-0603", "R1"),
            ("tabbed/tabbed-sot-23-3", "Q1"),
            ("gull-wing/gull-wing-soic-8-3.9", "U1"),
            ("no-lead/no-lead-qfn-12-3x3-0.5", "U2"),
        ]
        .into_iter()
        .enumerate()
        {
            let candidate = candidates
                .iter()
                .find(|c| c.name == name)
                .ok_or("Missing preset")?;
            board.push_str(&format!(
                r#"(footprint "Original:{name}" (layer "F.Cu") (at {} 12)
                (property "Reference" "{reference}")"#,
                10 + index * 15
            ));
            for pad in &candidate.pads {
                let [x, y] = pad.position.to_mm_f64();
                let y = -y; // interchange board coordinates point downward
                let [sx, sy] = pad.shape.extent();
                let angle = pad.rotation_mdeg.to_degrees_f64();
                board.push_str(&format!(
                    r#"(pad "{}" smd rect (at {x} {y} {angle}) (size {sx} {sy})
                    (layers "F.Cu" "F.Mask" "F.Paste"))"#,
                    pad.number
                ));
            }
            board.push_str(r#"(model "${KICAD9_3DMODEL_DIR}/never-read.step"))"#);
        }
        for (reference, side, x, angle) in [("U3", "F", 20, 25), ("U4", "B", 50, -30)] {
            board.push_str(&format!(r#"(footprint "Original:Vendor" (layer "{side}.Cu") (at {x} 28 {angle})
                (property "Reference" "{reference}")
                (pad "1" smd rect (at -2.4 0 {angle}) (size 1 2) (layers "{side}.Cu" "{side}.Mask" "{side}.Paste"))
                (pad "2" smd rect (at 2.4 0 {angle}) (size 1 2) (layers "{side}.Cu" "{side}.Mask" "{side}.Paste"))
                (model "${{KIPRJMOD}}/original-device.step" (offset (xyz 0 0 0)) (scale (xyz 1 1 1)) (rotate (xyz 0 0 0))))"#));
        }
        board.push(')');
        let path = directory.join("original-models.kicad_pcb");
        std::fs::write(&path, board)?;
        let started = std::time::Instant::now();
        let imported = crate::kicad_import::import(&path, None)?;
        eprintln!(
            "Original six-instance fixture imported in {:?}",
            started.elapsed()
        );
        assert_eq!(
            imported
                .report
                .count(sparksmith_kicad::report::Case::VendorModel),
            2
        );
        assert_eq!(
            imported
                .report
                .count(sparksmith_kicad::report::Case::LibraryModelGenerated),
            4
        );
        assert_eq!(
            imported
                .report
                .count(sparksmith_kicad::report::Case::NoModel),
            0
        );
        sparksmith_io::write_project(
            &directory.join("original-models.sparksmith"),
            &imported.engine.to_file(),
        )?;
        std::fs::write(
            directory.join("report.json"),
            serde_json::to_vec_pretty(&imported.report)?,
        )?;
        Ok(())
    }
}
