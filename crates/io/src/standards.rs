//! The standards from the shipped library (`docs/shipped-library.md`): the
//! standard fields, the marking-code tables, the generic-part search
//! grammar and the schematic symbol drawings. They are read here, checked
//! by the core's constructors and handed to every engine in a [`Context`],
//! so every engine (desktop, CLI, MCP) of one host uses the same data. A missing or broken file
//! is an error naming it.
use crate::shipped::{Shelf, ShippedLibrary};
use crate::{Error, Result};
use serde::de::DeserializeOwned;
use sparksmith_core::catalog::generic;
use sparksmith_core::library::categories::Categories;
use sparksmith_core::library::marking::codes::{
    ColourCode, Eia96Code, MarkingCodes, PreferredSeries, VoltageLetterTables,
};
use sparksmith_core::library::standard_fields::StandardFields;
use sparksmith_core::library::standards::{Context, ShippedGenerator, ShippedPart, Standards};
use sparksmith_core::library::symbol_style::{
    KindDrawing, SymbolDrawings, SymbolKind, SymbolStyle,
};
use std::sync::Arc;

/// The largest standards file read.
const MAX_FILE_BYTES: u64 = 1 << 20;

/// The standard fields bundle, as `library.import` takes it.
pub const STANDARD_FIELDS: (Shelf, &str) = (Shelf::StandardFields, "standard-fields.json");
/// What the standard fields are for.
pub const FIELD_ROLES: (Shelf, &str) = (Shelf::StandardFields, "roles.json");
/// The E96 preferred number series.
pub const E96: (Shelf, &str) = (Shelf::Standards, "e96.json");
/// The EIA-96 resistor code's multiplier letters.
pub const EIA96: (Shelf, &str) = (Shelf::Standards, "eia-96.json");
/// The named rated-voltage letter tables.
pub const VOLTAGE_LETTERS: (Shelf, &str) = (Shelf::Standards, "voltage-letters.json");
/// Differential pair name conventions and sourced targets.
pub const DIFFERENTIAL_PAIRS: (Shelf, &str) = (Shelf::Standards, "differential-pairs.json");
/// The resistor colour code.
pub const COLOUR_CODE: (Shelf, &str) = (Shelf::Standards, "colour-code.json");
/// The generic-part search grammar.
pub const GENERIC_FAMILIES: (Shelf, &str) = (Shelf::Catalog, "generic-families.json");
/// The part categories.
pub const CATEGORIES: (Shelf, &str) = (Shelf::Catalog, "categories.json");
/// The shared sizes of generated symbols.
pub const SYMBOL_STYLE: (Shelf, &str) = (Shelf::Standards, "symbols/style.json");

/// The path a message names for `name` on `shelf`.
fn shown(shipped: &ShippedLibrary, (shelf, name): (Shelf, &str)) -> String {
    shipped.shelf(shelf).join(name).display().to_string()
}

/// `file` read and parsed, any failure naming the file.
fn read<T: DeserializeOwned>(shipped: &ShippedLibrary, file: (Shelf, &str)) -> Result<T> {
    let (shelf, name) = file;
    let bytes = shipped
        .read(shelf, name, MAX_FILE_BYTES)
        .map_err(|e| Error::Invalid(format!("{}: {e}", shown(shipped, file))))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| Error::Invalid(format!("{}: {e}", shown(shipped, file))))
}

/// A core check of what `file` holds, failing with the file named.
fn checked<T>(
    shipped: &ShippedLibrary,
    file: (Shelf, &str),
    value: sparksmith_core::Result<T>,
) -> Result<T> {
    value.map_err(|e| Error::Invalid(format!("{}: {e}", shown(shipped, file))))
}

/// The drawing file of a drawn symbol kind: `symbols/<kind>.json`.
pub fn symbol_file(kind: SymbolKind) -> Result<String> {
    let value = serde_json::to_value(kind)?;
    let name = value
        .as_str()
        .ok_or_else(|| Error::Invalid("a symbol kind is not named by text".into()))?;
    Ok(format!("symbols/{name}.json"))
}

