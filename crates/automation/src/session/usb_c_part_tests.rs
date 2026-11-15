//! Phase C, job I (docs/design/part-generation-plan.md): the USB-C
//! receptacle becomes an authored library part, `library/parts/gct-usb4105`
//! (part file, sidecar and component records), in place of its in-code
//! template (the `usb_c.json` starter row, the typed form's lands and
//! terminals, the `models/usb_c.rs` builder).
//!
//! [`write_the_shipped_usb_c_component`] made the component records once
//! from the starter row, through the ordinary commands: the template's
//! component, symbol, package, footprint and binding, the package made
//! generic and given the shipped part file as its body, and the GCT
//! manufacturer Part as the root. [`the_shipped_usb_c_component_equals_the_template`]
//! measures the parity bars against the template and the builder. Both are
//! deleted with them.
use super::Session;
use super::pinout_tests::library_op;
use serde_json::{Value, json};
use sparksmith_core::ids::{DefinitionRef, LibraryId, SourceLineageId};
use sparksmith_core::library::bundle::{Bundle, RevisionRecord};
use sparksmith_core::library::package_form::usb_c::UsbCForm;
use sparksmith_core::library::{
    BodyRole, Definition, GeneratedMaterials, PackageDefinition, PackageForm, PackageModel,
};
use sparksmith_core::{DefinitionKind, Engine, Revision};
use sparksmith_solidcraft::models::{self, family, part_file};
use sparksmith_solidcraft::native::Native;
use std::collections::BTreeMap;
use std::path::PathBuf;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The starter row the component was made from.
const ROW: &str = "USB-C receptacle, USB 2.0, 16 contacts, top mount (GCT USB4105)";
/// The shipped records' library ("Sparksmith built-in parts") and the
/// fresh lineage ids they were given.
const LIBRARY: &str = "f94c168fc18b4d5b8a57f1dcd2600a34";
const FRESH: [&str; 9] = [
    "87a97fd3de794565b8c1b180bf964095",
    "e0fd511555174fecbec0f6d31af40f4a",
    "5fab9c6903814ec68784cfcbf212f087",
    "5b19dbd98f1c4be793e0684da1932e90",
    "25622c085dcb49cbb28880b4a21446db",
    "a3943c2f645b42e6990f38ff7a6f194a",
    "0244a5a1928d4f37ab3c9e3868121cbb",
    "c8e2348ae50443f1be2b2c7fef289087",
    "058aacec2986418b9c5a615e744c55a2",
];

const DRAWING: &str = "GCT USB4105, USB Type C receptacle for USB 2.0, SMT type, PCB top mount: drawing sheet 1, outline and recommended PCB layout";
const DRAWING_URL: &str = "https://gct.co/files/drawings/usb4105.pdf";
const DRAWING_REVISION: &str = "B4 (2023-12-18)";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn part_path() -> PathBuf {
    root().join("library/parts/gct-usb4105.solidcraft")
}

fn apply(session: &mut Session, command: &str, params: Value) -> Fallible<Value> {
    let revision = session.engine().revision();
    let reply = session.dispatch(json!({"command": "project.apply", "params": {
        "expected_revision": revision, "operation": library_op(command, params)}}));
    if reply["ok"] != true {
        return Err(format!("{command}: {reply}").into());
    }
    Ok(reply["result"].clone())
}

/// The template's five drafts for the row: component, symbol, package,
/// footprint, binding.
fn from_template(session: &mut Session) -> Fallible<Vec<u64>> {
    let made = apply(
        session,
        "library.new_from_template",
        json!({"row": {"source": "starter", "name": ROW}, "skeleton": {"skeleton": "connector"},
            "name": ROW}),
    )?;
    let ids: Vec<u64> = made["outcome"]["ids"]
        .as_array()
        .ok_or("ids")?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    if ids.len() != 5 {
        return Err(format!("five drafts: {made}").into());
    }
    Ok(ids)
}

