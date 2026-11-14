//! Phase C parity of the shipped `grid` generator with the in-code grid
//! family it replaces, over every grid starter row
//! (docs/design/part-generation-plan.md, "Parity bars"): the footprint,
//! symbol, binding and package the module makes (expanded as
//! `library.generate` stores them) against `library.new_from_template` for
//! the same row, and its part file, evaluated at the module's values,
//! against the in-code grid builder, for the row and for the row with
//! empty positions (the cut-out files). Deleted with the builder.
use super::part_file::{self, PartValues};
use super::*;
use serde_json::Value;
use sparksmith_core::ids::{
    BindingRef, DefinitionId, FootprintRef, ModelRef, PackageRef, Revision, SymbolRef,
};
use sparksmith_core::library::generated::expand::{self, KeyIds, Refs};
use sparksmith_core::library::package_model::{PartDerived, PartFileModel};
use sparksmith_core::library::samples::SampleRefs;
use sparksmith_core::library::symbol_style::SymbolConventions;
use sparksmith_core::library::templates::catalog::{
    NewFromTemplate, RowSource, new_from_template_with,
};
use sparksmith_core::library::templates::starter;
use sparksmith_core::library::{Definition, Library};
use sparksmith_generators::abi::v1::{Generation, Model, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};

/// The shipped module (test code may read it).
const MODULE: &[u8] = include_bytes!("../../../../library/generators/grid/grid.wasm");
/// Its part files, as shipped.
const PARTS: [&[u8]; 4] = [
    include_bytes!("../../../../library/generators/grid/parts/grid-ball.solidcraft"),
    include_bytes!("../../../../library/generators/grid/parts/grid-ball-voids.solidcraft"),
    include_bytes!("../../../../library/generators/grid/parts/grid-land.solidcraft"),
    include_bytes!("../../../../library/generators/grid/parts/grid-land-voids.solidcraft"),
];

/// 1 µm and 1 nm, in mm.
const UM: f64 = 1e-3;
const NM: f64 = 1e-6;

fn fail(f: Failure) -> Error {
    Error::Verification(f.to_string())
}

fn v(e: impl std::fmt::Display) -> Error {
    Error::Verification(e.to_string())
}

/// The bundled part file of SHA-256 `asset`.
fn part(asset: &str) -> Result<&'static [u8]> {
    let hash = sparksmith_core::ids::ContentHash::parse(asset);
    PARTS
        .into_iter()
        .find(|bytes| Some(sparksmith_core::library::bundle::asset_hash(bytes)) == hash)
        .ok_or_else(|| v(format!("no bundled part file sha256 {asset}")))
}

/// The module's generation for starter row `name` (its preset of the same
/// name), in `scope`.
fn generation(name: &str, scope: Scope) -> Result<Generation> {
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(MODULE).map_err(fail)?;
    let preset = description
        .presets()
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| v(format!("no preset {name}")))?;
    let request = complete(&description, scope, &preset.params).map_err(fail)?;
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

/// The footprint bars; the worst pad and drawing differences, nm.
fn footprints(code: &Five, made: &Five) -> Result<(f64, f64)> {
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

/// Each body: name, role, material, terminal, bounds and volume (mm).
type Measured = Vec<(String, BodyRole, MaterialId, Option<u32>, [f64; 6], f64)>;

fn measure(state: &solidcraft_engine::doc::ModelState, name: &str) -> Result<([f64; 6], f64)> {
    let body = state
        .body(name)
        .ok_or_else(|| v(format!("no body {name}")))?;
    let m = body.measure().map_err(v)?;
    let (lo, hi) = (m.bbox.min, m.bbox.max);
    Ok(([lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], m.volume))
}

/// The in-code builder's package for `form`, its terminals the ball names
/// in lead order.
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

/// The module's generation for `form` (package scope), through the same
/// mapping the board uses.
fn generation_of(form: &PackageForm) -> Result<Generation> {
    let (_, params) = family::generator(form).ok_or_else(|| v("no generator"))?;
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(MODULE).map_err(fail)?;
    let request = complete(&description, Scope::Package, &params).map_err(fail)?;
    Ok(runtime.generate(MODULE, &request).map_err(fail)?.generation)
}

/// One measured body.
type Body = (String, BodyRole, MaterialId, Option<u32>, [f64; 6], f64);

/// The 3D bars: the generator's model (the bridge the board builds for a
/// stored grid package) against the builder. Bodies, roles, materials,
/// bounds within 1 µm, volume within surface × 1 µm; each bound body
/// standing on its land of the module's footprint. Returns the worst
/// bounds and relative volume differences, the cold times of both (ms)
/// and how many contacts the builder bound to another terminal than the
/// part file (the builder numbered contacts by their row and column
/// rounded from the centre over the pitch, which ties on even grids).
fn models(name: &str, form: &PackageForm) -> Result<(f64, f64, f64, f64, usize)> {
    let package = package_of(form);
    let full = matches!(form, PackageForm::Grid(g) if g.depopulated.is_empty());
    let started = std::time::Instant::now();
    let (part, bodies) = family::evaluate(&package)?;
    let file_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut file: Measured = Vec::new();
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
    let mut code: Measured = Vec::new();
    for b in &built {
        let (bounds, volume) = measure(&state, &b.name)?;
        // The part file's balls are terminations by their material, where
        // the builder said lead (as the header's posts).
        let role = if b.role == BodyRole::Lead {
            BodyRole::Termination
        } else {
            b.role
        };
        code.push((b.name.clone(), role, b.material, b.terminal, bounds, volume));
    }
    // Bodies matched by where they are: the body first, then each contact
    // by its centre to the nanometre.
    let key = |m: &Body| {
        let c = |i: usize| ((m.4[i] + m.4[i + 3]) / 2.0 / NM).round() as i64;
        (m.3.is_some(), c(0), c(1))
    };
    file.sort_by_key(key);
    code.sort_by_key(key);
    // Names are compared on a full grid (the builder named a depopulated
    // grid's runs, the part file its one pattern); terminals separately.
    let ids = |m: &Measured| -> Vec<(String, BodyRole, MaterialId, bool)> {
        m.iter()
            .map(|(n, r, mat, t, _, _)| {
                let n = if full || t.is_none() {
                    n.clone()
                } else {
                    String::new()
                };
                (n, *r, *mat, t.is_some())
            })
            .collect()
    };
    assert_eq!(ids(&file), ids(&code), "{name}: bodies");
    let swapped = file.iter().zip(&code).filter(|(a, b)| a.3 != b.3).count();
    // Each bound body stands on its own land: its centre within the land
    // of the same terminal in the module's footprint, and every terminal
    // has one.
    let g = generation_of(form)?;
    for b in &bodies {
        let Some(t) = b.terminal else { continue };
        let (bounds, _) = measure(part.state(), &b.name)?;
        let centre = [(bounds[0] + bounds[3]) / 2.0, (bounds[1] + bounds[4]) / 2.0];
        let pad = usize::try_from(t)
            .ok()
            .and_then(|t| g.footprint.pads.get(t - 1))
            .ok_or_else(|| v(format!("{name}: no pad {t}")))?;
        let sparksmith_generators::abi::v1::PadShape::Circle { diameter_nm } = pad.shape else {
            return Err(v("not a round land"));
        };
        let off = (centre[0] - pad.at[0] as f64 * NM).hypot(centre[1] - pad.at[1] as f64 * NM);
        assert!(
            off <= diameter_nm as f64 * NM / 2.0,
            "{name}: {} bound to {t} lies off its land",
            b.name
        );
        assert!(
            bounds[2].abs() < UM,
            "{name}: {} off the seating plane",
            b.name
        );
    }
    assert_eq!(
        bodies.iter().filter(|b| b.terminal.is_some()).count(),
        g.footprint.pads.len(),
        "{name}: every terminal bound"
    );
    let mut worst = (0.0f64, 0.0f64);
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
        worst = (
            worst.0.max(off),
            worst.1.max(dv / vb.max(f64::MIN_POSITIVE)),
        );
    }
    Ok((worst.0, worst.1, file_ms, code_ms, swapped))
}