/// Every standard the shipped library holds, checked.
pub fn load(shipped: &ShippedLibrary) -> Result<Standards> {
    let fields = checked(
        shipped,
        STANDARD_FIELDS,
        StandardFields::new(read(shipped, STANDARD_FIELDS)?, read(shipped, FIELD_ROLES)?),
    )?;
    let e96: PreferredSeries = read(shipped, E96)?;
    checked(shipped, E96, e96.check())?;
    let eia96: Eia96Code = read(shipped, EIA96)?;
    checked(shipped, EIA96, eia96.check())?;
    let letters: VoltageLetterTables = read(shipped, VOLTAGE_LETTERS)?;
    checked(shipped, VOLTAGE_LETTERS, letters.check())?;
    let pairs: sparksmith_core::rules::signal::PairCatalog = read(shipped, DIFFERENTIAL_PAIRS)?;
    checked(shipped, DIFFERENTIAL_PAIRS, pairs.check())?;
    let colours: ColourCode = read(shipped, COLOUR_CODE)?;
    checked(shipped, COLOUR_CODE, colours.check())?;
    let marking = MarkingCodes::new(e96, eia96, letters, colours)?;
    let grammar = read(shipped, GENERIC_FAMILIES)?;
    checked(shipped, GENERIC_FAMILIES, generic::check(&grammar))?;
    let mut kinds = Vec::new();
    for kind in SymbolKind::ALL {
        if kind.terminal_count().is_none() {
            continue;
        }
        let name = symbol_file(kind)?;
        let file = (Shelf::Standards, name.as_str());
        let drawing: KindDrawing = read(shipped, file)?;
        if drawing.kind != kind {
            return Err(Error::Invalid(format!(
                "{}: it holds the {} drawing",
                shown(shipped, file),
                drawing.kind.label()
            )));
        }
        checked(shipped, file, drawing.check())?;
        kinds.push(drawing);
    }
    let style: SymbolStyle = read(shipped, SYMBOL_STYLE)?;
    checked(shipped, SYMBOL_STYLE, style.check())?;
    let symbols = SymbolDrawings::new(style, kinds)?;
    let categories: Categories = read(shipped, CATEGORIES)?;
    checked(shipped, CATEGORIES, categories.check())?;
    checked(
        shipped,
        GENERIC_FAMILIES,
        Standards::new(fields, marking, grammar, symbols, categories, pairs),
    )
}

/// The context of `shipped`: its standards, and its material library and
/// generator bundles or why those failed to load (kept, so a host without
/// 3D still runs).
pub fn context(shipped: &ShippedLibrary) -> Result<Arc<Context>> {
    let materials = crate::materials::load(shipped).map_err(|e| e.to_string());
    Ok(Arc::new(
        Context::new(load(shipped)?)
            .with_help(crate::help::load(shipped).map_err(|e| e.to_string()))
            .with_materials(materials)
            .with_generators(generators(shipped).map_err(|e| e.to_string()))
            .with_parts(parts(shipped).map_err(|e| e.to_string()))
            .with_showcase(showcase(shipped).map_err(|e| e.to_string()))
            .with_artwork(artwork(shipped)),
    ))
}

/// Most marking artwork files read.
const MAX_ARTWORK: usize = 512;

/// The installed marking artwork (`artwork/<id>.svg`) by id, the file's
/// stem (`a-z`, `0-9`, `-`, at most 64 bytes). Optional data: a missing
/// folder is none, and a file that is not a readable SVG of at most
/// [`crate::artwork::MAX_SVG_BYTES`] is left out (a template draws
/// without it), so a broken artwork file never stops the program.
pub fn artwork(shipped: &ShippedLibrary) -> std::collections::BTreeMap<String, Arc<str>> {
    const SUFFIX: &str = ".svg";
    let mut out = std::collections::BTreeMap::new();
    let Ok(files) = shipped.list(Shelf::Artwork, SUFFIX) else {
        return out;
    };
    for file in files.into_iter().take(MAX_ARTWORK) {
        let id = file.trim_end_matches(SUFFIX);
        let valid = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !valid {
            continue;
        }
        let Ok(text) = crate::artwork::read_svg(&shipped.shelf(Shelf::Artwork).join(&file)) else {
            continue;
        };
        out.insert(id.to_owned(), Arc::from(text));
    }
    out
}

/// Largest showcase parts file read, bytes.
const MAX_SHOWCASE_BYTES: u64 = 16 * 1024 * 1024;

