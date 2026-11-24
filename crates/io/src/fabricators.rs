//! Fabricator records: the shipped ones (the shipped library's
//! `fabricators/` shelf) and the user's own, one versioned JSON
//! [`FabricatorFile`] per fabricator in `UserDirs::config/fabricators/`,
//! named `<id>.json`. A user record with a shipped record's id takes its
//! place. Writes are atomic; reads are bounded; an unsupported format or a
//! damaged file is reported, never rewritten or silently skipped.
//!
//! A fab profile names its fabricator by id
//! ([`sparksmith_core::rules::RuleProfile::fabricator`]); the records give
//! that id a name and a website, and profile suggestions prefer the board's
//! own fabricator.
use crate::library_folder::{pretty, read_bounded};
use crate::persistence::atomic_write;
use crate::settings::UserDirs;
use crate::shipped::{Shelf, ShippedLibrary};
use crate::{Error, Result};
use sparksmith_core::rules::{FABRICATOR_FILE_FORMAT, Fabricator, FabricatorFile};
use std::path::{Path, PathBuf};

/// The user folder's name under the config directory.
pub const FABRICATORS_DIR: &str = "fabricators";
/// Bytes of one fabricator file.
pub const MAX_FABRICATOR_FILE_BYTES: usize = 16 * 1024;
/// Fabricators either folder holds.
pub const MAX_FABRICATORS: usize = 256;
const EXTENSION: &str = "json";

/// The versioned JSON of one record.
pub fn encode(fabricator: &Fabricator) -> Result<Vec<u8>> {
    fabricator.validate()?;
    pretty(&FabricatorFile {
        format: FABRICATOR_FILE_FORMAT,
        fabricator: fabricator.clone(),
    })
}

