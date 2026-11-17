//! Project files on disk. A project is a ZIP container (see [`container`]).
//! Writes are atomic: a failed save leaves any existing file untouched and
//! no temporary file behind.
use crate::{Error, Result};
use sparksmith_core::format::{self, EditorFile, MAX_FILE_BYTES};
use std::fs::File;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub mod container;

pub fn check_project_path(path: &Path) -> Result<()> {
    if path.extension().is_none_or(|e| e != format::EXTENSION) {
        return Err(Error::Invalid("use a .sparksmith project filename".into()));
    }
    if path.as_os_str().len() > 4096 {
        return Err(Error::Invalid("project path is too long".into()));
    }
    Ok(())
}

/// Read and validate a project container of the one format version;
/// anything else is refused.
pub fn read_project(path: &Path) -> Result<EditorFile> {
    check_project_path(path)?;
    container::read_container(path)
}

/// Validate, encode and atomically replace a project container.
pub fn write_project(path: &Path, file: &EditorFile) -> Result<()> {
    check_project_path(path)?;
    container::write_container(path, file)
}

/// Validate, encode and create a project without replacing another file,
/// including one created while the container is being written.
pub fn create_project(path: &Path, file: &EditorFile) -> Result<()> {
    check_project_path(path)?;
    container::create_container(path, file)
}

pub fn read_bytes(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_BYTES as u64 {
        return Err(Error::TooLarge("file exceeds 16 MiB".into()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge("file exceeds 16 MiB".into()));
    }
    Ok(bytes)
}

/// Write to a sibling temporary file, flush it, then rename it over `path`.
/// On Unix a new file gets the usual permissions (0666 less the umask) and a
/// replaced file keeps its own.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_via_temporary(path, bytes, true)
}

/// Like [`atomic_write`], but `path` must not exist: the finished temporary
/// file is linked into place without replacing anything, so a file created
/// at `path` meanwhile is never overwritten. That case is an `Io` error of
/// kind `AlreadyExists`, and no temporary file is left behind.
pub fn atomic_create(path: &Path, bytes: &[u8]) -> Result<()> {
    write_via_temporary(path, bytes, false)
}

fn write_via_temporary(path: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge("file exceeds 16 MiB".into()));
    }
    write_with(path, replace, Access::Shared, |file| {
        Ok(file.write_all(bytes)?)
    })
}

/// Who may read a file the atomic writers make. Unix only; elsewhere the
/// platform's defaults apply.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Access {
    /// A new file gets 0666 less the umask, as any program's file would; a
    /// replaced file keeps its permissions.
    Shared,
    /// Owner only (0600), new or replaced: recovery snapshots.
    Private,
}

/// Let `fill` write a sibling temporary file, sync it, then rename it over
/// `path`. When `fill` fails, the temporary file is removed and `path` is
/// untouched. `fill` bounds what it writes. Permissions as for
/// [`atomic_write`].
pub fn atomic_write_with(path: &Path, fill: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    write_with(path, true, Access::Shared, fill)
}

/// [`atomic_write_with`], creating a new file. A competing creator wins
/// without having its file replaced; that case returns `AlreadyExists`.
pub fn atomic_create_with(path: &Path, fill: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    write_with(path, false, Access::Shared, fill)
}

/// An owned sibling path for a writer that accepts a path, such as the
/// native model writer. The target appears only on [`Self::persist`].
/// Dropping this value removes the staging file, including after errors.
pub struct StagedNew {
    target: PathBuf,
    temporary: tempfile::TempPath,
}

impl StagedNew {
    /// Stage a new file beside `target`, retaining its extension. Refuse
    /// existing targets early; persistence checks again atomically.
    pub fn new(target: &Path) -> Result<Self> {
        if target.file_name().is_none() {
            return Err(Error::Invalid("the output path has no file name".into()));
        }
        match std::fs::symlink_metadata(target) {
            Ok(_) => return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists).into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let temporary = temporary_in(
            parent_of(target),
            target,
            Access::Shared,
            &temporary_suffix(target),
        )?
        .into_temp_path();
        // A path-based Save As must see a new path: some native writers
        // back up any existing file, including an empty placeholder.
        std::fs::remove_file(&temporary)?;
        Ok(Self {
            target: target.to_path_buf(),
            temporary,
        })
    }

