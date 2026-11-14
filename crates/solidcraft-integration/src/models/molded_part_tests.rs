//! Phase C parity of the shipped `molded` generator with the in-code
//! molded family it replaces, over every molded starter row
//! (docs/design/part-generation-plan.md, "Parity bars"): the footprint,
//! symbol, binding and package the module makes (expanded as
//! `library.generate` stores them) against `library.new_from_template` for
//! the same row, its part file's marking area against the form's, and the
//! part file, evaluated at the module's values, against the in-code molded
//! builder. Deleted with the builder.
use super::part_file::{self, PartValues};
use super::*;
use serde_json::Value;
use sparksmith_core::ids::{
    BindingRef, DefinitionId, FootprintRef, ModelRef, PackageRef, Revision, SymbolRef,
};
use sparksmith_core::library::generated::expand::{self, KeyIds, Refs};
use sparksmith_core::library::marking::surface::SurfaceGeometry;
use sparksmith_core::library::package_form::molded::MoldedLead;
use sparksmith_core::library::package_model::{PartDerived, PartFileModel};
use sparksmith_core::library::samples::SampleRefs;
use sparksmith_core::library::symbol_style::SymbolConventions;
use sparksmith_core::library::templates::catalog::{
    NewFromTemplate, RowSource, new_from_template_with,
};
use sparksmith_core::library::templates::starter;
use sparksmith_core::library::{Definition, Library};
use sparksmith_generators::abi::v1::{Generation, Model, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime, hex, sha256};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};
use std::sync::Arc;

/// The shipped module (test code may read it).
const MOLDED: &[u8] = include_bytes!("../../../../library/generators/molded/molded.wasm");
/// The bundle's part files, as shipped.
const PARTS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../library/generators/molded/parts"
);

/// 1 µm and 1 nm, in mm.
const UM: f64 = 1e-3;
const NM: f64 = 1e-6;

fn fail(f: Failure) -> Error {
    Error::Verification(f.to_string())
}

fn v(e: impl std::fmt::Display) -> Error {
    Error::Verification(e.to_string())
}

/// The bundle's part file of SHA-256 `asset`.
fn part_bytes(asset: &str) -> Result<Vec<u8>> {
    for entry in std::fs::read_dir(PARTS).map_err(v)? {
        let bytes = std::fs::read(entry.map_err(v)?.path()).map_err(v)?;
        if hex(&sha256(&bytes)) == asset {
            return Ok(bytes);
        }
    }
    Err(v(format!("no part file sha256 {asset}")))
}

/// The module's generation for starter row `name` (its preset of the same
/// name), in `scope`.
fn generation(name: &str, scope: Scope) -> Result<Generation> {
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(MOLDED).map_err(fail)?;
    let preset = description
        .presets()
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| v(format!("no preset {name}")))?;
    let request = complete(&description, scope, &preset.params).map_err(fail)?;
    Ok(runtime.generate(MOLDED, &request).map_err(fail)?.generation)
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