fn draft(session: &Session, id: u64) -> Fallible<Definition> {
    session
        .engine()
        .drafts()
        .items
        .iter()
        .find(|d| d.id.0 == id)
        .map(|d| d.definition.clone())
        .ok_or_else(|| format!("no draft {id}").into())
}

/// The shipped component's records, made from the template: the package
/// generic with the part file as its body, and the manufacturer Part.
fn make_bundle() -> Fallible<Bundle> {
    let mut session = Session::new(Engine::demo()?);
    let [component, symbol, package, footprint, binding] = from_template(&mut session)?[..] else {
        return Err("five drafts".into());
    };
    // No typed form: the envelope stays as authored, the form's opening
    // datum goes; the part file declares it.
    apply(
        &mut session,
        "library.edit_package",
        json!({"draft": package, "edits": [{"edit": "set_form", "value": {"family": "generic"}}]}),
    )?;
    let revision = session.engine().revision();
    let used = session.dispatch(json!({"command": "library.use_part_file", "params": {
        "expected_revision": revision, "package": package,
        "path": part_path().to_string_lossy(), "name": "GCT USB4105 receptacle body",
        "licence": "MIT OR Apache-2.0", "redistribution": "permitted"}}));
    if used["ok"] != true {
        return Err(format!("use_part_file: {used}").into());
    }
    let model = used["result"]["outcome"]["id"]
        .as_u64()
        .ok_or_else(|| format!("model id: {used}"))?;
    let part = apply(
        &mut session,
        "library.new_draft",
        json!({"meta": {"name": "USB4105-GF-A", "description": "USB-C receptacle, USB 2.0, top mount", "provenance": ""},
            "definition": {"kind": "part", "body": {
                "next_local": 2, "manufacturer": "GCT", "mpn": "USB4105-GF-A",
                "component": {"id": component, "revision": 1},
                "binding": {"id": binding, "revision": 1},
                "specs": [],
                "references": [{"id": 1, "kind": "datasheet", "title": DRAWING, "url": DRAWING_URL,
                    "document_revision": DRAWING_REVISION, "asset": null, "licence": null,
                    "provenance": {"origin": null, "basis": {"basis": "authored"}, "licence": null}}],
                "offers": [],
                "provenance": {"origin": null, "basis": {"basis": "authored"}, "licence": null}}}}),
    )?;
    let part = part["outcome"]["id"].as_u64().ok_or("part id")?;
    apply(
        &mut session,
        "library.publish",
        json!({"drafts": [component, symbol, package, footprint, binding, model, part]}),
    )?;
    let library = &session.engine().project().library;
    let fresh: Vec<SourceLineageId> = FRESH
        .iter()
        .filter_map(|h| SourceLineageId::parse(h))
        .collect();
    let plan = sparksmith_core::library::import::export(
        library,
        DefinitionRef {
            kind: DefinitionKind::Part,
            id: sparksmith_core::DefinitionId(part),
            revision: Revision::FIRST,
        },
        LibraryId::parse(LIBRARY).ok_or("library id")?,
        "Sparksmith built-in parts",
        &fresh,
    )?;
    let mut bundle = plan.bundle;
    bundle.assets.clear();
    let provenance = sidecar()?["provenance"]
        .as_str()
        .ok_or("provenance")?
        .to_owned();
    // The row's description and note (the fab limits its land pattern
    // needs) describe the component.
    let row = sparksmith_core::library::templates::starter::row(ROW)?;
    let about = match &row.note {
        Some(note) => format!("{} {note}", row.description),
        None => row.description.clone(),
    };
    for record in &mut bundle.records {
        record.meta.provenance = provenance.clone();
        if record.kind == DefinitionKind::Component {
            record.meta.description = about.clone();
        }
    }
    Ok(bundle)
}

fn sidecar() -> Fallible<Value> {
    Ok(serde_json::from_slice(&std::fs::read(
        root().join("library/parts/gct-usb4105.part.json"),
    )?)?)
}

