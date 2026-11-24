//! A save refused because another program has the file open, and the one
//! copy that keeps the document's latest version meanwhile.
//!
//! A document has at most one copy, "board (unsaved 1).sparksmith" beside
//! the file it could not replace. The host holds it ([`UnsavedCopy`]) with
//! its document and gives it to the next save: refused again, that writes
//! over it; done, that removes it. Both only while it still holds what was
//! written there, so a copy somebody changed, and every other document's,
//! is left alone. A host that ended leaves its copy for the person.
use super::{DiskIdentity, beside, container, identity_of, same_file};
use crate::{Error, Result};
use sparksmith_core::format::EditorFile;
use std::fs::File;
use std::path::{Path, PathBuf};

/// A document's copy, as it was written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsavedCopy {
    pub path: PathBuf,
    pub identity: DiskIdentity,
}

impl UnsavedCopy {
    fn unchanged(&self) -> bool {
        File::open(&self.path)
            .map_err(Error::Io)
            .and_then(|mut file| identity_of(&mut file))
            .is_ok_and(|identity| identity == self.identity)
    }
}

/// A save that could not replace its file, which is as it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InUse {
    pub path: PathBuf,
    /// The system's answer.
    pub cause: String,
    /// The document's copy: of this version, or with `copy_problem` of an
    /// earlier one.
    pub copy: Option<UnsavedCopy>,
    /// Why this version could not be written to a copy.
    pub copy_problem: Option<String>,
}

impl std::fmt::Display for InUse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} was not saved: another program has it open, or it may not be written ({}). \
             The file is as it was.",
            self.path.display(),
            self.cause
        )?;
        match (&self.copy, &self.copy_problem) {
            (Some(copy), None) => write!(f, " Your version is in {}.", copy.path.display())?,
            (Some(copy), Some(why)) => write!(
                f,
                " Your latest changes could not be written to its copy ({why}); {} holds an \
                 earlier version. Keep this project open until it is saved.",
                copy.path.display()
            )?,
            (None, Some(why)) => write!(
                f,
                " No copy could be written either ({why}): keep this project open until it is \
                 saved."
            )?,
            (None, None) => {}
        }
        write!(
            f,
            " Close the other program and try again, or save under another name."
        )
    }
}

/// Keep `file` as the document's copy: over `earlier`, or as a new file.
pub(super) fn keep(in_use: &mut InUse, file: &EditorFile, earlier: Option<&UnsavedCopy>) {
    let ours = earlier.filter(|copy| copy.unchanged());
    let written = match ours {
        Some(copy) => {
            container::save_container(&copy.path, file, Some(copy.identity)).map(|identity| {
                UnsavedCopy {
                    path: copy.path.clone(),
                    identity,
                }
            })
        }
        None => new_copy(&in_use.path, file),
    };
    match written {
        Ok(copy) => in_use.copy = Some(copy),
        // Somebody changed it in that moment: theirs stays.
        Err(Error::Conflict(conflict)) => {
            in_use.copy = Some(UnsavedCopy {
                path: conflict.kept,
                identity: conflict.identity,
            });
        }
        Err(error) => {
            in_use.copy = ours.cloned();
            in_use.copy_problem = Some(match error {
                Error::InUse(copy) => copy.cause,
                other => other.to_string(),
            });
        }
    }
}

fn new_copy(path: &Path, file: &EditorFile) -> Result<UnsavedCopy> {
    let mut temporary = container::stage(path, file)?;
    let identity = identity_of(temporary.as_file_mut())?;
    Ok(UnsavedCopy {
        path: beside(temporary.into_temp_path(), path, "unsaved")?,
        identity,
    })
}

