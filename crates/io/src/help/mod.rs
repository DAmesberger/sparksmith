//! Load a version's help from the shipped library. Paths come from the shelf,
//! never from the caller's topic id.
pub mod locale;

use crate::{
    Error, Result,
    shipped::{Shelf, ShippedLibrary},
};
use serde::Deserialize;
use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, HelpKind, Index, inlines};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicHeader {
    pub id: HelpId,
    pub kind: HelpKind,
    pub since: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    #[serde(default)]
    pub covers: BTreeMap<HelpId, String>,
    #[serde(default)]
    pub related: Vec<HelpId>,
    #[serde(default)]
    pub keywords: Vec<String>,
    pub fix: Option<String>,
    pub cause: Option<String>,
    pub explanation: Option<String>,
    pub checks: Option<HelpId>,
    pub rule_key: Option<String>,
    #[serde(default)]
    pub fixed_by: Vec<HelpId>,
}

pub fn parse_topic(text: &str) -> std::result::Result<(TopicHeader, Vec<Block>), String> {
    let (head, md) = text
        .strip_prefix("---\n")
        .and_then(|s| s.split_once("\n---\n"))
        .ok_or("Expected a JSON frontmatter block between --- lines")?;
    let header: TopicHeader = serde_json::from_str(head).map_err(|e| e.to_string())?;
    if header.id.kind() != header.kind {
        return Err(format!("{} has the wrong kind", header.id));
    }
    Ok((header, blocks(md)?))
}

pub fn blocks(md: &str) -> std::result::Result<Vec<Block>, String> {
    let mut out = Vec::new();
    let mut lines = md.lines().map(str::trim).peekable();
    while let Some(line) = lines.next() {
        if line.is_empty() {
            continue;
        }
        if line == "```example" {
            let mut json = String::new();
            loop {
                let l = lines.next().ok_or("Unclosed example fence")?;
                if l == "```" {
                    break;
                }
                json.push_str(l);
                json.push('\n');
            }
            out.push(Block::Example(
                serde_json::from_str(&json).map_err(|e| e.to_string())?,
            ));
        } else if let Some(alt) = line.strip_prefix("![") {
            let (alt, target) = alt
                .strip_suffix(')')
                .and_then(|s| s.split_once("]("))
                .ok_or("Invalid help image")?;
            let (file, version) = target
                .strip_suffix('"').and_then(|s| s.rsplit_once(" \""))
                .ok_or("An image needs the captured app version in quotes")?;
            if version.is_empty() || version.len() > 64
                || !version.bytes().all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
            {
                return Err("Invalid captured app version".into());
            }
            if alt.trim().is_empty()
                || !file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                || file.starts_with('.')
                || file.is_empty()
            {
                return Err("An image needs alt text and a media filename".into());
            }
            out.push(Block::Image {
                file: file.into(),
                alt: alt.into(),
                version: version.into(),
            });
        } else if let Some(s) = line.strip_prefix("- ") {
            let mut items = vec![inlines(s)?];
            while let Some(l) = lines.next_if(|l| l.starts_with("- ")) {
                items.push(inlines(&l[2..])?);
            }
            out.push(Block::List(items));
        } else if line.starts_with('|') {
            let mut rows = Vec::new();
            let mut next = Some(line);
            while let Some(row) = next {
                if !row.bytes().all(|c| b"| :-".contains(&c)) {
                    rows.push(
                        row.trim_matches('|')
                            .split('|')
                            .map(|cell| inlines(cell.trim()))
                            .collect::<std::result::Result<_, _>>()?,
                    );
                }
                next = lines.next_if(|l| l.starts_with('|'));
            }
            out.push(Block::Table(rows));
        } else if let Some(note) = line.strip_prefix("> ") {
            out.push(Block::Note(inlines(note)?));
        } else {
            out.push(Block::Paragraph(inlines(line)?));
        }
    }
    Ok(out)
}