/// The showcase's parts (`showcase/parts.json`).
fn showcase(
    shipped: &ShippedLibrary,
) -> Result<sparksmith_core::library::showcase_parts::ShowcaseParts> {
    let bytes = shipped.read(Shelf::Showcase, "parts.json", MAX_SHOWCASE_BYTES)?;
    let text = String::from_utf8(bytes)
        .map_err(|_| Error::Invalid("showcase/parts.json is not UTF-8".into()))?;
    Ok(sparksmith_core::library::showcase_parts::ShowcaseParts::parse(&text)?)
}

/// Largest shipped part file read, bytes.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// Most shipped part files read.
const MAX_PARTS: usize = 1024;

/// Every shipped part file (`parts/<name>.solidcraft`), by its stem.
fn parts(shipped: &ShippedLibrary) -> Result<Vec<ShippedPart>> {
    const SUFFIX: &str = ".solidcraft";
    let files = shipped.list(Shelf::Parts, SUFFIX)?;
    if files.len() > MAX_PARTS {
        return Err(Error::Invalid(format!(
            "{}: more than {MAX_PARTS} part files",
            shipped.shelf(Shelf::Parts).display()
        )));
    }
    files
        .into_iter()
        .map(|file| {
            let bytes = shipped.read(Shelf::Parts, &file, MAX_PART_BYTES)?;
            Ok(ShippedPart {
                name: file.trim_end_matches(SUFFIX).to_owned(),
                bytes: Arc::from(bytes),
            })
        })
        .collect()
}

/// Every shipped generator bundle, each file checked against its manifest.
fn generators(shipped: &ShippedLibrary) -> Result<Vec<ShippedGenerator>> {
    shipped
        .generators()?
        .into_iter()
        .map(|name| {
            let bundle = shipped.generator(&name)?;
            Ok(ShippedGenerator {
                name,
                module: Arc::from(bundle.module),
                parts: bundle
                    .parts
                    .into_iter()
                    .map(|p| (p.entry.sha256, Arc::from(p.bytes)))
                    .collect(),
            })
        })
        .collect()
}

