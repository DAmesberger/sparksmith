//! Independent, bounded recovery snapshots. Each live session owns a unique file;
//! restoring never writes to the original project path.
//!
//! A snapshot is a project container (see [`crate::persistence::container`])
//! with one more entry, `recovery.json`, holding [`RecoveryRecord`]'s
//! metadata. Assets are stored rather than deflated, so frequent snapshots
//! stay cheap. Listing reads only the document and metadata entries.
use crate::persistence::container::{self, Load, Method, Role};
use crate::{DiskIdentity, Error, Result};
use serde::{Deserialize, Serialize};
use sparksmith_core::format::{EditorFile, encode_bounded};
use sparksmith_core::ids::ContentHash;
use std::{
    fs::File,
    io::{ErrorKind, Read, Seek, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const PREFIX: &str = "session-";
const SUFFIX: &str = ".sparksmith-recovery";
/// Sessions a listing reads, newest first by file time. Older ones are
/// listed in [`Listing::older`] without decompressing them, so they can be discarded.
pub const MAX_ENTRIES: usize = 128;
/// Bytes a listing decompresses in total: documents and metadata, never
/// asset entries. Sessions past it go to [`Listing::older`] too.
pub const MAX_SCAN_BYTES: u64 = 256 * 1024 * 1024;
/// Session files a recovery folder may hold; beyond it listing is refused.
pub const MAX_SESSION_FILES: usize = 65_536;
const RECORD_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRecord {
    pub version: u32,
    pub document: EditorFile,
    pub original_path: Option<PathBuf>,
    pub saved_unix_ms: u64,
}

/// The `recovery.json` entry: everything of a record but its document.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryMeta {
    version: u32,
    original_path: Option<PathBuf>,
    saved_unix_ms: u64,
}

impl RecoveryMeta {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let meta: Self = serde_json::from_slice(bytes)?;
        if meta.version != RECORD_VERSION {
            return Err(Error::Invalid("unsupported recovery record version".into()));
        }
        if meta
            .original_path
            .as_ref()
            .is_some_and(|p| p.as_os_str().len() > 4096)
        {
            return Err(Error::Invalid("recovery original path is too long".into()));
        }
        Ok(meta)
    }
}

/// The name an entry that cannot be read is listed under: a session saved
/// by another version (never migrated) or a damaged file.
pub const UNREADABLE: &str = "Older or damaged recovery";

#[derive(Clone, Debug)]
pub struct RecoveryEntry {
    pub snapshot: Snapshot,
    pub original_path: Option<PathBuf>,
    pub saved_unix_ms: u64,
    pub project_name: String,
    /// Component instances the recovered project holds, so sessions of one
    /// unnamed project can be told apart.
    pub instances: usize,
    pub error: Option<String>,
}

/// A session owns the lock beside its snapshot across atomic replacements.
/// A crash releases the OS lock and leaves the snapshot available to recover.
struct Owner {
    file: File,
    lock_path: PathBuf,
    path: PathBuf,
}

impl Owner {
    fn reserve(directory: &Path) -> Result<Self> {
        let lock = tempfile::Builder::new()
            .prefix(PREFIX)
            .suffix(".owner")
            .tempfile_in(directory)?;
        lock.as_file().lock()?;
        let path = lock.path().with_extension("sparksmith-recovery");
        if path.try_exists()? {
            return Err(Error::Invalid(
                "Recovery session name is already in use".into(),
            ));
        }
        let (file, lock_path) = lock.keep().map_err(|e| Error::Io(e.error))?;
        Ok(Self {
            file,
            lock_path,
            path,
        })
    }

    fn next_version(&mut self) -> Result<()> {
        let mut version = [0; 16];
        getrandom::fill(&mut version)
            .map_err(|e| Error::Invalid(format!("Cannot identify this recovery snapshot: {e}")))?;
        self.file.rewind()?;
        self.file.write_all(&version)?;
        self.file.sync_all()?;
        Ok(())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if matches!(self.path.try_exists(), Ok(false)) {
            let _ = std::fs::remove_file(&self.lock_path);
        }
    }
}

