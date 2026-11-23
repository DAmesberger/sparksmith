//! Build the offline manual from its registries, or compare it with a checkout.
mod registries;
mod site;
#[cfg(test)]
mod tests;

use crate::{Result, root};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sparksmith_core::help::{Block, Facts, HelpEntry, HelpId, Index, Inline};
use sparksmith_io::{
    help,
    shipped::{Shelf, ShippedLibrary},
};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub fn run(args: &[String]) -> Result<()> {
    let dir = root()?;
    let library = ShippedLibrary::at(&dir.join("library"))?;
    let shelf = library.shelf(Shelf::Help);
    if let [flag, id] = args
        && flag == "--stamp"
    {
        return stamp(&library, &id.parse()?);
    }
    if let [flag, out] = args
        && flag == "--site"
    {
        return site::render(Path::new(out), &help::load(&library)?, &library);
    }
    let update =
        match args {
            [flag] if flag == "--check" => false,
            [flag] if flag == "--update" => true,
            _ => return Err(
                "usage: cargo xtask help --check | --update | --stamp <topic-id> | --site <out>"
                    .into(),
            ),
        };
    let mut generated = registries::generate(&library, &dir, update)?;
    generated.push(("findings", sparksmith_core::help::finding_entries()?));
    for (registry, mut entries) in generated {
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        compare(
            &shelf.join("generated").join(format!("{registry}.json")),
            &(serde_json::to_string_pretty(&entries)? + "\n"),
            update,
        )?;
    }
    // The app and automation own these registries. Ask their test binaries
    // for the actual declarations without adding either as an xtask dependency.
    registry_test(
        &["-p", "sparksmith", "--bin", "sparksmith-desktop"],
        "commands::tests::help_registry_matches_shipped_commands",
        update,
    )?;
    registry_test(
        &["-p", "sparksmith-automation", "--lib"],
        "session::help_registry::entries_match_the_catalog",
        update,
    )?;
    let index = help::load(&library)?;
    validate_entries(&index, &shelf.join("media"))?;
    for path in help::files(&shelf.join("topics"), "md")? {
        let (header, _) = help::parse_topic(&help::source(&path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        check_covers(&header.id, &header.covers, &index)?;
    }
    for folder in fs::read_dir(&shelf)? {
        let folder = folder?;
        if !folder.file_type()?.is_dir() {
            continue;
        }
        let name = folder.file_name();
        let name = name.to_str().ok_or("non-UTF-8 help folder")?;
        if matches!(name, "generated" | "topics" | "media" | "en") {
            continue;
        }
        let localized = help::locale::load(&library, name)?;
        for (id, state) in &localized.status {
            if *state == help::locale::Status::Stale {
                eprintln!(
                    "{}: {id} fell back to English; the {name} translation is stale",
                    folder.path().display()
                );
            }
        }
        validate_entries(&localized.index, &shelf.join("media"))?;
    }
    println!("help: {} offline topics checked", index.len());
    Ok(())
}

fn registry_test(target: &[&str], test: &str, update: bool) -> Result<()> {
    let mut c = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    c.current_dir(root()?)
        .args(["test", "--locked", "--profile", "ci"])
        .args(target)
        .args([test, "--", "--exact"])
        .env_remove("SPARKSMITH_UPDATE_HELP");
    if update {
        c.env("SPARKSMITH_UPDATE_HELP", "1");
    }
    if !c.status()?.success() {
        return Err(format!("help registry test {test} failed").into());
    }
    Ok(())
}

fn compare(path: &Path, expected: &str, update: bool) -> Result<()> {
    if fs::read(path).is_ok_and(|bytes| bytes == expected.as_bytes()) {
        return Ok(());
    }
    if update {
        fs::write(path, expected)?;
        Ok(())
    } else {
        Err(format!("{} is stale; run cargo xtask help --update", path.display()).into())
    }
}

fn digest(e: &HelpEntry) -> Result<String> {
    let facts = match &e.facts {
        Facts::Entry { schema_digest, .. } => return Ok(schema_digest.clone()),
        // Exclude the prose being stamped against this declaration.
        Facts::Finding {
            severity,
            blocks,
            group,
            ..
        } => json!([severity, blocks, group]),
        facts => serde_json::to_value(facts)?,
    };
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(&facts)?)))
}