/// Decode one record: its format must be [`FABRICATOR_FILE_FORMAT`] and the
/// record valid.
pub fn decode(bytes: &[u8]) -> Result<Fabricator> {
    if bytes.len() > MAX_FABRICATOR_FILE_BYTES {
        return Err(Error::TooLarge(format!(
            "a fabricator file exceeds {} KiB",
            MAX_FABRICATOR_FILE_BYTES >> 10
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    match value.get("format").and_then(serde_json::Value::as_u64) {
        Some(format) if format == u64::from(FABRICATOR_FILE_FORMAT) => {}
        Some(format) => {
            return Err(Error::Document(sparksmith_core::Error::Unsupported(
                format!("unsupported fabricator format {format}"),
            )));
        }
        None => {
            return Err(Error::Invalid(
                "not a fabricator file: it has no format".into(),
            ));
        }
    }
    let file: FabricatorFile = serde_json::from_value(value)?;
    file.fabricator.validate()?;
    Ok(file.fabricator)
}

/// Decode the file `name` read from `bytes`, which must be `<id>.json`.
fn named(name: &str, bytes: &[u8]) -> Result<Fabricator> {
    let record = decode(bytes)?;
    if name.strip_suffix(&format!(".{EXTENSION}")) != Some(record.id.as_str()) {
        return Err(Error::Invalid(format!(
            "the file name does not match its fabricator id {}",
            record.id
        )));
    }
    Ok(record)
}

/// The shipped records, in file-name order. A file that does not read as
/// one is an error naming it.
pub fn shipped_fabricators(shipped: &ShippedLibrary) -> Result<Vec<Fabricator>> {
    let names = shipped.list(Shelf::Fabricators, &format!(".{EXTENSION}"))?;
    if names.len() > MAX_FABRICATORS {
        return Err(Error::TooLarge(format!(
            "{} holds more than {MAX_FABRICATORS} fabricators",
            shipped.shelf(Shelf::Fabricators).display()
        )));
    }
    let limit = u64::try_from(MAX_FABRICATOR_FILE_BYTES).unwrap_or(u64::MAX);
    names
        .iter()
        .map(|name| {
            let path = shipped.shelf(Shelf::Fabricators).join(name);
            let bytes = shipped.read(Shelf::Fabricators, name, limit)?;
            named(name, &bytes).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
        })
        .collect()
}

/// A user record that could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: PathBuf,
    pub message: String,
}

/// The user's records over the shipped ones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FabricatorLibrary {
    dir: PathBuf,
    shipped: ShippedLibrary,
}

/// What [`FabricatorLibrary::list`] found: every record by id (a user
/// record in place of a shipped one of its id), sorted by name, and every
/// user file it could not use.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub fabricators: Vec<Fabricator>,
    pub problems: Vec<Problem>,
}

impl FabricatorLibrary {
    pub fn new(dir: PathBuf, shipped: ShippedLibrary) -> Self {
        Self { dir, shipped }
    }

    /// `fabricators/` in the user config directory, over the located
    /// shipped library.
    pub fn from_dirs(dirs: &UserDirs) -> Result<Self> {
        Ok(Self::new(
            dirs.config.join(FABRICATORS_DIR),
            ShippedLibrary::locate()?,
        ))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Every record: the user's, then the shipped ones whose ids the user
    /// has none of, sorted by name. A damaged, unsupported or misnamed user
    /// file is a [`Problem`], never dropped silently.
    pub fn list(&self) -> Result<Listing> {
        let mut listing = Listing::default();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let mut files = Vec::new();
        for entry in entries.into_iter().flatten() {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == EXTENSION) && path.is_file() {
                files.push(path);
            }
            if files.len() > MAX_FABRICATORS {
                break;
            }
        }
        files.sort();
        for file in files {
            if listing.fabricators.len() >= MAX_FABRICATORS {
                listing.problems.push(Problem {
                    file,
                    message: format!("not read: at most {MAX_FABRICATORS} fabricators"),
                });
                continue;
            }
            let name = file
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_owned();
            match read_bounded(&file, MAX_FABRICATOR_FILE_BYTES).and_then(|b| named(&name, &b)) {
                Ok(record) => listing.fabricators.push(record),
                Err(error) => listing.problems.push(Problem {
                    file,
                    message: error.to_string(),
                }),
            }
        }
        for record in shipped_fabricators(&self.shipped)? {
            if !listing.fabricators.iter().any(|f| f.id == record.id) {
                listing.fabricators.push(record);
            }
        }
        listing.fabricators.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(listing)
    }

    /// Add or replace the user's record of `fabricator.id`.
    pub fn save(&self, fabricator: &Fabricator) -> Result<()> {
        let bytes = encode(fabricator)?;
        std::fs::create_dir_all(&self.dir)?;
        atomic_write(
            &self.dir.join(format!("{}.{EXTENSION}", fabricator.id)),
            &bytes,
        )
    }
}

/// Every record of the user's library over the located shipped one.
pub fn from_env() -> Result<Vec<Fabricator>> {
    Ok(FabricatorLibrary::from_dirs(&UserDirs::from_env()?)?
        .list()?
        .fabricators)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule_profiles::shipped_profiles;

    fn record(id: &str, name: &str) -> Fabricator {
        Fabricator {
            id: id.into(),
            name: name.into(),
            website: String::new(),
            source: String::new(),
        }
    }

    /// Every shipped profile names a shipped fabricator, but the one a new
    /// board starts with, which is no fabricator's, and every shipped
    /// fabricator has a profile: the hierarchy is complete data.
    #[test]
    fn every_shipped_profile_names_a_shipped_fabricator() -> Result<()> {
        let shipped = ShippedLibrary::locate()?;
        let records = shipped_fabricators(&shipped)?;
        assert!(!records.is_empty());
        let profiles = shipped_profiles(&shipped)?;
        for p in &profiles {
            if p.id == "standard" {
                assert_eq!(p.fabricator, None, "no fabricator's own");
                continue;
            }
            let id = p.fabricator.as_deref().unwrap_or_default();
            assert!(
                records.iter().any(|f| f.id == id),
                "{} names fabricator {id:?}, which has no record",
                p.id
            );
        }
        for f in &records {
            assert!(
                profiles
                    .iter()
                    .any(|p| p.fabricator.as_deref() == Some(&f.id)),
                "{} has no profile",
                f.id
            );
            // Each record round-trips byte for byte.
            let text = shipped.read(Shelf::Fabricators, &format!("{}.json", f.id), 1 << 14)?;
            assert_eq!(encode(f)?, text, "{}", f.id);
        }
        Ok(())
    }

    #[test]
    fn a_user_record_adds_or_replaces_and_a_bad_file_is_named() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let library = FabricatorLibrary::new(dir.path().join("f"), ShippedLibrary::locate()?);
        let before = library.list()?.fabricators.len();
        library.save(&record("my-fab", "My Fab"))?;
        let listing = library.list()?;
        assert_eq!(listing.fabricators.len(), before + 1);
        assert!(listing.fabricators.iter().any(|f| f.name == "My Fab"));
        // A record with a shipped id takes its place.
        let shipped = shipped_fabricators(&ShippedLibrary::locate()?)?;
        let first = shipped.first().map(|f| f.id.clone()).unwrap_or_default();
        library.save(&record(&first, "Renamed"))?;
        let listing = library.list()?;
        assert_eq!(listing.fabricators.len(), before + 1);
        assert_eq!(
            listing
                .fabricators
                .iter()
                .find(|f| f.id == first)
                .map(|f| f.name.as_str()),
            Some("Renamed")
        );
        std::fs::write(library.dir().join("broken.json"), b"{\"format\": 9}")?;
        std::fs::write(
            library.dir().join("other.json"),
            encode(&record("not-other", "X"))?,
        )?;
        let listing = library.list()?;
        assert_eq!(listing.problems.len(), 2, "{:?}", listing.problems);
        assert!(library.save(&record("Bad Id", "X")).is_err());
        Ok(())
    }
}
