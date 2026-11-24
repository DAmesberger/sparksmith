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
    let topic_dir = shelf.join("topics");
    let manifest = help::Manifest {
        format: 1,
        generated: generated
            .iter()
            .map(|(registry, _)| format!("{registry}.json"))
            .chain(["commands.json".to_owned(), "entries.json".to_owned()])
            .collect(),
        topics: help::files(&topic_dir, "md")?
            .iter()
            .map(|p| help::relative_name(&topic_dir, p))
            .collect::<sparksmith_io::Result<_>>()?,
    };
    compare(
        &shelf.join("manifest.json"),
        &(serde_json::to_string_pretty(&manifest)? + "\n"),
        update,
    )?;
    for (registry, entries) in generated {
        help::sync_registry(
            &shelf.join("generated").join(format!("{registry}.json")),
            entries,
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
        if matches!(name, "generated" | "topics" | "media" | "en" | help::MODEL) {
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
        .args([
            test, "--", "--exact", "--format", "pretty", "--color", "never",
        ])
        .stderr(std::process::Stdio::inherit())
        .env_remove("SPARKSMITH_UPDATE_HELP");
    if update {
        c.env("SPARKSMITH_UPDATE_HELP", "1");
    }
    let output = c.output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    if !output.status.success() || !registry_test_passed(&stdout, test) {
        return Err(format!("help registry test {test} did not pass").into());
    }
    Ok(())
}

// libtest exits successfully for an empty or ignored --exact selection.
fn registry_test_passed(stdout: &str, test: &str) -> bool {
    let success = format!("test {test} ... ok");
    stdout.lines().any(|line| line == success)
}

fn compare(path: &Path, expected: &str, update: bool) -> Result<()> {
    let max = sparksmith_core::format::MAX_FILE_BYTES;
    if expected.len() > max {
        return Err(format!(
            "{} needs {} bytes; the shelf reader accepts {max}",
            path.display(),
            expected.len()
        )
        .into());
    }
    if fs::metadata(path).is_ok_and(|m| m.len() == expected.len() as u64)
        && help::source(path).is_ok_and(|current| current == expected)
    {
        return Ok(());
    }
    if update {
        sparksmith_io::persistence::atomic_write(path, expected.as_bytes())?;
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
        let next = format!(
            "---\n{}\n---\n{body}",
            serde_json::to_string_pretty(&header)?
        );
        sparksmith_io::persistence::atomic_write(&path, next.as_bytes())?;
        return Ok(());
    }
    Err(format!("No authored topic {topic}").into())
}

fn validate_entries(index: &Index, media: &Path) -> Result<()> {
    let profiles: std::collections::BTreeSet<_> = index
        .keys()
        .filter_map(|id| {
            let (profile, _) = id.as_str().strip_prefix("rule/")?.split_once('/')?;
            Some(profile)
        })
        .collect();
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
                rule_key,
                ..
            } => {
                if let Some(name) = rule_key {
                    let key = sparksmith_core::rules::RuleKey::from_name(name)
                        .ok_or_else(|| fail(format!("unknown fabrication rule {name}")))?;
                    // Some fabricators publish no assembly rules. Required keys
                    // must resolve for every installed profile.
                    if !key.optional() {
                        for profile in &profiles {
                            link(&format!("rule/{profile}/{name}").parse()?)?;
                        }
                    }
                }
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
