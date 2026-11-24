//! The built-in part files: each `.solidcraft` in the shipped library's
//! `parts/` folder with its sidecar `NAME.part.json` (title, what it is,
//! designator prefix, whether it is a mechanical part or a package body,
//! land rules over its parameters, licence and provenance). They are read
//! once, at first use, through [`ShippedLibrary`]; nothing about them is
//! compiled in. A missing or broken file is an error naming it.
//!
//! The sidecar, format 1:
//!
//! ```json
//! {"format": 1, "name": "smd-nut", "title": "SMD nut",
//!  "summary": "Surface-mount threaded standoff nut: …",
//!  "use": "mechanical", "prefix": "H",
//!  "lands": [{"annulus": {"outer": {"param": "outer_d", "add_mm": 0.6},
//!                         "hole": {"param": "thread_d", "add_mm": 0.2}}}],
//!  "licence": "MIT OR Apache-2.0", "provenance": "Original work, …"}
//! ```
//!
//! `use` is `mechanical` (placed on the board), `package_body` (a
//! package's 3D model) or `component` (a whole shipped component whose
//! package uses this part file as its body); a land
//! is `annulus` (a round copper land around a non-plated hole at the
//! origin) or `hole` (a bare non-plated hole), each diameter `scale ×
//! value(param) + add_mm` over a length parameter of the part (its value
//! in mm; `scale` defaults to 1).
//!
//! A `component` part has a second sidecar, `NAME.component.json`, format
//! 1: `{"format": 1, "bundle": BUNDLE}`, the component's revision records
//! as `library.place_from` carries them (symbol, package, footprint,
//! binding, the part file's Model and, as the root, the manufacturer Part
//! or the component) with `"assets": []`. The loader packs the part file
//! as the bundle's one asset: every record names exactly that file.
use crate::Result;
use serde::Deserialize;
use sparksmith_core::DefinitionKind;
use sparksmith_core::library::bundle::{self, AssetBytes, PackedAsset};
use sparksmith_core::library::{Bundle, Definition};
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::path::Path;
use std::sync::{Arc, OnceLock};

/// The one sidecar format until the first release.
pub const SIDECAR_FORMAT: u32 = 1;
/// The suffix of a part file's sidecar.
pub const SIDECAR_SUFFIX: &str = ".part.json";
/// The suffix of a part file.
pub const PART_SUFFIX: &str = ".solidcraft";
/// The suffix of a component part's records.
pub const COMPONENT_SUFFIX: &str = ".component.json";
/// Largest component records file read, bytes.
const MAX_COMPONENT_BYTES: u64 = 4 * 1024 * 1024;
/// Largest sidecar read, bytes.
const MAX_SIDECAR_BYTES: u64 = 64 * 1024;
/// Largest built-in part file read, bytes.
const MAX_PART_BYTES: u64 = 64 * 1024 * 1024;
/// Most built-in part files read.
const MAX_PARTS: usize = 1024;
/// Longest name, title or prefix, characters.
const MAX_LABEL: usize = 200;
/// Longest summary or provenance, characters.
const MAX_TEXT: usize = 4096;
/// Most land rules of one part.
const MAX_LANDS: usize = 64;

/// `diameter = scale × value(param) + add_mm` over a part's own length
/// parameter (its value in mm).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Term {
    pub param: String,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub add_mm: f64,
}

fn one() -> f64 {
    1.0
}

/// A built-in part's land rule: shipped data over the part's parameters
/// (the part file never supplies lands).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum LandRule {
    /// A round copper land around a non-plated hole, at the origin.
    Annulus { outer: Term, hole: Term },
    /// A bare non-plated hole at the origin.
    Hole { diameter: Term },
}

/// What a built-in part file is used as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartUse {
    /// Placed on the board as a mechanical part.
    Mechanical,
    /// A package's 3D model.
    PackageBody,
    /// A whole component, placed in the schematic: its package's 3D body.
    Component,
}

/// `NAME.component.json` as stored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComponentFile {
    format: u32,
    bundle: Bundle,
}

/// A sidecar as stored.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sidecar {
    format: u32,
    name: String,
    title: String,
    summary: String,
    #[serde(rename = "use")]
    part_use: PartUse,
    prefix: String,
    #[serde(default)]
    lands: Vec<LandRule>,
    licence: String,
    provenance: String,
}