#[test]
#[ignore = "writes library/parts/gct-usb4105.component.json"]
fn write_the_shipped_usb_c_component() -> Fallible {
    let bundle = make_bundle()?;
    let mut text = serde_json::to_string_pretty(&json!({"format": 1, "bundle": bundle}))?;
    text.push('\n');
    std::fs::write(
        root().join("library/parts/gct-usb4105.component.json"),
        text,
    )?;
    Ok(())
}

/// The shipped records, as the program loads them (part file packed).
fn shipped() -> Fallible<std::sync::Arc<Bundle>> {
    let parts = super::shipped_parts::builtin_parts()?;
    let part = parts.require("gct-usb4105")?;
    Ok(part
        .component
        .clone()
        .ok_or("gct-usb4105 is not a component part")?)
}

fn record(bundle: &Bundle, kind: DefinitionKind) -> Fallible<&RevisionRecord> {
    bundle
        .records
        .iter()
        .find(|r| r.kind == kind)
        .ok_or_else(|| format!("no {kind:?} record").into())
}

fn json_of(value: &impl serde::Serialize) -> Fallible<Value> {
    Ok(serde_json::to_value(value)?)
}

/// The worst numeric difference between two JSON values of the same shape,
/// or `None` when their shapes or other values differ.
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

/// Each body: name, role, material, bounds and volume (mm).
type Measured = BTreeMap<String, (BodyRole, String, [f64; 6], f64)>;

/// A body's bounds and volume (the engine's `ModelState` is not named
/// here: this crate reaches Solidcraft only through the integration).
macro_rules! measure {
    ($state:expr, $name:expr) => {{
        let name: &str = $name;
        let body = $state.body(name).ok_or_else(|| format!("no body {name}"))?;
        let m = body.measure().map_err(|e| e.to_string())?;
        let (lo, hi) = (m.bbox.min, m.bbox.max);
        ([lo.x, lo.y, lo.z, hi.x, hi.y, hi.z], m.volume)
    }};
}

/// A stored USB-C package of the template's form and builder.
fn stored(form: UsbCForm) -> PackageDefinition {
    PackageDefinition {
        next_local: 14,
        terminals: (1..=13)
            .map(|i| sparksmith_core::library::PackageTerminal {
                id: sparksmith_core::PackageTerminalId(i),
                designation: sparksmith_core::library::package_form::usb_c::DESIGNATIONS
                    [usize::try_from(i - 1).unwrap_or(0)]
                .into(),
            })
            .collect(),
        model: PackageModel::Generated {
            family_rev: 1,
            materials: GeneratedMaterials::default(),
        },
        form: PackageForm::UsbC(Box::new(form)),
        ..PackageDefinition::default()
    }
}

