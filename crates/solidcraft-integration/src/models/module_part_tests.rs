//! Phase C parity of the shipped `module` generator with the in-code
//! module family it replaces (docs/design/part-generation-plan.md, "Parity
//! bars"). The module family has no starter rows: the forms are its custom
//! entry (`ModuleForm::typical`), the ESP32-S31-WROOM-3 package of the
//! example projects, the builder's own LGA test module and variants that
//! reach every slot, the parking and both part files. For each: the
//! footprint (keep-out included), symbol, binding and package the module
//! makes (expanded as `library.generate` stores them) against
//! `library.new_from_template` for the same form, and its part file,
//! evaluated at the module's values, against the in-code module builder.
//! The WROOM-3 is also measured against the example projects' own part
//! file (`library/parts/esp32-s31-wroom-3.solidcraft`). Deleted with the
//! builder.
use super::part_file::{self, PartValues};
use super::*;
use serde_json::Value;
use sparksmith_core::ids::{
    BindingRef, DefinitionId, FootprintRef, ModelRef, PackageRef, Revision, SymbolRef,
};
use sparksmith_core::library::generated::expand::{self, KeyIds, Refs};
use sparksmith_core::library::marking::surface;
use sparksmith_core::library::package_form::module::{
    AntennaKeepout, GroundPads, ModuleForm, ModuleSide, PadRow, SinglePad,
};
use sparksmith_core::library::package_model::{PartDerived, PartFileModel};
use sparksmith_core::library::samples::SampleRefs;
use sparksmith_core::library::symbol_style::SymbolConventions;
use sparksmith_core::library::templates::catalog::{
    FamilyKind, NewFromTemplate, RowSource, SkeletonHint, new_from_template_with,
};
use sparksmith_core::library::{Definition, Library};
use sparksmith_core::units::{Angle, Nm, PointNm};
use sparksmith_generators::abi::v1::{Generation, Model, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};

/// The shipped module (test code may read it).
const MODULE: &[u8] = include_bytes!("../../../../library/generators/module/module.wasm");
/// Its part files, as shipped.
const PARTS: [&[u8]; 4] = [
    include_bytes!("../../../../library/generators/module/parts/module.solidcraft"),
    include_bytes!("../../../../library/generators/module/parts/module-diagonal.solidcraft"),
    include_bytes!("../../../../library/generators/module/parts/module-castellated.solidcraft"),
    include_bytes!(
        "../../../../library/generators/module/parts/module-castellated-diagonal.solidcraft"
    ),
];
/// The example projects' WROOM-3 part file (library data, as drawn).
const WROOM3_FILE: &[u8] = include_bytes!("../../../../library/parts/esp32-s31-wroom-3.solidcraft");

/// 1 µm and 1 nm, in mm.
const UM: f64 = 1e-3;
const NM: f64 = 1e-6;

fn fail(f: Failure) -> Error {
    Error::Verification(f.to_string())
}

fn v(e: impl std::fmt::Display) -> Error {
    Error::Verification(e.to_string())
}

fn nm(mm: f64) -> Nm {
    Nm::from_mm_f64_rounded(mm).unwrap_or_default()
}

fn pt(p: [f64; 2]) -> PointNm {
    PointNm::from_mm_f64_rounded(p).unwrap_or_default()
}

/// The bundled part file of SHA-256 `asset`.
fn part(asset: &str) -> Result<&'static [u8]> {
    let hash = sparksmith_core::ids::ContentHash::parse(asset);
    PARTS
        .into_iter()
        .find(|bytes| Some(sparksmith_core::library::bundle::asset_hash(bytes)) == hash)
        .ok_or_else(|| v(format!("no bundled part file sha256 {asset}")))
}

