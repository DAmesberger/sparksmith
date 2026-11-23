//! Compare or write a help registry without changing an entry's first release.
use super::source;
use crate::{Error, Result, persistence::atomic_write};
use serde::Deserialize;
use sparksmith_core::{
    format::MAX_FILE_BYTES,
    help::{HelpEntry, HelpId},
};
use std::{collections::BTreeMap, path::Path};

#[derive(Deserialize)]
struct FirstRelease {
    id: HelpId,
    since: String,
}

/// The previous snapshot supplies only publication history. All other fields
/// come from the current registry, even when the topic's wording has changed.
pub fn sync_registry(path: &Path, mut entries: Vec<HelpEntry>, update: bool) -> Result<()> {
    let previous = if path.try_exists()? {
        Some(source(path)?)
    } else {
        None
    };
    if let Some(previous) = &previous {
        let released: Vec<FirstRelease> = serde_json::from_str(previous)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        let mut versions = BTreeMap::new();
        for old in released {
            versions.insert(old.id, old.since);
        }
        for entry in &mut entries {
            if let Some(since) = versions.remove(&entry.id) {
                entry.since = since;
            }
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    for pair in entries.windows(2) {
        if let [a, b] = pair
            && a.id == b.id
        {
            return Err(Error::Invalid(format!("{}: duplicate {}", path.display(), a.id)));
        }
    }
    let expected = serde_json::to_string_pretty(&entries)? + "\n";
    if expected.len() > MAX_FILE_BYTES {
        return Err(Error::TooLarge(format!(
            "{} needs {} bytes; the shelf reader accepts {MAX_FILE_BYTES}",
            path.display(),
            expected.len()
        )));
    }
    if previous.as_deref() == Some(expected.as_str()) {
        return Ok(());
    }
    if update {
        atomic_write(path, expected.as_bytes())
    } else {
        Err(Error::Invalid(format!(
            "{} is stale; run cargo xtask help --update",
            path.display()
        )))
    }
}
