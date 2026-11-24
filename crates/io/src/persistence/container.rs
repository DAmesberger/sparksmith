//! The `.sparksmith` ZIP container (slice D0, founder decision F5).
//!
//! A container holds exactly:
//! - `project.json`: the editor file, every packed asset's bytes left empty;
//! - `assets/sha256/<hash>`: the raw bytes of each packed asset;
//! - in a recovery snapshot only, `recovery.json`: the recovery metadata.
//!
//! Entries are stored or deflated. Anything else (an unknown, duplicate,
//! directory, link or encrypted entry, another compression method, a
//! missing, unreferenced or mismatched asset) refuses the file and names
//! the entry: nothing is dropped silently. Every read is bounded by the
//! `sparksmith_core::format::CONTAINER_*` limits, measured on the bytes that
//! actually decompress rather than on the sizes the archive declares.
//! Writes go through the atomic path of [`super::atomic_write_with`].
use crate::{Error, Result};
use sparksmith_core::format::{
    self, CONTAINER_ASSET_PREFIX, CONTAINER_DOCUMENT, CONTAINER_MAX_ASSET_BYTES,
    CONTAINER_MAX_BYTES, CONTAINER_MAX_DOCUMENT_BYTES, CONTAINER_MAX_ENTRIES, CONTAINER_RECOVERY,
    EditorFile,
};
use sparksmith_core::ids::ContentHash;
use sparksmith_core::library::bundle::AssetBytes;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::Path;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// The first bytes of a ZIP local file header.
const ZIP_MAGIC: &[u8; 4] = b"PK\x03\x04";

/// Read and validate a project container.
pub fn read_container(path: &Path) -> Result<EditorFile> {
    Ok(read(path, Role::Project, Load::Everything)?.file)
}

/// [`read_container`] from a file already open.
pub(crate) fn read_open(file: File) -> Result<EditorFile> {
    Ok(read_file(file, Role::Project, Load::Everything)?.file)
}

/// Encode `file` (assets as raw entries) and atomically replace `path`.
pub fn write_container(path: &Path, file: &EditorFile) -> Result<()> {
    write(path, file, None, Method::Deflated)
}

/// Encode `file` and replace `path` while it holds `expected`
/// ([`super::save_project`]).
pub(crate) fn save_container(
    path: &Path,
    file: &EditorFile,
    expected: Option<super::DiskIdentity>,
) -> Result<super::DiskIdentity> {
    super::replace_checked(stage(path, file)?, path, expected)
}

/// Encode `file` in a temporary file beside `path`.
pub(crate) fn stage(path: &Path, file: &EditorFile) -> Result<tempfile::NamedTempFile> {
    let document = format::encode_container_document(file)?;
    check_total(&document, None, file)?;
    super::staged_with(path, super::Access::Shared, |target| {
        fill(target, &document, None, file, Method::Deflated)
    })
}

/// Encode `file` and create a container without replacing an existing file.
pub fn create_container(path: &Path, file: &EditorFile) -> Result<()> {
    write_with_mode(path, file, None, Method::Deflated, false)
}

/// Whether a container is a project or a recovery snapshot, which holds
/// one more entry.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Project,
    Recovery,
}

/// How much of a container to read.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Load {
    /// Every entry, every asset hash checked, the whole file validated.
    Everything,
    /// The document and metadata only, for listings: asset entries are
    /// checked by name and declared size but not read, and the document is
    /// parsed but not validated. Its packed assets keep empty bytes.
    Summary,
}

/// How asset entries are compressed. The document is always deflated.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Method {
    Deflated,
    /// Recovery snapshots store assets, so frequent snapshots stay cheap.
    Stored,
}

pub(crate) struct Contents {
    pub file: EditorFile,
    /// The `recovery.json` bytes of a recovery snapshot.
    pub recovery: Option<Vec<u8>>,
    /// A recovery listing sees these exact metadata and document bytes. The
    /// document names every asset by its content hash.
    pub recovery_identity: Option<ContentHash>,
    /// Bytes decompressed while reading.
    pub read_bytes: u64,
}