/// The module's generation for `form` in `scope`, through the mapping the
/// board uses.
fn generation_of(form: &PackageForm, scope: Scope) -> Result<Generation> {
    let (_, params) = family::generator(form).ok_or_else(|| v("no generator"))?;
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(MODULE).map_err(fail)?;
    let request = complete(&description, scope, &params).map_err(fail)?;
    Ok(runtime.generate(MODULE, &request).map_err(fail)?.generation)
}

fn refs() -> SampleRefs {
    SampleRefs {
        component: DefinitionId(1),
        symbol: SymbolRef::new(DefinitionId(2), Revision::FIRST),
        package: PackageRef::new(DefinitionId(3), Revision::FIRST),
        footprint: FootprintRef::new(DefinitionId(4), Revision::FIRST),
        binding: BindingRef::new(DefinitionId(5), Revision::FIRST),
    }
}

/// The five definitions by kind.
type Five = BTreeMap<&'static str, Definition>;

fn kind(d: &Definition) -> &'static str {
    match d {
        Definition::Component(_) => "component",
        Definition::Symbol(_) => "symbol",
        Definition::Package(_) => "package",
        Definition::Footprint(_) => "footprint",
        Definition::Binding(_) => "binding",
        _ => "other",
    }
}

/// What `library.new_from_template` makes of the form (the family's
/// custom entry with these values, the IC-box skeleton).
fn in_code(form: &PackageForm) -> Result<Five> {
    let request = NewFromTemplate {
        row: RowSource::Custom {
            family: FamilyKind::Module,
        },
        values: Some(form.clone()),
        skeleton: SkeletonHint::IcBox,
        name: "module".into(),
        designator_prefix: None,
        symbol: None,
        without_fields: false,
    };
    let edits = new_from_template_with(
        &Library::default(),
        &request,
        refs(),
        &sparksmith_core::rules::DesignRules::default(),
        SymbolConventions::default(),
    )
    .map_err(v)?;
    Ok(edits
        .into_iter()
        .map(|e| (kind(&e.definition), e.definition))
        .collect())
}

/// What `library.generate` stores of the module's generation.
fn generated(g: &Generation) -> Result<Five> {
    let Some(Model::PartFile { asset, values }) = &g.model else {
        return Err(v("no model"));
    };
    let as_is = part_file::evaluate(part(asset)?, &PartValues::new())?;
    let values = part_file::generator_values(&as_is, values).map_err(v)?;
    let r = refs();
    let model = PackageModel::PartFile(Box::new(PartFileModel {
        model: ModelRef::new(DefinitionId(6), Revision::FIRST),
        values,
        derived: PartDerived::default(),
    }));
    let (made, _) = expand::definitions(
        g,
        Refs {
            component: Some((r.component, r.symbol, r.binding)),
            package: r.package,
            footprint: r.footprint,
        },
        model,
        &KeyIds::default(),
    )
    .map_err(v)?;
    Ok(made.into_iter().map(|(_, d)| (kind(&d), d)).collect())
}

/// The worst numeric difference between two JSON values of the same shape
/// (lengths in nm), or `None` when their shapes or other values differ.
fn diff(a: &Value, b: &Value) -> Option<f64> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => Some((x.as_f64()? - y.as_f64()?).abs()),
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x
            .iter()
            .zip(y)
            .map(|(p, q)| diff(p, q))
            .try_fold(0.0, |w, d| Some(f64::max(w, d?))),
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => x
            .iter()
            .map(|(k, p)| diff(p, y.get(k)?))
            .try_fold(0.0, |w, d| Some(f64::max(w, d?))),
        _ => (a == b).then_some(0.0),
    }
}

