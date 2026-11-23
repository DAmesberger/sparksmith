//! `cargo xtask affected [--base REV] [--run]` and `cargo xtask fast [--base REV]`.
//!
//! The changed files are `git diff --name-only BASE...HEAD` plus the
//! working tree (staged, unstaged and untracked files); BASE defaults to
//! the merge-base with `main`. A file belongs to the workspace package
//! whose folder holds it (the deepest one); the rules in
//! `xtask/affected.txt` add the packages whose tests read shared data
//! outside their folder, or every package for the toolchain and lock
//! files. The affected packages are the changed ones, every package that
//! depends on them (any dependency kind, transitively, from
//! `cargo metadata --no-deps`) and the data readers.
//!
//! `fast` is the quick gate before a full `cargo xtask ci`: formatting,
//! the structural gates that run in seconds, strict Clippy on the changed
//! packages only, and the surface lock when the automation crate is
//! affected. It prints one line per gate and stops after the first class
//! of gates that fails.
use crate::{Result, root};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;
use std::time::Instant;

/// A structural gate: its check.
type Gate = fn() -> Result<()>;

/// The shared-data rules, relative to the workspace root.
const RULES_FILE: &str = "xtask/affected.txt";
/// A rule's crate list meaning every workspace package.
const EVERY: &str = "*";
/// Bounds on what the gate reads, so a malformed input cannot exhaust it.
const MAX_FILES: usize = 100_000;
const MAX_RULES: usize = 1_000;
const MAX_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// A workspace package: its name, its folder relative to the root (with
/// a trailing `/`), and the workspace packages it depends on.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Package {
    name: String,
    dir: String,
    depends_on: BTreeSet<String>,
}

/// One line of the rules file.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule {
    prefix: String,
    /// `None`: every package.
    readers: Option<BTreeSet<String>>,
}

/// What the changed files touch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Changes {
    /// A file changed that can change any build.
    every: bool,
    /// Packages whose own folder holds a changed file.
    changed: BTreeSet<String>,
    /// Packages whose tests read changed shared data.
    readers: BTreeSet<String>,
}

/// The packages a command acts on: the whole workspace or a list.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    Workspace,
    Packages(BTreeSet<String>),
}

impl Selection {
    /// Cargo's package arguments for this selection.
    fn cargo_args(&self) -> Vec<String> {
        match self {
            Self::Workspace => vec!["--workspace".into()],
            Self::Packages(names) => names
                .iter()
                .flat_map(|n| ["-p".into(), n.clone()])
                .collect(),
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Packages(names) if names.is_empty())
    }

    fn contains(&self, name: &str) -> bool {
        match self {
            Self::Workspace => true,
            Self::Packages(names) => names.contains(name),
        }
    }

    fn describe(&self) -> String {
        match self {
            Self::Workspace => "the whole workspace".into(),
            Self::Packages(names) if names.is_empty() => "no package".into(),
            Self::Packages(names) => names.iter().cloned().collect::<Vec<_>>().join(" "),
        }
    }
}

/// The workspace packages in `cargo metadata --no-deps` output, with
/// folders relative to its `workspace_root`.
fn packages(metadata: &Value) -> Result<Vec<Package>> {
    let root = metadata
        .get("workspace_root")
        .and_then(Value::as_str)
        .ok_or("metadata has no workspace_root")?;
    let list = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("metadata has no packages")?;
    let relative = |absolute: &Path| -> Result<String> {
        let rel = absolute
            .strip_prefix(root)
            .map_err(|_| format!("{} is outside the workspace", absolute.display()))?;
        let text = rel.to_str().ok_or("a package path is not UTF-8")?;
        Ok(if text.is_empty() {
            String::new()
        } else {
            format!("{}/", text.replace('\\', "/"))
        })
    };
    let mut by_dir = BTreeMap::new();
    let mut found = Vec::new();
    for package in list {
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .ok_or("package has no name")?;
        let manifest = package
            .get("manifest_path")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{name} has no manifest_path"))?;
        let dir = relative(
            Path::new(manifest)
                .parent()
                .ok_or("manifest has no folder")?,
        )?;
        by_dir.insert(dir.clone(), name.to_owned());
        found.push((name.to_owned(), dir, package));
    }
    let mut result = Vec::new();
    for (name, dir, package) in found {
        let mut depends_on = BTreeSet::new();
        for dependency in package
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{name} has no dependencies"))?
        {
            // Only path dependencies can be workspace packages.
            if let Some(path) = dependency.get("path").and_then(Value::as_str)
                && let Some(target) = by_dir.get(&relative(Path::new(path))?)
            {
                depends_on.insert(target.clone());
            }
        }
        result.push(Package {
            name,
            dir,
            depends_on,
        });
    }
    Ok(result)
}