/// A writer that has never taken a snapshot creates no session files.
pub struct RecoveryStore {
    directory: PathBuf,
    owner: Option<Owner>,
}

impl RecoveryStore {
    pub fn new(directory: &Path) -> Result<Self> {
        std::fs::create_dir_all(directory)?;
        Ok(Self {
            directory: directory.canonicalize()?,
            owner: None,
        })
    }

    pub fn path(&self) -> Option<&Path> {
        self.owner.as_ref().map(|owner| owner.path.as_path())
    }

    pub fn write(&mut self, document: &EditorFile, original_path: Option<&Path>) -> Result<()> {
        document.validate()?;
        if original_path.is_some_and(|p| p.as_os_str().len() > 4096) {
            return Err(Error::Invalid("recovery original path is too long".into()));
        }
        let owner = match &mut self.owner {
            Some(owner) => owner,
            slot @ None => slot.insert(Owner::reserve(&self.directory)?),
        };
        owner.next_version()?;
        let path = &owner.path;
        match std::fs::symlink_metadata(path) {
            Ok(meta) if !meta.is_file() || meta.file_type().is_symlink() => {
                return Err(Error::Invalid(
                    "recovery path is not a regular session file".into(),
                ));
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
            _ => {}
        }
        let saved_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid("system clock precedes the Unix epoch".into()))?
            .as_millis();
        let saved_unix_ms = u64::try_from(saved_unix_ms)
            .map_err(|_| Error::Invalid("recovery timestamp exceeds supported range".into()))?;
        let meta = RecoveryMeta {
            version: RECORD_VERSION,
            original_path: original_path.map(Path::to_path_buf),
            saved_unix_ms,
        };
        container::write(
            path,
            document,
            Some(&encode_bounded(&meta)?),
            Method::Stored,
        )
    }
    pub fn remove_own(&mut self) -> Result<()> {
        if let Some(owner) = &self.owner {
            match regular(&owner.path) {
                Ok(_) => std::fs::remove_file(&owner.path)?,
                Err(Error::Io(e)) if e.kind() == ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
            self.owner = None;
        }
        Ok(())
    }
}

fn regular(path: &Path) -> Result<std::fs::Metadata> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(Error::Invalid("Recovery requires a regular file".into()));
    }
    Ok(meta)
}

/// Read a snapshot for inspection. A restoration and its later cleanup use
/// [`restore`] so both operations carry the identity of the same open file.
pub fn read(path: &Path) -> Result<RecoveryRecord> {
    regular(path)?;
    Ok(read_snapshot(File::open(path)?, Load::Everything)?.record)
}

struct SnapshotContents {
    record: RecoveryRecord,
    identity: ContentHash,
    read_bytes: u64,
}

fn read_snapshot(file: File, load: Load) -> Result<SnapshotContents> {
    let contents = container::read_file(file, Role::Recovery, load)?;
    let meta = RecoveryMeta::parse(contents.recovery.as_deref().unwrap_or_default())?;
    Ok(SnapshotContents {
        record: RecoveryRecord {
            version: meta.version,
            document: contents.file,
            original_path: meta.original_path,
            saved_unix_ms: meta.saved_unix_ms,
        },
        identity: contents
            .recovery_identity
            .ok_or_else(|| Error::Invalid("Missing recovery metadata".into()))?,
        read_bytes: contents.read_bytes,
    })
}

fn session_name(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| {
            name.starts_with(PREFIX)
                && name.ends_with(SUFFIX)
                && name.len() > PREFIX.len() + SUFFIX.len()
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        })
}

/// The listed generation. A restored reference additionally carries the hash
/// of the entire container, including the asset bytes skipped by the listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    path: PathBuf,
    stamp: FileStamp,
    summary: Option<ContentHash>,
    content: Option<DiskIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileStamp {
    version: Vec<u8>,
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    inode_change: (u64, u64, i64, i64),
}

