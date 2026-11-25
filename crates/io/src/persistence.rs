//! Project files on disk. A project is a ZIP container (see [`container`]).
//! Writes are atomic: a failed save leaves any existing file untouched and
//! no temporary file behind. A save refused because another program has the
//! file locked (on Windows: open) is not waited for and keeps its version in
//! a copy beside it (`unsaved`).
use crate::{Error, Result};
use sparksmith_core::format::{self, EditorFile, MAX_FILE_BYTES};
use sparksmith_core::{Document, FirstSave};
use std::fs::File;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub mod container;
mod unsaved;
#[cfg(windows)]
mod windows;

pub use unsaved::{InUse, UnsavedCopy};

pub fn check_project_path(path: &Path) -> Result<()> {
    if !format::is_project_path(path) {
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
    Ok(open_project(path)?.0)
}

/// Validate, encode and atomically replace a project container, whatever
/// the file holds now. A save of a file that was opened goes through
/// [`save_project`] instead, which notices another writer's save.
pub fn write_project(path: &Path, file: &EditorFile) -> Result<()> {
    save_project(path, file, None).map(|_| ())
}

/// What a project file held when it was read or written: the SHA-256 of
/// its bytes. A save carries the identity its document was opened (or
/// last saved) with, so a file another window or program saved since is
/// never silently replaced (review finding 9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiskIdentity(pub sparksmith_core::ids::ContentHash);

impl std::fmt::Display for DiskIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A save that found the file changed on disk since its document was
/// opened or saved. Both versions are kept: the file on disk as the other
/// writer left it, and this save's document at `kept`, a new file beside
/// it, which the saving host takes as its file from now on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// The file this save was for, left as the other writer saved it.
    pub path: PathBuf,
    /// The new file holding this save's document.
    pub kept: PathBuf,
    /// The identity of `kept`.
    pub identity: DiskIdentity,
}

/// [`read_project`], with the identity of the bytes it read. The hash and
/// the container are read through one open file: a writer replaces the
/// file by renaming a new one over it, so both see the same bytes.
pub fn open_project(path: &Path) -> Result<(EditorFile, DiskIdentity)> {
    check_project_path(path)?;
    let mut file = File::open(path)?;
    let identity = identity_of(&mut file)?;
    let read = container::read_open(file)?;
    Ok((read, identity))
}

/// Whether `a` and `b` name one file: the same path, or two spellings that
/// reach it (relative and absolute, `..`, a symbolic link, another letter
/// case on a volume that ignores it). A save to the file a document was
/// opened from is checked against what it held, whatever its spelling
/// (review finding 16). Two paths of which only one exists name two files.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (located(a), located(b)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Where `path` leads: the file it names, else (no file there yet) its
/// resolved folder and name.
#[derive(PartialEq, Eq)]
enum Location {
    #[cfg(unix)]
    File(u64, u64),
    #[cfg(not(unix))]
    File(PathBuf),
    Absent(PathBuf, std::ffi::OsString),
}

fn located(path: &Path) -> Option<Location> {
    match std::fs::metadata(path) {
        Ok(_metadata) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                Some(Location::File(_metadata.dev(), _metadata.ino()))
            }
            #[cfg(not(unix))]
            {
                std::fs::canonicalize(path).ok().map(Location::File)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name()?.to_os_string();
            let folder = std::fs::canonicalize(parent_of(path)).ok()?;
            Some(Location::Absent(folder, name))
        }
        Err(_) => None,
    }
}

/// Validate, encode and atomically replace a project container. With
/// `expected`, the identity its document was opened or last saved with,
/// the replacement happens only while the file still holds those bytes:
/// writers that save this way lock the file, check it and rename over it
/// as one step. A file that changed meanwhile is kept and this document
/// is written to a new file beside it ([`Error::Conflict`]). Without
/// `expected` (Save As to a file the person chose) any file is replaced,
/// but a write-protected one ([`Error::WriteProtected`]). Returns the
/// identity of the saved file.
pub fn save_project(
    path: &Path,
    file: &EditorFile,
    expected: Option<DiskIdentity>,
) -> Result<DiskIdentity> {
    save_project_over(path, file, expected, WriteProtected::Refuse)
}

