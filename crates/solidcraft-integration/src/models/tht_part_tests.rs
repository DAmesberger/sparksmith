//! Phase C parity of the shipped `tht` generator with the in-code
//! through-hole family it replaces, over every through-hole starter row
//! (docs/design/part-generation-plan.md, "Parity bars"): the footprint,
//! symbol, binding and package the module makes (expanded as
//! `library.generate` stores them) against `library.new_from_template` for
//! the same row, and its part files, evaluated at the module's values,
//! against the in-code through-hole builder. Deleted with the builder.
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
use sparksmith_core::library::{Definition, Library, PadShape};
use sparksmith_generators::abi::v1::{Generation, Model, Scope};
use sparksmith_generators::abi::{Failure, GeneratorRuntime};
use sparksmith_generators::params::complete;
use sparksmith_generators::{Limits, WasmiRuntime};

/// The shipped module (test code may read it).
const THT: &[u8] = include_bytes!("../../../../library/generators/tht/tht.wasm");
/// Its part files, as shipped.
const PARTS: [&[u8]; 6] = [
    include_bytes!("../../../../library/generators/tht/parts/tht-axial.solidcraft"),
    include_bytes!("../../../../library/generators/tht/parts/tht-radial.solidcraft"),
    include_bytes!("../../../../library/generators/tht/parts/tht-radial-formed.solidcraft"),
    include_bytes!("../../../../library/generators/tht/parts/tht-dip.solidcraft"),
    include_bytes!("../../../../library/generators/tht/parts/tht-to.solidcraft"),
    include_bytes!("../../../../library/generators/tht/parts/tht-to-formed.solidcraft"),
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

/// The module's generation for starter row `name` (its preset of the same
/// name), in `scope`.
fn generation(name: &str, scope: Scope) -> Result<Generation> {
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let description = runtime.describe(THT).map_err(fail)?;
    let preset = description
        .presets()
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| v(format!("no preset {name}")))?;
    let request = complete(&description, scope, &preset.params).map_err(fail)?;
    Ok(runtime.generate(THT, &request).map_err(fail)?.generation)
}

/// The bundled part file of `asset` (its SHA-256).
fn part_bytes(asset: &str) -> Result<&'static [u8]> {
    PARTS
        .iter()
        .copied()
        .find(|bytes| sparksmith_core::library::bundle::asset_hash(bytes).to_string() == asset)
        .ok_or_else(|| v(format!("no bundled part file sha256 {asset}")))
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

/// The five definitions, component first, by kind.
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
    let as_is = part_file::evaluate(part_bytes(asset)?, &PartValues::new())?;
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
            "{what}: {} items in code, {} generated: {} / {}",
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

/// A rounded rectangle by the corner radius it draws: the host's
/// template stores pin 1 as a quarter of the side, the expansion as half
/// the side capped at the module's radius; both draw the same corners.
fn drawn(shape: &PadShape) -> PadShape {
    match shape {
        PadShape::RoundRect {
            size,
            radius_ratio,
            max_radius_nm,
            corners,
        } => {
            let short = size.x_nm.get().min(size.y_nm.get()) as f64;
            let ratio = radius_ratio * short;
            let r = max_radius_nm.map_or(ratio, |m| ratio.min(m.get() as f64));
            PadShape::RoundRect {
                size: *size,
                radius_ratio: 0.0,
                max_radius_nm: sparksmith_core::units::Nm::new(r.round() as i64).ok(),
                corners: *corners,
            }
        }
        other => other.clone(),
    }
}