/// `form` with empty positions: a centre block of 2 × 2 (one position on
/// a 3 × 3 grid) and the position beside A1.
fn depopulated(form: &PackageForm) -> Result<PackageForm> {
    let PackageForm::Grid(g) = form else {
        return Err(v("not a grid row"));
    };
    let mut g = g.clone();
    let (r, c) = (g.rows / 2, g.cols / 2);
    g.depopulated = if g.rows >= 4 && g.cols >= 4 {
        vec![[r - 1, c - 1], [r - 1, c], [r, c - 1], [r, c]]
    } else {
        vec![[r, c]]
    };
    if g.cols >= 2 && !g.depopulated.contains(&[0, 1]) {
        g.depopulated.push([0, 1]);
    }
    Ok(PackageForm::Grid(g))
}

#[test]
fn the_grid_generator_equals_the_in_code_grid_family_on_every_starter_row() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let rows: Vec<&starter::StarterRow> = starter::rows()
        .map_err(v)?
        .iter()
        .filter(|r| r.file == "grid.json")
        .collect();
    assert_eq!(rows.len(), 14, "grid starter rows");
    let (mut pads, mut drawings, mut symbol) = (0.0f64, 0.0f64, 0.0f64);
    let (mut bounds, mut volume) = (0.0f64, 0.0f64);
    let (mut slowest_file, mut slowest_code) = (0.0f64, 0.0f64);
    let mut slowest_voids = 0.0f64;
    let mut swapped_rows = Vec::new();
    for row in &rows {
        let g = generation(&row.name, Scope::Component)?;
        let made = generated(&g)?;
        let code = in_code(row)?;
        // The package: terminals by designation, the envelope.
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
        // The row's model, and the same grid with empty positions (the
        // cut-out files).
        let form = &row.definition.values;
        for (form, empty) in [(form.clone(), false), (depopulated(form)?, true)] {
            let (b, vol, file_ms, code_ms, swapped) = models(&row.name, &form)?;
            if swapped > 0 {
                let with = if empty { ", with empty positions" } else { "" };
                swapped_rows.push(format!("{}{with}: {swapped}", row.name));
            }
            println!(
                "  {}{}: generator {file_ms:.0} ms, builder {code_ms:.0} ms",
                row.name,
                if empty { " (empty positions)" } else { "" }
            );
            if empty {
                slowest_voids = slowest_voids.max(file_ms);
            }
            bounds = bounds.max(b);
            volume = volume.max(vol);
            slowest_file = slowest_file.max(file_ms);
            slowest_code = slowest_code.max(code_ms);
        }
    }
    println!(
        "grid parity over {} rows: pads within {pads} nm, drawings within {drawings} nm, \
         symbols within {symbol} nm; 3D (full and with empty positions) bounds within \
         {bounds:.3e} mm, volumes within {volume:.3e} relative; slowest cold model: generator \
         {slowest_file:.0} ms ({slowest_voids:.0} ms with cut-outs), builder {slowest_code:.0} ms; builder bound other terminals in \
         {swapped_rows:?}",
        rows.len()
    );
    assert!(pads <= 1.0 && drawings <= 1.0 && symbol <= 1.0);
    Ok(())
}