/// What a save or an export does with a write-protected file. A rename
/// needs no right to write the file it replaces, so without asking first it
/// would go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteProtected {
    /// Leave it as it is and say so.
    Refuse,
    /// Replace it, as the person said; the new file is write-protected too.
    Replace,
}

impl WriteProtected {
    /// `Replace` when the person said so.
    pub fn told(replace: bool) -> Self {
        if replace { Self::Replace } else { Self::Refuse }
    }
}

/// While it lives, the files of one set may be replaced although they are
/// write-protected; those there afterwards are write-protected again.
pub(crate) struct Allowed {
    #[cfg(windows)]
    _writable: Vec<windows::Writable>,
}

/// Whether the files at `paths` may be replaced, all of them or none: with
/// [`WriteProtected::Refuse`] the first write-protected one is named, and
/// nothing has changed.
pub(crate) fn allow(paths: &[&Path], protected: WriteProtected) -> Result<Allowed> {
    if protected == WriteProtected::Refuse
        && let Some(path) = paths.iter().find(|p| write_protected(p))
    {
        return Err(Error::WriteProtected(path.to_path_buf()));
    }
    Ok(Allowed {
        #[cfg(windows)]
        _writable: paths
            .iter()
            .filter_map(|p| windows::Writable::for_replacement(p).transpose())
            .collect::<Result<_>>()?,
    })
}

/// [`atomic_write`] over a file the person chose: a write-protected one as
/// `protected` says.
pub(crate) fn atomic_write_over(
    path: &Path,
    bytes: &[u8],
    protected: WriteProtected,
) -> Result<()> {
    let _allowed = allow(&[path], protected)?;
    atomic_write(path, bytes)
}

/// Whether `path` says it must not be written: without any right to write
/// (Unix) or read-only (Windows), or on Unix a file this person may not write
/// (another's, an access list, a locked file). A folder counts on Unix only:
/// Windows gives the attribute another meaning there.
pub(crate) fn write_protected(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return cfg!(unix) && meta.permissions().readonly();
    }
    #[cfg(unix)]
    let refused = matches!(
        std::fs::OpenOptions::new().write(true).open(path),
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied
    );
    // Elsewhere an open another program refuses answers the same.
    #[cfg(not(unix))]
    let refused = false;
    refused || meta.permissions().readonly()
}

fn save_project_over(
    path: &Path,
    file: &EditorFile,
    expected: Option<DiskIdentity>,
    protected: WriteProtected,
) -> Result<DiskIdentity> {
    check_project_path(path)?;
    // Stage and publish beside the linked file, including across volumes.
    // A dangling link cannot safely identify an existing save destination.
    let target = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_symlink() => path.canonicalize()?,
        Ok(_) => path.to_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_owned(),
        Err(error) => return Err(error.into()),
    };
    let _allowed = allow(&[&target], protected)?;
    container::save_container(&target, file, expected)
}

/// A document in its file ([`save_document`]).
#[derive(Debug)]
pub struct SavedDocument {
    /// The identity of the file.
    pub identity: DiskIdentity,
    /// The document as the file holds it.
    pub document: Document,
    /// What this save gave it, for the host's engine
    /// ([`sparksmith_core::Engine::adopt_first_save`]).
    pub first: FirstSave,
}

/// A host's save of its document: [`sparksmith_core::first_save`] on the
/// copy written, then [`save_project`]. A failed write gives the project
/// nothing, and neither does one that found the file changed: the version
/// kept beside it holds `document` as it was, to be named and identified
/// by its own file at the next save. Refused because another program has
/// the file open or locked ([`Error::InUse`]), `document` is kept in its one copy,
/// over the `copy` an earlier refused save made; saved, that is removed.
/// `protected` says what becomes of a write-protected file.
pub fn save_document(
    path: &Path,
    document: &Document,
    expected: Option<DiskIdentity>,
    nonce: &str,
    copy: Option<&UnsavedCopy>,
    protected: WriteProtected,
) -> Result<SavedDocument> {
    save_document_with(
        path,
        document,
        expected,
        nonce,
        copy,
        |path, file, expected| save_project_over(path, file, expected, protected),
    )
}