/// A built-in part file: its bytes and its sidecar's data.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltinPart {
    /// The file's stem, the name requests give.
    pub name: String,
    pub title: String,
    /// What it is, in a sentence or two.
    pub summary: String,
    pub prefix: String,
    /// Whether it is a mechanical part (placed on the board) rather than a
    /// package body (used by a package) or a component's body.
    pub mechanical: bool,
    /// A component part's records with the part file packed: what
    /// `library.place_from` imports and places. `None` for the others.
    pub component: Option<Arc<Bundle>>,
    pub lands: Vec<LandRule>,
    /// SPDX licence of the file.
    pub licence: String,
    pub provenance: String,
    /// The `.solidcraft` file's bytes.
    pub bytes: Arc<[u8]>,
}

impl BuiltinPart {
    /// The file name it is packed under.
    pub fn file_name(&self) -> String {
        format!("{}{PART_SUFFIX}", self.name)
    }
}

/// The shipped built-in part files, sorted by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuiltinParts {
    parts: Vec<BuiltinPart>,
}

impl BuiltinParts {
    /// Read every part file in `shipped`'s `parts/` folder with its sidecar.
    pub fn load(shipped: &ShippedLibrary) -> Result<Self> {
        let folder = shipped.shelf(Shelf::Parts);
        let files = shipped.list(Shelf::Parts, PART_SUFFIX)?;
        let sidecars = shipped.list(Shelf::Parts, SIDECAR_SUFFIX)?;
        let components = shipped.list(Shelf::Parts, COMPONENT_SUFFIX)?;
        if files.len() > MAX_PARTS {
            return Err(invalid(format!(
                "{}: more than {MAX_PARTS} part files",
                folder.display()
            )));
        }
        for sidecar in &sidecars {
            let stem = sidecar.trim_end_matches(SIDECAR_SUFFIX);
            if !files
                .iter()
                .any(|f| f.trim_end_matches(PART_SUFFIX) == stem)
            {
                return Err(invalid(format!(
                    "{}: no part file {stem}{PART_SUFFIX} beside it",
                    folder.join(sidecar).display()
                )));
            }
        }
        let parts = files
            .iter()
            .map(|file| load_part(shipped, &folder, file))
            .collect::<Result<Vec<_>>>()?;
        for records in &components {
            let stem = records.trim_end_matches(COMPONENT_SUFFIX);
            if !parts
                .iter()
                .any(|p| p.name == stem && p.component.is_some())
            {
                return Err(invalid(format!(
                    "{}: no component part {stem} beside it (a part file whose sidecar says \"use\": \"component\")",
                    folder.join(records).display()
                )));
            }
        }
        Ok(Self { parts })
    }

    /// Every built-in part file, by name.
    pub fn all(&self) -> &[BuiltinPart] {
        &self.parts
    }

    /// The built-in part file `name`.
    pub fn get(&self, name: &str) -> Option<&BuiltinPart> {
        self.parts.iter().find(|p| p.name == name)
    }

    /// The built-in part files a search text names: every word of `query`
    /// is in the part's name, title or summary, in any letter case. An
    /// empty text names none.
    pub fn find(&self, query: &str) -> Vec<&BuiltinPart> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return Vec::new();
        }
        self.parts
            .iter()
            .filter(|part| {
                let text = format!("{} {} {}", part.name, part.title, part.summary).to_lowercase();
                words.iter().all(|word| text.contains(word.as_str()))
            })
            .collect()
    }

    /// The shipped component whose bundle is of `library` with root
    /// `root`: what a palette row or a place request names.
    pub fn component(
        &self,
        library: sparksmith_core::ids::LibraryId,
        root: bundle::SourceRevision,
    ) -> Option<&Arc<Bundle>> {
        self.parts
            .iter()
            .filter_map(|p| p.component.as_ref())
            .find(|b| b.library == library && b.root == root)
    }

    /// The built-in `name`, or an error listing the ones there are.
    pub fn require(&self, name: &str) -> Result<&BuiltinPart> {
        self.get(name).ok_or_else(|| {
            let known: Vec<&str> = self.parts.iter().map(|p| p.name.as_str()).collect();
            crate::Error::Document(sparksmith_core::Error::Invalid(format!(
                "part_file.read: no built-in part file {name}; built-ins: {}",
                known.join(", ")
            )))
        })
    }
}