/// What `library.new_from_template` makes of the row.
fn in_code(row: &starter::StarterRow) -> Result<Five> {
    let request = NewFromTemplate {
        row: RowSource::Starter {
            name: row.name.clone(),
        },
        values: None,
        skeleton: row.definition.skeleton,
        name: row.name.clone(),
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

/// What `library.generate` stores of the module's generation, and the
/// part file evaluated at its values.
fn generated(g: &Generation) -> Result<(Five, Arc<part_file::EvaluatedPart>)> {
    let Some(Model::PartFile { asset, values }) = &g.model else {
        return Err(v("no model"));
    };
    let bytes = part_bytes(asset)?;
    let as_is = part_file::evaluate(&bytes, &PartValues::new())?;
    let values = part_file::generator_values(&as_is, values).map_err(v)?;
    let part = part_file::evaluate(&bytes, &values)?;
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
    Ok((made.into_iter().map(|(_, d)| (kind(&d), d)).collect(), part))
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
            "{what}: {} items in code, {} generated",
            a.len(),
            b.len()
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
fn graphics(fp: &sparksmith_core::library::FootprintDefinition, layer: &str) -> Result<Vec<Value>> {
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

/// A pad as JSON with a rounded rectangle's corner as the radius it draws
/// (the template stores a quarter of the short side as a ratio, the
/// generation the radius it computed).
fn pad_json(p: &sparksmith_core::library::Pad) -> Result<Value> {
    use sparksmith_core::library::PadShape;
    let mut j = json(p)?;
    if let PadShape::RoundRect {
        size,
        radius_ratio,
        max_radius_nm,
        ..
    } = &p.shape
    {
        let short = size.x_nm.get().min(size.y_nm.get()) as f64;
        let mut radius = (radius_ratio * short).round();
        if let Some(max) = max_radius_nm {
            radius = radius.min(max.get() as f64);
        }
        if let Some(m) = j.as_object_mut() {
            m.insert(
                "shape".into(),
                serde_json::json!({"round_rect": {"size": json(size)?, "radius_nm": radius}}),
            );
        }
    }
    Ok(j)
}

/// The footprint bars; the worst pad and drawing differences, nm.
fn footprints(code: &Five, made: &Five) -> Result<(f64, f64)> {
    let (Some(Definition::Footprint(a)), Some(Definition::Footprint(b))) =
        (code.get("footprint"), made.get("footprint"))
    else {
        return Err(v("no footprints"));
    };
    assert_eq!((a.pads.len(), b.pads.len()), (2, 2));
    let mut pad_worst = 0.0f64;
    for (p, q) in a.pads.iter().zip(&b.pads) {
        let (x, y) = (pad_json(p)?, pad_json(q)?);
        let d = diff(&x, &y)
            .ok_or_else(|| v(format!("pad {}: {x} in code, {y} generated", p.number)))?;
        pad_worst = pad_worst.max(d);
    }
    let silk = matched(
        "silkscreen",
        &graphics(a, "silkscreen")?,
        &graphics(b, "silkscreen")?,
    )?;
    let assembly = matched(
        "assembly",
        &graphics(a, "assembly")?,
        &graphics(b, "assembly")?,
    )?;
    // The courtyard: a rectangle in code, its four corners generated.
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
    Ok((pad_worst, silk.max(assembly).max(courtyard)))
}

/// The symbol and binding bars: equal terminals (names, directions),
/// prefix, binding, pins, units and drawings within 1 nm.
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

/// The part file's `marking-top` against the form's marking area; the
/// worst difference, nm.
fn marking(row: &starter::StarterRow, part: &part_file::EvaluatedPart) -> Result<f64> {
    let PackageForm::Molded(form) = &row.definition.values else {
        return Err(v("not a molded row"));
    };
    let area = form
        .marking_area()
        .ok_or_else(|| v(format!("{}: no marking area", row.name)))?;
    let top = part
        .surfaces
        .iter()
        .find(|s| s.name == "top")
        .ok_or_else(|| v(format!("{}: no marking-top", row.name)))?;
    let SurfaceGeometry::Plane(plane) = &top.geometry else {
        return Err(v("marking-top is not a plane"));
    };
    diff(&json(&area)?, &json(plane)?)
        .ok_or_else(|| v(format!("{}: {area:?} against {plane:?}", row.name)))
}

/// Each body: name, role, material, bounds and volume (mm).
type Measured = Vec<(String, BodyRole, MaterialId, [f64; 6], f64)>;

fn measure(state: &solidcraft_engine::doc::ModelState, name: &str) -> Result<([f64; 6], f64)> {
    let body = state
        .body(name)
        .ok_or_else(|| v(format!("no body {name}")))?;
    let m = body.measure().map_err(v)?;
    let (lo, hi) = (m.bbox.min, m.bbox.max);
    Ok(([lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], m.volume))
}

/// The in-code builder's package for the row, its terminals 1 and 2.
fn package_of(row: &starter::StarterRow) -> PackageDefinition {
    PackageDefinition {
        next_local: 3,
        terminals: (1..=2)
            .map(|i| sparksmith_core::library::PackageTerminal {
                id: sparksmith_core::PackageTerminalId(i),
                designation: i.to_string(),
            })
            .collect(),
        model: PackageModel::Generated {
            family_rev: 1,
            materials: GeneratedMaterials::default(),
        },
        form: row.definition.values.clone(),
        ..PackageDefinition::default()
    }
}

/// The worst differences of one model: bounds (mm), relative volume, and
/// the cold times of the part file and the builder (ms).
struct Model3d {
    bounds: f64,
    volume: f64,
    file_ms: f64,
    code_ms: f64,
}

/// The 3D bars: the generator's model (the bridge the board builds for a
/// stored molded package) against the builder. Bodies, roles as the part
/// file gives them (a lead is a `termination`, the builder's `lead`; a
/// MELF's dark band a `body`, the builder's `marking`), materials,
/// terminals, bounds within 1 µm, volume within surface × 1 µm.
fn models(package: &PackageDefinition, what: &str) -> Result<Model3d> {
    let started = std::time::Instant::now();
    let (part, bodies) = family::evaluate(package)?;
    let file_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut file: Measured = Vec::new();
    for b in &bodies {
        let (bounds, volume) = measure(part.state(), &b.name)?;
        file.push((b.name.clone(), b.role, b.material, bounds, volume));
    }
    let started = std::time::Instant::now();
    let mut native = Native::new("parity")?;
    let built = build_standalone(&mut native, package)?;
    let code_ms = started.elapsed().as_secs_f64() * 1e3;
    let state = native.session().model.state();
    let melf = matches!(&package.form, PackageForm::Molded(m) if m.lead == MoldedLead::Melf);
    let mut code: Measured = Vec::new();
    for b in &built {
        let (bounds, volume) = measure(&state, &b.name)?;
        let role = match b.role {
            BodyRole::Lead => BodyRole::Termination,
            BodyRole::Marking if melf => BodyRole::Body,
            other => other,
        };
        code.push((b.name.clone(), role, b.material, bounds, volume));
    }
    file.sort_by(|a, b| a.0.cmp(&b.0));
    code.sort_by(|a, b| a.0.cmp(&b.0));
    let ids = |m: &Measured| -> Vec<(String, BodyRole, MaterialId)> {
        m.iter()
            .map(|(n, r, mat, _, _)| (n.clone(), *r, *mat))
            .collect()
    };
    assert_eq!(ids(&file), ids(&code), "{what}: bodies");
    // The same terminals, and each lead under its land: the module's lead
    // contact lies within the body's x range, on the seating plane.
    let terminal = |b: &[GeneratedBody]| -> BTreeMap<String, Option<u32>> {
        b.iter().map(|b| (b.name.clone(), b.terminal)).collect()
    };
    assert_eq!(terminal(&bodies), terminal(&built), "{what}: terminals");
    let PackageForm::Molded(form) = &package.form else {
        return Err(v("not molded"));
    };
    let contact = form.span() / 2.0 - form.terminal_length() / 2.0;
    for b in &bodies {
        let Some(t) = b.terminal else { continue };
        let (bounds, _) = measure(part.state(), &b.name)?;
        let x = if t == 1 { -contact } else { contact };
        assert!(
            bounds[0] - UM <= x && x <= bounds[3] + UM && bounds[2].abs() < UM,
            "{what}: {} (terminal {t}) not on its contact at x = {x}: {bounds:?}",
            b.name
        );
    }
    assert_eq!(
        bodies.iter().filter(|b| b.terminal.is_some()).count(),
        2,
        "{what}: both leads bound"
    );
    let mut worst = (0.0f64, 0.0f64);
    for ((name, _, _, a, va), (_, _, _, b, vb)) in file.iter().zip(&code) {
        let off = a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        let [x, y, z] = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
        let surface = 2.0 * (x * y + y * z + z * x);
        let dv = (va - vb).abs();
        assert!(
            off < UM,
            "{what} {name}: bounds differ by {off} mm: {a:?} against {b:?}"
        );
        assert!(
            dv < surface * UM,
            "{what} {name}: volume {va} mm³ against {vb} mm³; part file {file:?}; builder {code:?}"
        );
        worst = (
            worst.0.max(off),
            worst.1.max(dv / vb.max(f64::MIN_POSITIVE)),
        );
    }
    Ok(Model3d {
        bounds: worst.0,
        volume: worst.1,
        file_ms,
        code_ms,
    })
}

#[test]
fn the_molded_generator_equals_the_in_code_molded_family_on_every_starter_row() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let rows: Vec<&starter::StarterRow> = starter::rows()
        .map_err(v)?
        .iter()
        .filter(|r| r.file == "molded.json")
        .collect();
    assert_eq!(rows.len(), 13, "molded starter rows");
    let (mut pads, mut drawings, mut symbol, mut marks) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let (mut bounds, mut volume) = (0.0f64, 0.0f64);
    let (mut slowest_file, mut slowest_code) = (0.0f64, 0.0f64);
    for row in &rows {
        let g = generation(&row.name, Scope::Component)?;
        let (made, part) = generated(&g)?;
        let code = in_code(row)?;
        let (Some(Definition::Package(a)), Some(Definition::Package(b))) =
            (code.get("package"), made.get("package"))
        else {
            return Err(v("no packages"));
        };
        assert_eq!(json(&a.terminals)?, json(&b.terminals)?, "{}", row.name);
        assert_eq!(a.envelope_nm, b.envelope_nm, "{}: envelope", row.name);
        let (p, d) = footprints(&code, &made).map_err(|e| v(format!("{}: {e}", row.name)))?;
        pads = pads.max(p);
        drawings = drawings.max(d);
        symbol = symbol.max(symbols(&code, &made).map_err(|e| v(format!("{}: {e}", row.name)))?);
        marks = marks.max(marking(row, &part)?);
        let m = models(&package_of(row), &row.name)?;
        bounds = bounds.max(m.bounds);
        volume = volume.max(m.volume);
        slowest_file = slowest_file.max(m.file_ms);
        slowest_code = slowest_code.max(m.code_ms);
    }
    println!(
        "molded parity over {} rows: pads within {pads} nm, drawings within {drawings} nm, \
         symbols within {symbol} nm, marking area within {marks} nm; 3D bounds within \
         {bounds:.3e} mm, volumes within {volume:.3e} relative; slowest cold model: generator \
         {slowest_file:.0} ms, builder {slowest_code:.0} ms",
        rows.len()
    );
    assert!(pads <= 1.0 && drawings <= 1.0 && symbol <= 1.0 && marks <= 1.0);
    Ok(())
}

/// The forms no starter row has: bent leads (the core's typical form), and
/// every terminal form without its band.
#[test]
fn the_molded_generator_equals_the_builder_off_the_starter_rows() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let rows = starter::rows().map_err(v)?;
    let mut forms = vec![(
        "typical (bent)".to_owned(),
        sparksmith_core::library::package_form::MoldedForm::typical(),
    )];
    for name in ["SOD-123", "SMA (DO-214AC)", "MiniMELF (DO-213AA)"] {
        let row = rows
            .iter()
            .find(|r| r.name == name)
            .ok_or_else(|| v(format!("no row {name}")))?;
        if let PackageForm::Molded(m) = &row.definition.values {
            let mut plain = **m;
            plain.polarised = false;
            forms.push((format!("{name} without a band"), plain));
        }
    }
    let mut typical = sparksmith_core::library::package_form::MoldedForm::typical();
    typical.polarised = false;
    forms.push(("typical (bent) without a band".into(), typical));
    let (mut bounds, mut volume) = (0.0f64, 0.0f64);
    for (what, form) in &forms {
        let package = PackageDefinition {
            form: PackageForm::Molded(Box::new(*form)),
            ..package_of(
                rows.iter()
                    .find(|r| r.file == "molded.json")
                    .ok_or_else(|| v("rows"))?,
            )
        };
        let m = models(&package, what)?;
        bounds = bounds.max(m.bounds);
        volume = volume.max(m.volume);
    }
    println!(
        "molded parity off the rows ({} forms): 3D bounds within {bounds:.3e} mm, volumes \
         within {volume:.3e} relative",
        forms.len()
    );
    Ok(())
}
