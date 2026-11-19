//! Part categories by function, as data (`catalog/categories.json`,
//! `docs/design/part-search-and-package-picker.md` §1.3, §3.1): the one
//! tree every list of parts files them under. The tree and the designator
//! map are read by `sparksmith-io`; nothing here names a category.
//!
//! A component is filed ([`file_component`]) by the family of the generator
//! that made it or its package (`generic-families.json`'s `category`), else
//! by its designator prefix through the file's `designators`, else under
//! `other`. Nothing is inferred from names or values.
use super::generated;
use super::standards::{FamilyPatterns, Standards, check_format};
use super::{ComponentDefinition, Library, designator_prefix_valid};
use crate::diagnostics::{Error, Result};
use crate::ids::ComponentRef;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// The fallback category's id: always present, last, without children.
pub const OTHER: &str = "other";
/// Levels of the tree.
pub const MAX_DEPTH: usize = 3;
/// Bytes of one id.
pub const MAX_ID: usize = 32;
/// Bytes of one title.
pub const MAX_TITLE: usize = 64;
/// Categories one file holds, at every level.
pub const MAX_CATEGORIES: usize = 512;
/// Designator prefixes one file maps.
pub const MAX_DESIGNATORS: usize = 256;

/// One category and the categories under it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CategoryDef {
    pub id: String,
    pub title: String,
    /// The app icon id of its rows and of the parts in it (`part_ic`);
    /// without one, its parent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<CategoryDef>,
}

/// Whether `icon` may name an app icon: 1–32 lower-case ASCII letters,
/// digits and underscores.
fn icon_id(icon: &str) -> bool {
    !icon.is_empty()
        && icon.len() <= MAX_ID
        && icon
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A designator prefix and the category a part with it is filed under
/// when nothing else files it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DesignatorCategory {
    pub prefix: String,
    /// A category path of the same file (`passives/resistors`).
    pub category: String,
}

/// `categories.json`, checked.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Categories {
    pub format: u32,
    #[serde(default)]
    pub about: String,
    pub categories: Vec<CategoryDef>,
    #[serde(default)]
    pub designators: Vec<DesignatorCategory>,
}

/// One category of the tree in order, with its path and depth (0 at the
/// top level).
#[derive(Clone, Copy, Debug)]
pub struct Node<'a> {
    pub def: &'a CategoryDef,
    pub depth: usize,
}

