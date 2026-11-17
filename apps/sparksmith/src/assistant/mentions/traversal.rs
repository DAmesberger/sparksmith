//! Filesystem work for completion. Only the search worker calls this.
use super::{Candidate, MAX_DEPTH, MAX_PROJECT_FILES, MAX_VISITED, file_name};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) struct Found {
    pub candidates: Vec<Candidate>,
    pub limited: bool,
}

pub(super) fn collect(
    known: &[PathBuf],
    folder: Option<&Path>,
    cancel: &AtomicBool,
) -> Option<Found> {
    let mut found = Found {
        candidates: Vec::new(),
        limited: known.len() > MAX_PROJECT_FILES,
    };
    let mut seen = HashSet::new();
    let mut add = |path: PathBuf, known| {
        if !seen.insert(path.clone()) {
            return;
        }
        let parent = path.parent().unwrap_or(Path::new(""));
        let detail = match folder.and_then(|folder| parent.strip_prefix(folder).ok()) {
            Some(inside) if inside.as_os_str().is_empty() => "Project folder".to_owned(),
            Some(inside) => inside.display().to_string(),
            None => parent.display().to_string(),
        };
        found.candidates.push(Candidate {
            name: file_name(&path),
            detail,
            path,
            known,
        });
    };
    for path in known.iter().rev().take(MAX_PROJECT_FILES) {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        add(path.clone(), true);
    }
    if let Some(folder) = folder {
        let (paths, limited) = project_files(folder, cancel)?;
        for path in paths {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            add(path, false);
        }
        found.limited |= limited;
    }
    (!cancel.load(Ordering::Relaxed)).then_some(found)
}

/// Consume no more than the remaining entry budget, even when entries
/// fail to read. Check cancellation before asking the filesystem for another.
fn batch(
    mut entries: impl Iterator<Item = std::io::Result<std::fs::DirEntry>>,
    remaining: &mut usize,
    cancel: &AtomicBool,
) -> Vec<std::fs::DirEntry> {
    let mut out = Vec::new();
    while *remaining > 0 && !cancel.load(Ordering::Relaxed) {
        let Some(entry) = entries.next() else {
            break;
        };
        *remaining -= 1;
        if let Ok(entry) = entry {
            out.push(entry);
        }
    }
    out.sort_by_key(std::fs::DirEntry::file_name);
    out
}

fn project_files(folder: &Path, cancel: &AtomicBool) -> Option<(Vec<PathBuf>, bool)> {
    let mut out = Vec::new();
    let mut remaining = MAX_VISITED;
    let mut limited = false;
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in batch(entries, &mut remaining, cancel) {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((entry.path(), depth + 1));
                } else {
                    limited = true;
                }
            } else if kind.is_file() {
                out.push(entry.path());
                if out.len() >= MAX_PROJECT_FILES {
                    out.sort();
                    return Some((out, true));
                }
            }
        }
        if remaining == 0 {
            limited = true;
            break;
        }
    }
    out.sort();
    (!cancel.load(Ordering::Relaxed)).then_some((out, limited))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    type Fallible = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn the_budget_bounds_directory_reads_before_collecting_and_sorting() -> Fallible {
        let dir = tempfile::tempdir()?;
        for n in 0..20 {
            std::fs::write(dir.path().join(n.to_string()), b"x")?;
        }
        let reads = Cell::new(0usize);
        let entries = std::fs::read_dir(dir.path())?.inspect(|_| reads.set(reads.get() + 1));
        let mut remaining = 3;
        let found = batch(entries, &mut remaining, &AtomicBool::new(false));
        assert_eq!(reads.get(), 3);
        assert_eq!(found.len(), 3);
        assert_eq!(remaining, 0);
        Ok(())
    }

    #[test]
    fn cancellation_stops_before_the_next_directory_entry() -> Fallible {
        let dir = tempfile::tempdir()?;
        for n in 0..20 {
            std::fs::write(dir.path().join(n.to_string()), b"x")?;
        }
        let cancel = AtomicBool::new(false);
        let reads = Cell::new(0usize);
        let entries = std::fs::read_dir(dir.path())?.inspect(|_| {
            reads.set(reads.get() + 1);
            if reads.get() == 2 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        let mut remaining = 20;
        let _ = batch(entries, &mut remaining, &cancel);
        assert_eq!(reads.get(), 2);
        assert!(collect(&[], Some(dir.path()), &cancel).is_none());
        Ok(())
    }

    #[test]
    fn a_search_limit_is_reported_and_symlinks_are_not_followed() -> Fallible {
        let dir = tempfile::tempdir()?;
        for n in 0..MAX_PROJECT_FILES + 1 {
            std::fs::write(dir.path().join(n.to_string()), b"x")?;
        }
        let found = collect(&[], Some(dir.path()), &AtomicBool::new(false)).ok_or("cancelled")?;
        assert_eq!(found.candidates.len(), MAX_PROJECT_FILES);
        assert!(found.limited);
        #[cfg(unix)]
        {
            let elsewhere = tempfile::tempdir()?;
            let empty = tempfile::tempdir()?;
            std::fs::write(elsewhere.path().join("private.pdf"), b"x")?;
            std::os::unix::fs::symlink(elsewhere.path(), empty.path().join("linked"))?;
            let found =
                collect(&[], Some(empty.path()), &AtomicBool::new(false)).ok_or("cancelled")?;
            assert!(found.candidates.is_empty());
        }
        Ok(())
    }
}
