//! Bounded, project-local lookup of user-supplied STEP assets. No environment
//! variable is expanded except KIPRJMOD; EDA library roots are never searched.
//! ZIP entries are read in memory, never extracted.
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

pub const MAX_VISITS: usize = 20_000;
pub const MAX_DEPTH: usize = 8;
pub const MAX_ARCHIVE_ENTRIES: usize = 4096;
pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_MODEL_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug)]
enum Location {
    File(PathBuf),
    Zip(PathBuf, usize, String),
}

#[derive(Debug)]
pub struct Resolved {
    pub bytes: Vec<u8>,
    /// Project-relative source, with `!entry` for a ZIP member.
    pub source: String,
}

pub fn library_reference(reference: &str) -> bool {
    let upper = reference.to_ascii_uppercase();
    upper.contains("KISYS3DMOD")
        || (upper.contains("KICAD") && upper.contains("3DMODEL_DIR"))
        || upper
            .replace('\\', "/")
            .split('/')
            .any(|p| p.eq_ignore_ascii_case("kicad"))
}

fn step(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("step") || s.eq_ignore_ascii_case("stp"))
}
fn relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.to_string_lossy().contains(':')
        && !path.to_string_lossy().starts_with(['/', '\\'])
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}
/// An index is built lazily and at most once per import, before any filename
/// fallback. Ambiguous basenames refuse rather than choosing an arbitrary file.
pub struct ProjectModels {
    root: cap_std::fs::Dir,
    names: Option<BTreeMap<String, Vec<Location>>>,
    consumed: u64,
    index_failure: Option<String>,
}
impl ProjectModels {
    pub fn new(root: &Path) -> Result<Self> {
        Ok(Self {
            root: cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())?,
            names: None,
            consumed: 0,
            index_failure: None,
        })
    }
    fn local_file(&self, path: &Path) -> Result<Option<PathBuf>> {
        if !relative(path) {
            return Ok(None);
        }
        let mut parent = PathBuf::new();
        for component in path.components() {
            parent.push(component);
            let meta = match self.root.symlink_metadata(&parent) {
                Ok(meta) => meta,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error.into()),
            };
            if meta.file_type().is_symlink() {
                return Ok(None);
            }
        }
        Ok(self
            .root
            .metadata(path)?
            .is_file()
            .then(|| path.to_path_buf()))
    }
    fn read(&mut self, path: &Path, max: u64) -> Result<Vec<u8>> {
        // All resolution is relative to this already-open root capability;
        // replacing a parent or file with a symlink cannot escape it.
        let file = self.root.open(path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(Error::Invalid(
                "3D model source is not a regular file".into(),
            ));
        }
        let allowed = max.min(MAX_TOTAL_BYTES.saturating_sub(self.consumed));
        if metadata.len() > allowed {
            return Err(Error::TooLarge(
                "3D model read exceeds its file or cumulative byte limit".into(),
            ));
        }
        self.consume(file, allowed)
    }
    fn consume(&mut self, reader: impl Read, max: u64) -> Result<Vec<u8>> {
        let allowed = max.min(MAX_TOTAL_BYTES.saturating_sub(self.consumed));
        let mut bytes = Vec::new();
        let result = reader.take(allowed + 1).read_to_end(&mut bytes);
        self.consumed = self.consumed.saturating_add(bytes.len() as u64);
        result?;
        if bytes.len() as u64 > allowed {
            return Err(Error::TooLarge(
                "3D model data exceeds its file or cumulative byte limit".into(),
            ));
        }
        Ok(bytes)
    }
    fn index(&mut self) -> Result<()> {
        if let Some(failure) = &self.index_failure {
            return Err(Error::Invalid(failure.clone()));
        }
        let result = self.build_index();
        if let Err(error) = &result {
            self.index_failure = Some(error.to_string());
        }
        result
    }
    fn build_index(&mut self) -> Result<()> {
        if self.names.is_some() {
            return Ok(());
        }
        let mut names: BTreeMap<String, Vec<Location>> = BTreeMap::new();
        let mut pending = vec![(PathBuf::from("."), 0)];
        let mut visits = 0;
        let mut expanded_bytes = 0u64;
        while let Some((dir, depth)) = pending.pop() {
            for entry in self.root.read_dir(&dir)? {
                visits += 1;
                if visits > MAX_VISITS {
                    return Err(Error::TooLarge(
                        "3D model search exceeds 20000 entries".into(),
                    ));
                }
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let kind = entry.file_type()?;
                if kind.is_symlink() {
                    continue;
                }
                let path = dir.join(entry.file_name());
                if library_reference(&path.to_string_lossy()) {
                    continue;
                }
                if kind.is_dir() {
                    if depth >= MAX_DEPTH {
                        return Err(Error::TooLarge(
                            "3D model search exceeds eight folder levels".into(),
                        ));
                    }
                    // Never traverse an explicitly named EDA installation/library.
                    if name.eq_ignore_ascii_case("kicad") {
                        continue;
                    }
                    pending.push((path, depth + 1));
                } else if kind.is_file() && step(&path) {
                    names
                        .entry(name.to_lowercase())
                        .or_default()
                        .push(Location::File(path));
                } else if kind.is_file()
                    && path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
                {
                    let bytes = self.read(&path, MAX_ARCHIVE_BYTES)?;
                    if self.consumed > MAX_TOTAL_BYTES {
                        return Err(Error::TooLarge(
                            "3D model ZIP search exceeds 128 MiB".into(),
                        ));
                    }
                    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
                        .map_err(|e| Error::Invalid(e.to_string()))?;
                    if zip.len() > MAX_ARCHIVE_ENTRIES {
                        return Err(Error::TooLarge("3D model ZIP exceeds 4096 entries".into()));
                    }
                    for n in 0..zip.len() {
                        visits += 1;
                        if visits > MAX_VISITS {
                            return Err(Error::TooLarge(
                                "3D model search exceeds 20000 entries".into(),
                            ));
                        }
                        let file = zip.by_index(n).map_err(|e| Error::Invalid(e.to_string()))?;
                        let member = file
                            .name()
                            .map_err(|e| Error::Invalid(e.to_string()))?
                            .replace('\\', "/");
                        let member_path = Path::new(&member);
                        expanded_bytes = expanded_bytes.saturating_add(file.size());
                        if expanded_bytes > MAX_TOTAL_BYTES {
                            return Err(Error::TooLarge(
                                "3D model ZIP entries exceed 128 MiB uncompressed".into(),
                            ));
                        }
                        if !relative(member_path)
                            || member_path.components().count() > MAX_DEPTH
                            || file.is_symlink()
                        {
                            return Err(Error::Invalid(
                                "3D model ZIP contains an unsafe or overly deep entry".into(),
                            ));
                        }
                        if library_reference(&member) {
                            continue;
                        }
                        if file.is_file() && step(member_path) {
                            if file.size() > MAX_MODEL_BYTES {
                                return Err(Error::TooLarge(
                                    "3D model ZIP member exceeds 32 MiB".into(),
                                ));
                            }
                            if let Some(base) = member_path.file_name().and_then(|n| n.to_str()) {
                                names
                                    .entry(base.to_lowercase())
                                    .or_default()
                                    .push(Location::Zip(path.clone(), n, member));
                            }
                        }
                    }
                }
            }
        }
        self.names = Some(names);
        Ok(())
    }
    pub fn resolve(&mut self, reference: &str) -> Result<Option<Resolved>> {
        if reference.len() > 4096 {
            return Err(Error::TooLarge("3D model path exceeds 4096 bytes".into()));
        }
        if library_reference(reference) {
            return Ok(None);
        }
        let normalized = reference.replace('\\', "/");
        let normalized = normalized
            .strip_prefix("${KIPRJMOD}/")
            .or_else(|| normalized.strip_prefix("$(KIPRJMOD)/"))
            .unwrap_or(&normalized);
        let path = Path::new(normalized);
        if !step(path) {
            return Ok(None);
        }
        let direct = if relative(path) {
            self.local_file(path)?
        } else {
            None
        };
        let location = if let Some(path) = direct {
            Some(Location::File(path))
        } else {
            self.index()?;
            let base = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_lowercase();
            match self
                .names
                .as_ref()
                .and_then(|n| n.get(&base))
                .map(Vec::as_slice)
            {
                Some([one]) => Some(one.clone()),
                Some(many) if !many.is_empty() => {
                    return Err(Error::Invalid(
                        "3D model filename is ambiguous in the project".into(),
                    ));
                }
                _ => None,
            }
        };
        let Some(location) = location else {
            return Ok(None);
        };
        let (bytes, source) = match location {
            Location::File(path) => (
                self.read(&path, MAX_MODEL_BYTES)?,
                path.display().to_string(),
            ),
            Location::Zip(path, n, member) => {
                let archive = self.read(&path, MAX_ARCHIVE_BYTES)?;
                let mut zip = zip::ZipArchive::new(Cursor::new(archive))
                    .map_err(|e| Error::Invalid(e.to_string()))?;
                let file = zip.by_index(n).map_err(|e| Error::Invalid(e.to_string()))?;
                if file
                    .name()
                    .map_err(|e| Error::Invalid(e.to_string()))?
                    .replace('\\', "/")
                    != member
                    || !file.is_file()
                    || file.is_symlink()
                    || file.size() > MAX_MODEL_BYTES
                {
                    return Err(Error::Invalid("3D model ZIP changed during import".into()));
                }
                let bytes = self.consume(file, MAX_MODEL_BYTES)?;
                let source = format!("{}!{member}", path.display());
                (bytes, source)
            }
        };
        Ok(Some(Resolved { bytes, source }))
    }
}