/// The 3D bars for a stored package of `form`: the shipped part file at
/// its values (`family::evaluate`, the bridge the board builds) against
/// the builder. Bodies, roles, materials and terminals equal; the worst
/// bounds difference (mm) and relative volume difference, and each body
/// that differs beyond the bars with why.
fn models_of(form: UsbCForm) -> Fallible<(f64, f64, Vec<String>, f64, f64)> {
    let context = sparksmith_core::library::standards::Context::fixture()?;
    let _context = context.enter();
    let package = stored(form);
    let started = std::time::Instant::now();
    let (part, bodies) = family::evaluate(&package)?;
    let file_ms = started.elapsed().as_secs_f64() * 1e3;
    let mut file: Measured = BTreeMap::new();
    for b in &bodies {
        let (bounds, volume) = measure!(part.state(), &b.name);
        file.insert(
            b.name.clone(),
            (
                b.role,
                format!("{:?}/{:?}", b.material, b.terminal),
                bounds,
                volume,
            ),
        );
    }
    let started = std::time::Instant::now();
    let mut native = Native::new("parity")?;
    let built = models::build_standalone(&mut native, &package)?;
    let code_ms = started.elapsed().as_secs_f64() * 1e3;
    let state = native.session().model.state();
    let mut code: Measured = BTreeMap::new();
    for b in &built {
        let (bounds, volume) = measure!(state, &b.name);
        // The part file's lead role is `termination`, the builder's `lead`.
        let role = match b.role {
            BodyRole::Lead => BodyRole::Termination,
            other => other,
        };
        code.insert(
            b.name.clone(),
            (
                role,
                format!("{:?}/{:?}", b.material, b.terminal),
                bounds,
                volume,
            ),
        );
    }
    let ids = |m: &Measured| -> Vec<(String, BodyRole, String)> {
        m.iter()
            .map(|(n, (r, mat, _, _))| (n.clone(), *r, mat.clone()))
            .collect()
    };
    assert_eq!(
        ids(&file),
        ids(&code),
        "bodies, roles, materials, terminals"
    );
    let mut worst = (0.0f64, 0.0f64);
    let mut differ = Vec::new();
    for ((name, (_, _, a, va)), (_, (_, _, b, vb))) in file.iter().zip(&code) {
        let off = a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0, f64::max);
        let dv = (va - vb).abs() / vb.max(f64::MIN_POSITIVE);
        if name.starts_with("peg ") {
            // Listed: the pegs stand on the seating plane, where the
            // builder sank them 0.5 mm into their holes.
            differ.push(format!(
                "{name}: z from {:.3} (builder {:.3}) mm, volume {va:.6} against {vb:.6} mm³",
                a[2], b[2]
            ));
            continue;
        }
        assert!(off < 1e-3, "{name}: bounds differ by {off} mm");
        assert!(dv < 1e-6, "{name}: volume {va} mm³ against {vb} mm³");
        worst = (worst.0.max(off), worst.1.max(dv));
    }
    Ok((worst.0, worst.1, differ, file_ms, code_ms))
}