impl FileStamp {
    fn new(meta: &std::fs::Metadata, version: Vec<u8>) -> Result<Self> {
        Ok(Self {
            version,
            len: meta.len(),
            modified: meta.modified()?,
            #[cfg(unix)]
            inode_change: {
                use std::os::unix::fs::MetadataExt;
                (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec())
            },
        })
    }
}

impl Snapshot {
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn observe(path: PathBuf, meta: &std::fs::Metadata, version: Vec<u8>) -> Result<Self> {
        Ok(Self {
            path,
            stamp: FileStamp::new(meta, version)?,
            summary: None,
            content: None,
        })
    }

    fn verify(&self, owner: Option<&mut File>) -> Result<File> {
        regular(&self.path)?;
        let file = File::open(&self.path)?;
        if FileStamp::new(&file.metadata()?, generation(owner)?)? != self.stamp {
            return Err(changed());
        }
        Ok(file)
    }
}

fn changed() -> Error {
    Error::Invalid("The recovery snapshot changed. Its newer contents have been kept.".into())
}

/// Old snapshots may have no owner file. A recovery client opening such
/// a snapshot creates an empty lock, which represents the same generation.
fn generation(owner: Option<&mut File>) -> Result<Vec<u8>> {
    let mut version = Vec::new();
    if let Some(file) = owner {
        file.rewind()?;
        file.take(16).read_to_end(&mut version)?;
    }
    Ok(version)
}

enum Ownership {
    Available(Option<File>),
    Busy,
}