/// [`save_document`], its project written by `save`.
fn save_document_with(
    path: &Path,
    document: &Document,
    expected: Option<DiskIdentity>,
    nonce: &str,
    copy: Option<&UnsavedCopy>,
    save: impl Fn(&Path, &EditorFile, Option<DiskIdentity>) -> Result<DiskIdentity>,
) -> Result<SavedDocument> {
    let mut written = document.clone();
    let first = sparksmith_core::first_save(&mut written, path, nonce);
    match save(path, &EditorFile::from_document(&written), expected) {
        Ok(identity) => {
            unsaved::discard(copy, path);
            Ok(SavedDocument {
                identity,
                document: written,
                first,
            })
        }
        Err(Error::Conflict(mut conflict)) => {
            if !first.is_empty() {
                let as_it_was = EditorFile::from_document(document);
                if let Ok(identity) = save(&conflict.kept, &as_it_was, Some(conflict.identity)) {
                    conflict.identity = identity;
                }
            }
            unsaved::discard(copy, &conflict.kept);
            Err(Error::Conflict(conflict))
        }
        Err(Error::InUse(mut in_use)) => {
            unsaved::keep(&mut in_use, &EditorFile::from_document(document), copy);
            Err(Error::InUse(in_use))
        }
        Err(error) => Err(error),
    }
}

/// The SHA-256 of `file`'s bytes, read from its start (bounded by the
/// container's size limit); the file is left at its start.
pub(crate) fn identity_of(file: &mut File) -> Result<DiskIdentity> {
    use sha2::{Digest, Sha256};
    use std::io::Seek;
    file.rewind()?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > format::CONTAINER_MAX_BYTES {
            return Err(Error::TooLarge(format!(
                "project container exceeds {} MiB",
                format::CONTAINER_MAX_BYTES / (1024 * 1024)
            )));
        }
        hasher.update(&buffer[..read]);
    }
    file.rewind()?;
    Ok(DiskIdentity(sparksmith_core::ids::ContentHash(
        hasher.finalize().into(),
    )))
}

/// What a save that finds the file's lock taken says.
pub(crate) const HELD: &str = "another window or program is saving it or has it locked";

/// Publish the finished `temporary` at `path` as one step among writers
/// that save through here: the file at `path` is locked, checked against
/// `expected` and replaced while the lock is held, so two saves of one
/// file cannot both pass the check. A lock somebody else has is not waited
/// for ([`Error::InUse`]). On a mismatch the temporary file is
/// published beside `path` instead ([`Conflict`]).
#[cfg(not(windows))]
pub(crate) fn replace_checked(
    mut temporary: tempfile::NamedTempFile,
    path: &Path,
    expected: Option<DiskIdentity>,
) -> Result<DiskIdentity> {
    let identity = identity_of(temporary.as_file_mut())?;
    // A bounded number of tries: each one that fails lost a race with a
    // writer that replaced or created the file meanwhile.
    for _ in 0..16 {
        let mut current = match File::open(path) {
            Ok(file) => file,
            // No file (a first save, or the opened one was moved away):
            // this document takes the name without replacing anything.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match temporary.persist_noclobber(path) {
                    Ok(_) => {
                        sync_directory(parent_of(path))?;
                        return Ok(identity);
                    }
                    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                        temporary = error.file;
                        continue;
                    }
                    Err(error) => return Err(Error::Io(error.error)),
                }
            }
            Err(error) => return Err(error.into()),
        };
        match current.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => return Err(InUse::refused(path, HELD)),
            Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
        }
        if !still_at(&current, path)? {
            // Another writer renamed a new file over the one we locked.
            continue;
        }
        if let Some(expected) = expected
            && identity_of(&mut current)? != expected
        {
            drop(current);
            return Err(keep_beside(temporary.into_temp_path(), path, identity));
        }
        temporary.persist(path).map_err(|e| Error::Io(e.error))?;
        // The lock is released once the new file is in place.
        drop(current);
        sync_directory(parent_of(path))?;
        return Ok(identity);
    }
    Err(keeps_changing(path))
}

