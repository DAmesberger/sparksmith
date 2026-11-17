//! Recoverable publication. The journal is the commit decision; its immutable
//! payloads survive until both public files are durable. All participants hold
//! the persistent lock while recovering or reading either member of the pair.
use super::{check_design_path, sidecar_path};
use crate::assembly_link::MAX_LINKED_DESIGN_BYTES;
use crate::persistence::{atomic_create, atomic_write, atomic_write_with, sync_directory};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sparksmith_core::ids::ContentHash;
use sparksmith_core::interface::{Sidecar, assembly_link::file_hash};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const JOURNAL_LIMIT: u64 = 4096;
const SIDECAR_LIMIT: u64 = sparksmith_core::format::MAX_FILE_BYTES as u64;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    transaction: String,
    design_name: String,
    design_hash: ContentHash,
    sidecar_hash: ContentHash,
}

/// The exact pair a publisher built against. Compared again under the writer
/// lock, including native bytes even when the sidecar's datum hash is unchanged.
#[derive(Debug, PartialEq)]
pub struct Baseline {
    pub sidecar: Option<Sidecar>,
    design_hash: Option<ContentHash>,
    sidecar_hash: Option<ContentHash>,
}

/// A participating read: recovery completes before this is returned, and its
/// lock remains held until drop. Keep it alive through native file opens too.
/// Never unlink the lock file: another process may already be waiting on it.
pub struct Publication {
    design: PathBuf,
    _lock: File,
}