/// Listings can share the lock. Restoring and removing take it exclusively;
/// a live writer holds it from its first write until release.
fn ownership(path: &Path, create: bool) -> Result<Ownership> {
    let lock = path.with_extension("owner");
    match regular(&lock) {
        Ok(_) => (),
        Err(Error::Io(e)) if e.kind() == ErrorKind::NotFound => {
            if !create {
                return Ok(Ownership::Available(None));
            }
        }
        Err(e) => return Err(e),
    }
    let mut options = File::options();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&lock) {
        Ok(file) => file,
        Err(e) if !create && e.kind() == ErrorKind::NotFound => {
            return Ok(Ownership::Available(None));
        }
        Err(e) => return Err(e.into()),
    };
    let result = if create {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    match result {
        Ok(()) => Ok(Ownership::Available(Some(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Ownership::Busy),
        Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
    }
}

fn claim(directory: &Path, snapshot: &Snapshot) -> Result<(File, File)> {
    check_path(directory, snapshot.path())?;
    // Do not create an owner file for an already removed snapshot.
    regular(snapshot.path())?;
    match ownership(snapshot.path(), true)? {
        Ownership::Available(Some(mut owner)) => {
            let file = snapshot.verify(Some(&mut owner))?;
            Ok((owner, file))
        }
        _ => Err(Error::Invalid(
            "Another editor is using this recovery snapshot.".into(),
        )),
    }
}

/// Read exactly the listed version and return the reference required to
/// clean up that version after the restoring editor has protected its copy.
pub fn restore(directory: &Path, snapshot: &Snapshot) -> Result<(RecoveryRecord, Snapshot)> {
    let (_owner, mut file) = claim(directory, snapshot)?;
    let mut restored = snapshot.clone();
    restored.content = Some(crate::persistence::identity_of(&mut file)?);
    let contents = read_snapshot(file, Load::Everything)?;
    if snapshot.summary != Some(contents.identity) {
        return Err(changed());
    }
    Ok((contents.record, restored))
}

/// What a recovery folder holds.
#[derive(Clone, Debug, Default)]
pub struct Listing {
    /// The newest sessions (at most [`MAX_ENTRIES`], within
    /// [`MAX_SCAN_BYTES`]), most recently saved first.
    pub entries: Vec<RecoveryEntry>,
    /// Older session files that were not read, newest first: listed so the
    /// user is told about them and can discard them, never dropped unseen.
    pub older: Vec<Snapshot>,
}

/// Corrupt records remain visible with an error so the user can discard one
/// explicitly without losing access to valid recovery candidates. A listing
/// reads each snapshot's document and metadata only; [`read`] checks its
/// assets and validates it before anything is restored.
pub fn list(directory: &Path) -> Result<Listing> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Listing::default());
        }
        Err(error) => return Err(error.into()),
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if !session_name(&path) {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        if !meta.is_file() || meta.file_type().is_symlink() {
            continue;
        }
        if meta.len() == 0 {
            continue;
        } // reserved by a session writing its first snapshot
        if candidates.len() == MAX_SESSION_FILES {
            return Err(Error::TooLarge(format!(
                "the recovery folder {} holds more than {MAX_SESSION_FILES} session files",
                directory.display()
            )));
        }
        candidates.push((meta.modified().ok(), path));
    }
    // Newest first; a file without a time sorts last.
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut listing = Listing::default();
    let mut result = Vec::new();
    let mut bytes = 0u64;
    for (modified, path) in candidates {
        let Ownership::Available(mut owner) = ownership(&path, false)? else {
            continue;
        };
        let meta = match regular(&path) {
            Ok(meta) => meta,
            Err(Error::Io(e)) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let mut snapshot = Snapshot::observe(path.clone(), &meta, generation(owner.as_mut())?)?;
        if result.len() == MAX_ENTRIES || bytes > MAX_SCAN_BYTES {
            listing.older.push(snapshot);
            continue;
        }
        match File::open(&path)
            .map_err(Error::from)
            .and_then(|file| read_snapshot(file, Load::Summary))
        {
            Ok(contents) => {
                let record = contents.record;
                bytes = bytes.saturating_add(contents.read_bytes);
                if bytes > MAX_SCAN_BYTES {
                    listing.older.push(snapshot);
                    continue;
                }
                snapshot.summary = Some(contents.identity);
                result.push(RecoveryEntry {
                    snapshot,
                    original_path: record.original_path,
                    saved_unix_ms: record.saved_unix_ms,
                    instances: record.document.project.instances.len(),
                    project_name: record.document.project.name,
                    error: None,
                })
            }
            // Dated by the file, so unreadable sessions tell apart by age.
            Err(error) => result.push(RecoveryEntry {
                snapshot,
                original_path: None,
                saved_unix_ms: modified
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
                project_name: UNREADABLE.into(),
                instances: 0,
                error: Some(error.to_string()),
            }),
        }
    }
    result.sort_by(|a, b| {
        b.saved_unix_ms
            .cmp(&a.saved_unix_ms)
            .then_with(|| a.snapshot.path.cmp(&b.snapshot.path))
    });
    listing.entries = result;
    Ok(listing)
}

/// Only an explicitly selected, direct-child session file can be discarded.
/// Symlinks, directories, unrelated filenames and paths outside the recovery
/// directory are rejected. Missing owned entries are harmless after cleanup.
pub fn remove_entry(directory: &Path, snapshot: &Snapshot) -> Result<()> {
    let (_owner, mut file) = match claim(directory, snapshot) {
        Ok(claimed) => claimed,
        Err(Error::Io(e)) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if let Some(identity) = snapshot.content {
        if crate::persistence::identity_of(&mut file)? != identity {
            return Err(changed());
        }
    } else if let Some(summary) = snapshot.summary
        && read_snapshot(file.try_clone()?, Load::Summary)?.identity != summary
    {
        return Err(changed());
    }
    drop(file);
    std::fs::remove_file(snapshot.path())?;
    std::fs::remove_file(snapshot.path().with_extension("owner"))?;
    Ok(())
}

fn check_path(directory: &Path, path: &Path) -> Result<()> {
    if !session_name(path) {
        return Err(Error::Invalid("not a recovery session filename".into()));
    }
    let directory = directory.canonicalize()?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid("recovery path has no parent".into()))?
        .canonicalize()?;
    if parent != directory {
        return Err(Error::Invalid(
            "recovery entry is outside its directory".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