fn load_part(shipped: &ShippedLibrary, folder: &Path, file: &str) -> Result<BuiltinPart> {
    let stem = file.trim_end_matches(PART_SUFFIX);
    let sidecar_name = format!("{stem}{SIDECAR_SUFFIX}");
    let sidecar_path = folder.join(&sidecar_name);
    let named = |why: String| invalid(format!("{}: {why}", sidecar_path.display()));
    let text = shipped
        .read(Shelf::Parts, &sidecar_name, MAX_SIDECAR_BYTES)
        .map_err(|e| named(format!("the part file {file} needs this sidecar: {e}")))?;
    let sidecar: Sidecar = serde_json::from_slice(&text).map_err(|e| named(e.to_string()))?;
    if sidecar.format != SIDECAR_FORMAT {
        return Err(named(format!(
            "format {} is not supported (this version reads format {SIDECAR_FORMAT})",
            sidecar.format
        )));
    }
    if sidecar.name != stem {
        return Err(named(format!(
            "its name is {:?}; it must be the part file's name {stem:?}",
            sidecar.name
        )));
    }
    for (field, value, max) in [
        ("title", &sidecar.title, MAX_LABEL),
        ("prefix", &sidecar.prefix, MAX_LABEL),
        ("licence", &sidecar.licence, MAX_LABEL),
        ("summary", &sidecar.summary, MAX_TEXT),
        ("provenance", &sidecar.provenance, MAX_TEXT),
    ] {
        if value.trim().is_empty() || value.chars().count() > max {
            return Err(named(format!("{field}: 1 to {max} characters")));
        }
    }
    if sidecar.lands.len() > MAX_LANDS {
        return Err(named(format!("lands: at most {MAX_LANDS}")));
    }
    if sidecar.part_use != PartUse::Mechanical && !sidecar.lands.is_empty() {
        return Err(named(
            "lands: a package body takes its package's footprint, not lands".into(),
        ));
    }
    let terms = sidecar.lands.iter().flat_map(|rule| match rule {
        LandRule::Annulus { outer, hole } => vec![outer, hole],
        LandRule::Hole { diameter } => vec![diameter],
    });
    for term in terms {
        if term.param.trim().is_empty() || !term.scale.is_finite() || !term.add_mm.is_finite() {
            return Err(named(
                "lands: each term names a parameter and has finite numbers".into(),
            ));
        }
    }
    let bytes = shipped
        .read(Shelf::Parts, file, MAX_PART_BYTES)
        .map_err(|e| invalid(format!("{}: {e}", folder.join(file).display())))?;
    let component = match sidecar.part_use {
        PartUse::Component => Some(Arc::new(load_component(shipped, folder, stem, &bytes)?)),
        PartUse::Mechanical | PartUse::PackageBody => None,
    };
    Ok(BuiltinPart {
        name: sidecar.name,
        title: sidecar.title,
        summary: sidecar.summary,
        prefix: sidecar.prefix,
        mechanical: sidecar.part_use == PartUse::Mechanical,
        component,
        lands: sidecar.lands,
        licence: sidecar.licence,
        provenance: sidecar.provenance,
        bytes: bytes.into(),
    })
}