/// Match every item of `a` with one of `b` within 1 nm (greedy); the worst
/// difference, or the first unmatched item.
fn matched(what: &str, a: &[Value], b: &[Value]) -> Result<f64> {
    if a.len() != b.len() {
        return Err(v(format!(
            "{what}: {} items in code, {} generated: {}  /  {}",
            a.len(),
            b.len(),
            serde_json::to_string(a)?,
            serde_json::to_string(b)?
        )));
    }
    let mut free: Vec<&Value> = b.iter().collect();
    let mut worst = 0.0f64;
    for item in a {
        let best = free
            .iter()
            .enumerate()
            .filter_map(|(i, other)| diff(item, other).map(|d| (i, d)))
            .min_by(|x, y| x.1.total_cmp(&y.1));
        match best {
            Some((i, d)) if d <= 1.0 => {
                worst = worst.max(d);
                free.remove(i);
            }
            _ => {
                return Err(v(format!(
                    "{what}: no generated item matches {item}; left: {}",
                    serde_json::to_string(&free)?
                )));
            }
        }
    }
    Ok(worst)
}

fn json(value: &impl serde::Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}

/// A footprint graphic without its id, by layer.
fn graphics(fp: &Definition, layer: &str) -> Result<Vec<Value>> {
    let Definition::Footprint(fp) = fp else {
        return Err(v("not a footprint"));
    };
    let mut out = Vec::new();
    for g in &fp.graphics {
        let mut j = json(g)?;
        if j.get("layer").and_then(Value::as_str) != Some(layer) {
            continue;
        }
        if let Some(m) = j.as_object_mut() {
            m.remove("id");
        }
        out.push(j);
    }
    Ok(out)
}