fn slug(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

impl Categories {
    /// Format, ids (slugs, unique among siblings), titles, depth and size
    /// bounds, `other` last with no children, and designators naming a
    /// valid prefix once and a category of the file.
    pub fn check(&self) -> Result<()> {
        check_format(self.format, "the category tree")?;
        let mut count = 0usize;
        check_level(&self.categories, "", 1, &mut count)?;
        match self.categories.last() {
            Some(last) if last.id == OTHER && last.children.is_empty() => {}
            _ => {
                return Err(Error::Invalid(format!(
                    "the category tree must end in ‘{OTHER}’, with no categories under it"
                )));
            }
        }
        if self.categories.iter().filter(|c| c.id == OTHER).count() != 1 {
            return Err(Error::Invalid(format!(
                "the category tree lists ‘{OTHER}’ more than once"
            )));
        }
        if self.designators.len() > MAX_DESIGNATORS {
            return Err(Error::Limit(format!(
                "the category tree maps more than {MAX_DESIGNATORS} designator prefixes"
            )));
        }
        let mut prefixes = BTreeSet::new();
        for d in &self.designators {
            if !designator_prefix_valid(&d.prefix) {
                return Err(Error::Invalid(format!(
                    "the designator prefix ‘{}’ is not 1–8 capital letters",
                    d.prefix.chars().take(16).collect::<String>()
                )));
            }
            if !prefixes.insert(d.prefix.as_str()) {
                return Err(Error::Invalid(format!(
                    "the designator prefix {} is mapped twice",
                    d.prefix
                )));
            }
            if self.find(&d.category).is_none() {
                return Err(Error::Invalid(format!(
                    "the designator prefix {} names ‘{}’, which is no category",
                    d.prefix,
                    d.category.chars().take(80).collect::<String>()
                )));
            }
        }
        Ok(())
    }

    /// The category at `path` (`passives/resistors`).
    pub fn find(&self, path: &str) -> Option<&CategoryDef> {
        let mut level = &self.categories;
        let mut found = None;
        for id in path.split('/') {
            let def = level.iter().find(|c| c.id == id)?;
            level = &def.children;
            found = Some(def);
        }
        found
    }

    /// The icon of the category at `path`: its own, else its nearest
    /// parent's, else Other's.
    pub fn icon(&self, path: &str) -> Option<&str> {
        let mut level = &self.categories;
        let mut icon = None;
        for id in path.split('/') {
            let Some(def) = level.iter().find(|c| c.id == id) else {
                break;
            };
            icon = def.icon.as_deref().or(icon);
            level = &def.children;
        }
        icon.or_else(|| self.find(OTHER).and_then(|o| o.icon.as_deref()))
    }

    /// The category a designator prefix files a part under.
    pub fn by_prefix(&self, prefix: &str) -> Option<&str> {
        self.designators
            .iter()
            .find(|d| d.prefix == prefix)
            .map(|d| d.category.as_str())
    }

    /// Every category, depth first in file order, with its path.
    pub fn walk(&self) -> Vec<(String, Node<'_>)> {
        fn visit<'a>(
            level: &'a [CategoryDef],
            parent: &str,
            depth: usize,
            out: &mut Vec<(String, Node<'a>)>,
        ) {
            for def in level {
                let path = join(parent, &def.id);
                out.push((path.clone(), Node { def, depth }));
                visit(&def.children, &path, depth + 1, out);
            }
        }
        let mut out = Vec::new();
        visit(&self.categories, "", 0, &mut out);
        out
    }
}

fn join(parent: &str, id: &str) -> String {
    if parent.is_empty() {
        id.to_owned()
    } else {
        format!("{parent}/{id}")
    }
}

fn check_level(level: &[CategoryDef], parent: &str, depth: usize, count: &mut usize) -> Result<()> {
    let mut ids = BTreeSet::new();
    for def in level {
        *count += 1;
        if *count > MAX_CATEGORIES {
            return Err(Error::Limit(format!(
                "the category tree holds more than {MAX_CATEGORIES} categories"
            )));
        }
        let path = join(parent, &def.id);
        if !slug(&def.id) {
            return Err(Error::Invalid(format!(
                "the category id ‘{}’ is not 1–{MAX_ID} lower-case letters, digits and hyphens",
                path.chars().take(80).collect::<String>()
            )));
        }
        if !ids.insert(def.id.as_str()) {
            return Err(Error::Invalid(format!(
                "the category {path} is listed twice"
            )));
        }
        if def.icon.as_deref().is_some_and(|i| !icon_id(i)) {
            return Err(Error::Invalid(format!(
                "the category {path} names an icon that is not 1–{MAX_ID} lower-case letters, digits and underscores"
            )));
        }
        let title = def.title.trim();
        if title.is_empty()
            || def.title.len() > MAX_TITLE
            || def.title.chars().any(char::is_control)
        {
            return Err(Error::Invalid(format!(
                "the category {path} needs a printable title of at most {MAX_TITLE} bytes"
            )));
        }
        if !def.children.is_empty() && depth >= MAX_DEPTH {
            return Err(Error::Invalid(format!(
                "the category {path} is nested deeper than {MAX_DEPTH} levels"
            )));
        }
        check_level(&def.children, &path, depth + 1, count)?;
    }
    Ok(())
}

/// The category path a family declares, when it names one.
fn family_category(family: Option<&FamilyPatterns>) -> Option<&str> {
    family.and_then(|f| f.category.as_deref())
}

/// The category path `definition` is filed under (`component` its
/// published revision, `None` for a draft never published): its
/// generator's family, else the family of a generated package it is
/// implemented on, else its designator prefix, else [`OTHER`]. Every
/// answer is a path of `standards.categories`.
pub fn file_component<'a>(
    library: &Library,
    standards: &'a Standards,
    component: Option<ComponentRef>,
    definition: &ComponentDefinition,
) -> &'a str {
    let categories = &standards.categories;
    let of_record = |record: Option<&generated::GeneratedPart>| {
        record.and_then(|r| {
            family_category(
                standards
                    .generic
                    .family_of(&r.from.generator, &r.from.params),
            )
        })
    };
    let made = component.and_then(|c| of_record(generated::record_of(library, c.erased())));
    let on_package = || {
        definition.implementations.iter().find_map(|b| {
            let package = library.binding(*b)?.definition.package;
            of_record(generated::record_of(library, package.erased()))
        })
    };
    made.or_else(on_package)
        .filter(|p| categories.find(p).is_some())
        .or_else(|| categories.by_prefix(&definition.designator_prefix))
        .unwrap_or(OTHER)
}

#[cfg(test)]
mod tests;