/// A component part's records (`NAME.component.json`) with the part file
/// `bytes` packed as the one asset its records name.
fn load_component(
    shipped: &ShippedLibrary,
    folder: &Path,
    stem: &str,
    bytes: &[u8],
) -> Result<Bundle> {
    let name = format!("{stem}{COMPONENT_SUFFIX}");
    let named = |why: String| invalid(format!("{}: {why}", folder.join(&name).display()));
    let text = shipped
        .read(Shelf::Parts, &name, MAX_COMPONENT_BYTES)
        .map_err(|e| named(format!("the component part {stem} needs this file: {e}")))?;
    let file: ComponentFile = serde_json::from_slice(&text).map_err(|e| named(e.to_string()))?;
    if file.format != SIDECAR_FORMAT {
        return Err(named(format!(
            "format {} is not supported (this version reads format {SIDECAR_FORMAT})",
            file.format
        )));
    }
    let mut bundle = file.bundle;
    if !bundle.assets.is_empty() {
        return Err(named(
            "assets: the part file beside it is the one asset; list none".into(),
        ));
    }
    let hash = bundle::asset_hash(bytes);
    let mut media_type = None;
    for record in &bundle.records {
        let assets = bundle::named_assets(&record.definition).map_err(|e| named(e.to_string()))?;
        if assets.iter().any(|h| *h != hash) {
            return Err(named(format!(
                "‘{}’ names an asset other than {stem}{PART_SUFFIX} (sha256 {hash})",
                record.meta.name
            )));
        }
        if let Definition::Model(model) = &record.definition {
            media_type = Some(model.asset.media_type.clone());
        }
    }
    let media_type = media_type.ok_or_else(|| {
        named(format!(
            "no Model record names {stem}{PART_SUFFIX} (sha256 {hash})"
        ))
    })?;
    let root = bundle
        .records
        .iter()
        .find(|r| r.lineage == bundle.root.lineage && r.revision == bundle.root.revision)
        .ok_or_else(|| named("its root is not one of its records".into()))?;
    if !matches!(root.kind, DefinitionKind::Component | DefinitionKind::Part) {
        return Err(named(
            "its root must be a component or a manufacturer Part".into(),
        ));
    }
    bundle.assets.push(PackedAsset {
        hash,
        media_type,
        data: AssetBytes::new(bytes.to_vec()),
    });
    bundle::validate_bundle(&bundle).map_err(|e| named(e.to_string()))?;
    Ok(bundle)
}

fn invalid(message: String) -> crate::Error {
    crate::Error::File(sparksmith_io::Error::Invalid(message))
}

