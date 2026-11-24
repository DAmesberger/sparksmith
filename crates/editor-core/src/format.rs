//! The `.sparksmith` document format: version check, validation and bounded
//! encoding. This module is pure; reading and writing files belongs to
//! `sparksmith-io`.
//!
//! Versions: until the first release the format is version 1
//! ([`FILE_VERSION`]) and keeps no backward compatibility. A stored-type
//! change edits version 1 in place, and a file of any other version is
//! refused.
//!
//! The project tree that replaces the single document (docs/file-format.md)
//! is built from [`files`] (the tree in memory), [`layout`] (the two file
//! layouts), [`records`] (declared field order and sort keys) and
//! [`table`] (the table reply shape). Nothing saves through them yet.
pub mod files;
pub mod layout;
pub mod order;
pub mod records;
pub mod room;
pub mod table;

use crate::diagnostics::{Error, Result};
use crate::library::Drafts;
use crate::library::bundle::AssetBytes;
use crate::model::{Document, Project, sheet::PhysicalPart};
use crate::validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::Arc;

pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// The one format version this build reads and writes. Until the first
/// release a stored-type change edits it in place.
pub const FILE_VERSION: u32 = 1;
pub const EXTENSION: &str = "sparksmith";

/// Whether `extension` (without the dot) names a project file, in any
/// letter case.
pub fn is_project_extension(extension: &str) -> bool {
    extension.eq_ignore_ascii_case(EXTENSION)
}

/// Whether `path` names a project file: the one predicate the platform's
/// file arrivals and persistence both use, so a file recognised as a
/// project is never refused for its name.
pub fn is_project_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(is_project_extension)
}

/// The `.sparksmith` file is a ZIP container (D0, founder decision F5). It
/// holds [`CONTAINER_DOCUMENT`], the editor file with every packed asset's
/// bytes left out, and one raw entry per asset at
/// [`CONTAINER_ASSET_PREFIX`]`<sha256>`. Recovery snapshots use the same
/// container with one more entry, [`CONTAINER_RECOVERY`]. Nothing else may
/// appear in a container: an unknown entry is refused, never dropped.
pub const CONTAINER_DOCUMENT: &str = "project.json";
pub const CONTAINER_ASSET_PREFIX: &str = "assets/sha256/";
/// The recovery metadata entry of a recovery snapshot.
pub const CONTAINER_RECOVERY: &str = "recovery.json";
/// Bytes of one container asset (target).
pub const CONTAINER_MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes of one container, uncompressed (target).
pub const CONTAINER_MAX_BYTES: u64 = 512 * 1024 * 1024;
/// Entries one container holds.
pub const CONTAINER_MAX_ENTRIES: usize = 4096;
/// Bytes of the recovery metadata entry.
pub const CONTAINER_MAX_RECOVERY_BYTES: usize = 64 * 1024;
/// Bytes of a container's document, [`CONTAINER_DOCUMENT`], as it is read
/// and as it is written: one bound, so a document the container admits is
/// one it can write again. It is larger than [`MAX_FILE_BYTES`] because the
/// saved form is indented and a document read may be compact.
pub const CONTAINER_MAX_DOCUMENT_BYTES: usize = crate::model::DOCUMENT_BYTES;
const _: () = assert!(CONTAINER_MAX_DOCUMENT_BYTES as u64 <= CONTAINER_MAX_BYTES);
const _: () = assert!(crate::library::bundle::MAX_ASSETS + 2 <= CONTAINER_MAX_ENTRIES);

/// Validate `file` and encode the container's document: the editor file
/// with every packed asset's bytes empty, since each asset is an entry of
/// its own. The document stays bounded by [`CONTAINER_MAX_DOCUMENT_BYTES`].
pub fn encode_container_document(file: &EditorFile) -> Result<Vec<u8>> {
    file.validate()?;
    let bytes = encode_within(
        &Stored::of(&without_asset_bytes(file)),
        CONTAINER_MAX_DOCUMENT_BYTES,
    )?;
    // A document that would not fit once a first save identifies it is
    // one reading refuses, so it is not written either: near the limit,
    // measure it as that save would write it.
    if file.project.production.project_id.is_none()
        && bytes.len().saturating_add(room::IDENTITY_SLACK) > CONTAINER_MAX_DOCUMENT_BYTES
    {
        room::check_file(file)?;
    }
    Ok(bytes)
}