fn sibling(design: &Path, suffix: &str) -> PathBuf {
    let sidecar = sidecar_path(design);
    let name = sidecar.file_name().unwrap_or_default().to_string_lossy();
    design.with_file_name(format!(".{name}.{suffix}"))
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn invalid(message: &str) -> Error {
    Error::Invalid(format!("interface.transaction: {message}"))
}

fn regular(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(true),
        Ok(_) => Err(invalid(
            "publication files must be regular files, not links or directories",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    if !regular(path)? {
        return Err(invalid("a publication payload is missing"));
    }
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(Error::TooLarge(
            "interface publication file exceeds its size limit".into(),
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge(
            "interface publication file exceeds its size limit".into(),
        ));
    }
    Ok(bytes)
}

fn parse_sidecar(bytes: &[u8]) -> Result<Sidecar> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("the sidecar is not UTF-8 text"))?;
    Ok(Sidecar::read(text)?)
}

fn before_commit(error: Error) -> Error {
    match error {
        Error::Unsynced(message) => std::io::Error::other(format!(
            "The interface publication was not committed; staging could not be synchronized: {message}"
        )).into(),
        other => other,
    }
}

fn pending(error: Error) -> Error {
    if matches!(error, Error::Unsynced(_)) {
        return error;
    }
    Error::Unsynced(format!(
        "The interface publication is committed, but completing or synchronizing its files failed; reopen it to recover: {error}"
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Staged,
    Committed,
    Design,
    Sidecar,
    Removed,
}

impl Publication {
    /// Lock the pair and finish any committed publication. Existing read-only
    /// lock files can be opened read-only on platforms supporting that lock;
    /// creating the first lock or recovering an interrupted publish needs write
    /// access to the folder. No unlocked fallback is used.
    pub fn open(design: &Path) -> Result<Self> {
        check_design_path(design)?;
        let design = parent(design).canonicalize()?.join(
            design
                .file_name()
                .ok_or_else(|| invalid("a design filename is required"))?,
        );
        let path = sibling(&design, "publish-lock");
        regular(&path)?;
        let lock = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
        {
            Ok(lock) => lock,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => File::open(&path)?,
            Err(e) => return Err(e.into()),
        };
        lock.lock()?;
        let publication = Self {
            design,
            _lock: lock,
        };
        publication.recover(&mut |_| Ok(()))?;
        if regular(&publication.design)? != regular(&sidecar_path(&publication.design))? {
            return Err(invalid(
                "the design and sidecar are incomplete or unowned; restore the pair or choose another file",
            ));
        }
        Ok(publication)
    }

    pub fn sidecar(&self) -> Result<Option<Sidecar>> {
        let path = sidecar_path(&self.design);
        if !regular(&path)? {
            return Ok(None);
        }
        Ok(Some(parse_sidecar(&read(&path, SIDECAR_LIMIT)?)?))
    }

    /// Read native bytes while retaining the pair lock. IO never interprets or
    /// edits the native format; Solidcraft remains its only writer.
    pub fn design_bytes(&self) -> Result<Vec<u8>> {
        read(&self.design, MAX_LINKED_DESIGN_BYTES)
    }

    pub fn baseline(&self) -> Result<Baseline> {
        let path = sidecar_path(&self.design);
        let sidecar = if regular(&path)? {
            Some(read(&path, SIDECAR_LIMIT)?)
        } else {
            None
        };
        let design_hash = if regular(&self.design)? {
            Some(file_hash(&self.design_bytes()?))
        } else {
            None
        };
        if sidecar.is_some() != design_hash.is_some() {
            return Err(invalid(
                "the design and sidecar are incomplete; choose another file or restore the pair",
            ));
        }
        Ok(Baseline {
            sidecar: sidecar.as_deref().map(parse_sidecar).transpose()?,
            sidecar_hash: sidecar.as_deref().map(file_hash),
            design_hash,
        })
    }

    fn recover(&self, boundary: &mut impl FnMut(Boundary) -> Result<()>) -> Result<()> {
        let journal_path = sibling(&self.design, "publish-journal");
        if !regular(&journal_path)? {
            return Ok(());
        }
        let journal: Journal = serde_json::from_slice(&read(&journal_path, JOURNAL_LIMIT)?)?;
        if journal.version != 1 {
            return Err(invalid(&format!(
                "unsupported publication journal version {}",
                journal.version
            )));
        }
        if self.design.file_name().and_then(|n| n.to_str()) != Some(journal.design_name.as_str()) {
            return Err(invalid(
                "the journal belongs to another native filename; open that publication to recover",
            ));
        }
        if journal.transaction.len() > 64
            || !journal.transaction.starts_with(".interface-")
            || !journal.transaction[11..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric())
            || journal.transaction.len() <= 11
        {
            return Err(invalid("invalid publication transaction name"));
        }
        let staged = parent(&self.design).join(&journal.transaction);
        if !std::fs::symlink_metadata(&staged)?.is_dir() {
            return Err(invalid("publication staging must be a real directory"));
        }
        let design = read(&staged.join("design.solidcraft"), MAX_LINKED_DESIGN_BYTES)?;
        let sidecar = read(&staged.join("sidecar.json"), SIDECAR_LIMIT)?;
        if file_hash(&design) != journal.design_hash || file_hash(&sidecar) != journal.sidecar_hash
        {
            return Err(invalid(
                "publication payload hash mismatch; no files were replaced",
            ));
        }
        parse_sidecar(&sidecar)?;
        // Validate both destinations before replacing either, including on replay.
        regular(&self.design)?;
        regular(&sidecar_path(&self.design))?;
        let replay = || -> Result<()> {
            atomic_write_with(&self.design, |file| Ok(file.write_all(&design)?))?;
            boundary(Boundary::Design)?;
            atomic_write(&sidecar_path(&self.design), &sidecar)?;
            boundary(Boundary::Sidecar)?;
            std::fs::remove_file(&journal_path)?;
            sync_directory(parent(&self.design))?;
            boundary(Boundary::Removed)?;
            // Once the journal removal is durable, these bytes are no longer
            // needed for recovery. A failed cleanup merely leaves an inert orphan.
            let _ = std::fs::remove_dir_all(staged);
            Ok(())
        };
        replay().map_err(pending)
    }
}

/// Build the immutable payloads, then compare the complete previous pair under
/// the writer lock and record the commit decision before replacing public files.
/// The caller owns `temporary`; this function copies bytes and never deletes it.
pub fn put_in_place(
    design: &Path,
    temporary: &Path,
    text: &str,
    expected: &Baseline,
) -> Result<()> {
    put_with(design, temporary, text, expected, &mut |_| Ok(()))
}

fn put_with(
    design: &Path,
    temporary: &Path,
    text: &str,
    expected: &Baseline,
    boundary: &mut impl FnMut(Boundary) -> Result<()>,
) -> Result<()> {
    check_design_path(design)?;
    if text.len() as u64 > SIDECAR_LIMIT {
        return Err(Error::TooLarge("interface sidecar exceeds 16 MiB".into()));
    }
    let candidate = parse_sidecar(text.as_bytes())?;
    let bytes = read(temporary, MAX_LINKED_DESIGN_BYTES)?;
    let staged = tempfile::Builder::new()
        .prefix(".interface-")
        .rand_bytes(16)
        .tempdir_in(parent(design))?;
    atomic_write_with(&staged.path().join("design.solidcraft"), |file| {
        Ok(file.write_all(&bytes)?)
    })
    .map_err(before_commit)?;
    atomic_write(&staged.path().join("sidecar.json"), text.as_bytes()).map_err(before_commit)?;
    sync_directory(parent(design)).map_err(before_commit)?;
    boundary(Boundary::Staged)?;
    let publication = Publication::open(design)?;
    if publication.baseline()? != *expected {
        return Err(Error::Invalid("interface.changed: the published files changed while building; preview and publish again".into()));
    }
    if let Some(previous) = expected.sidecar.as_ref() {
        if candidate.source_id != previous.source_id {
            return Err(Error::Invalid(
                "interface.source: the publication belongs to another board".into(),
            ));
        }
        if candidate.revision < previous.revision
            || (candidate.revision == previous.revision && candidate.hash != previous.hash)
        {
            return Err(Error::Invalid("interface.changed: the candidate would replace a newer or different published revision".into()));
        }
    }
    let journal = Journal {
        version: 1,
        design_name: publication
            .design
            .file_name()
            .ok_or_else(|| invalid("missing design filename"))?
            .to_string_lossy()
            .into_owned(),
        transaction: staged
            .path()
            .file_name()
            .ok_or_else(|| invalid("missing transaction name"))?
            .to_string_lossy()
            .into_owned(),
        design_hash: file_hash(&bytes),
        sidecar_hash: file_hash(text.as_bytes()),
    };
    let journal_bytes = serde_json::to_vec(&journal)?;
    let staged = staged.keep();
    if let Err(error) = atomic_create(
        &sibling(&publication.design, "publish-journal"),
        &journal_bytes,
    ) {
        if matches!(error, Error::Unsynced(_)) {
            return Err(pending(error));
        }
        let _ = std::fs::remove_dir_all(staged);
        return Err(error);
    }
    boundary(Boundary::Committed).map_err(pending)?;
    publication.recover(boundary).map_err(pending)
}

#[cfg(test)]
#[path = "transaction_tests.rs"]
mod tests;

/// Bounded hash of a native publication payload, without interpreting its format.
pub fn design_hash(path: &Path) -> Result<ContentHash> {
    Ok(file_hash(&read(path, MAX_LINKED_DESIGN_BYTES)?))
}