    /// The owned, initially absent path the external writer creates. It
    /// may subsequently replace this file while producing the output.
    pub fn path(&self) -> &Path {
        &self.temporary
    }

    /// Sync the writer's actual file and publish without replacing any
    /// target. `Unsynced` means the file is in place but its directory
    /// flush failed, as for the other atomic writers.
    pub fn persist(self) -> Result<()> {
        // A path-based writer may replace the staging file. Reopen the
        // named file so we sync exactly the bytes that will be published.
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.path())?
            .sync_all()?;
        persist(self.temporary, &self.target, false)
    }
}

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// [`atomic_write_with`], replacing `path` only when `replace` is set;
/// otherwise an existing `path` is an `AlreadyExists` error.
pub(crate) fn write_with(
    path: &Path,
    replace: bool,
    access: Access,
    fill: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let mut temporary = temporary_in(parent_of(path), path, access, std::ffi::OsStr::new(""))?;
    fill(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    persist(temporary.into_temp_path(), path, replace)
}

/// The one publication primitive used by byte, container and path writers.
fn persist(temporary: tempfile::TempPath, path: &Path, replace: bool) -> Result<()> {
    if replace {
        temporary.persist(path).map_err(|e| Error::Io(e.error))?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|e| Error::Io(e.error))?;
    }
    sync_directory(parent_of(path))
}

/// The sibling temporary file for `path`, with the permissions `path` should
/// end up with; tempfile's own default is owner-only.
#[cfg(unix)]
fn temporary_in(
    parent: &Path,
    path: &Path,
    access: Access,
    suffix: &std::ffi::OsStr,
) -> Result<tempfile::NamedTempFile> {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    let existing = match std::fs::metadata(path) {
        Ok(meta) => Some(meta.permissions().mode() & 0o777),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    // A new shared file is created with 0666 and the umask applies, as for
    // any newly created file. A replaced one is created owner-only and then
    // set exactly, so the umask cannot narrow what the file had.
    let mode = match (access, existing) {
        (Access::Shared, None) => 0o666,
        _ => 0o600,
    };
    let temporary = tempfile::Builder::new()
        .suffix(suffix)
        .permissions(Permissions::from_mode(mode))
        .tempfile_in(parent)?;
    if let (Access::Shared, Some(existing)) = (access, existing) {
        temporary
            .as_file()
            .set_permissions(Permissions::from_mode(existing))?;
    }
    Ok(temporary)
}

#[cfg(not(unix))]
fn temporary_in(
    parent: &Path,
    _path: &Path,
    _access: Access,
    suffix: &std::ffi::OsStr,
) -> Result<tempfile::NamedTempFile> {
    Ok(tempfile::Builder::new()
        .suffix(suffix)
        .tempfile_in(parent)?)
}

fn temporary_suffix(path: &Path) -> std::ffi::OsString {
    let mut suffix = std::ffi::OsString::new();
    if let Some(extension) = path.extension() {
        suffix.push(".");
        suffix.push(extension);
    }
    suffix
}

/// Make a rename in `directory` durable. A failure is [`Error::Unsynced`]:
/// the file is already in place, so it is not reported as refused input.
///
/// A folder the system will not let us open or sync is not a failure: the
/// file's own data was synced before the rename, and only the folder
/// entry's flush is left to the file system. macOS refuses both in a
/// protected folder such as Documents (E2E run 6, finding 4), and some
/// file systems do not sync folders at all. Reporting that on every save
/// would mark a saved project unsaved, so it is not reported.
pub(crate) fn sync_directory(directory: &Path) -> Result<()> {
    #[cfg(unix)]
    match std::fs::File::open(directory).and_then(|directory| directory.sync_all()) {
        Ok(()) => {}
        Err(error) if sync_not_offered(&error) => {}
        Err(error) => {
            return Err(Error::Unsynced(format!(
                "The file was written, but synchronizing {} failed; power-loss durability is unconfirmed: {error}",
                directory.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

/// Whether a folder sync failed because the system does not offer it here
/// (permission, an unsupported operation or file system), not because
/// writing failed.
#[cfg(unix)]
fn sync_not_offered(error: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        error.kind(),
        ErrorKind::PermissionDenied | ErrorKind::Unsupported | ErrorKind::InvalidInput
    )
}

#[cfg(test)]
pub(crate) mod tests;