pub(crate) fn read(path: &Path, role: Role, load: Load) -> Result<Contents> {
    read_file(File::open(path)?, role, load)
}

pub(crate) fn read_file(mut file: File, role: Role, load: Load) -> Result<Contents> {
    let length = file.metadata()?.len();
    if length > CONTAINER_MAX_BYTES {
        return Err(Error::TooLarge(format!(
            "project container exceeds {} MiB",
            CONTAINER_MAX_BYTES / (1024 * 1024)
        )));
    }
    let mut magic = Vec::with_capacity(ZIP_MAGIC.len());
    (&mut file)
        .take(ZIP_MAGIC.len() as u64)
        .read_to_end(&mut magic)?;
    if magic != ZIP_MAGIC {
        return Err(Error::Invalid(
            concat!(
                "container.format: not a ",
                sparksmith_core::product_name!(),
                " project container (a .sparksmith file is a ZIP archive holding project.json)"
            )
            .into(),
        ));
    }
    file.rewind()?;
    let mut archive = ZipArchive::new(file).map_err(malformed)?;
    if archive.len() > CONTAINER_MAX_ENTRIES {
        return Err(Error::TooLarge(format!(
            "project container holds more than {CONTAINER_MAX_ENTRIES} entries"
        )));
    }
    if archive.has_duplicate_names() {
        return Err(Error::Invalid(
            "container.entry: the container names one entry twice".into(),
        ));
    }
    let mut document = None;
    let mut recovery = None;
    let mut assets: BTreeMap<ContentHash, AssetBytes> = BTreeMap::new();
    let mut declared: BTreeMap<ContentHash, u64> = BTreeMap::new();
    // The first asset whose bytes do not hash to its name: refused once
    // the document says what it is (a generator module is named as one).
    let mut mismatch: Option<(String, ContentHash, ContentHash)> = None;
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(malformed)?;
        let name = entry.name().map_err(malformed)?.into_owned();
        if entry.is_dir() || entry.is_symlink() {
            return Err(entry_error(&name, "is a directory or a link"));
        }
        if entry.encrypted() {
            return Err(entry_error(&name, "is encrypted"));
        }
        if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(entry_error(
                &name,
                "uses a compression method other than stored or deflated",
            ));
        }
        let kind = if name == CONTAINER_DOCUMENT {
            Entry::Document
        } else if name == CONTAINER_RECOVERY && role == Role::Recovery {
            Entry::Recovery
        } else if let Some(hash) = asset_entry_hash(&name) {
            Entry::Asset(hash)
        } else {
            return Err(entry_error(&name, "is not part of the project format"));
        };
        let limit = match kind {
            Entry::Document => CONTAINER_MAX_DOCUMENT_BYTES as u64,
            Entry::Recovery => format::CONTAINER_MAX_RECOVERY_BYTES as u64,
            Entry::Asset(_) => CONTAINER_MAX_ASSET_BYTES,
        };
        if entry.size() > limit {
            return Err(Error::TooLarge(format!(
                "container entry {name} exceeds {limit} bytes"
            )));
        }
        if let Entry::Asset(hash) = kind
            && load == Load::Summary
        {
            total = add_total(total, entry.size())?;
            declared.insert(hash, entry.size());
            continue;
        }
        let bytes = read_bounded(&mut entry, limit, &name)?;
        total = add_total(total, bytes.len() as u64)?;
        match kind {
            Entry::Document => document = Some(bytes),
            Entry::Recovery => recovery = Some(bytes),
            Entry::Asset(hash) => {
                // Hashed once here; the shared bytes remember it, so the
                // file's validation does not hash them again.
                let bytes = AssetBytes::new(bytes);
                if bytes.hash() != hash {
                    mismatch.get_or_insert((name.clone(), hash, bytes.hash()));
                    continue;
                }
                declared.insert(hash, bytes.len() as u64);
                assets.insert(hash, bytes);
            }
        }
    }
    let document = document.ok_or_else(|| {
        Error::Invalid(format!(
            "container.entry: the container has no {CONTAINER_DOCUMENT}"
        ))
    })?;
    if role == Role::Recovery && recovery.is_none() {
        return Err(Error::Invalid(format!(
            "container.entry: the recovery snapshot has no {CONTAINER_RECOVERY}"
        )));
    }
    let read_bytes = (document.len() as u64)
        .saturating_add(recovery.as_ref().map_or(0, |r| r.len() as u64))
        .saturating_add(assets.values().map(|a| a.len() as u64).sum());
    let decoded = format::decode_container_document(&document);
    if let Some((name, recorded, actual)) = mismatch {
        return Err(changed_asset(
            decoded.ok().as_ref(),
            &name,
            recorded,
            actual,
        ));
    }
    let mut file = decoded?;
    for asset in &mut file.project.library.assets {
        if declared.remove(&asset.hash).is_none() {
            return Err(Error::Invalid(format!(
                "container.asset_missing: asset {} is missing from the container (expected entry {})",
                asset.hash,
                asset_name(asset.hash)
            )));
        }
        if let Some(bytes) = assets.remove(&asset.hash) {
            asset.data = bytes;
        }
    }
    if let Some(hash) = declared.keys().next() {
        return Err(entry_error(
            &asset_name(*hash),
            "is not an asset the project packs",
        ));
    }
    if load == Load::Everything {
        file.validate()?;
        crate::generated::verify(&file)?;
        admit(&file, CONTAINER_MAX_DOCUMENT_BYTES)?;
    }
    let recovery_identity = recovery.as_deref().map(|metadata| {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update((metadata.len() as u64).to_le_bytes());
        hash.update(metadata);
        hash.update(&document);
        ContentHash(hash.finalize().into())
    });
    Ok(Contents {
        file,
        recovery_identity,
        recovery,
        read_bytes,
    })
}

