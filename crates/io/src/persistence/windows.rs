//! Replacing a file on Windows.
//!
//! Windows refuses to rename over a file that is open
//! (`MoveFileExW`: access denied, os error 5), also when it is our own
//! handle that has it open, so a save here holds no handle of the file it
//! replaces: writers exclude each other with [`SaveLock`] instead of a lock
//! on the file itself. `std::fs::rename` replaces atomically, and where the
//! volume can (NTFS) also a file others have open shared for deletion, as
//! every Sparksmith reader does. Where it cannot, or a program shares
//! nothing (a virus scanner, a backup), the step is tried again for a
//! moment; after that the old file stays as it was.
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

/// The pauses between tries of a step another program is in the way of:
/// scanners and indexers hold a file for milliseconds. About 1.6 s in all.
const PAUSES_MS: [u64; 10] = [1, 2, 5, 10, 20, 50, 100, 200, 400, 800];

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

/// Run `step`, again after each pause while the file is in use.
pub(super) fn retrying<T>(mut step: impl FnMut() -> Result<T>) -> Result<T> {
    for pause in PAUSES_MS {
        match step() {
            Err(Error::Io(error)) if in_use(&error) => {
                std::thread::sleep(Duration::from_millis(pause));
            }
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
                Err(Error::Io(error)) if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => {}
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

/// `error` kept the file at `path` from being replaced. The file is as it
/// was; what was to replace it is kept beside it, when it can be.
pub(super) fn not_replaced(staged: Staged, path: &Path, error: Error) -> Error {
    let Error::Io(cause) = &error else {
        return error;
    };
    if !in_use(cause) {
        return error;
    }
    let kept = match super::beside(staged.into_temp_path(), path, "unsaved") {
        Ok(kept) => format!(
            "This version is in {}.",
            kept.file_name().unwrap_or_default().to_string_lossy()
        ),
        Err(_) => "This version was not written.".to_owned(),
    };
    Error::Io(std::io::Error::new(
        cause.kind(),
        format!(
            "{} was not replaced: another program has it open, or it may not be written ({cause}). \
             It is as it was. {kept} Close the other program and save again, or use Save As.",
            path.display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::{open_project, read_project, save_project, write_project};
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
    /// replaced: the old file stays, this version is kept beside it, the
    /// message says what to do, and the next save goes through.
    #[test]
    fn a_file_held_by_another_program_stays_and_this_version_is_kept() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        let old = Engine::demo()?.to_file();
        write_project(&path, &old)?;
        let (_, opened) = open_project(&path)?;
        let holder = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)?;
        let mut new = old.clone();
        new.project.name = "Second save".into();
        let Err(error) = save_project(&path, &new, Some(opened)) else {
            return Err(Error::Invalid("the held file was replaced".into()));
        };
        let message = error.to_string();
        assert!(
            message.contains("It is as it was") && message.contains("Save As"),
            "{message}"
        );
        assert!(
            message.contains("board (unsaved 1).sparksmith"),
            "{message}"
        );
        assert_eq!(read_project(&path)?, old);
        let kept = dir.path().join("board (unsaved 1).sparksmith");
        assert_eq!(read_project(&kept)?, new);
        drop(holder);
        save_project(&path, &new, Some(opened))?;
        assert_eq!(read_project(&path)?, new);
        assert_eq!(
            files(dir.path())?,
            ["board (unsaved 1).sparksmith", "board.sparksmith"]
        );
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