fn check_covers(from: &HelpId, covers: &BTreeMap<HelpId, String>, index: &Index) -> Result<()> {
    for (id, stamp) in covers {
        let entry = index
            .get(id)
            .ok_or_else(|| format!("{from} covers missing {id}"))?;
        if digest(entry)? != *stamp {
            return Err(format!(
                "{from} covers a changed {id}; reread it, then cargo xtask help --stamp {from}"
            )
            .into());
        }
    }
    Ok(())
}

fn stamp(library: &ShippedLibrary, topic: &HelpId) -> Result<()> {
    let index = help::load(library)?;
    for path in help::files(&library.shelf(Shelf::Help).join("topics"), "md")? {
        let src = help::source(&path)?;
        let (head, _) = help::parse_topic(&src).map_err(|e| format!("{}: {e}", path.display()))?;
        if head.id != *topic {
            continue;
        }
        let (header, body) = src
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .ok_or("Invalid frontmatter")?;
        let mut header: Value = serde_json::from_str(header)?;
        let mut covers = BTreeMap::new();
        for id in head.covers.keys() {
            covers.insert(
                id,
                digest(
                    index
                        .get(id)
                        .ok_or_else(|| format!("{topic} covers missing {id}"))?,
                )?,
            );
        }
        header["covers"] = serde_json::to_value(covers)?;
        fs::write(
            path,
            format!(
                "---\n{}\n---\n{body}",
                serde_json::to_string_pretty(&header)?
            ),
        )?;
        return Ok(());
    }
    Err(format!("No authored topic {topic}").into())
}

fn validate_entries(index: &Index, media: &Path) -> Result<()> {
    for e in index.values() {
        let fail = |why: String| format!("{}: {why}", e.id);
        let link = |id: &HelpId| -> Result<()> {
            if index.contains_key(id) {
                Ok(())
            } else {
                Err(fail(format!("unknown reference {id}")).into())
            }
        };
        let text = |parts: &[Inline]| -> Result<()> {
            for span in parts {
                match span {
                    Inline::Reference(id) => link(id)?,
                    Inline::Key(id) if id.kind() == sparksmith_core::help::HelpKind::Command => {
                        link(id)?
                    }
                    Inline::Key(id) => {
                        return Err(fail(format!("{id} is not a command key")).into());
                    }
                    Inline::Text(_) => {}
                }
            }
            Ok(())
        };
        let style = |s: &str| -> Result<()> {
            let errors = sparksmith_core::issues::style::check(s);
            if errors.is_empty() {
                Ok(())
            } else {
                Err(fail(format!("{s:?} fails issue style: {errors:?}")).into())
            }
        };
        style(&e.title)?;
        if e.since.trim().is_empty() {
            return Err(fail("missing since version".into()).into());
        }
        for id in &e.related {
            link(id)?;
        }
        match &e.facts {
            Facts::Finding {
                cause,
                fix,
                fixed_by,
                checks,
                ..
            } => {
                if cause.trim().is_empty() {
                    return Err(fail("missing finding cause".into()).into());
                }
                style(fix)?;
                for id in fixed_by.iter().chain(checks) {
                    link(id)?;
                }
            }
            Facts::Command {
                reaches: Some(id), ..
            } => link(id)?,
            Facts::Entry { examples, .. } => {
                for example in examples {
                    link(&example.entry)?;
                }
            }
            _ => {}
        }
        for block in &e.body {
            match block {
                Block::Paragraph(p) | Block::Note(p) => text(p)?,
                Block::List(items) => {
                    for p in items {
                        text(p)?;
                    }
                }
                Block::Table(rows) => {
                    for p in rows.iter().flatten() {
                        text(p)?;
                    }
                }
                Block::Example(call) => link(&call.entry)?,
                Block::Image { file, alt, version } => {
                    if !matches!(
                        Path::new(file).components().collect::<Vec<_>>().as_slice(),
                        [std::path::Component::Normal(_)]
                    ) || alt.trim().is_empty()
                        || version.is_empty()
                        || !media.join(file).is_file()
                    {
                        return Err(fail(format!(
                            "invalid image {file:?}, alt text or capture version"
                        ))
                        .into());
                    }
                }
            }
        }
    }
    Ok(())
}