/// Refuse a project this build could not write again: its saved document
/// past `limit` bytes, or the whole container past
/// [`CONTAINER_MAX_BYTES`]. Reading applies the writer's own bounds, so a
/// project that opens always has a save and a recovery snapshot (review
/// finding 11); the refusal comes at open, before any work is done in it.
pub(crate) fn admit(file: &EditorFile, limit: usize) -> Result<()> {
    // The core's one measure, which also counts the identity a first save
    // adds (review finding 15): an engine refuses an edit past it the same way.
    format::room::check_saveable(file, limit).map_err(|error| {
        Error::TooLarge(format!(
            "container.too_large: this project is not opened because it could not be saved again: {error}"
        ))
    })
}

/// Validate `file` and atomically write it as a container to `path`, with
/// `recovery` as the metadata entry of a recovery snapshot.
pub(crate) fn write(
    path: &Path,
    file: &EditorFile,
    recovery: Option<&[u8]>,
    assets: Method,
) -> Result<()> {
    write_with_mode(path, file, recovery, assets, true)
}

fn write_with_mode(
    path: &Path,
    file: &EditorFile,
    recovery: Option<&[u8]>,
    assets: Method,
    replace: bool,
) -> Result<()> {
    let document = format::encode_container_document(file)?;
    check_total(&document, recovery, file)?;
    // Only a recovery snapshot carries a recovery entry; it stays owner-only.
    let access = if recovery.is_some() {
        super::Access::Private
    } else {
        super::Access::Shared
    };
    super::write_with(path, replace, access, |target| {
        fill(target, &document, recovery, file, assets)
    })
}

/// The container's size bounds, checked before anything is written.
fn check_total(document: &[u8], recovery: Option<&[u8]>, file: &EditorFile) -> Result<()> {
    if recovery.is_some_and(|r| r.len() > format::CONTAINER_MAX_RECOVERY_BYTES) {
        return Err(Error::TooLarge(
            "recovery metadata exceeds its entry limit".into(),
        ));
    }
    let mut total = add_total(0, document.len() as u64)?;
    total = add_total(total, recovery.map_or(0, |r| r.len() as u64))?;
    for asset in &file.project.library.assets {
        if asset.data.len() as u64 > CONTAINER_MAX_ASSET_BYTES {
            return Err(Error::TooLarge(format!(
                "asset {} exceeds {CONTAINER_MAX_ASSET_BYTES} bytes",
                asset.hash
            )));
        }
        total = add_total(total, asset.data.len() as u64)?;
    }
    Ok(())
}