/// The document is saved at `saved`: its copy is needed no more.
pub(super) fn discard(copy: Option<&UnsavedCopy>, saved: &Path) {
    if let Some(copy) = copy
        && !same_file(&copy.path, saved)
        && copy.unchanged()
    {
        let _ = std::fs::remove_file(&copy.path);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{read_project, save_document_with, save_project, write_project};
    use super::*;
    use sparksmith_core::{Document, Engine};

    fn refused(path: &Path) -> Error {
        Error::InUse(Box::new(InUse {
            path: path.to_path_buf(),
            cause: "in use".into(),
            copy: None,
            copy_problem: None,
        }))
    }

    fn named(name: &str) -> Result<Document> {
        let mut file = Engine::demo()?.to_file();
        file.project.name = name.into();
        let mut engine = Engine::demo()?;
        engine.replace_document(file)?;
        Ok(engine.document().clone())
    }

    /// A refused save of `document` at `path`.
    fn refuse(path: &Path, document: &Document, copy: Option<&UnsavedCopy>) -> Result<InUse> {
        let saved = save_document_with(path, document, None, "", copy, |path, _, _| {
            Err(refused(path))
        });
        match saved {
            Err(Error::InUse(in_use)) => Ok(*in_use),
            other => Err(Error::Invalid(format!("{other:?}"))),
        }
    }

    fn save(path: &Path, document: &Document, copy: Option<&UnsavedCopy>) -> Result<()> {
        save_document_with(path, document, None, "", copy, save_project).map(drop)
    }

    fn names(folder: &Path) -> Result<Vec<String>> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(folder)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    /// One copy per document, written over by its later refused saves and
    /// removed by a save that succeeds; another document's stays.
    #[test]
    fn a_document_keeps_one_copy_until_it_is_saved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        write_project(&path, &Engine::demo()?.to_file())?;
        let before = std::fs::read(&path)?;
        let first = refuse(&path, &named("One")?, None)?;
        let copy = first.copy.clone().ok_or(Error::Invalid("a copy".into()))?;
        assert_eq!(copy.path, dir.path().join("board (unsaved 1).sparksmith"));
        assert!(first.to_string().contains("(unsaved 1)"), "{first}");
        assert_eq!(read_project(&copy.path)?.project.name, "One");

        // Another document of the same file has its own.
        let other = refuse(&path, &named("Other")?, None)?
            .copy
            .ok_or(Error::Invalid("a copy".into()))?;
        assert_ne!(other.path, copy.path);

        let again = refuse(&path, &named("Two")?, Some(&copy))?
            .copy
            .ok_or(Error::Invalid("a copy".into()))?;
        assert_eq!(again.path, copy.path);
        assert_eq!(read_project(&copy.path)?.project.name, "Two");
        assert_eq!(std::fs::read(&path)?, before, "the file is as it was");

        save(&path, &named("Two")?, Some(&again))?;
        assert_eq!(read_project(&path)?.project.name, "Two");
        assert_eq!(
            names(dir.path())?,
            ["board (unsaved 2).sparksmith", "board.sparksmith"]
        );
        Ok(())
    }

    /// A copy somebody changed is neither written over nor removed, and a
    /// document saved as its own copy keeps it.
    #[test]
    fn a_changed_copy_is_left_alone() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("board.sparksmith");
        let document = named("One")?;
        let copy = refuse(&path, &document, None)?
            .copy
            .ok_or(Error::Invalid("a copy".into()))?;
        std::fs::write(&copy.path, b"somebody's")?;
        let next = refuse(&path, &document, Some(&copy))?
            .copy
            .ok_or(Error::Invalid("a copy".into()))?;
        assert_ne!(next.path, copy.path);
        save(&path, &document, Some(&copy))?;
        assert_eq!(std::fs::read(&copy.path)?, b"somebody's");

        save(&next.path, &document, Some(&next))?;
        assert!(next.path.exists());
        Ok(())
    }

    /// When the copy cannot be written the message says so, and names the
    /// earlier copy when there is one.
    #[test]
    fn a_copy_that_cannot_be_written_is_said() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let gone = dir.path().join("gone").join("board.sparksmith");
        let in_use = refuse(&gone, &named("One")?, None)?;
        assert_eq!(in_use.copy, None);
        assert!(in_use.copy_problem.is_some());
        let said = in_use.to_string();
        assert!(said.contains("No copy could be written"), "{said}");
        assert!(said.contains("keep this project open"), "{said}");

        let path = dir.path().join("board.sparksmith");
        let earlier = InUse {
            copy: Some(UnsavedCopy {
                path,
                identity: DiskIdentity(sparksmith_core::ids::ContentHash([0; 32])),
            }),
            ..in_use
        };
        let said = earlier.to_string();
        assert!(said.contains("holds an earlier version"), "{said}");
        Ok(())
    }
}
