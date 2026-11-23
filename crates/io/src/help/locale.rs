//! English is authoritative. A locale supplies only text slots from a
//! topic, bound to the exact English text the translator read. References,
//! keys, examples, measurements, citations and schema fields keep their types.
use crate::{
    Error, Result,
    shipped::{Shelf, ShippedLibrary},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, Index, Inline};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Status {
    English,
    Missing,
    Stale,
    Translated,
}
pub struct Localized {
    pub index: Index,
    pub status: BTreeMap<HelpId, Status>,
}

/// A translator edits only the values in `text` and keeps the digest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Translation {
    pub id: HelpId,
    pub english_digest: String,
    pub text: BTreeMap<String, String>,
}

pub fn is_locale(s: &str) -> bool {
    if s.len() > 35 {
        return false;
    }
    let mut parts = s.split('-');
    parts
        .next()
        .is_some_and(|p| (2..=3).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_lowercase()))
        && parts.all(|p| (2..=8).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_alphanumeric()))
}

pub fn load(shipped: &ShippedLibrary, locale: &str) -> Result<Localized> {
    if !is_locale(locale) {
        return Err(Error::Invalid(format!("Invalid help locale {locale:?}")));
    }
    let index = super::load(shipped)?;
    if locale == "en" {
        let status = index
            .keys()
            .map(|id| (id.clone(), Status::English))
            .collect();
        return Ok(Localized { index, status });
    }
    overlay(index, &shipped.shelf(Shelf::Help).join(locale))
}

fn overlay(mut index: Index, dir: &Path) -> Result<Localized> {
    let mut status: BTreeMap<_, _> = index
        .keys()
        .map(|id| (id.clone(), Status::Missing))
        .collect();
    if !dir.try_exists()? {
        return Ok(Localized { index, status });
    }
    if dir.is_symlink() {
        return Err(Error::Invalid(format!(
            "Unexpected locale link {}",
            dir.display()
        )));
    }
    let mut seen = BTreeSet::new();
    for file in super::files(dir, "json")? {
        let tr: Translation = serde_json::from_str(&super::source(&file)?)
            .map_err(|e| Error::Invalid(format!("{}: {e}", file.display())))?;
        if !seen.insert(tr.id.clone()) {
            return Err(Error::Invalid(format!(
                "{}: duplicate translation for {}",
                file.display(),
                tr.id
            )));
        }
        let entry = index.get_mut(&tr.id).ok_or_else(|| {
            Error::Invalid(format!(
                "{}: unknown translated topic {}",
                file.display(),
                tr.id
            ))
        })?;
        let en = translation_source(entry)?;
        let state = if en.english_digest != tr.english_digest {
            Status::Stale
        } else {
            if en.text.keys().ne(tr.text.keys()) {
                return Err(Error::Invalid(format!(
                    "{}: the translation of {} must supply exactly its English text fields",
                    file.display(),
                    tr.id
                )));
            }
            visit_text(entry, |key, s| {
                let translated = tr
                    .text
                    .get(&key)
                    .ok_or_else(|| Error::Invalid(format!("{}: missing {key}", file.display())))?;
                if (!s.trim().is_empty() && translated.trim().is_empty())
                    || translated.contains("[[")
                    || translated.contains("{key:")
                {
                    return Err(Error::Invalid(format!(
                        "{}: invalid translated text in {key}",
                        file.display()
                    )));
                }
                s.clone_from(translated);
                Ok(())
            })?;
            Status::Translated
        };
        status.insert(tr.id, state);
    }
    Ok(Localized { index, status })
}

/// A deterministic skeleton and digest for the given English entry. Changing
/// a measured value, revision or key binding leaves its text digest intact.
pub fn translation_source(entry: &HelpEntry) -> Result<Translation> {
    let mut text = BTreeMap::new();
    visit_text(&mut entry.clone(), |key, s| {
        text.insert(key, s.clone());
        Ok(())
    })?;
    let english_digest = format!("sha256:{:x}", Sha256::digest(serde_json::to_vec(&text)?));
    Ok(Translation {
        id: entry.id.clone(),
        english_digest,
        text,
    })
}

fn visit_text(
    entry: &mut HelpEntry,
    mut visit: impl FnMut(String, &mut String) -> Result<()>,
) -> Result<()> {
    visit("title".into(), &mut entry.title)?;
    visit("summary".into(), &mut entry.summary)?;
    match &mut entry.facts {
        Facts::Finding {
            fix,
            cause,
            explanation,
            ..
        } => {
            visit("finding.fix".into(), fix)?;
            visit("finding.cause".into(), cause)?;
            if let Some(p) = explanation {
                visit("finding.explanation".into(), p)?;
            }
        }
        Facts::Term {
            definition, avoid, ..
        } => {
            visit("term.definition".into(), definition)?;
            for (i, s) in avoid.iter_mut().enumerate() {
                visit(format!("term.avoid.{i}"), s)?;
            }
        }
        _ => {}
    }
    for (i, block) in entry.body.iter_mut().enumerate() {
        let base = format!("body.{i}");
        let mut spans = |key: &str, spans: &mut [Inline]| -> Result<()> {
            for (n, span) in spans.iter_mut().enumerate() {
                if let Inline::Text(text) = span {
                    visit(format!("{key}.{n}"), text)?;
                }
            }
            Ok(())
        };
        match block {
            Block::Paragraph(items) | Block::Note(items) => spans(&base, items)?,
            Block::List(items) => {
                for (n, list) in items.iter_mut().enumerate() {
                    spans(&format!("{base}.{n}"), list)?;
                }
            }
            Block::Table(rows) => {
                for (n, row) in rows.iter_mut().enumerate() {
                    for (c, cell) in row.iter_mut().enumerate() {
                        spans(&format!("{base}.{n}.{c}"), cell)?;
                    }
                }
            }
            Block::Image { alt, .. } => visit(format!("{base}.alt"), alt)?,
            Block::Example(_) => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
