//! Replacing a file on Windows.
//!
//! Windows refuses to rename over a file that is open
//! (`MoveFileExW`: access denied, os error 5), also when it is our own
//! handle that has it open, so a save here holds no handle of the file it
//! replaces: writers exclude each other with [`SaveLock`] instead of a lock
//! on the file itself. `std::fs::rename` replaces atomically, and where the
//! volume can (NTFS) also a file others have open shared for deletion, as
//! every Sparksmith reader does. Where it cannot, or a program shares
//! nothing (a virus scanner, a backup), the step is tried again at once, a
//! few times; then the save is refused ([`Error::InUse`]) and the old file
//! stays as it was.
use super::InUse;
use crate::{Error, Result};
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;

const ERROR_ACCESS_DENIED: i32 = 5;
const ERROR_SHARING_VIOLATION: i32 = 32;
const ERROR_LOCK_VIOLATION: i32 = 33;
const ERROR_DELETE_PENDING: i32 = 303;
const ERROR_USER_MAPPED_FILE: i32 = 1224;

/// Tries of a step another program is in the way of, one right after the
/// other: the second and third cover a virus scanner's moment with a file
/// just written. There is no waiting; the person is told and tries again.
const TRIES: usize = 3;

/// How often a save looks whether the save before it has ended.
const POLL: Duration = Duration::from_millis(10);

/// Whether `error` is another program having the file open, as far as its
/// code tells: a file that may not be written answers access denied too.
fn in_use(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(
            ERROR_ACCESS_DENIED
                | ERROR_SHARING_VIOLATION
                | ERROR_LOCK_VIOLATION
                | ERROR_DELETE_PENDING
                | ERROR_USER_MAPPED_FILE
        )
    )
}

/// Run `step`, up to [`TRIES`] times while the file is in use.
pub(super) fn retrying<T>(mut step: impl FnMut() -> Result<T>) -> Result<T> {
    for _ in 1..TRIES {
        match step() {
            Err(Error::Io(error)) if in_use(&error) => std::thread::yield_now(),
            done => return done,
        }
    }
    step()
}

/// Excludes every other save of one file, in this process and others, for
/// as long as it lives: a hidden file beside it that only its holder has
/// open. Windows removes it when it is closed, also when the process ends;
/// one a power cut left behind holds up no one.
pub(super) struct SaveLock {
    _file: File,
}

impl SaveLock {
    pub(super) fn hold(path: &Path) -> Result<Self> {
        let mut name = std::ffi::OsString::from(".");
        name.push(path.file_name().unwrap_or_default());
        name.push(".saving");
        let lock = path.with_file_name(name);
        loop {
            let opened = retrying(|| {
                let opened = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .share_mode(0)
                    .attributes(FILE_ATTRIBUTE_HIDDEN)
                    .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
                    .open(&lock);
                Ok(opened?)
            });
            match opened {
                Ok(file) => return Ok(Self { _file: file }),
                // Another save of this file: it ends with its writer.
                Err(Error::Io(error)) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {
                    std::thread::sleep(POLL);
                }
                Err(error) => return Err(error),
            }
        }
    }
}

/// A finished temporary file, closed and without the attributes of one;
/// removed when dropped, unless it was put in place.
pub(super) struct Staged(tempfile::TempPath);

impl Staged {
    pub(super) fn new(temporary: tempfile::TempPath) -> Result<Self> {
        let path = temporary.keep().map_err(|e| Error::Io(e.error))?;
        Ok(Self(
            tempfile::TempPath::try_from_path(path).map_err(Error::Io)?,
        ))
    }

    pub(super) fn into_temp_path(self) -> tempfile::TempPath {
        self.0
    }

    /// Rename it over `path`, once.
    pub(super) fn rename_over(&self, path: &Path) -> Result<()> {
        Ok(std::fs::rename(&self.0, path)?)
    }
}

/// What kept the file at `path` from being replaced, which is as it was.
pub(super) fn refused(path: &Path, error: Error) -> Error {
    match error {
        Error::Io(cause) if in_use(&cause) => Error::InUse(Box::new(InUse {
            path: path.to_path_buf(),
            cause: cause.to_string(),
            copy: None,
            copy_problem: None,
        })),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::super::{open_project, save_document, save_project, write_project};
    use super::*;
    use sparksmith_core::Engine;

    const FILE_SHARE_READ: u32 = 0x1;

    fn files(folder: &Path) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    /// The desktop's second save of an opened project, with another window
    /// reading the file meanwhile.
    #[test]
    fn a_file_another_window_reads_is_replaced() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        let mut file = Engine::demo()?.to_file();
        write_project(&path, &file)?;
        let (_, opened) = open_project(&path)?;
        let reader = File::open(&path)?;
        file.project.name = "Second save".into();
        let saved = save_project(&path, &file, Some(opened))?;
        assert_eq!(open_project(&path)?, (file, saved));
        drop(reader);
        assert_eq!(files(dir.path())?, ["board.sparksmith"]);
        Ok(())
    }

    /// A program that shares its file for reading only keeps it from being
    /// replaced: the save is refused, the old file stays, this version is
    /// in the document's one copy, and the next save goes through and
    /// removes the copy.
    #[test]
    fn a_file_held_by_another_program_stays_and_this_version_is_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        let engine = Engine::demo()?;
        let saved = save_document(&path, engine.document(), None, "", None)?;
        let before = std::fs::read(&path)?;
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)?;
        let document = &saved.document;
        let mut copy = None;
        for _ in 0..2 {
            let again = save_document(&path, document, Some(saved.identity), "", copy.as_ref());
            let Err(Error::InUse(in_use)) = again else {
                return Err(Error::Invalid(format!("{again:?}")));
            };
            assert_eq!(in_use.copy_problem, None);
            copy = in_use.copy;
            assert_eq!(std::fs::read(&path)?, before);
            assert_eq!(
                files(dir.path())?,
                ["board (unsaved 1).sparksmith", "board.sparksmith"]
            );
        }
        drop(holder);
        save_document(&path, document, Some(saved.identity), "", copy.as_ref())?;
        assert_eq!(files(dir.path())?, ["board.sparksmith"]);
        Ok(())
    }

    /// Only one save of a file holds its lock, whatever the letter case it
    /// is named in; it leaves nothing behind, and a lock file a power cut
    /// left holds up no one.
    #[test]
    fn a_save_lock_excludes_and_leaves_nothing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        std::fs::write(dir.path().join(".board.sparksmith.saving"), b"")?;
        let held = SaveLock::hold(&path)?;
        let (send, entered) = std::sync::mpsc::channel();
        let other = dir.path().join("BOARD.sparksmith");
        let waiting = std::thread::spawn(move || {
            let held = SaveLock::hold(&other);
            let _ = send.send(());
            held.map(drop)
        });
        assert!(
            entered.recv_timeout(Duration::from_millis(100)).is_err(),
            "two saves at once"
        );
        drop(held);
        waiting
            .join()
            .map_err(|_| Error::Invalid("the waiting save panicked".into()))??;
        assert!(files(dir.path())?.is_empty());
        Ok(())
    }
}