/// What one try of a replacement on Windows found.
#[cfg(windows)]
enum Found {
    Replaced,
    Absent,
    Changed,
}

/// The same on Windows, where a file that is open cannot be replaced: the
/// writers' lock is a file beside it ([`windows::SaveLock`]), and the file
/// itself is closed before the replacement.
#[cfg(windows)]
pub(crate) fn replace_checked(
    mut temporary: tempfile::NamedTempFile,
    path: &Path,
    expected: Option<DiskIdentity>,
) -> Result<DiskIdentity> {
    let identity = identity_of(temporary.as_file_mut())?;
    let mut temporary = temporary.into_temp_path();
    let _saving = windows::SaveLock::hold(path)?;
    // A bounded number of tries: each one that fails lost a race with a
    // program that created or moved the file meanwhile.
    for _ in 0..16 {
        let staged = windows::Staged::new(temporary)?;
        let replaced = windows::retrying(|| {
            if path.is_dir() {
                return Err(std::io::Error::from(std::io::ErrorKind::IsADirectory).into());
            }
            // Looked at again before each try of the replacement.
            let mut current = match File::open(path) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Found::Absent);
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(expected) = expected
                && identity_of(&mut current)? != expected
            {
                return Ok(Found::Changed);
            }
            drop(current);
            staged.rename_over(path).map(|()| Found::Replaced)
        });
        match replaced {
            Ok(Found::Replaced) => return Ok(identity),
            Ok(Found::Changed) => {
                return Err(keep_beside(staged.into_temp_path(), path, identity));
            }
            Err(error) => return Err(windows::refused(path, error)),
            // No file (a first save, or the opened one was moved away):
            // this document takes the name without replacing anything.
            Ok(Found::Absent) => match staged.into_temp_path().persist_noclobber(path) {
                Ok(()) => return Ok(identity),
                Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                    temporary = error.path;
                }
                Err(error) => return Err(Error::Io(error.error)),
            },
        }
    }
    Err(keeps_changing(path))
}

fn keeps_changing(path: &Path) -> Error {
    Error::Invalid(format!(
        "{} keeps changing while it is being saved; try again",
        path.display()
    ))
}