/// Return the source files in a shelf subfolder. Do not follow links out
/// of the installed library or descend an unbounded directory tree.
pub fn files(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![(root.to_owned(), 0)];
    let mut seen = 0;
    while let Some((dir, depth)) = dirs.pop() {
        if depth > 16 {
            return Err(Error::Invalid(format!(
                "Too many help folders under {}",
                root.display()
            )));
        }
        for e in
            fs::read_dir(&dir).map_err(|e| Error::Invalid(format!("{}: {e}", dir.display())))?
        {
            seen += 1;
            if seen > 16384 {
                return Err(Error::TooLarge(format!(
                    "Too many help files in {}",
                    root.display()
                )));
            }
            let entry = e?;
            let ty = entry.file_type()?;
            let path = entry.path();
            if ty.is_symlink() {
                return Err(Error::Invalid(format!(
                    "Unexpected link {}",
                    path.display()
                )));
            }
            if ty.is_dir() {
                dirs.push((path, depth + 1));
            } else if ty.is_file() && path.extension().is_some_and(|ext| ext == extension) {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The registry and topic files that make up one release. The installer
/// can lose an entire shard; a directory scan alone cannot distinguish
/// that from a library with no entries of that kind.
#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub generated: BTreeSet<String>,
    pub topics: BTreeSet<String>,
}

pub fn relative_name(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .map_err(|_| Error::Invalid(format!("{} is outside {}", path.display(), root.display())))?
        .components()
        .map(|part| {
            part.as_os_str()
                .to_str()
                .ok_or_else(|| Error::Invalid(format!("Invalid UTF-8 in {}", path.display())))
        })
        .collect::<Result<Vec<_>>>()
        .map(|s| s.join("/"))
}

fn installed(dir: &Path, ext: &str, expected: &BTreeSet<String>) -> Result<Vec<PathBuf>> {
    let paths = files(dir, ext)?;
    let actual = paths
        .iter()
        .map(|p| relative_name(dir, p))
        .collect::<Result<BTreeSet<_>>>()?;
    if let Some(missing) = expected.difference(&actual).next() {
        return Err(Error::Invalid(format!(
            "Missing help file {}/{missing}",
            dir.display()
        )));
    }
    if let Some(extra) = actual.difference(expected).next() {
        return Err(Error::Invalid(format!(
            "Unexpected help file {}/{extra}",
            dir.display()
        )));
    }
    Ok(paths)
}

pub fn source(path: &Path) -> Result<String> {
    let bytes = crate::library_folder::read_bounded(path, 16 * 1024 * 1024)
        .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    String::from_utf8(bytes).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
}

pub fn load(shipped: &ShippedLibrary) -> Result<Index> {
    let root = shipped.shelf(Shelf::Help);
    let manifest_path = root.join("manifest.json");
    let manifest: Manifest = serde_json::from_str(&source(&manifest_path)?)
        .map_err(|e| Error::Invalid(format!("{}: {e}", manifest_path.display())))?;
    if manifest.format != 1 {
        return Err(Error::Invalid(format!(
            "{} has unsupported help format {}",
            manifest_path.display(),
            manifest.format
        )));
    }
    let mut index = Index::new();
    for path in installed(&root.join("generated"), "json", &manifest.generated)? {
        let entries: Vec<HelpEntry> = serde_json::from_str(&source(&path)?)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        for entry in entries {
            let id = entry.id.clone();
            if id.kind() != entry.facts.kind() || index.insert(id.clone(), entry).is_some() {
                return Err(Error::Invalid(format!(
                    "{}: duplicate or mismatched {id}",
                    path.display()
                )));
            }
        }
    }
    let mut topics = BTreeSet::new();
    for path in installed(&root.join("topics"), "md", &manifest.topics)? {
        let text = source(&path)?;
        let (head, body) =
            parse_topic(&text).map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        if !topics.insert(head.id.clone()) {
            return Err(Error::Invalid(format!(
                "{} duplicates {}",
                path.display(),
                head.id
            )));
        }
        apply_topic(&mut index, head, body)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
    }
    for entry in index
        .values_mut()
        .filter(|e| e.id.kind() == HelpKind::Workflow)
    {
        let method = shipped
            .shelf(Shelf::Assist)
            .join("workflows")
            .join(entry.id.name())
            .join("method.md");
        let raw = source(&method)?;
        entry.body = blocks(&raw)
            .map_err(|e| Error::Invalid(format!("{}: {e}", method.display())))?;
    }
    // This topic appears when the harness ships its shared method. Both
    // MCP and chat consume that same library file; no help copy is stored.
    let path = shipped.shelf(Shelf::Assist).join("method.md");
    if path.try_exists()? {
        let raw = source(&path)?;
        let (title, body) = raw
            .lines()
            .next()
            .and_then(|s| s.strip_prefix("# "))
            .map_or(("Agent method", raw.as_str()), |title| {
                (title, raw.split_once('\n').map_or("", |(_, rest)| rest))
            });
        let id: HelpId = "guide/agent-method".parse().map_err(Error::Invalid)?;
        let entry = HelpEntry {
            id: id.clone(),
            title: title.into(),
            summary: body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or(title)
                .into(),
            since: env!("CARGO_PKG_VERSION").into(),
            facts: Facts::Guide,
            body: blocks(body)
                .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?,
            related: vec![],
            keywords: vec![],
        };
        if index.insert(id.clone(), entry).is_some() {
            return Err(Error::Invalid(format!(
                "{id} duplicates {}",
                path.display()
            )));
        }
    }
    Ok(index)
}

fn apply_topic(
    index: &mut Index,
    h: TopicHeader,
    body: Vec<Block>,
) -> std::result::Result<(), String> {
    if !index.contains_key(&h.id) {
        let facts = match h.kind {
            HelpKind::Guide => Facts::Guide,
            HelpKind::Panel => Facts::Panel,
            HelpKind::Workflow => Facts::Workflow,
            _ => return Err(format!("No generated entry for {}", h.id)),
        };
        let title = h.title.clone().ok_or("Topic needs a title")?;
        let summary = h.summary.clone().ok_or("Topic needs a summary")?;
        index.insert(
            h.id.clone(),
            HelpEntry {
                id: h.id.clone(),
                title,
                summary,
                since: h.since.clone(),
                facts,
                body: vec![],
                related: vec![],
                keywords: vec![],
            },
        );
    }
    let e = index.get_mut(&h.id).ok_or("Missing help topic")?;
    if let Some(title) = h.title {
        e.title = title;
    }
    if let Some(summary) = h.summary {
        e.summary = summary;
    }
    if let Facts::Finding {
        fix,
        cause,
        explanation,
        fixed_by,
        checks,
        rule_key,
        ..
    } = &mut e.facts
    {
        *fix = h.fix.ok_or("Finding needs a fix")?;
        *cause = h.cause.ok_or("Finding needs a cause")?;
        *explanation = h.explanation;
        *fixed_by = h.fixed_by;
        *checks = h.checks;
        *rule_key = h.rule_key;
        e.summary = cause.clone();
    }
    e.body.extend(body);
    e.keywords.extend(h.keywords);
    e.related.extend(h.related);
    Ok(())
}
#[cfg(test)]
mod tests;