#[test]
fn the_shipped_usb_c_component_equals_the_template() -> Fallible {
    let bundle = shipped()?;
    // What the template makes of the row, now.
    let mut session = Session::new(Engine::demo()?);
    let [component, symbol, package, footprint, binding] = from_template(&mut session)?[..] else {
        return Err("five drafts".into());
    };
    let code_symbol = draft(&session, symbol)?;
    let code_package = draft(&session, package)?;
    let code_footprint = draft(&session, footprint)?;
    let code_binding = draft(&session, binding)?;
    let code_component = draft(&session, component)?;

    // Footprint: pads (numbers, terminals, kinds, shapes, holes incl. the
    // slots and NPTH, mask and paste) and every drawing, the same after the
    // records' ids are mapped back.
    let Definition::Footprint(a) = &code_footprint else {
        return Err("footprint".into());
    };
    let Definition::Footprint(b) = &record(&bundle, DefinitionKind::Footprint)?.definition else {
        return Err("shipped footprint".into());
    };
    assert_eq!(a.pads.len(), 18, "12 contacts, 4 stake slots, 2 peg holes");
    let mut pads = 0.0f64;
    for (p, q) in a.pads.iter().zip(&b.pads) {
        let (x, y) = (json_of(p)?, json_of(q)?);
        pads = pads.max(diff(&x, &y).ok_or_else(|| format!("pad {}: {x} / {y}", p.number))?);
    }
    assert_eq!(a.pads.len(), b.pads.len());
    let mut drawings = 0.0f64;
    assert_eq!(a.graphics.len(), b.graphics.len(), "drawings");
    for (p, q) in a.graphics.iter().zip(&b.graphics) {
        let (x, y) = (json_of(p)?, json_of(q)?);
        drawings = drawings.max(diff(&x, &y).ok_or_else(|| format!("drawing: {x} / {y}"))?);
    }
    // Symbol, component terminals and prefix, binding.
    let Definition::Symbol(a) = &code_symbol else {
        return Err("symbol".into());
    };
    let Definition::Symbol(b) = &record(&bundle, DefinitionKind::Symbol)?.definition else {
        return Err("shipped symbol".into());
    };
    let symbol = diff(&json_of(&a.units)?, &json_of(&b.units)?).ok_or("symbol units")?;
    let Definition::Component(a) = &code_component else {
        return Err("component".into());
    };
    let Definition::Component(b) = &record(&bundle, DefinitionKind::Component)?.definition else {
        return Err("shipped component".into());
    };
    assert_eq!(json_of(&a.terminals)?, json_of(&b.terminals)?, "terminals");
    assert_eq!(a.designator_prefix, b.designator_prefix, "prefix");
    assert_eq!(json_of(&a.fields)?, json_of(&b.fields)?, "fields");
    let Definition::Binding(a) = &code_binding else {
        return Err("binding".into());
    };
    let Definition::Binding(b) = &record(&bundle, DefinitionKind::Binding)?.definition else {
        return Err("shipped binding".into());
    };
    assert_eq!(json_of(&a.mappings)?, json_of(&b.mappings)?, "binding");
    // Package: terminals, envelope; the opening datum the part file
    // declares against the form's.
    let Definition::Package(a) = &code_package else {
        return Err("package".into());
    };
    let Definition::Package(b) = &record(&bundle, DefinitionKind::Package)?.definition else {
        return Err("shipped package".into());
    };
    assert_eq!(
        json_of(&a.terminals)?,
        json_of(&b.terminals)?,
        "package terminals"
    );
    assert_eq!(a.envelope_nm, b.envelope_nm, "envelope");
    assert_eq!(b.form, PackageForm::Generic);
    let (Some(da), Some(db)) = (a.datum("opening"), b.datum("opening")) else {
        return Err("opening datums".into());
    };
    let datum = diff(&json_of(&da.geometry)?, &json_of(&db.geometry)?).ok_or("datum")?;
    assert!(datum <= 1.0, "opening within 1 nm: {datum}");
    assert_eq!(
        (da.external, da.clearance_nm),
        (db.external, db.clearance_nm)
    );
    // The shipped package's part file, as is, against the builder (the
    // template's form); and a stored form off the row.
    let PackageModel::PartFile(model) = &b.model else {
        return Err("part file model".into());
    };
    assert!(model.values.is_empty(), "the file's own values");
    // Its lead bodies bound by the footprint's pads as the builder bound
    // them: each tail to its land, the four stakes to the shell, the shell
    // itself to none.
    let leads: BTreeMap<&str, &str> = model
        .derived
        .leads
        .iter()
        .map(|l| (l.body.as_str(), l.terminal.as_str()))
        .collect();
    assert_eq!(leads.len(), 16, "{leads:?}");
    assert_eq!(leads.get("tail 1"), Some(&"A1B12"));
    assert_eq!(leads.get("tail 12"), Some(&"B1A12"));
    assert!((1..=4).all(|k| leads.get(format!("stake {k}").as_str()) == Some(&"S")));
    assert!(!leads.contains_key("shell"));
    let (bounds, volume, differ, file_ms, code_ms) = models_of(UsbCForm::typical())?;
    let mut off = UsbCForm::typical();
    off.depth_nm = sparksmith_core::units::Nm::from_mm_f64_rounded(8.0)?;
    off.opening_nm[0] = sparksmith_core::units::Nm::from_mm_f64_rounded(8.2)?;
    off.pitch_nm = sparksmith_core::units::Nm::from_mm_f64_rounded(0.45)?;
    off.tabs[1].y_nm = sparksmith_core::units::Nm::from_mm_f64_rounded(3.3)?;
    let (off_bounds, off_volume, _, _, _) = models_of(off)?;
    // The shipped package's part file at no values is the typical form's.
    let part = part_file::evaluate(&std::fs::read(part_path())?, &Default::default())?;
    assert!(part.refusal().is_none(), "{:?}", part.report().findings);
    println!(
        "USB-C parity: pads within {pads} nm, drawings within {drawings} nm, symbol within \
         {symbol} nm, opening datum within {datum} nm; 3D bounds within {bounds:.3e} mm, \
         volumes within {volume:.3e} relative (off the row: {off_bounds:.3e} mm, \
         {off_volume:.3e}); listed: {differ:?}; cold model: part file {file_ms:.0} ms, \
         builder {code_ms:.0} ms"
    );
    assert!(pads <= 1.0 && drawings <= 1.0 && symbol <= 1.0);
    Ok(())
}