/// The shipped built-in part files, read on first use from the located
/// shipped library and kept for the process.
pub fn builtin_parts() -> Result<&'static BuiltinParts> {
    static LOADED: OnceLock<std::result::Result<BuiltinParts, String>> = OnceLock::new();
    LOADED
        .get_or_init(|| {
            ShippedLibrary::locate()
                .map_err(crate::Error::from)
                .and_then(|shipped| BuiltinParts::load(&shipped))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|why| invalid(why.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn the_shipped_parts_load_with_their_sidecars() -> TestResult {
        let parts = builtin_parts()?;
        let names: Vec<&str> = parts.all().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names.len(), 5, "{names:?}");
        for part in parts.all() {
            assert!(!part.bytes.is_empty() && !part.title.is_empty());
        }
        let mechanical = parts.all().iter().filter(|p| p.mechanical).count();
        assert_eq!(mechanical, 3);
        assert!(parts.require("no-such-part").is_err());
        // The USB-C receptacle is a whole component: its records with the
        // part file packed as their one asset, the GCT Part as the root.
        let usb = parts.require("gct-usb4105")?;
        let bundle = usb.component.as_ref().ok_or("a component part")?;
        assert_eq!(bundle.assets.len(), 1);
        assert_eq!(bundle.assets[0].hash, bundle::asset_hash(&usb.bytes));
        let root = bundle
            .records
            .iter()
            .find(|r| r.lineage == bundle.root.lineage)
            .ok_or("root")?;
        assert_eq!(root.kind, DefinitionKind::Part);
        assert_eq!(
            parts.component(bundle.library, bundle.root).map(|b| b.root),
            Some(bundle.root)
        );
        Ok(())
    }

    #[test]
    fn a_component_part_names_only_its_part_file() -> TestResult {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("library");
        let parts = root.join("parts");
        std::fs::create_dir_all(&parts)?;
        std::fs::write(root.join(sparksmith_io::shipped::MARKER), "{\"format\":1}")?;
        let shipped = ShippedLibrary::at(&root)?;
        let real = builtin_parts()?.require("gct-usb4105")?;
        let bundle = real.component.as_ref().ok_or("component")?;
        let mut stored = Bundle::clone(bundle);
        stored.assets.clear();
        let records = serde_json::json!({"format": 1, "bundle": stored}).to_string();
        let sidecar = r#"{"format": 1, "name": "usb", "title": "USB", "summary": "A receptacle.",
            "use": "component", "prefix": "J", "licence": "MIT", "provenance": "Invented."}"#;
        std::fs::write(parts.join("usb.part.json"), sidecar)?;
        // Other bytes than the records name: refused, naming the file.
        std::fs::write(parts.join("usb.solidcraft"), b"other bytes")?;
        std::fs::write(parts.join("usb.component.json"), &records)?;
        let error = BuiltinParts::load(&shipped).err().ok_or("refused")?;
        assert!(error.to_string().contains("usb.component.json"), "{error}");
        // The file it names: loaded, packed.
        std::fs::write(parts.join("usb.solidcraft"), &*real.bytes)?;
        let loaded = BuiltinParts::load(&shipped)?;
        assert!(loaded.require("usb")?.component.is_some());
        // Records without their component part: refused.
        std::fs::write(parts.join("orphan.component.json"), &records)?;
        let error = BuiltinParts::load(&shipped).err().ok_or("refused")?;
        assert!(
            error.to_string().contains("orphan.component.json"),
            "{error}"
        );
        Ok(())
    }

    /// E2E run 6, finding 9: the shipped parts are found by the words a
    /// designer types, in any letter case, every word counting.
    #[test]
    fn the_shipped_parts_are_found_by_their_words() -> TestResult {
        let parts = builtin_parts()?;
        let names = |query: &str| -> Vec<String> {
            parts.find(query).iter().map(|p| p.name.clone()).collect()
        };
        assert_eq!(names("esp32"), ["esp32-s31-wroom-3"]);
        assert_eq!(names("ESP32 module"), ["esp32-s31-wroom-3"]);
        assert_eq!(names("heatsink"), ["finned-heatsink"]);
        assert_eq!(names("spacer"), ["round-spacer"]);
        assert_eq!(names("nut"), ["smd-nut"]);
        for usb in ["usb-c", "USB C", "type-c", "usb connector", "receptacle"] {
            assert_eq!(names(usb), ["gct-usb4105"], "{usb}");
        }
        assert!(names("heatsink nut").is_empty(), "every word counts");
        assert!(names("10k 0603").is_empty());
        assert!(names("  ").is_empty());
        Ok(())
    }

    #[test]
    fn a_missing_or_broken_sidecar_is_named() -> TestResult {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("library");
        let parts = root.join("parts");
        std::fs::create_dir_all(&parts)?;
        std::fs::write(root.join(sparksmith_io::shipped::MARKER), "{\"format\":1}")?;
        let shipped = ShippedLibrary::at(&root)?;
        assert!(BuiltinParts::load(&shipped)?.all().is_empty());

        std::fs::write(parts.join("nut.solidcraft"), b"x")?;
        let error = BuiltinParts::load(&shipped).err().ok_or("refused")?;
        assert!(error.to_string().contains("nut.part.json"), "{error}");

        let good = r#"{"format": 1, "name": "nut", "title": "Nut", "summary": "A nut.",
            "use": "mechanical", "prefix": "H", "licence": "MIT", "provenance": "Invented.",
            "lands": [{"hole": {"diameter": {"param": "d"}}}]}"#;
        for (broken, why) in [
            (good.replace("\"format\": 1", "\"format\": 2"), "format 2"),
            (
                good.replace("\"name\": \"nut\"", "\"name\": \"bolt\""),
                "bolt",
            ),
            (
                good.replace("\"title\": \"Nut\"", "\"title\": \" \""),
                "title",
            ),
            (good.replace("mechanical", "package_body"), "package body"),
            (good.replace("\"param\"", "\"parameter\""), "unknown field"),
        ] {
            std::fs::write(parts.join("nut.part.json"), &broken)?;
            let error = BuiltinParts::load(&shipped).err().ok_or("refused")?;
            let text = error.to_string();
            assert!(
                text.contains("nut.part.json") && text.contains(why),
                "{text}"
            );
        }
        std::fs::write(parts.join("nut.part.json"), good)?;
        let loaded = BuiltinParts::load(&shipped)?;
        let nut = loaded.require("nut")?;
        assert_eq!(
            nut.lands,
            vec![LandRule::Hole {
                diameter: Term {
                    param: "d".into(),
                    scale: 1.0,
                    add_mm: 0.0
                }
            }]
        );
        assert_eq!(&*nut.bytes, b"x");

        std::fs::write(parts.join("orphan.part.json"), good)?;
        let error = BuiltinParts::load(&shipped).err().ok_or("refused")?;
        assert!(error.to_string().contains("orphan.part.json"), "{error}");
        Ok(())
    }
}