/// Whether the locked `file` is still the one `path` names.
#[cfg(unix)]
pub(crate) fn still_at(file: &File, path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let locked = file.metadata()?;
    match std::fs::metadata(path) {
        Ok(named) => Ok(named.dev() == locked.dev() && named.ino() == locked.ino()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Compare file identity while both Windows handles remain open. The path
/// can name a replacement even while another writer holds the old handle.
#[cfg(windows)]
pub(crate) fn still_at(file: &File, path: &Path) -> Result<bool> {
    let locked = same_file::Handle::from_file(file.try_clone()?)?;
    match same_file::Handle::from_path(path) {
        Ok(named) => Ok(locked == named),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// Elsewhere the check is the identity comparison alone.
#[cfg(not(any(unix, windows)))]
pub(crate) fn still_at(_file: &File, path: &Path) -> Result<bool> {
    Ok(path.exists())
}

/// Publish `temporary` as a new file beside `path` ("board (conflict
/// 1).sparksmith", the first free number) and say so.
fn keep_beside(temporary: tempfile::TempPath, path: &Path, identity: DiskIdentity) -> Error {
    match beside(temporary, path, "conflict") {
        Ok(kept) => {
            let _ = sync_directory(parent_of(path));
            Error::Conflict(Box::new(Conflict {
                path: path.to_path_buf(),
                kept,
                identity,
            }))
        }
        Err(error) => error,
    }
}

/// Put `temporary` in place as a new file beside `path`: "board (`what`
/// 1).sparksmith", the first free number.
fn beside(mut temporary: tempfile::TempPath, path: &Path, what: &str) -> Result<PathBuf> {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| format::EXTENSION.to_owned());
    for n in 1..=99 {
        let kept = parent_of(path).join(format!("{stem} ({what} {n}).{extension}"));
        match temporary.persist_noclobber(&kept) {
            Ok(()) => return Ok(kept),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.path;
            }
            Err(error) => return Err(Error::Io(error.error)),
        }
    }
    Err(Error::Invalid(format!(
        "{} was not replaced, and no free name was found beside it to keep this version; use Save As",
        path.display()
    )))
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

/// New files that are published as one set, under names no other writer
/// holds. Each is staged under its own file name in a folder beside its
/// target (a project takes its identity from its file's name) and
/// [`Self::publish`] puts them in place in the order they were staged.
/// Dropping the set removes whatever was not published.
pub struct NewSet {
    folder: tempfile::TempDir,
    /// Each staged file and where it goes.
    files: Vec<(PathBuf, PathBuf)>,
}

impl NewSet {
    /// A set whose files go beside `target`.
    pub fn beside(target: &Path) -> Result<Self> {
        let folder = tempfile::Builder::new()
            .prefix(".sparksmith-set-")
            .tempdir_in(parent_of(target))?;
        Ok(Self {
            folder,
            files: Vec::new(),
        })
    }

    /// The path to write `target`'s content to.
    pub fn stage(&mut self, target: &Path) -> Result<PathBuf> {
        let name = target
            .file_name()
            .ok_or_else(|| Error::Invalid("the output path has no file name".into()))?;
        let staged = self.folder.path().join(name);
        self.files.push((staged.clone(), target.to_path_buf()));
        Ok(staged)
    }

    /// Put every staged file in place, refusing an existing one. The first
    /// file claims the set's name: of two sets with the same files, one
    /// gets past it. When a later file cannot be placed, the ones this set
    /// placed are removed again, so nothing of it stays. Stopped half way
    /// (a crash), the files placed so far stay and keep the name taken, so
    /// another set never mixes with them; staged last, the file that
    /// matters most is there only when the others are.
    pub fn publish(self) -> Result<()> {
        let mut placed: Vec<&Path> = Vec::new();
        for (staged, target) in &self.files {
            let placing = tempfile::TempPath::try_from_path(staged)
                .map_err(Error::Io)
                .and_then(|staged| persist(staged, target, false));
            let error = match placing {
                Ok(()) => {
                    placed.push(target);
                    continue;
                }
                Err(error) => error,
            };
            // Written, but its folder was not synchronized: it is in place.
            if matches!(error, Error::Unsynced(_)) {
                placed.push(target);
            }
            for path in placed {
                let _ = std::fs::remove_file(path);
            }
            return Err(match error {
                Error::Io(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Error::Invalid(
                    format!("{} exists; choose a new output name", target.display()),
                ),
                other => other,
            });
        }
        Ok(())
    }
}

fn parent_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Let `fill` write a sibling temporary file for `path` and sync it; the
/// caller publishes it.
pub(crate) fn staged_with(
    path: &Path,
    access: Access,
    fill: impl FnOnce(&mut File) -> Result<()>,
) -> Result<tempfile::NamedTempFile> {
    let mut temporary = temporary_in(parent_of(path), path, access, std::ffi::OsStr::new(""))?;
    fill(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    Ok(temporary)
}

/// [`atomic_write_with`], replacing `path` only when `replace` is set;
/// otherwise an existing `path` is an `AlreadyExists` error.
pub(crate) fn write_with(
    path: &Path,
    replace: bool,
    access: Access,
    fill: impl FnOnce(&mut File) -> Result<()>,
) -> Result<()> {
    let temporary = staged_with(path, access, fill)?;
    persist(temporary.into_temp_path(), path, replace)
}

/// The one publication primitive used by byte, container and path writers.
fn persist(temporary: tempfile::TempPath, path: &Path, replace: bool) -> Result<()> {
    if replace {
        replace_with(temporary, path)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|e| Error::Io(e.error))?;
    }
    sync_directory(parent_of(path))
}

#[cfg(not(windows))]
fn replace_with(temporary: tempfile::TempPath, path: &Path) -> Result<()> {
    temporary.persist(path).map_err(|e| Error::Io(e.error))
}

/// Windows refuses to replace a file that is open: see [`windows`].
#[cfg(windows)]
fn replace_with(temporary: tempfile::TempPath, path: &Path) -> Result<()> {
    let staged = windows::Staged::new(temporary)?;
    windows::retrying(|| staged.rename_over(path))
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
