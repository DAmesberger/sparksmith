//! The third-party notices of a package: every crate compiled into it, with
//! its licence, its source and the licence and notice files it ships.
//!
//! The crates are the normal dependencies of the programs and of the shipped
//! generator modules for one target, from `cargo metadata`. Whether each may
//! be used at all is `cargo xtask licences`, which runs first.
use crate::{Result, root};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Folders below a package searched for licence files (a bundled font's).
const DEPTH: usize = 3;
/// The terms of a package that ships no licence file: the first of these its
/// expression names, as a file of the repository.
const STANDARD: [(&str, &str); 3] = [
    ("MIT", "licenses/standard/MIT.txt"),
    ("Apache-2.0", "LICENSE-APACHE"),
    ("BSD-3-Clause", "licenses/standard/BSD-3-Clause.txt"),
];
/// The largest licence or notice file read.
const MAX_BYTES: u64 = 1024 * 1024;

/// The notices of the programs and the shipped modules, built for `target`.
pub(super) fn text(target: &str) -> Result<String> {
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .args(["metadata", "--locked", "--format-version", "1"])
        .args(["--filter-platform", target])
        .current_dir(root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let metadata: Value = serde_json::from_slice(&output.stdout)?;
    let mut crates = Vec::new();
    for package in linked(&metadata)? {
        crates.push(Crate::of(package)?);
    }
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    render(&crates)
}

struct Crate {
    name: String,
    version: String,
    licence: String,
    source: String,
    authors: String,
    /// The licence and notice texts; the standard ones when it ships none.
    files: Vec<String>,
    /// Whether it ships `files` itself.
    own: bool,
}

impl Crate {
    fn of(package: &Value) -> Result<Self> {
        let text = |key: &str| package[key].as_str().unwrap_or_default().to_owned();
        let manifest = PathBuf::from(text("manifest_path"));
        let folder = manifest.parent().ok_or("a package without a folder")?;
        let name = text("name");
        let version = text("version");
        let source = match text("source") {
            s if s.starts_with("registry+") => {
                format!("https://crates.io/crates/{name}/{version}")
            }
            s => s.trim_start_matches("git+").to_owned(),
        };
        let authors = package["authors"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let files = licence_files(folder, package["source"].as_str())?;
        let own = !files.is_empty();
        Ok(Self {
            licence: match text("license") {
                l if l.is_empty() => format!("see its {}", text("license_file")),
                l => l,
            },
            files: if own {
                files
            } else {
                vec![standard(&name, &text("license"))?]
            },
            own,
            name,
            version,
            source,
            authors,
        })
    }
}

/// The standard terms of the package `name`, which ships none.
fn standard(name: &str, expression: &str) -> Result<String> {
    let ids: Vec<&str> = expression
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | '(' | ')'))
        .collect();
    // Terms that all hold need each one's text.
    let (_, file) = STANDARD
        .iter()
        .find(|(id, _)| ids.contains(id) && !ids.contains(&"AND"))
        .ok_or_else(|| {
            format!("{name} ships no licence file, and {expression} has no text here")
        })?;
    Ok(std::fs::read_to_string(root()?.join(file))?
        .trim_end()
        .to_owned())
}

/// The packages linked into the programs and the shipped modules, without
/// Sparksmith's own.
fn linked(metadata: &Value) -> Result<Vec<&Value>> {
    let list = |v: &'_ Value| v.as_array().cloned().unwrap_or_default();
    let packages = metadata["packages"].as_array().ok_or("no packages")?;
    let members: BTreeSet<String> = list(&metadata["workspace_members"])
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let mut queue: Vec<String> = packages
        .iter()
        .filter(|p| {
            let name = p["name"].as_str().unwrap_or_default();
            let manifest = p["manifest_path"].as_str().unwrap_or_default();
            let generator = manifest.replace('\\', "/").contains("/generators/")
                && members.contains(p["id"].as_str().unwrap_or_default());
            super::PROGRAMS.iter().any(|(package, _)| *package == name) || generator
        })
        .filter_map(|p| p["id"].as_str().map(str::to_owned))
        .collect();
    if queue.len() <= super::PROGRAMS.len() {
        return Err("the programs or the generator modules are not in the workspace".into());
    }
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("no resolved graph")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let mut seen = BTreeSet::new();
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes.get(id.as_str()).ok_or("a package without a node")?;
        for dep in list(&node["deps"]) {
            let normal = list(&dep["dep_kinds"]).iter().any(|k| k["kind"].is_null());
            if normal && let Some(id) = dep["pkg"].as_str() {
                queue.push(id.to_owned());
            }
        }
    }
    Ok(packages
        .iter()
        .filter(|p| {
            let id = p["id"].as_str().unwrap_or_default();
            seen.contains(id) && !members.contains(id)
        })
        .collect())
}

/// Whether `name` is a licence or a notice. SolidCraft's brand licence covers
/// crates that are not linked (`cargo xtask brand-crates`), and its collected
/// third-party texts are of its own program.
fn is_notice(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let text = !name.contains('.') || name.ends_with(".txt") || name.ends_with(".md");
    let named = [
        "license",
        "licence",
        "notice",
        "copying",
        "copyright",
        "ofl",
        "ufl",
        "unlicense",
    ]
    .iter()
    .any(|word| name.contains(word));
    (text || name.starts_with("license") || name.starts_with("licence"))
        && named
        && !name.ends_with(".rs")
        && !name.contains("brand")
        && !name.starts_with("third-party")
}