/// The bytes [`encode_container_document`] would write for an already
/// validated `file`, counted without keeping them, refused past `limit`.
/// Reading a container measures the document it admits with this, so a
/// project that opens is one that saves and that recovery can keep.
pub fn container_document_len(file: &EditorFile, limit: usize) -> Result<usize> {
    struct Counted {
        len: usize,
        limit: usize,
    }
    impl Write for Counted {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit.saturating_sub(self.len) {
                return Err(std::io::Error::other("document exceeds its limit"));
            }
            self.len += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counted = Counted { len: 0, limit };
    serde_json::to_writer_pretty(&mut counted, &Stored::of(&without_asset_bytes(file)))
        .map_err(|e| encoding_error(e, limit))?;
    Ok(counted.len)
}

/// `file` with every packed asset's bytes left out (they share, so this
/// copies no asset).
fn without_asset_bytes(file: &EditorFile) -> EditorFile {
    let mut stripped = file.clone();
    for asset in &mut stripped.project.library.assets {
        asset.data = AssetBytes::default();
    }
    stripped
}

/// The saved form of an editor file, sorted by stable identity.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored<'a> {
    version: u32,
    project: Project,
    drafts: std::borrow::Cow<'a, Drafts>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    occurrence_parts: Vec<PhysicalPart>,
}

impl<'a> Stored<'a> {
    fn of(file: &'a EditorFile) -> Self {
        let mut project = file.project.clone();
        order::sort(&mut project);
        let mut occurrence_parts = Vec::new();
        let derived: std::collections::BTreeSet<_> = project
            .sheets
            .uses
            .iter()
            .flat_map(|u| &u.parts)
            .filter(|(local, id)| local != id)
            .map(|(_, id)| *id)
            .collect();
        if !derived.is_empty() {
            project.instances.retain(|i| {
                if !derived.contains(&i.id) {
                    return true;
                }
                occurrence_parts.push(PhysicalPart {
                    id: i.id,
                    designator: i.designator.clone(),
                    marking: i.marking.clone(),
                    text_placements: i.text_placements.clone(),
                    population: i.population,
                });
                false
            });
        }
        Self {
            version: file.version,
            project,
            drafts: std::borrow::Cow::Borrowed(&file.drafts),
            occurrence_parts,
        }
    }

    fn unpack(mut self) -> Result<EditorFile> {
        let project = &mut self.project;
        let sources: std::collections::BTreeMap<_, _> = project
            .sheets
            .uses
            .iter()
            .flat_map(|u| &u.parts)
            .filter(|(local, id)| local != id)
            .map(|(local, id)| (*id, *local))
            .collect();
        if sources.len() != self.occurrence_parts.len() {
            return Err(Error::Invalid(
                "Missing or extra physical parts for the sheet occurrences.".into(),
            ));
        }
        // Charge the shared data before duplicating it in the live board view.
        let mut bytes = 0usize;
        let mut sizes = std::collections::BTreeMap::new();
        for part in self.occurrence_parts {
            let local = sources
                .get(&part.id)
                .ok_or_else(|| Error::Invalid("Unknown occurrence part.".into()))?;
            let source = project
                .drawing_instance(*local)
                .ok_or_else(|| Error::Invalid("Missing sheet definition component.".into()))?;
            let size = match sizes.entry(*local) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    *entry.insert(serde_json::to_vec(source)?.len())
                }
                std::collections::btree_map::Entry::Occupied(entry) => *entry.get(),
            };
            bytes = bytes
                .checked_add(size)
                .filter(|n| *n <= CONTAINER_MAX_DOCUMENT_BYTES)
                .ok_or_else(|| {
                    Error::Limit("The expanded sheet components exceed the document size.".into())
                })?;
            let mut i = source.duplicate(part.id, part.designator);
            i.marking = part.marking;
            i.text_placements = part.text_placements;
            i.population = part.population;
            project.instances.push(i);
        }
        Ok(EditorFile {
            version: self.version,
            project: self.project,
            drafts: self.drafts.into_owned(),
        })
    }
}

