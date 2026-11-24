//! The fabricators' stackups that ship: one versioned JSON
//! [`FabStackupFile`] each in the shipped library's `stackups/<fabricator>/`
//! folders, named `<id>.json`. Reads are bounded and every file is checked:
//! an unknown field, a value of the wrong kind or out of range, or a name
//! that is not the file's is an error naming the file and the field, never
//! skipped. Nothing here knows a fabricator.
use crate::shipped::{Shelf, ShippedLibrary};
use crate::{Error, Result};
use sparksmith_core::layout::fab_stackup::{FAB_STACKUP_FORMAT, FabStackup, FabStackupFile};
use std::path::Path;

/// Bytes of one stackup file.
pub const MAX_BYTES: u64 = 256 * 1024;
/// Fabricator folders, and stackups one folder holds, at most.
const MAX_ENTRIES: usize = 1024;
const EXTENSION: &str = "json";

/// Decode one file. The error names the field that is wrong.
pub fn decode(bytes: &[u8]) -> Result<FabStackup> {
    // The format first: a newer file is refused as unsupported rather than
    // as malformed.
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value.get("format").and_then(serde_json::Value::as_u64) {
        Some(format) if format == u64::from(FAB_STACKUP_FORMAT) => {}
        Some(format) => {
            return Err(Error::Document(sparksmith_core::Error::Unsupported(
                format!("unsupported stackup format {format}"),
            )));
        }
        None => {
            return Err(Error::Invalid(
                "not a stackup file: it has no format".into(),
            ));
        }
    }
    let file: FabStackupFile = serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e.path().to_string();
        Error::Invalid(format!("{path}: {}", e.into_inner()))
    })?;
    file.fab_stackup
        .check()
        .map_err(|e| Error::Invalid(format!("fab_stackup.{e}")))?;
    Ok(file.fab_stackup)
}

/// The file names directly in `folder`, sorted; `folders` lists folders.
fn names(folder: &Path, folders: bool) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(folder)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let wanted = if folders {
            kind.is_dir()
        } else {
            kind.is_file()
        };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if wanted && (folders || name.ends_with(&format!(".{EXTENSION}"))) {
            out.push(name);
        }
        if out.len() > MAX_ENTRIES {
            return Err(Error::TooLarge(format!(
                "{} holds more than {MAX_ENTRIES} entries",
                folder.display()
            )));
        }
    }
    out.sort();
    Ok(out)
}

/// Every stackup of `shipped`, by fabricator folder and file name.
pub fn load(shipped: &ShippedLibrary) -> Result<Vec<FabStackup>> {
    let shelf = shipped.shelf(Shelf::Stackups);
    if !shelf.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for fabricator in names(&shelf, true)? {
        for file in names(&shelf.join(&fabricator), false)? {
            let path = shelf.join(&fabricator).join(&file);
            let named = |e: Error| Error::Invalid(format!("{}: {e}", path.display()));
            let name = format!("{fabricator}/{file}");
            let bytes = shipped
                .read(Shelf::Stackups, &name, MAX_BYTES)
                .map_err(named)?;
            let stackup = decode(&bytes).map_err(named)?;
            if file.strip_suffix(&format!(".{EXTENSION}")) != Some(stackup.id.as_str()) {
                return Err(named(Error::Invalid(format!(
                    "fab_stackup.id: the file is not named for its id {}",
                    stackup.id
                ))));
            }
            if stackup.fabricator != fabricator {
                return Err(named(Error::Invalid(format!(
                    "fab_stackup.fabricator: the folder is not named for {}",
                    stackup.fabricator
                ))));
            }
            if out.iter().any(|s: &FabStackup| s.id == stackup.id) {
                return Err(named(Error::Invalid(format!(
                    "fab_stackup.id: {} is another stackup's id",
                    stackup.id
                ))));
            }
            out.push(stackup);
        }
    }
    Ok(out)
}

/// [`load`] of the located shipped library.
pub fn shipped() -> Result<Vec<FabStackup>> {
    load(&ShippedLibrary::locate()?)
}

/// The stackup of `id` in `library`.
pub fn find<'a>(library: &'a [FabStackup], id: &str) -> Result<&'a FabStackup> {
    library.iter().find(|s| s.id == id).ok_or_else(|| {
        Error::Document(sparksmith_core::Error::NotFound(format!(
            "layout.stackup_unknown: no stackup {id}; layout.stackups lists them"
        )))
    })
}

#[cfg(test)]
#[path = "fab_stackups_tests.rs"]
mod tests;