/// The licence and notice files of the package at `folder`; of one in a git
/// checkout also those of the folders above it, up to the checkout.
fn licence_files(folder: &Path, source: Option<&str>) -> Result<Vec<String>> {
    let mut texts = Vec::new();
    collect(folder, DEPTH, &mut texts)?;
    if source.is_some_and(|s| s.starts_with("git+")) {
        let mut at = folder;
        while !at.join(".git").exists()
            && let Some(parent) = at.parent()
        {
            at = parent;
            collect(at, 0, &mut texts)?;
        }
    }
    Ok(texts)
}

fn collect(folder: &Path, depth: usize, texts: &mut Vec<String>) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(folder)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            let code = matches!(
                name.as_str(),
                "src" | "tests" | "target" | "benches" | "plan"
            );
            if depth > 0 && !code && !name.starts_with('.') {
                collect(&entry.path(), depth - 1, texts)?;
            }
        } else if kind.is_file() && is_notice(&name) && entry.metadata()?.len() <= MAX_BYTES {
            let bytes = std::fs::read(entry.path())?;
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            let text = text.trim_end().to_owned();
            if !text.trim().is_empty() && !texts.contains(&text) {
                texts.push(text);
            }
        }
    }
    Ok(())
}

fn render(crates: &[Crate]) -> Result<String> {
    let mut out = format!(
        "Third-party software in {name}\n\n\
         {name} itself is MIT OR Apache-2.0 (LICENSE-MIT, LICENSE-APACHE). NOTICE and\n\
         ATTRIBUTION.md say what was adapted, and licenses/ holds the notices of the fonts, the\n\
         search model and the certificate data. The programs and the shipped generator modules\n\
         also contain the Rust standard library (MIT OR Apache-2.0, https://www.rust-lang.org)\n\
         and the {count} packages below. Where a package offers a choice of licences with GPL,\n\
         LGPL or AGPL among them, another one is taken.\n\n",
        name = sparksmith_core::product::NAME,
        count = crates.len(),
    );
    let mut texts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut order = Vec::new();
    for c in crates {
        write!(
            out,
            "{} {}\n  licence: {}\n  source: {}\n",
            c.name, c.version, c.licence, c.source
        )?;
        if !c.own {
            let by = if c.authors.is_empty() {
                "its authors"
            } else {
                &c.authors
            };
            writeln!(
                out,
                "  copyright: {by} (the package ships no licence file: its licence's own text)"
            )?;
        }
        let mut numbers = Vec::new();
        for text in &c.files {
            let number = *texts.entry(text).or_insert_with(|| {
                order.push(text.as_str());
                order.len()
            });
            numbers.push(number.to_string());
        }
        if !numbers.is_empty() {
            writeln!(out, "  texts: {}", numbers.join(", "))?;
        }
    }
    for (n, text) in order.iter().enumerate() {
        write!(
            out,
            "\n{}\nText {}\n{}\n\n{text}\n",
            "=".repeat(78),
            n + 1,
            "=".repeat(78)
        )?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn licence_files_are_told_from_code() {
        for name in [
            "LICENSE",
            "LICENSE-MIT",
            "LICENSE-MIT.txt",
            "NOTICE",
            "OFL.txt",
            "COPYING",
        ] {
            assert!(is_notice(name), "{name}");
        }
        for name in [
            "licences.rs",
            "LICENSE-BRAND",
            "THIRD-PARTY-LICENSES.txt",
            "lib.rs",
        ] {
            assert!(!is_notice(name), "{name}");
        }
    }

    #[test]
    fn a_package_without_a_file_takes_its_licences_text() -> Result<()> {
        assert!(standard("a", "MIT OR Apache-2.0")?.starts_with("MIT License"));
        assert!(standard("a", "MIT/Apache-2.0")?.starts_with("MIT License"));
        let llvm = standard("a", "Apache-2.0 WITH LLVM-exception")?;
        assert!(llvm.contains("Apache License"));
        assert!(standard("a", "MPL-2.0").is_err());
        Ok(())
    }

    #[test]
    fn only_linked_packages_are_listed() -> Result<()> {
        let package = |id: &str, name: &str, manifest: &str| {
            json!({"id": id, "name": name, "version": "1.0.0", "manifest_path": manifest,
                   "license": "MIT", "authors": [], "source": "registry+x"})
        };
        let dep = |id: &str, kind: Value| json!({"pkg": id, "dep_kinds": [{"kind": kind}]});
        let metadata = json!({
            "workspace_members": ["app", "cli", "chip"],
            "packages": [
                package("app", "sparksmith", "/r/apps/sparksmith/Cargo.toml"),
                package("cli", "sparksmith-cli", "/r/apps/sparksmith-cli/Cargo.toml"),
                package("chip", "generator-chip", "/r/generators/chip/Cargo.toml"),
                package("used", "used", "/c/used/Cargo.toml"),
                package("module", "module", "/c/module/Cargo.toml"),
                package("tested", "tested", "/c/tested/Cargo.toml"),
            ],
            "resolve": {"nodes": [
                {"id": "app", "deps": [dep("used", Value::Null), dep("tested", json!("dev"))]},
                {"id": "cli", "deps": []},
                {"id": "chip", "deps": [dep("module", Value::Null)]},
                {"id": "used", "deps": []},
                {"id": "module", "deps": []},
                {"id": "tested", "deps": []},
            ]},
        });
        let names: Vec<&str> = linked(&metadata)?
            .into_iter()
            .filter_map(|p| p["name"].as_str())
            .collect();
        assert_eq!(names, ["used", "module"]);
        Ok(())
    }
}
