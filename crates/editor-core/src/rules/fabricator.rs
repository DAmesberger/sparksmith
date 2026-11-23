//! Fabricators as data: who builds boards, above the fab profiles they
//! publish limits for.
//!
//! A fabricator record (`fabricators/<id>.json` in the shipped library or
//! the user's config folder) names a board house: its id, its shown name,
//! its website and where those came from. A fab profile names its
//! fabricator by that id ([`super::RuleProfile::fabricator`]), so the
//! hierarchy fabricator → profiles is data, not text inside a profile's
//! name: profile suggestions prefer another profile of the board's own
//! fabricator before any other one's
//! (`fabrication::suggest`). Nothing in the program names a fabricator.
//!
//! ```json
//! {"format": 1, "fabricator": {"id": "example-fab", "name": "Example Fab",
//!   "website": "https://fab.example", "source": "The fabricator's site, read 2026-11-23"}}
//! ```
use super::MAX_PROFILE_TEXT;
use crate::diagnostics::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The fabricator file's format; any other version is refused.
pub const FABRICATOR_FILE_FORMAT: u32 = 1;

/// Bytes of a fabricator id (it names its file).
pub const MAX_FABRICATOR_ID: usize = 64;

/// One board house.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Fabricator {
    /// Stable identity: lower-case letters, digits and single dashes
    /// ([`valid_id`]); the profiles that are its name it.
    pub id: String,
    /// Shown name.
    pub name: String,
    /// Its website, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub website: String,
    /// Where the name and website were read.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
}

impl Fabricator {
    /// A valid id and name; texts bounded and printable.
    pub fn validate(&self) -> Result<()> {
        let printable = |t: &str| t.len() <= MAX_PROFILE_TEXT && !t.chars().any(char::is_control);
        if !valid_id(&self.id)
            || self.name.trim().is_empty()
            || ![&self.name, &self.website, &self.source]
                .iter()
                .all(|t| printable(t))
        {
            return Err(Error::Invalid(format!(
                "rules.fabricator: a fabricator needs an id (lower-case letters, digits and \
                 dashes, at most {MAX_FABRICATOR_ID} bytes) and a name; texts are printable, at \
                 most {MAX_PROFILE_TEXT} bytes"
            )));
        }
        Ok(())
    }
}

/// One fabricator file: [`FABRICATOR_FILE_FORMAT`] and the record.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FabricatorFile {
    pub format: u32,
    pub fabricator: Fabricator,
}

/// Whether `id` is a fabricator id: lower-case ASCII letters, digits and
/// single dashes, not at either end, at most [`MAX_FABRICATOR_ID`] bytes.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_FABRICATOR_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
}

/// The shown name of the fabricator `id` among `records`, else the id.
pub fn name_of<'a>(records: &'a [Fabricator], id: &'a str) -> &'a str {
    records
        .iter()
        .find(|f| f.id == id)
        .map_or(id, |f| f.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_slugs() {
        for good in ["a", "fab-1", "x2y"] {
            assert!(valid_id(good), "{good}");
        }
        for bad in ["", "-a", "a-", "a--b", "A", "a b", "a_b"] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_record_needs_an_id_and_a_name() {
        let ok = Fabricator {
            id: "fab-a".into(),
            name: "Fab A".into(),
            website: String::new(),
            source: String::new(),
        };
        assert!(ok.validate().is_ok());
        for bad in [
            Fabricator {
                id: "Fab A".into(),
                ..ok.clone()
            },
            Fabricator {
                name: " ".into(),
                ..ok.clone()
            },
            Fabricator {
                source: "a\u{0}b".into(),
                ..ok.clone()
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_profile_names_its_fabricator_by_id() -> Result<()> {
        let mut profile = crate::rules::fixtures::prototype()?;
        profile.fabricator = Some("fab-a".into());
        let text = serde_json::to_string(&profile)?;
        assert!(text.contains(r#""fabricator":"fab-a""#), "{text}");
        let back: crate::rules::RuleProfile = serde_json::from_str(&text)?;
        assert_eq!(back.fabricator.as_deref(), Some("fab-a"));
        profile.fabricator = Some("Fab A".into());
        let text = serde_json::to_string(&profile)?;
        assert!(serde_json::from_str::<crate::rules::RuleProfile>(&text).is_err());
        Ok(())
    }
}