/// The rules in `text`; every named package must exist in `packages`.
fn parse_rules(text: &str, packages: &[Package]) -> Result<Vec<Rule>> {
    let names: BTreeSet<&str> = packages.iter().map(|p| p.name.as_str()).collect();
    let mut rules = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if rules.len() >= MAX_RULES {
            return Err(format!("{RULES_FILE} has more than {MAX_RULES} rules").into());
        }
        let mut words = line.split_whitespace();
        let prefix = words.next().ok_or("empty rule")?.to_owned();
        let crates: Vec<&str> = words.collect();
        let at = number.saturating_add(1);
        if crates.is_empty() {
            return Err(format!("{RULES_FILE}:{at}: {prefix} names no package").into());
        }
        let readers = if crates == [EVERY] {
            None
        } else {
            let mut set = BTreeSet::new();
            for name in crates {
                if !names.contains(name) {
                    return Err(
                        format!("{RULES_FILE}:{at}: {name} is not a workspace package").into(),
                    );
                }
                set.insert(name.to_owned());
            }
            Some(set)
        };
        rules.push(Rule { prefix, readers });
    }
    Ok(rules)
}

/// What `files` (paths relative to the root, `/`-separated) touch.
fn classify(files: &[String], packages: &[Package], rules: &[Rule]) -> Changes {
    let mut changes = Changes::default();
    for file in files {
        for rule in rules.iter().filter(|r| file.starts_with(&r.prefix)) {
            match &rule.readers {
                None => changes.every = true,
                Some(readers) => changes.readers.extend(readers.iter().cloned()),
            }
        }
        // The deepest package folder holding the file; the root package
        // (folder "") holds files no other package does.
        if let Some(owner) = packages
            .iter()
            .filter(|p| file.starts_with(&p.dir))
            .max_by_key(|p| p.dir.len())
        {
            changes.changed.insert(owner.name.clone());
        }
    }
    changes
}

/// `names` and every package that depends on one of them, transitively.
fn with_dependents(names: &BTreeSet<String>, packages: &[Package]) -> BTreeSet<String> {
    let mut result = names.clone();
    let mut queue: Vec<String> = names.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        for package in packages.iter().filter(|p| p.depends_on.contains(&name)) {
            if result.insert(package.name.clone()) {
                queue.push(package.name.clone());
            }
        }
    }
    result
}

/// The packages whose tests a change can change.
fn affected(changes: &Changes, packages: &[Package]) -> Selection {
    if changes.every {
        return Selection::Workspace;
    }
    let mut set = with_dependents(&changes.changed, packages);
    set.extend(changes.readers.iter().cloned());
    Selection::Packages(set)
}

/// The packages whose own code changed, for Clippy.
fn changed(changes: &Changes) -> Selection {
    if changes.every {
        Selection::Workspace
    } else {
        Selection::Packages(changes.changed.clone())
    }
}

