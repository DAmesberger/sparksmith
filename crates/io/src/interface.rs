//! The published board interface's files (docs/architecture/
//! board-interface.md, "The export format"): the `.solidcraft` design an
//! assembly links and its sidecar `<name>.interface.json` beside it. Both
//! form one recoverable publication. Participating readers hold a lock while
//! consuming the pair; raw filesystem readers must coordinate externally.
use crate::{Error, Result};
use sparksmith_core::ids::InterfaceSourceId;
use sparksmith_core::interface::Sidecar;
use std::path::{Path, PathBuf};

/// The sidecar's file name ending.
pub const SIDECAR_SUFFIX: &str = ".interface.json";

/// A published design is a `.solidcraft` file with a name.
pub fn check_design_path(path: &Path) -> Result<()> {
    let named = path
        .file_stem()
        .is_some_and(|stem| !stem.is_empty() && !stem.to_string_lossy().starts_with('.'));
    let extension = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("solidcraft"));
    if named && extension {
        Ok(())
    } else {
        Err(Error::Invalid(
            "interface.path: publish the interface to a named .solidcraft file".into(),
        ))
    }
}

/// `<folder>/<name>.interface.json` for the design `<folder>/<name>.solidcraft`.
pub fn sidecar_path(design: &Path) -> PathBuf {
    let stem = design
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    design.with_file_name(format!("{stem}{SIDECAR_SUFFIX}"))
}

/// The board's interface identity for its first publish, from the
/// operating system's random source.
pub fn new_source_id() -> Result<InterfaceSourceId> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| Error::Invalid(format!("the system random source failed: {e}")))?;
    Ok(InterfaceSourceId::from_bytes(bytes))
}

/// The sidecar published beside `design`, or `None` when there is none.
/// One that is there but unreadable, or of a format this build does not
/// read, is refused.
pub fn read_sidecar(design: &Path) -> Result<Option<Sidecar>> {
    Publication::open(design)?.sidecar()
}

mod transaction;
pub use transaction::{Baseline, Publication, design_hash, put_in_place};

/// A fresh sibling name for unpublished native output. StagedNew owns the
/// actual scratch file; this name is never committed to the public destination.
pub fn temporary_design(design: &Path) -> Result<PathBuf> {
    check_design_path(design)?;
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| Error::Invalid(format!("the system random source failed: {e}")))?;
    let tag: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(design.with_file_name(format!(".interface-native-{tag}.solidcraft")))
}