/// The footprint bars; the worst pad, drawing and keep-out differences, nm.
fn footprints(code: &Five, made: &Five) -> Result<(f64, f64, f64)> {
    let (Some(Definition::Footprint(a)), Some(Definition::Footprint(b))) =
        (code.get("footprint"), made.get("footprint"))
    else {
        return Err(v("no footprints"));
    };
    assert_eq!(a.pads.len(), b.pads.len(), "pads");
    let mut pad_worst = 0.0f64;
    for (p, q) in a.pads.iter().zip(&b.pads) {
        // Ids and terminal ids both run 1…n in lead order.
        let (x, y) = (json(p)?, json(q)?);
        let d = diff(&x, &y)
            .ok_or_else(|| v(format!("pad {}: {x} in code, {y} generated", p.number)))?;
        pad_worst = pad_worst.max(d);
    }
    let fp = |five: &Five| five.get("footprint").cloned().ok_or_else(|| v("fp"));
    let (fa, fb) = (fp(code)?, fp(made)?);
    let silk = matched(
        "silkscreen",
        &graphics(&fa, "silkscreen")?,
        &graphics(&fb, "silkscreen")?,
    )?;
    let assembly = matched(
        "assembly",
        &graphics(&fa, "assembly")?,
        &graphics(&fb, "assembly")?,
    )?;
    let bounds = |fp: &sparksmith_core::library::FootprintDefinition| {
        fp.graphics
            .iter()
            .filter(|g| g.layer == sparksmith_core::library::DrawingLayer::Courtyard)
            .filter_map(|g| match &g.item {
                sparksmith_core::library::Item::Shape { shape, .. } => {
                    Some(shape.to_mm().bounds(0.0))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let (ca, cb) = (bounds(a), bounds(b));
    assert_eq!((ca.len(), cb.len()), (1, 1), "one courtyard each");
    let courtyard = ca
        .iter()
        .zip(&cb)
        .flat_map(|(p, q)| {
            p.min
                .iter()
                .chain(&p.max)
                .zip(q.min.iter().chain(&q.max))
                .map(|(x, y)| (x - y).abs() / NM)
        })
        .fold(0.0, f64::max);
    // Keep-outs: name, scope, outline within 1 nm.
    assert_eq!(a.keepouts.len(), b.keepouts.len(), "keep-outs");
    let mut keepout = 0.0f64;
    for (p, q) in a.keepouts.iter().zip(&b.keepouts) {
        let (mut x, mut y) = (json(p)?, json(q)?);
        // Outlines in mm: compare in nm.
        for j in [&mut x, &mut y] {
            scale_numbers(j, 1e6);
        }
        let d = diff(&x, &y).ok_or_else(|| v(format!("keep-out: {x} in code, {y} generated")))?;
        keepout = keepout.max(d);
    }
    Ok((pad_worst, silk.max(assembly).max(courtyard), keepout))
}

fn scale_numbers(j: &mut Value, by: f64) {
    match j {
        Value::Number(n) => {
            if let Some(x) = n
                .as_f64()
                .and_then(|x| serde_json::Number::from_f64(x * by))
            {
                *n = x;
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| scale_numbers(x, by)),
        Value::Object(o) => o.values_mut().for_each(|x| scale_numbers(x, by)),
        _ => {}
    }
}

/// The symbol and binding bars: equal terminals (names, directions),
/// pins, units and drawings within 1 nm.
fn symbols(code: &Five, made: &Five) -> Result<f64> {
    let (Some(Definition::Component(a)), Some(Definition::Component(b))) =
        (code.get("component"), made.get("component"))
    else {
        return Err(v("no components"));
    };
    assert_eq!(json(&a.terminals)?, json(&b.terminals)?, "terminals");
    assert_eq!(a.designator_prefix, b.designator_prefix);
    let (Some(Definition::Binding(a)), Some(Definition::Binding(b))) =
        (code.get("binding"), made.get("binding"))
    else {
        return Err(v("no bindings"));
    };
    assert_eq!(a.mappings, b.mappings, "binding");
    let (Some(Definition::Symbol(a)), Some(Definition::Symbol(b))) =
        (code.get("symbol"), made.get("symbol"))
    else {
        return Err(v("no symbols"));
    };
    assert_eq!(a.units.len(), b.units.len());
    let mut worst = 0.0f64;
    for (p, q) in a.units.iter().zip(&b.units) {
        assert_eq!(p.label, q.label);
        let pins = |u: &sparksmith_core::library::Unit| -> Result<Vec<Value>> {
            u.pins.iter().map(json).collect()
        };
        worst = worst.max(matched("pins", &pins(p)?, &pins(q)?)?);
        let drawn = |u: &sparksmith_core::library::Unit| -> Result<Vec<Value>> {
            u.graphics.iter().map(|g| json(&g.item)).collect()
        };
        worst = worst.max(matched("symbol drawings", &drawn(p)?, &drawn(q)?)?);
    }
    Ok(worst)
}

/// One measured body: name, role, material, terminal, bounds, volume (mm).
type Body = (String, BodyRole, MaterialId, Option<u32>, [f64; 6], f64);

fn measure(state: &solidcraft_engine::doc::ModelState, name: &str) -> Result<([f64; 6], f64)> {
    let body = state
        .body(name)
        .ok_or_else(|| v(format!("no body {name}")))?;
    let m = body.measure().map_err(v)?;
    let (lo, hi) = (m.bbox.min, m.bbox.max);
    Ok(([lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], m.volume))
}

/// The in-code builder's package for `form`, its terminals the form's
/// numbers in lead order.
fn package_of(form: &PackageForm) -> PackageDefinition {
    let n = sparksmith_core::library::samples::form_terminal_count(form);
    let names =
        sparksmith_core::library::samples::form_designations(form, usize::try_from(n).unwrap_or(0));
    PackageDefinition {
        next_local: n + 1,
        terminals: names
            .into_iter()
            .zip(1..)
            .map(
                |(designation, i)| sparksmith_core::library::PackageTerminal {
                    id: sparksmith_core::PackageTerminalId(i),
                    designation,
                },
            )
            .collect(),
        model: PackageModel::Generated {
            family_rev: 1,
            materials: GeneratedMaterials::default(),
        },
        form: form.clone(),
        ..PackageDefinition::default()
    }
}

/// Bodies sorted by what they are and where: terminal (bound or not),
/// then centre to the nanometre, then size.
fn sorted(mut m: Vec<Body>) -> Vec<Body> {
    let key = |b: &Body| {
        let c = |i: usize| ((b.4[i] + b.4[i + 3]) / 2.0 / NM).round() as i64;
        let s = |i: usize| ((b.4[i + 3] - b.4[i]) / NM).round() as i64;
        (b.3, c(0), c(1), c(2), s(0), s(1))
    };
    m.sort_by_key(key);
    m
}

/// The builder's name of a body the part file names otherwise: its single
/// pads are `single pad k` where the builder said `pad <number>`.
fn comparable(name: &str) -> String {
    if name.starts_with("single pad ") || (name.starts_with("pad ") && !name.contains('(')) {
        "single pad".into()
    } else {
        name.into()
    }
}

/// What the 3D bars measured for one form.
#[derive(Default)]
struct Measured3d {
    bounds: f64,
    volume: f64,
    file_ms: f64,
    /// The part file's own cold evaluation at the module's values (kept
    /// with a cached evaluation).
    part_ms: f64,
    code_ms: f64,
    bodies: usize,
}

/// The 3D bars: the generator's model (the bridge the board builds for a
/// stored module package) against the builder: the same bodies (names
/// but the single pads', roles, materials, terminals), bounds within 1 µm,
/// volume within surface × 1 µm; the marking surfaces the module form's
/// anchors; every terminal bound, each bound body over a land of its
/// terminal.
fn models(name: &str, form: &PackageForm, names: bool) -> Result<Measured3d> {
    let package = package_of(form);
    let started = std::time::Instant::now();
    let (part, bodies) = family::evaluate(&package)?;
    let file_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut file: Vec<Body> = Vec::new();
    for b in &bodies {
        let (bounds, volume) = measure(part.state(), &b.name)?;
        file.push((
            b.name.clone(),
            b.role,
            b.material,
            b.terminal,
            bounds,
            volume,
        ));
    }
    let started = std::time::Instant::now();
    let mut native = Native::new("parity")?;
    let built = build_standalone(&mut native, &package)?;
    let code_ms = started.elapsed().as_secs_f64() * 1e3;
    let state = native.session().model.state();
    let mut code: Vec<Body> = Vec::new();
    for b in &built {
        let (bounds, volume) = measure(&state, &b.name)?;
        code.push((
            b.name.clone(),
            b.role,
            b.material,
            b.terminal,
            bounds,
            volume,
        ));
    }
    let (file, code) = (sorted(file), sorted(code));
    let ids = |m: &[Body]| -> Vec<(String, BodyRole, MaterialId, Option<u32>)> {
        m.iter()
            .map(|(n, r, mat, t, _, _)| {
                let n = if names { comparable(n) } else { String::new() };
                (n, *r, *mat, *t)
            })
            .collect()
    };
    let (fi, ci) = (ids(&file), ids(&code));
    if fi != ci {
        for (k, (a, b)) in fi.iter().zip(&ci).enumerate() {
            if a != b {
                eprintln!("{name}: body {k}: generator {a:?}, builder {b:?}");
            }
        }
    }
    assert_eq!(fi.len(), ci.len(), "{name}: body count");
    assert_eq!(fi, ci, "{name}: bodies");
    let mut out = Measured3d {
        file_ms,
        part_ms: part.evaluate_ms,
        code_ms,
        bodies: file.len(),
        ..Measured3d::default()
    };
    for (a, b) in file.iter().zip(&code) {
        let (body, va, vb) = (&a.0, a.5, b.5);
        let off =
            a.4.iter()
                .zip(&b.4)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0, f64::max);
        let k = &b.4;
        let [x, y, z] = [k[3] - k[0], k[4] - k[1], k[5] - k[2]];
        let surface = 2.0 * (x * y + y * z + z * x);
        let dv = (va - vb).abs();
        assert!(off < UM, "{name} {body}: bounds differ by {off} mm");
        assert!(
            dv < surface * UM,
            "{name} {body}: volume {va} mm³ against {vb} mm³"
        );
        out.bounds = out.bounds.max(off);
        out.volume = out.volume.max(dv / vb.max(f64::MIN_POSITIVE));
    }
    // The marking surfaces are the form's anchors.
    let anchors = surface::anchors(form);
    assert!(!anchors.is_empty());
    for a in &anchors {
        let found = part
            .surfaces
            .iter()
            .find(|s| s.name == a.name)
            .ok_or_else(|| v(format!("{name}: no surface {}", a.name)))?;
        let close = |p: &[f64], q: &[f64]| p.iter().zip(q).all(|(u, w)| (u - w).abs() < 1e-9);
        match (a.geometry, found.geometry) {
            (
                sparksmith_core::library::marking::SurfaceGeometry::Plane(x),
                sparksmith_core::library::marking::SurfaceGeometry::Plane(y),
            ) => assert!(
                close(&x.origin(), &y.origin())
                    && close(&x.normal, &y.normal)
                    && close(&x.u_axis, &y.u_axis)
                    && close(&x.size(), &y.size()),
                "{name} {}: form {x:?}, part {y:?}",
                a.name
            ),
            _ => return Err(v(format!("{name}: {} is not a plane", a.name))),
        }
    }
    // Every terminal bound; each bound body over a land of its terminal.
    let g = generation_of(form, Scope::Package)?;
    let terminals: std::collections::BTreeSet<u32> =
        bodies.iter().filter_map(|b| b.terminal).collect();
    assert_eq!(
        terminals.len(),
        package.terminals.len(),
        "{name}: every terminal bound"
    );
    for b in &bodies {
        let Some(t) = b.terminal else { continue };
        let (bounds, _) = measure(part.state(), &b.name)?;
        let key = usize::try_from(t)
            .ok()
            .and_then(|t| g.package.terminals.get(t - 1))
            .ok_or_else(|| v(format!("{name}: no terminal {t}")))?;
        let over = g
            .footprint
            .pads
            .iter()
            .filter(|p| &p.terminal == key)
            .any(|p| {
                let (x, y) = (p.at[0] as f64 * NM, p.at[1] as f64 * NM);
                // The land's centre within the body's box, or the body within
                // the land's (a castellation's plating beside its pad).
                let reach = match &p.shape {
                    sparksmith_generators::abi::v1::PadShape::Rect { size } => {
                        size[0].max(size[1]) as f64 * NM
                    }
                    _ => 0.0,
                };
                bounds[0] - reach <= x
                    && x <= bounds[3] + reach
                    && bounds[1] - reach <= y
                    && y <= bounds[4] + reach
            });
        assert!(over, "{name}: {} bound to {t} lies off its lands", b.name);
    }
    Ok(out)
}

/// The example projects' WROOM-3 package (the module datasheet's
/// millimetres: the wire form).
fn wroom3() -> Result<PackageForm> {
    Ok(
        serde_json::from_str::<sparksmith_core::library::package_form::PackageFormParams>(
            include_str!("../../tests/template_spike_wroom3.json"),
        )?
        .into(),
    )
}

/// The builder's own synthetic LGA module: 16 × 20 mm, 3 mm high, pads on
/// four sides at 1.0 mm, a 2 × 2 ground array turned 45°, two corner pads.
fn lga() -> ModuleForm {
    ModuleForm {
        width_nm: nm(16.0),
        length_nm: nm(20.0),
        height_nm: nm(3.0),
        pcb_thickness_nm: nm(0.8),
        antenna_length_nm: nm(5.0),
        can_size: pt([15.0, 14.0]),
        can_centre: pt([0.0, -2.5]),
        pitch_nm: nm(1.0),
        pad_size_nm: [nm(0.8), nm(0.5)],
        rows: vec![
            PadRow {
                side: ModuleSide::Left,
                count: 10,
                first: pt([-7.4, 3.5]),
            },
            PadRow {
                side: ModuleSide::Bottom,
                count: 8,
                first: pt([-3.5, -9.4]),
            },
            PadRow {
                side: ModuleSide::Right,
                count: 10,
                first: pt([7.4, -5.5]),
            },
            PadRow {
                side: ModuleSide::Top,
                count: 8,
                first: pt([3.5, 4.4]),
            },
        ],
        castellation_nm: nm(0.0),
        land_width_nm: nm(0.5),
        land_extend_nm: [nm(0.0), nm(0.0)],
        ground: Some(GroundPads {
            rows: 2,
            cols: 2,
            pitch: pt([2.0, 2.0]),
            size: pt([1.2, 1.2]),
            centre: pt([0.0, -2.5]),
            rotation_mdeg: Angle::new(45_000).unwrap_or_default(),
            number: "37".into(),
        }),
        singles: vec![
            SinglePad {
                number: "38".into(),
                centre: pt([-7.4, -9.4]),
                size: pt([0.8, 0.8]),
            },
            SinglePad {
                number: "39".into(),
                centre: pt([7.4, -9.4]),
                size: pt([0.8, 0.8]),
            },
        ],
        keepout: Some(AntennaKeepout {
            beyond_nm: [nm(0.0), nm(0.0), nm(0.0)],
        }),
    }
}

/// The forms measured, by name, and whether the part file names their
/// bodies as the builder did (a row in another slot than its place in the
/// form, and a ground array turned a quarter, are named otherwise).
fn forms() -> Result<Vec<(&'static str, PackageForm, bool)>> {
    let typical = ModuleForm::typical();
    // Without an antenna (the can centred), without ground or keep-out.
    let mut bare = typical.clone();
    bare.antenna_length_nm = Nm::ZERO;
    bare.can_centre = pt([0.0, 0.0]);
    bare.ground = None;
    bare.keepout = None;
    // A second (inner) left row, a 2 × 3 ground array turned a quarter,
    // a single pad, a grown keep-out.
    let mut busy = typical.clone();
    busy.rows.push(PadRow {
        side: ModuleSide::Left,
        count: 4,
        first: pt([-6.5, 4.0]),
    });
    if let Some(g) = &mut busy.ground {
        g.rows = 2;
        g.cols = 3;
        g.rotation_mdeg = Angle::new(90_000).unwrap_or_default();
        g.number = "60".into();
    }
    busy.singles.push(SinglePad {
        number: "50".into(),
        centre: pt([7.0, 10.0]),
        size: pt([1.0, 1.0]),
    });
    busy.keepout = Some(AntennaKeepout {
        beyond_nm: [nm(1.0), nm(2.0), nm(3.0)],
    });
    // The WROOM-3 with its ground array turned 135°.
    let PackageForm::Module(wroom) = wroom3()? else {
        return Err(v("not a module"));
    };
    let mut turned = (*wroom).clone();
    if let Some(g) = &mut turned.ground {
        g.rotation_mdeg = Angle::new(135_000).unwrap_or_default();
    }
    let module = |m: ModuleForm| PackageForm::Module(Box::new(m));
    Ok(vec![
        ("typical", module(typical), true),
        ("ESP32-S31-WROOM-3", module(*wroom), true),
        ("builder's LGA module", module(lga()), true),
        ("typical without antenna or ground", module(bare), true),
        (
            "typical with an inner row, a turned 2 × 3 array, a single pad",
            module(busy),
            false,
        ),
        ("WROOM-3, ground turned 135°", module(turned), false),
    ])
}

#[test]
fn the_module_generator_equals_the_in_code_module_family() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let (mut pads, mut drawings, mut symbol, mut keepout) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut bounds, mut volume) = (0.0f64, 0.0f64);
    for (name, form, names) in forms()? {
        let g = generation_of(&form, Scope::Component)?;
        let made = generated(&g)?;
        let code = in_code(&form)?;
        let (Some(Definition::Package(a)), Some(Definition::Package(b))) =
            (code.get("package"), made.get("package"))
        else {
            return Err(v("no packages"));
        };
        assert_eq!(json(&a.terminals)?, json(&b.terminals)?, "{name}");
        assert_eq!(a.envelope_nm, b.envelope_nm, "{name}: envelope");
        let (p, d, k) = footprints(&code, &made).map_err(|e| v(format!("{name}: {e}")))?;
        pads = pads.max(p);
        drawings = drawings.max(d);
        keepout = keepout.max(k);
        symbol = symbol.max(symbols(&code, &made).map_err(|e| v(format!("{name}: {e}")))?);
        let m = models(name, &form, names)?;
        println!(
            "  {name}: {} bodies, generator {:.0} ms (part file cold at its values {:.0} ms), \
             builder {:.0} ms",
            m.bodies, m.file_ms, m.part_ms, m.code_ms
        );
        bounds = bounds.max(m.bounds);
        volume = volume.max(m.volume);
    }
    println!(
        "module parity: pads within {pads} nm, drawings within {drawings} nm, keep-outs within \
         {keepout} nm, symbols within {symbol} nm; 3D bounds within {bounds:.3e} mm, volumes \
         within {volume:.3e} relative"
    );
    assert!(pads <= 1.0 && drawings <= 1.0 && symbol <= 1.0 && keepout <= 1.0);
    Ok(())
}

/// The generator's WROOM-3 against the example projects' own part file
/// (`library/parts/esp32-s31-wroom-3.solidcraft`, as drawn): the same 110
/// bodies but the corner pads' names, bounds within 1 µm, volumes within
/// surface × 1 µm.
#[test]
fn the_generated_wroom3_builds_what_the_examples_part_file_draws() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let form = wroom3()?;
    let (part, bodies) = family::evaluate(&package_of(&form))?;
    let drawn = part_file::evaluate(WROOM3_FILE, &PartValues::new())?;
    assert_eq!(bodies.len(), drawn.bodies.len());
    assert_eq!(bodies.len(), 110);
    let row = |p: &part_file::PartBody| -> Result<Body> {
        let (bounds, volume) = measure(p_state(p, &part, &drawn), &p.name)?;
        Ok((
            p.name.clone(),
            part_file::role_of(p.material),
            p.material.unwrap_or(MaterialId::EpoxyMould),
            None,
            bounds,
            volume,
        ))
    };
    let a = sorted(part.bodies.iter().map(row).collect::<Result<_>>()?);
    let b = sorted(drawn.bodies.iter().map(row).collect::<Result<_>>()?);
    let (mut worst_box, mut worst_volume) = (0.0f64, 0.0f64);
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(comparable(&x.0), comparable(&y.0));
        assert_eq!((x.1, x.2), (y.1, y.2), "{}", x.0);
        let off =
            x.4.iter()
                .zip(&y.4)
                .map(|(p, q)| (p - q).abs())
                .fold(0.0, f64::max);
        worst_box = worst_box.max(off);
        worst_volume = worst_volume.max((x.5 - y.5).abs() / y.5);
    }
    println!(
        "WROOM-3, generator against the example part file: {} bodies, worst bbox Δ \
         {worst_box:.2e} mm, worst volume Δ {worst_volume:.2e} relative",
        a.len()
    );
    assert!(worst_box < UM && worst_volume < 1e-6);
    Ok(())
}

/// The evaluated state a part body belongs to.
fn p_state<'a>(
    p: &part_file::PartBody,
    a: &'a part_file::EvaluatedPart,
    b: &'a part_file::EvaluatedPart,
) -> &'a solidcraft_engine::doc::ModelState {
    if a.bodies.iter().any(|x| std::ptr::eq(x, p)) {
        a.state()
    } else {
        b.state()
    }
}