/// Write the container's entries to `target`.
fn fill(
    target: &mut File,
    document: &[u8],
    recovery: Option<&[u8]>,
    file: &EditorFile,
    assets: Method,
) -> Result<()> {
    let mut zip = ZipWriter::new(target);
    let deflated = options(CompressionMethod::Deflated);
    zip.start_file(CONTAINER_DOCUMENT, deflated)
        .map_err(malformed)?;
    zip.write_all(document)?;
    if let Some(recovery) = recovery {
        zip.start_file(CONTAINER_RECOVERY, deflated)
            .map_err(malformed)?;
        zip.write_all(recovery)?;
    }
    let method = match assets {
        Method::Deflated => CompressionMethod::Deflated,
        Method::Stored => CompressionMethod::Stored,
    };
    for asset in &file.project.library.assets {
        zip.start_file(asset_name(asset.hash), options(method))
            .map_err(malformed)?;
        zip.write_all(&asset.data[..])?;
    }
    zip.finish().map_err(malformed)?;
    Ok(())
}

/// The entry name of the asset with content hash `hash`.
pub fn asset_name(hash: ContentHash) -> String {
    format!("{CONTAINER_ASSET_PREFIX}{hash}")
}

/// The content hash an asset entry name carries, if `name` is one: the
/// prefix and exactly 64 lower-case hex digits.
pub fn asset_entry_hash(name: &str) -> Option<ContentHash> {
    name.strip_prefix(CONTAINER_ASSET_PREFIX)
        .and_then(ContentHash::parse)
}

enum Entry {
    Document,
    Recovery,
    Asset(ContentHash),
}

/// Reproducible entries: a fixed timestamp, ordinary file permissions and
/// a fixed host system (the zip crate otherwise records DOS on Windows), so
/// every platform writes the same bytes.
fn options(method: CompressionMethod) -> SimpleFileOptions {
    SimpleFileOptions::default()
        .compression_method(method)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644)
        .system(zip::System::Unix)
}

/// Read at most `limit` decompressed bytes. Reading to the end lets the
/// archive check the entry's CRC.
fn read_bounded(entry: &mut impl Read, limit: u64, name: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    entry
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| entry_error(name, &format!("cannot be read ({error})")))?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge(format!(
            "container entry {name} exceeds {limit} bytes"
        )));
    }
    Ok(bytes)
}

fn add_total(total: u64, bytes: u64) -> Result<u64> {
    total
        .checked_add(bytes)
        .filter(|t| *t <= CONTAINER_MAX_BYTES)
        .ok_or_else(|| {
            Error::TooLarge(format!(
                "project container exceeds {} MiB uncompressed",
                CONTAINER_MAX_BYTES / (1024 * 1024)
            ))
        })
}

/// The refusal of an asset entry whose bytes hash to `actual`, not to the
/// `recorded` hash its name carries: in the words of a generator module
/// when the document records one under that hash, else as an entry.
fn changed_asset(
    file: Option<&EditorFile>,
    name: &str,
    recorded: ContentHash,
    actual: ContentHash,
) -> Error {
    use sparksmith_core::library::generated;
    let part = file.and_then(|f| {
        let library = &f.project.library;
        library
            .generated
            .iter()
            .find(|g| g.from.module == recorded)
            .map(|g| generated::describe(library, g))
    });
    match part {
        Some(part) => Error::Document(generated::module_changed(&part, recorded, actual)),
        None => entry_error(
            name,
            &format!(
                "does not match its content hash: its name records sha256 {recorded}, but its bytes hash to sha256 {actual}"
            ),
        ),
    }
}

fn entry_error(name: &str, problem: &str) -> Error {
    let name: String = name.chars().take(160).collect();
    let kind = if name.starts_with(CONTAINER_ASSET_PREFIX) {
        "container.asset"
    } else {
        "container.entry"
    };
    Error::Invalid(format!("{kind}: container entry {name:?} {problem}"))
}

fn malformed(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::Io(error) => Error::Io(error),
        other => Error::Invalid(format!("container.malformed: {other}")),
    }
}

#[cfg(test)]
pub(crate) mod tests;