/// Parse the container's document, refusing any other format version. It
/// is not validated: its packed assets have no bytes until the container's
/// asset entries fill them in, after which the caller validates the whole
/// file. A document that carries asset bytes inline is refused.
pub fn decode_container_document(bytes: &[u8]) -> Result<EditorFile> {
    let file = parse_versioned(bytes, CONTAINER_MAX_DOCUMENT_BYTES)?;
    if let Some(asset) = file
        .project
        .library
        .assets
        .iter()
        .find(|a| !a.data.is_empty())
    {
        return Err(Error::Invalid(format!(
            "{CONTAINER_DOCUMENT} carries the bytes of asset {} inline; asset bytes belong in {CONTAINER_ASSET_PREFIX}{}",
            asset.hash, asset.hash
        )));
    }
    Ok(file)
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EditorFile {
    pub version: u32,
    pub project: Project,
    /// Unfinished definitions, saved and recovered exactly as written.
    pub drafts: Drafts,
}

impl EditorFile {
    pub fn from_document(document: &Document) -> Self {
        Self {
            version: FILE_VERSION,
            project: (*document.project).clone(),
            drafts: (*document.drafts).clone(),
        }
    }
    pub fn into_document(mut self) -> Document {
        order::sort(&mut self.project);
        Document {
            project: Arc::new(self.project),
            drafts: Arc::new(self.drafts),
        }
    }
    pub fn validate(&self) -> Result<()> {
        check_version(u64::from(self.version))?;
        validate::document(&self.project, &self.drafts)
    }
}

/// `file` as saving and reopening gives it back: no fill copper (derived,
/// never saved), collections in key order. For tests that compare a
/// reopened document with the one saved.
#[cfg(test)]
pub(crate) fn as_saved(mut file: EditorFile) -> EditorFile {
    order::sort(&mut file.project);
    if file.project.fills.iter().any(|f| f.computed.is_some()) {
        for fill in file.project.fills.iter_mut() {
            fill.computed = None;
        }
    }
    file
}

/// Parse and validate a document of the current format version.
pub fn decode(bytes: &[u8]) -> Result<EditorFile> {
    let file = parse_versioned(bytes, MAX_FILE_BYTES)?;
    file.validate()?;
    Ok(file)
}

/// Parse a document of at most `limit` bytes of the current format
/// version, unvalidated.
fn parse_versioned(bytes: &[u8], limit: usize) -> Result<EditorFile> {
    if bytes.len() > limit {
        return Err(Error::Limit(format!(
            "editor file exceeds {} MiB",
            limit / (1024 * 1024)
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| Error::Invalid("editor file version is missing or invalid".into()))?;
    check_version(version)?;
    let mut file = serde_json::from_value::<Stored>(value)?.unpack()?;
    validate::adopt_single_use_sheets(&mut file.project)?;
    // A file this build wrote is in order already; one edited by hand is
    // read as if it were, since the order of its collections means nothing.
    order::sort(&mut file.project);
    Ok(file)
}

/// Refuses every version but [`FILE_VERSION`].
fn check_version(version: u64) -> Result<()> {
    if version != u64::from(FILE_VERSION) {
        return Err(Error::Unsupported(format!(
            "this file is format version {version}; this build of {} reads only version {FILE_VERSION}",
            crate::product::NAME
        )));
    }
    Ok(())
}

/// Validate and serialize a document.
pub fn encode(file: &EditorFile) -> Result<Vec<u8>> {
    file.validate()?;
    encode_bounded(&Stored::of(file))
}

/// Bound serialization itself rather than allocating an arbitrarily large
/// buffer and checking it afterwards.
pub fn encode_bounded(value: &impl Serialize) -> Result<Vec<u8>> {
    encode_within(value, MAX_FILE_BYTES)
}

/// [`encode_bounded`] with its own `limit` in bytes.
fn encode_within(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    struct Bounded(Vec<u8>, usize);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.1.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("file exceeds its limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Bounded(Vec::new(), limit);
    serde_json::to_writer_pretty(&mut buffer, value).map_err(|e| encoding_error(e, limit))?;
    Ok(buffer.0)
}

fn encoding_error(error: serde_json::Error, limit: usize) -> Error {
    if error.is_io() {
        Error::Limit(format!("editor file exceeds {} MiB", limit / (1024 * 1024)))
    } else {
        Error::Invalid(format!("the document cannot be written: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> Result<serde_json::Value> {
        let file = EditorFile::from_document(&Document::default());
        Ok(serde_json::from_slice(&encode(&file)?)?)
    }

    fn refusal(value: &serde_json::Value) -> Result<String> {
        match decode(&serde_json::to_vec(value)?) {
            Ok(_) => Ok("accepted".into()),
            Err(error) => Ok(error.to_string()),
        }
    }

    /// A file is written as the current version, and any other version is
    /// refused, naming both.
    #[test]
    fn another_version_is_refused() -> Result<()> {
        let mut value = saved()?;
        assert_eq!(value["version"], FILE_VERSION);
        assert_eq!(refusal(&value)?, "accepted");
        for other in [0, FILE_VERSION + 1, 5] {
            value["version"] = serde_json::json!(other);
            let message = refusal(&value)?;
            assert!(
                message.contains(&format!("format version {other};")),
                "{message}"
            );
            assert!(message.contains("reads only version 1"), "{message}");
        }
        Ok(())
    }

    /// A stored field the format does not have is refused, never dropped.
    #[test]
    fn an_unknown_field_is_refused() -> Result<()> {
        let mut value = saved()?;
        value["features"] = serde_json::json!(["teleport_vias"]);
        assert!(refusal(&value)?.contains("features"));
        Ok(())
    }

    /// F2: a document saves with its collections in key order, its fill
    /// copper left out and its waiver keys as JSON, and reopening and saving
    /// it again gives the same bytes.
    #[test]
    fn a_saved_document_is_ordered_derived_free_and_reads_back_byte_for_byte() -> Result<()> {
        use crate::model::production::Waiver;
        let engine = crate::Engine::showcase()?;
        let mut file = engine.to_file();
        assert!(file.project.fills.iter().any(|f| f.computed.is_some()));
        file.project.tracks.reverse();
        file.project.production.waivers = vec![Waiver {
            key: r#"board.clearance [{"kind":"track","id":4}]"#.into(),
            reason: "checked".into(),
            accepted: Vec::new(),
            footprints: Vec::new(),
            given_for: None,
        }];
        let bytes = encode(&file)?;
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert!(!String::from_utf8_lossy(&bytes).contains("\"computed\""));
        assert_eq!(
            value["project"]["production"]["waivers"][0]["key"],
            serde_json::json!({"code": "board.clearance", "objects": [{"kind": "track", "id": 4}]})
        );
        let ids: Vec<u64> = value["project"]["tracks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["id"].as_u64())
            .collect();
        assert!(ids.len() > 1 && ids.is_sorted());
        let reread = decode(&bytes)?;
        assert_eq!(reread, as_saved(file));
        assert_eq!(encode(&reread)?, bytes);
        Ok(())
    }

    /// A sketch is stored as a JSON object with sorted keys and reads back
    /// as that sorted text; a waiver key that is not a finding's cannot be
    /// written, and a key stored as a string is refused.
    #[test]
    fn sketches_and_waiver_keys_are_json_values() -> Result<()> {
        use crate::library::footprint_source::SketchBlob;
        use crate::model::production::Waiver;
        let blob = SketchBlob(
            r#"{"points":[{"id":"origin","pos":[0.0,0.0],"fixed":true}],"curves":[]}"#.into(),
        );
        let stored = serde_json::to_string(&blob)?;
        assert_eq!(
            stored,
            r#"{"curves":[],"points":[{"fixed":true,"id":"origin","pos":[0.0,0.0]}]}"#
        );
        let read: SketchBlob = serde_json::from_str(&stored)?;
        assert_eq!(read.0, stored);
        assert_eq!(serde_json::to_string(&read)?, stored);
        assert!(serde_json::from_str::<SketchBlob>(r#""{}""#).is_err());
        let bad = Waiver {
            key: "not a finding key".into(),
            reason: String::new(),
            accepted: Vec::new(),
            footprints: Vec::new(),
            given_for: None,
        };
        assert!(serde_json::to_string(&bad).is_err());
        assert!(serde_json::from_str::<Waiver>(r#"{"key":"fill.stale []","reason":""}"#).is_err());
        Ok(())
    }
}