fn run_output(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root()?)
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    if output.stdout.len() > MAX_OUTPUT_BYTES {
        return Err(format!("{program} {} printed too much", args.join(" ")).into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn cargo_program() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

/// The changed files since `base` (default: the merge-base with `main`),
/// committed or not.
fn changed_files(base: Option<&str>) -> Result<Vec<String>> {
    let base = match base {
        Some(rev) => rev.to_owned(),
        None => run_output("git", &["merge-base", "main", "HEAD"])?
            .trim()
            .to_owned(),
    };
    let range = format!("{base}...HEAD");
    let mut files = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", "--no-renames", range.as_str()],
        vec!["diff", "--name-only", "--no-renames", "HEAD"],
        vec!["ls-files", "--others", "--exclude-standard"],
    ] {
        for line in run_output("git", &args)?.lines() {
            if !line.is_empty() {
                files.insert(line.to_owned());
            }
        }
        if files.len() > MAX_FILES {
            return Err(format!("more than {MAX_FILES} changed files").into());
        }
    }
    Ok(files.into_iter().collect())
}

/// The workspace, its rules and what the changed files touch.
fn load(base: Option<&str>) -> Result<(Vec<String>, Vec<Package>, Changes)> {
    let metadata: Value = serde_json::from_str(&run_output(
        &cargo_program(),
        &["metadata", "--locked", "--format-version", "1", "--no-deps"],
    )?)?;
    let packages = packages(&metadata)?;
    let rules = parse_rules(
        &std::fs::read_to_string(root()?.join(RULES_FILE))?,
        &packages,
    )?;
    let files = changed_files(base)?;
    let changes = classify(&files, &packages, &rules);
    Ok((files, packages, changes))
}

/// `--base REV` and the flags `allowed`, from `args`.
fn options<'a>(args: &'a [String], allowed: &[&str]) -> Result<(Option<&'a str>, Vec<&'a str>)> {
    let mut base = None;
    let mut flags = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == "--base" {
            base = Some(rest.next().ok_or("--base needs a revision")?.as_str());
        } else if allowed.contains(&arg.as_str()) {
            flags.push(arg.as_str());
        } else {
            return Err(format!("unknown argument {arg}").into());
        }
    }
    Ok((base, flags))
}

/// `cargo xtask affected [--base REV] [--run]`.
pub(crate) fn run_affected(args: &[String]) -> Result<()> {
    let (base, flags) = options(args, &["--run"])
        .map_err(|e| format!("{e}; usage: cargo xtask affected [--base REV] [--run]"))?;
    let (files, packages, changes) = load(base)?;
    let selection = affected(&changes, &packages);
    eprintln!("affected: {} changed files", files.len());
    match &selection {
        Selection::Workspace => println!("--workspace"),
        Selection::Packages(names) => {
            for name in names {
                println!("{name}");
            }
        }
    }
    if !flags.contains(&"--run") {
        return Ok(());
    }
    if selection.is_empty() {
        eprintln!("affected: no package to test");
        return Ok(());
    }
    let mut test = vec![
        "test".to_owned(),
        "--locked".into(),
        "--profile".into(),
        "ci".into(),
        "--no-fail-fast".into(),
    ];
    test.extend(selection.cargo_args());
    let test: Vec<&str> = test.iter().map(String::as_str).collect();
    crate::cargo(&test)
}

/// One gate's result line.
struct Line {
    gate: String,
    seconds: f64,
    error: Option<String>,
}

fn timed(lines: &mut Vec<Line>, gate: &str, check: impl FnOnce() -> Result<()>) -> bool {
    let start = Instant::now();
    let result = check();
    let line = Line {
        gate: gate.to_owned(),
        seconds: start.elapsed().as_secs_f64(),
        error: result.err().map(|e| e.to_string()),
    };
    let ok = line.error.is_none();
    print_line(&line);
    lines.push(line);
    ok
}

fn print_line(line: &Line) {
    match &line.error {
        None => eprintln!("fast: ok    {:<16} {:>6.1}s", line.gate, line.seconds),
        Some(error) => eprintln!(
            "fast: FAIL  {:<16} {:>6.1}s  {error}",
            line.gate, line.seconds
        ),
    }
}

fn summary(lines: &[Line], start: Instant) {
    eprintln!("fast: summary");
    for line in lines {
        print_line(line);
    }
    eprintln!("fast: {:.1}s in all", start.elapsed().as_secs_f64());
}