/// The footprint bars; the worst pad and drawing differences, nm, and
/// the pads whose lands differ (number, in code, generated).
#[allow(clippy::type_complexity)]
fn footprints(
    code: &Five,
    made: &Five,
    pins: usize,
) -> Result<(f64, f64, Vec<(String, Value, Value)>)> {
    let (Some(Definition::Footprint(a)), Some(Definition::Footprint(b))) =
        (code.get("footprint"), made.get("footprint"))
    else {
        return Err(v("no footprints"));
    };
    assert_eq!((a.pads.len(), b.pads.len()), (pins, pins));
    let mut pad_worst = 0.0f64;
    let mut lands = Vec::new();
    for (p, q) in a.pads.iter().zip(&b.pads) {
        // Ids and terminal ids both run 1…n in pin order.
        let (mut p, mut q) = (p.clone(), q.clone());
        p.shape = drawn(&p.shape);
        q.shape = drawn(&q.shape);
        let (x, y) = (json(&p)?, json(&q)?);
        match diff(&x, &y) {
            Some(d) if d <= 1.0 => pad_worst = pad_worst.max(d),
            _ => lands.push((p.number.clone(), json(&p.shape)?, json(&q.shape)?)),
        }
        // Everything but the land is equal regardless.
        let mut q2 = q.clone();
        q2.shape = p.shape.clone();
        let d = diff(&x, &json(&q2)?)
            .ok_or_else(|| v(format!("pad {}: {x} in code, {y} generated", p.number)))?;
        pad_worst = pad_worst.max(d);
    }
    let fp = |f: &Five| f.get("footprint").cloned().ok_or_else(|| v("fp"));
    let silk = if lands.is_empty() {
        matched(
            "silkscreen",
            &graphics(&fp(code)?, "silkscreen")?,
            &graphics(&fp(made)?, "silkscreen")?,
        )?
    } else {
        // A different land breaks the outline elsewhere and moves the
        // pin-1 dot; counted, not compared.
        0.0
    };
    let assembly = matched(
        "assembly",
        &graphics(&fp(code)?, "assembly")?,
        &graphics(&fp(made)?, "assembly")?,
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
    if !lands.is_empty() {
        // The courtyard grows with the larger land; reported, not compared.
        println!("  courtyard {courtyard} nm apart where the lands differ");
        return Ok((pad_worst, silk.max(assembly), lands));
    }
    Ok((pad_worst, silk.max(assembly).max(courtyard), lands))
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

/// The in-code builder's package for the row, its terminals 1…n.
fn package_of(row: &starter::StarterRow) -> PackageDefinition {
    let n = u32::try_from(row.definition.values.lead_count()).unwrap_or(0);
    PackageDefinition {
        next_local: n + 1,
        terminals: (1..=n)
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

/// The 3D bars of one row: the generator's model (the bridge the board
/// builds for a stored through-hole package) against the builder.
struct Models {
    bounds: f64,
    volume: f64,
    file_ms: f64,
    code_ms: f64,
    bodies: usize,
}

/// Bodies, roles as the part file gives them (a lead is a `termination`,
/// the builder's `lead`), materials, terminals, bounds within 1 µm, volume
/// within surface × 1 µm; every lead bound to the terminal of the land it
/// enters, reaching `lead_below` under it.
fn models(row: &starter::StarterRow) -> Result<Models> {
    let package = package_of(row);
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
        let role = match b.role {
            BodyRole::Lead => BodyRole::Termination,
            other => other,
        };
        code.push((b.name.clone(), role, b.material, b.terminal, bounds, volume));
    }
    file.sort_by(|a, b| a.0.cmp(&b.0));
    code.sort_by(|a, b| a.0.cmp(&b.0));
    type Ids = Vec<(String, BodyRole, MaterialId, Option<u32>)>;
    let ids = |m: &Measured| -> Ids {
        m.iter()
            .map(|(n, r, mat, t, _, _)| (n.clone(), *r, *mat, *t))
            .collect()
    };
    assert_eq!(ids(&file), ids(&code), "{}: bodies", row.name);
    // Each lead enters the land of its terminal and reaches below it.
    let g = generation(&row.name, Scope::Package)?;
    let below = match &row.definition.values {
        PackageForm::Tht(t) => -t.lead_below(),
        _ => f64::NAN,
    };
    for (name, _, _, terminal, bounds, _) in &file {
        let Some(t) = terminal else { continue };
        let pad = usize::try_from(*t)
            .ok()
            .and_then(|t| g.footprint.pads.get(t - 1))
            .ok_or_else(|| v(format!("{}: no pad {t}", row.name)))?;
        let [x, y] = [pad.at[0] as f64 * NM, pad.at[1] as f64 * NM];
        assert!(
            x >= bounds[0] - UM
                && x <= bounds[3] + UM
                && y >= bounds[1] - UM
                && y <= bounds[4] + UM,
            "{}: {name} bound to {t} does not stand over its land",
            row.name
        );
        assert!(
            (bounds[2] - below).abs() < UM,
            "{}: {name} reaches {} under the board, not {below}",
            row.name,
            bounds[2]
        );
    }
    assert_eq!(
        file.iter().filter(|b| b.3.is_some()).count(),
        g.footprint.pads.len(),
        "{}: every lead bound",
        row.name
    );
    // Marking surfaces: a part file carries planes only, so the radial can
    // top is compared; the form's cylinders (a radial sleeve, an axial
    // body) are listed in the plan as not carried.
    use sparksmith_core::library::marking::surface::{self, SurfaceGeometry};
    let planes = |s: &[sparksmith_core::library::marking::surface::AnchorInfo]| {
        s.iter()
            .filter_map(|a| match &a.geometry {
                SurfaceGeometry::Plane(p) => Some((
                    a.name.clone(),
                    [
                        p.origin().to_vec(),
                        p.normal.to_vec(),
                        p.u_axis.to_vec(),
                        p.size().to_vec(),
                    ]
                    .concat(),
                )),
                SurfaceGeometry::Cylinder(_) => None,
            })
            .collect::<Vec<_>>()
    };
    let (want, have) = (
        planes(&surface::anchors(&package.form)),
        planes(&part.surfaces),
    );
    assert_eq!(
        want.iter().map(|w| &w.0).collect::<Vec<_>>(),
        have.iter().map(|h| &h.0).collect::<Vec<_>>(),
        "{}: marking planes",
        row.name
    );
    for ((name, a), (_, b)) in want.iter().zip(&have) {
        let off = a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        assert!(off < 1e-9, "{}: marking {name} {off} mm off", row.name);
    }
    let mut worst = (0.0f64, 0.0f64);
    for ((name, _, _, _, a, va), (_, _, _, _, b, vb)) in file.iter().zip(&code) {
        let off = a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        let [x, y, z] = [b[3] - b[0], b[4] - b[1], b[5] - b[2]];
        let surface = 2.0 * (x * y + y * z + z * x);
        let dv = (va - vb).abs();
        assert!(off < UM, "{} {name}: bounds differ by {off} mm", row.name);
        assert!(
            dv < surface * UM,
            "{} {name}: volume {va} mm³ against {vb} mm³",
            row.name
        );
        worst = (
            worst.0.max(off),
            worst.1.max(dv / vb.max(f64::MIN_POSITIVE)),
        );
    }
    Ok(Models {
        bounds: worst.0,
        volume: worst.1,
        file_ms,
        code_ms,
        bodies: file.len(),
    })
}

#[test]
fn the_tht_generator_equals_the_in_code_through_hole_family_on_every_starter_row() -> Result<()> {
    let _context = crate::looks::test_context().map(|c| c.enter());
    let rows: Vec<&starter::StarterRow> = starter::rows()
        .map_err(v)?
        .iter()
        .filter(|r| r.file == "tht.json")
        .collect();
    assert_eq!(rows.len(), 22, "through-hole starter rows");
    let (mut pads, mut drawings, mut symbol) = (0.0f64, 0.0f64, 0.0f64);
    let (mut bounds, mut volume) = (0.0f64, 0.0f64);
    let mut slowest: (f64, f64, String) = (0.0, 0.0, String::new());
    let mut lands = Vec::new();
    for row in &rows {
        let g = generation(&row.name, Scope::Component)?;
        let made = generated(&g)?;
        let code = in_code(row)?;
        let pins = g.footprint.pads.len();
        // The package: terminals by designation, the envelope.
        let (Some(Definition::Package(a)), Some(Definition::Package(b))) =
            (code.get("package"), made.get("package"))
        else {
            return Err(v("no packages"));
        };
        assert_eq!(json(&a.terminals)?, json(&b.terminals)?, "{}", row.name);
        assert_eq!(a.envelope_nm, b.envelope_nm, "{}: envelope", row.name);
        let (p, d, differ) =
            footprints(&code, &made, pins).map_err(|e| v(format!("{}: {e}", row.name)))?;
        pads = pads.max(p);
        drawings = drawings.max(d);
        for (number, a, b) in differ {
            lands.push(format!(
                "{} pad {number}: {a} in code, {b} generated",
                row.name
            ));
        }
        symbol = symbol.max(symbols(&code, &made).map_err(|e| v(format!("{}: {e}", row.name)))?);
        let m = models(row)?;
        bounds = bounds.max(m.bounds);
        volume = volume.max(m.volume);
        println!(
            "{}: {} bodies, cold model {:.0} ms (builder {:.0} ms)",
            row.name, m.bodies, m.file_ms, m.code_ms
        );
        if m.file_ms > slowest.0 {
            slowest = (m.file_ms, m.code_ms, row.name.clone());
        }
    }
    println!(
        "through-hole parity over {} rows: pads within {pads} nm, drawings within {drawings} nm, \
         symbols within {symbol} nm; 3D bounds within {bounds:.3e} mm, volumes within \
         {volume:.3e} relative; slowest cold model {}: generator {:.0} ms, builder {:.0} ms; \
         lands that differ: {lands:#?}",
        rows.len(),
        slowest.2,
        slowest.0,
        slowest.1,
    );
    assert!(pads <= 1.0 && drawings <= 1.0 && symbol <= 1.0);
    // The one land the builder rounds down by f64 noise (listed in the
    // plan): the formed TO-92's 0.95 + 0.6 = 1.55 mm, which it takes as
    // 1.5 mm.
    assert_eq!(lands.len(), 3, "{lands:#?}");
    assert!(lands.iter().all(|l| l.starts_with("TO-92-3 2.54 formed")));
    Ok(())
}