/// Find the shipped library and make its context: what each program does
/// at startup, before it makes an engine, and hands to every engine.
pub fn shipped_context() -> Result<Arc<Context>> {
    context(&ShippedLibrary::locate()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn repository() -> Result<ShippedLibrary> {
        ShippedLibrary::for_checkout()
    }

    #[test]
    fn repository_library_resolves_to_the_checkout_or_override() -> Result<()> {
        // Each child gets its own environment, including when tests run in parallel.
        const EXPECTED: &str = "SPARKSMITH_TEST_EXPECTED_STANDARDS_ROOT";
        if let Some(expected) = std::env::var_os(EXPECTED) {
            assert_eq!(
                repository()?.root().canonicalize()?,
                Path::new(&expected).canonicalize()?
            );
            return Ok(());
        }
        let checkout = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
        let alternate = tempfile::tempdir()?;
        std::fs::write(alternate.path().join(crate::shipped::MARKER), "{}")?;
        for (override_dir, expected) in [
            (None, checkout.as_path()),
            (Some(alternate.path()), alternate.path()),
        ] {
            let mut child = std::process::Command::new(std::env::current_exe()?);
            child
                .args([
                    "--exact",
                    "standards::tests::repository_library_resolves_to_the_checkout_or_override",
                ])
                .env(EXPECTED, expected)
                .env_remove(crate::shipped::ENV);
            if let Some(dir) = override_dir {
                child.env(crate::shipped::ENV, dir);
            }
            let output = child.output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success() && stdout.contains("1 passed"),
                "{stdout}{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    /// A copy of the shipped library's standards in a temporary folder.
    fn copy(to: &Path) -> Result<ShippedLibrary> {
        let from = repository()?;
        std::fs::write(to.join(crate::shipped::MARKER), "{\"format\":1}\n")?;
        for shelf in [Shelf::StandardFields, Shelf::Standards, Shelf::Catalog] {
            copy_dir(&from.shelf(shelf), &to.join(shelf.dir_name()))?;
        }
        ShippedLibrary::at(to)
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), target)?;
            }
        }
        Ok(())
    }

    /// The error of a load that must fail.
    fn message(loaded: Result<Standards>) -> String {
        match loaded {
            Ok(_) => "loaded".into(),
            Err(error) => error.to_string(),
        }
    }

    #[test]
    fn the_shipped_standards_load_and_match_what_the_core_tests_use() -> Result<()> {
        let loaded = load(&repository()?)?;
        assert_eq!(&loaded, &**Context::fixture()?.standards());
        // The context of the repository carries the same standards.
        assert_eq!(&loaded, &**context(&repository()?)?.standards());
        Ok(())
    }

    #[test]
    fn a_missing_file_is_named() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let shipped = copy(temp.path())?;
        std::fs::remove_file(temp.path().join("standards/symbols/zener.json"))?;
        let error = message(load(&shipped));
        assert!(error.contains("zener.json"), "{error}");
        Ok(())
    }

    #[test]
    fn a_broken_file_is_named() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let shipped = copy(temp.path())?;
        std::fs::write(temp.path().join("standards/e96.json"), "{\"format\": 1")?;
        let error = message(load(&shipped));
        assert!(error.contains("e96.json"), "{error}");

        let shipped = copy(temp.path())?;
        let grammar = temp.path().join("catalog/generic-families.json");
        let text = std::fs::read_to_string(&grammar)?.replace("{value:ohm}?", "{value:watt}?");
        std::fs::write(&grammar, text)?;
        let error = message(load(&shipped));
        assert!(
            error.contains("generic-families.json") && error.contains("watt"),
            "{error}"
        );

        let shipped = copy(temp.path())?;
        let colours = temp.path().join("standards/colour-code.json");
        let text = std::fs::read_to_string(&colours)?.replace("\"digit\": 3", "\"digit\": 2");
        std::fs::write(&colours, text)?;
        let error = message(load(&shipped));
        assert!(error.contains("colour-code.json"), "{error}");
        Ok(())
    }

    #[test]
    fn pair_recognition_and_targets_follow_the_library() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let shipped = copy(temp.path())?;
        let file = temp.path().join("standards/differential-pairs.json");
        let mut catalog = serde_json::json!({
            "format": 1, "endings": [[".high", ".low"]],
            "interfaces": [{
                "target": { "interface": "Fixture", "ohms": 123.5, "tolerance_pct": 4.5,
                    "source": "Fixture routing table", "url": "https://example.org/fixture" },
                "names": [{"name": "Link", "positive": "Alpha", "negative": "Beta"}]
            }, {
                "names": [{"name": "Clock", "positive": "ClockA", "negative": "ClockB"}]
            }]
        });
        std::fs::write(&file, serde_json::to_vec(&catalog)?)?;
        let library = load(&shipped)?;
        let rows = library.pairs.suggest(
            &Default::default(),
            [
                (1, "Alpha"),
                (2, "Beta"),
                (3, "Wire.high"),
                (4, "Wire.low"),
                (5, "USB_D+"),
                (6, "USB_D-"),
                (7, "ClockA"),
                (8, "ClockB"),
            ],
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0].target.as_ref().map(|t| (t.ohms, t.tolerance_pct)),
            Some((123.5, 4.5))
        );
        assert!(rows[1..].iter().all(|s| s.target.is_none()));
        catalog["interfaces"][0]["target"]["source"] = "".into();
        std::fs::write(&file, serde_json::to_vec(&catalog)?)?;
        assert!(message(load(&shipped)).contains("differential-pairs.json"));
        Ok(())
    }

    /// Installed marking artwork is read by id; a missing folder, a bad
    /// name and a file that is not SVG text are left out quietly.
    #[test]
    fn marking_artwork_is_optional_data_by_id() -> Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join(crate::shipped::MARKER), "{\"format\":1}\n")?;
        let shipped = ShippedLibrary::at(dir.path())?;
        assert!(artwork(&shipped).is_empty());
        let shelf = dir.path().join("artwork");
        std::fs::create_dir_all(&shelf)?;
        std::fs::write(shelf.join("maker-mark.svg"), "<svg viewBox='0 0 1 1'/>")?;
        std::fs::write(shelf.join("Bad Name.svg"), "<svg/>")?;
        std::fs::write(shelf.join("binary.svg"), [0xff_u8, 0xfe])?;
        std::fs::write(shelf.join("notes.txt"), "not artwork")?;
        let read = artwork(&shipped);
        assert_eq!(read.keys().collect::<Vec<_>>(), ["maker-mark"]);
        let context = context(&repository()?)?;
        assert!(context.artwork("maker-mark").is_none());
        Ok(())
    }
}