/// `cargo xtask fast [--base REV]`.
pub(crate) fn run_fast(args: &[String]) -> Result<()> {
    let start = Instant::now();
    let (base, _) =
        options(args, &[]).map_err(|e| format!("{e}; usage: cargo xtask fast [--base REV]"))?;
    let (files, packages, changes) = load(base)?;
    let lint = changed(&changes);
    let tested = affected(&changes, &packages);
    let lock_changed = files.iter().any(|f| f == "Cargo.lock");
    eprintln!(
        "fast: {} changed files; Clippy on {}",
        files.len(),
        lint.describe()
    );
    let mut lines = Vec::new();
    let fail = |lines: &[Line], class: &str| -> Result<()> {
        summary(lines, start);
        Err(format!("fast: the {class} gates failed").into())
    };

    if !timed(&mut lines, "fmt", || {
        crate::cargo(&["fmt", "--all", "--", "--check"])
    }) {
        return fail(&lines, "format");
    }

    let mut structural: Vec<(&str, Gate)> = vec![
        ("layers", crate::layers::check),
        ("core-modules", crate::core_modules::check),
        ("connectivity", crate::connectivity::check),
        ("copper-source", crate::copper_source::check),
        ("f64-lengths", crate::f64_lengths::check),
        ("generic-terms", crate::generic_terms::check),
        ("no-builtin-data", crate::no_builtin_data::check),
        ("clean-room", crate::clean_room::check),
        ("brand-crates", crate::brand_crates::check),
        ("ui-containers", crate::ui_containers::check),
        ("ui-forms", crate::ui_forms::check),
        ("raw-keys", crate::raw_keys::check),
    ];
    if lock_changed {
        structural.push(("licences", crate::licences::check));
    }
    let mut passed = true;
    for (gate, check) in structural {
        passed &= timed(&mut lines, gate, check);
    }
    if !passed {
        return fail(&lines, "structural");
    }

    if !lint.is_empty() {
        let mut clippy = vec![
            "clippy".to_owned(),
            "--locked".into(),
            "--all-targets".into(),
            "--profile".into(),
            "ci".into(),
        ];
        clippy.extend(lint.cargo_args());
        clippy.extend(["--".into(), "-D".into(), "warnings".into()]);
        let clippy: Vec<&str> = clippy.iter().map(String::as_str).collect();
        if !timed(&mut lines, "clippy", || crate::cargo(&clippy)) {
            return fail(&lines, "Clippy");
        }
    }

    // The catalog lock is built from the automation crate's types, so it
    // can change only when that crate is affected; it compiles that
    // crate's tests, so it runs last.
    if tested.contains("sparksmith-automation")
        && !timed(&mut lines, "surface", || {
            crate::surface(&["--check".into()])
        })
    {
        return fail(&lines, "surface");
    }

    summary(&lines, start);
    eprintln!(
        "fast: passed; while iterating run `cargo xtask affected --run` ({})",
        tested.describe()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package(name: &str, dir: &str, depends_on: &[&str]) -> Package {
        Package {
            name: name.into(),
            dir: dir.into(),
            depends_on: depends_on.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    /// core <- io <- automation <- app; core <- ui <- app; ui-extra is a
    /// sibling folder of ui with a name ui is a prefix of; tool is a leaf.
    fn workspace() -> Vec<Package> {
        vec![
            package("core", "crates/core/", &[]),
            package("io", "crates/io/", &["core"]),
            package("automation", "crates/automation/", &["io", "core"]),
            package("ui", "crates/ui/", &["core"]),
            package("ui-extra", "crates/ui-extra/", &[]),
            package("app", "apps/app/", &["automation", "ui"]),
            package("tool", "xtask/", &[]),
        ]
    }

    fn files(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| (*p).to_owned()).collect()
    }

    fn set(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn rules() -> Result<Vec<Rule>> {
        parse_rules("# comment\n\nCargo.lock *\nlibrary/ io app\n", &workspace())
    }

    #[test]
    fn a_leaf_change_affects_its_crate_alone() -> Result<()> {
        let changes = classify(&files(&["xtask/src/main.rs"]), &workspace(), &rules()?);
        assert_eq!(changes.changed, set(&["tool"]));
        assert_eq!(
            affected(&changes, &workspace()),
            Selection::Packages(set(&["tool"]))
        );
        Ok(())
    }

    #[test]
    fn a_core_change_affects_every_dependent_transitively() -> Result<()> {
        let changes = classify(&files(&["crates/core/src/lib.rs"]), &workspace(), &rules()?);
        assert_eq!(changed(&changes), Selection::Packages(set(&["core"])));
        assert_eq!(
            affected(&changes, &workspace()),
            Selection::Packages(set(&["core", "io", "automation", "ui", "app"]))
        );
        Ok(())
    }

    #[test]
    fn a_file_belongs_to_its_own_folder_not_a_name_prefix() -> Result<()> {
        let changes = classify(
            &files(&["crates/ui-extra/src/lib.rs"]),
            &workspace(),
            &rules()?,
        );
        assert_eq!(changes.changed, set(&["ui-extra"]));
        Ok(())
    }

    #[test]
    fn shared_data_adds_its_readers_without_their_dependents() -> Result<()> {
        let changes = classify(
            &files(&["library/profiles/a.json"]),
            &workspace(),
            &rules()?,
        );
        assert!(changes.changed.is_empty());
        assert_eq!(changed(&changes), Selection::Packages(BTreeSet::new()));
        assert_eq!(
            affected(&changes, &workspace()),
            Selection::Packages(set(&["io", "app"]))
        );
        Ok(())
    }

    #[test]
    fn the_lock_file_selects_the_whole_workspace() -> Result<()> {
        let changes = classify(
            &files(&["Cargo.lock", "crates/io/src/lib.rs"]),
            &workspace(),
            &rules()?,
        );
        assert_eq!(affected(&changes, &workspace()), Selection::Workspace);
        assert_eq!(changed(&changes), Selection::Workspace);
        assert_eq!(Selection::Workspace.cargo_args(), vec!["--workspace"]);
        Ok(())
    }

    #[test]
    fn docs_affect_nothing() -> Result<()> {
        let changes = classify(
            &files(&["docs/development.md", "README.md"]),
            &workspace(),
            &rules()?,
        );
        let selection = affected(&changes, &workspace());
        assert!(selection.is_empty(), "{selection:?}");
        Ok(())
    }

    #[test]
    fn a_rule_naming_an_unknown_package_is_refused() {
        let error = parse_rules("library/ nosuch\n", &workspace()).err();
        assert!(error.is_some_and(|e| e.to_string().contains("nosuch is not a workspace package")),);
        assert!(parse_rules("library/\n", &workspace()).is_err());
    }

    #[test]
    fn packages_and_their_path_dependencies_come_from_metadata() -> Result<()> {
        let metadata = json!({
            "workspace_root": "/w",
            "packages": [
                {"name": "core", "manifest_path": "/w/crates/core/Cargo.toml", "dependencies": [
                    {"name": "serde"}
                ]},
                {"name": "io", "manifest_path": "/w/crates/io/Cargo.toml", "dependencies": [
                    {"name": "core", "path": "/w/crates/core"},
                    {"name": "core", "path": "/w/crates/core", "kind": "dev"},
                    {"name": "serde"}
                ]}
            ]
        });
        assert_eq!(
            packages(&metadata)?,
            vec![
                package("core", "crates/core/", &[]),
                package("io", "crates/io/", &["core"]),
            ]
        );
        Ok(())
    }

    #[test]
    fn the_committed_rules_name_workspace_packages() -> Result<()> {
        let metadata: Value = serde_json::from_str(&run_output(
            &cargo_program(),
            &["metadata", "--locked", "--format-version", "1", "--no-deps"],
        )?)?;
        let packages = packages(&metadata)?;
        let rules = parse_rules(
            &std::fs::read_to_string(root()?.join(RULES_FILE))?,
            &packages,
        )?;
        let changes = classify(
            &files(&["crates/editor-core/src/lib.rs"]),
            &packages,
            &rules,
        );
        let Selection::Packages(names) = affected(&changes, &packages) else {
            return Err("a core change is not a workspace-wide change".into());
        };
        for name in ["sparksmith-core", "sparksmith-io", "sparksmith", "xtask"] {
            assert!(names.contains(name), "{name} in {names:?}");
        }
        assert!(!names.contains("sparksmith-gen-chip"), "{names:?}");
        Ok(())
    }
}
