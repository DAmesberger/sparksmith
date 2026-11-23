//! The linked design's file (docs/architecture/board-interface.md,
//! "Link updates"): read once, bounded, hashed, and copied into a scratch
//! folder that the engine opens. The user's linked design file is only ever
//! read; the dry run and every read work on the copy.
use crate::{Error, Result};
use sparksmith_core::ids::ContentHash;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The largest linked design read (it carries the linked board's
/// bodies, so it may be large).
pub const MAX_LINKED_DESIGN_BYTES: u64 = 512 * 1024 * 1024;

/// A linked design read from disk: its bytes' hash and a scratch copy the
/// engine opens. The copy and its folder go when this is dropped.
pub struct LinkedDesignCopy {
    pub hash: ContentHash,
    /// The scratch copy.
    pub path: PathBuf,
    /// The folder the user keeps the linked design in, against which its
    /// relative link paths resolve.
    pub folder: PathBuf,
    dir: tempfile::TempDir,
}

impl LinkedDesignCopy {
    /// The scratch folder, for files the dry run writes beside the copy.
    pub fn scratch(&self) -> &Path {
        self.dir.path()
    }
}

/// An linked design is a `.solidcraft` file.
pub fn check_path(path: &Path) -> Result<()> {
    let ok = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("solidcraft"));
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(
            "assembly.path: the linked design is a .solidcraft design".into(),
        ))
    }
}

/// The hash of the linked design at `path` now, or `None` when it is not there.
pub fn hash(path: &Path) -> Result<Option<ContentHash>> {
    if !path.try_exists()? {
        return Ok(None);
    }
    Ok(Some(sparksmith_core::interface::assembly_link::file_hash(
        &read(path)?,
    )))
}

fn read(path: &Path) -> Result<Vec<u8>> {
    read_file(&std::fs::File::open(path)?)
}

fn read_file(file: &std::fs::File) -> Result<Vec<u8>> {
    if file.metadata()?.len() > MAX_LINKED_DESIGN_BYTES {
        return Err(Error::TooLarge("the linked design exceeds 512 MiB".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_LINKED_DESIGN_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LINKED_DESIGN_BYTES {
        return Err(Error::TooLarge("the linked design exceeds 512 MiB".into()));
    }
    Ok(bytes)
}

/// Hold a shared lock on the reviewed assembly through the publication decision.
/// A replacing writer must take the exclusive lock on the current file, check
/// that the name still identifies it, and hold the lock across its rename.
pub(crate) fn hold_reviewed(path: &Path, expected: ContentHash) -> Result<std::fs::File> {
    let changed = || {
        Error::Document(sparksmith_core::Error::Conflict(
            "interface.preview_changed: the linked design changed; preview the publication again"
                .into(),
        ))
    };
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => changed(),
        _ => e.into(),
    })?;
    file.lock_shared()?;
    if sparksmith_core::interface::assembly_link::file_hash(&read_file(&file)?) != expected
        || !crate::persistence::still_at(&file, path)?
    {
        return Err(changed());
    }
    Ok(file)
}

/// Read the linked design at `path` once and copy it into a new scratch folder.
pub fn copy(path: &Path) -> Result<LinkedDesignCopy> {
    check_path(path)?;
    let bytes = read(path)?;
    let dir = tempfile::Builder::new()
        .prefix("sparksmith-linked-design-")
        .tempdir()?;
    let copy = dir.path().join("linked.solidcraft");
    std::fs::write(&copy, &bytes)?;
    let folder = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    Ok(LinkedDesignCopy {
        hash: sparksmith_core::interface::assembly_link::file_hash(&bytes),
        path: copy,
        folder,
        dir,
    })
}
