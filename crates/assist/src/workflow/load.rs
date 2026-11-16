//! Loading workflows from their folders and deciding which can run here.
//!
//! Shipped workflows are in the library's `assist/workflows/<id>/`; a
//! person's own in `<config>/assist/workflows/<id>/`, which takes the place
//! of a shipped one of the same id. Each folder is checked when read
//! ([`load_folder`]); one that fails is listed as unavailable with the
//! reason and never runs ([`list`]). A workflow whose `needs` this host
//! cannot meet, or whose steps call an entry it lacks or cannot reach, is
//! listed as unavailable too, naming both versions or the entry
//! ([`Workflow::availability`]).
use super::{METHOD_FILE, StepKind, WORKFLOW_FILE, Workflow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sparksmith_core::Class;
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The workflows' folder inside the library's `assist/` and the user's
/// config `assist/`.
pub const FOLDER: &str = "workflows";
/// The most bytes of `workflow.json` and of `method.md`.
pub const MAX_FILE_BYTES: u64 = 256 * 1024;
/// The most workflows read from one folder.
pub const MAX_WORKFLOWS: usize = 64;

/// Where a workflow came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Shipped,
    User,
}

/// What a host offers a workflow: its modules' versions and its entries.
pub trait Catalog {
    /// The version of module `id`, when the host has it.
    fn module_version(&self, id: &str) -> Option<(u16, u16)>;
    /// The module and class of entry `name`, when the host has it.
    fn entry(&self, name: &str) -> Option<(String, Class)>;
}

/// This program's catalog: the declared modules and every catalog entry.
#[derive(Clone, Copy, Debug, Default)]
pub struct HostCatalog;

impl Catalog for HostCatalog {
    fn module_version(&self, id: &str) -> Option<(u16, u16)> {
        sparksmith_core::permission::MODULES
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.version)
    }

    fn entry(&self, name: &str) -> Option<(String, Class)> {
        sparksmith_automation::session::metadata::find(name).map(|e| (e.module.to_owned(), e.class))
    }
}

/// A workflow read and checked from its folder.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    pub workflow: Workflow,
    /// The text of `method.md`.
    pub method: String,
    /// `sha256:` of its files, as a recording names it.
    pub digest: String,
    pub folder: PathBuf,
    pub source: Source,
}

/// A workflow as listed: available, or why not.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    /// The workflow's id, or the folder's name when it did not read.
    pub id: String,
    pub source: Source,
    pub folder: PathBuf,
    /// `None` when the folder failed its check.
    pub loaded: Option<Loaded>,
    /// Empty when it can run here.
    pub why_not: Vec<String>,
}

impl Listed {
    pub fn available(&self) -> bool {
        self.loaded.is_some() && self.why_not.is_empty()
    }
}

/// The shipped workflows' folder.
pub fn shipped_folder(shipped: &ShippedLibrary) -> PathBuf {
    shipped.shelf(Shelf::Assist).join(FOLDER)
}

/// The user's workflows' folder under their config folder.
pub fn user_folder(config: &Path) -> PathBuf {
    config.join("assist").join(FOLDER)
}

/// Read and check the workflow in `folder`: `workflow.json` by the format's
/// rules, its id equal to the folder's name, and `method.md` holding every
/// heading its model steps name. Every refusal names the file.
pub fn load_folder(folder: &Path, source: Source) -> Result<Loaded, String> {
    let json_path = folder.join(WORKFLOW_FILE);
    let json = read_text(&json_path)?;
    let origin = json_path.display().to_string();
    let workflow = Workflow::parse(&json, &origin)?;
    let name = folder.file_name().map(|n| n.to_string_lossy().into_owned());
    if name.as_deref() != Some(workflow.id.as_str()) {
        return Err(format!(
            "workflow {origin}: its id is {:?}; the folder must be named {}",
            workflow.id, workflow.id
        ));
    }
    let method_path = folder.join(METHOD_FILE);
    let method = read_text(&method_path)?;
    workflow.check_method(&method).map_err(|problems| {
        let list: Vec<String> = problems.iter().map(ToString::to_string).collect();
        format!("workflow {origin}: {}", list.join("; "))
    })?;
    Ok(Loaded {
        digest: digest(json.as_bytes(), method.as_bytes()),
        workflow,
        method,
        folder: folder.to_owned(),
        source,
    })
}

/// `sha256:` over both files, each preceded by its name and length.
pub fn digest(workflow: &[u8], method: &[u8]) -> String {
    let mut hash = Sha256::new();
    for (name, bytes) in [(WORKFLOW_FILE, workflow), (METHOD_FILE, method)] {
        hash.update(name.as_bytes());
        hash.update([0]);
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    let hex: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// Every workflow in the shipped and the user's folders, by id; a user's
/// takes the place of a shipped one of the same id. A folder that fails its
/// check is listed with the reason, as is one this host cannot run.
pub fn list(shipped: Option<&Path>, user: Option<&Path>, catalog: &dyn Catalog) -> Vec<Listed> {
    let mut listed: Vec<Listed> = Vec::new();
    for (folder, source) in [(shipped, Source::Shipped), (user, Source::User)] {
        let Some(folder) = folder else { continue };
        for entry in read_folders(folder, source) {
            listed.retain(|l| l.id != entry.id);
            listed.push(entry);
        }
    }
    for entry in &mut listed {
        if let Some(loaded) = &entry.loaded {
            entry.why_not = loaded.workflow.availability(catalog);
        }
    }
    listed.sort_by(|a, b| a.id.cmp(&b.id));
    listed
}

fn read_folders(folder: &Path, source: Source) -> Vec<Listed> {
    let fail = |why: String| Listed {
        id: folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        source,
        folder: folder.to_owned(),
        loaded: None,
        why_not: vec![why],
    };
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![fail(format!("{}: {e}", folder.display()))],
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    if dirs.len() > MAX_WORKFLOWS {
        return vec![fail(format!(
            "{}: more than {MAX_WORKFLOWS} workflows",
            folder.display()
        ))];
    }
    dirs.into_iter()
        .map(|dir| {
            let id = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match load_folder(&dir, source) {
                Ok(loaded) => Listed {
                    id,
                    source,
                    folder: dir,
                    loaded: Some(loaded),
                    why_not: Vec::new(),
                },
                Err(why) => Listed {
                    id,
                    source,
                    folder: dir,
                    loaded: None,
                    why_not: vec![why],
                },
            }
        })
        .collect()
}

fn read_text(path: &Path) -> Result<String, String> {
    let mut text = String::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(format!(
            "{}: larger than {MAX_FILE_BYTES} bytes",
            path.display()
        ));
    }
    Ok(text)
}

impl Workflow {
    /// Why this host cannot run the workflow; empty when it can. Each
    /// `needs` module must be here with the same major and at least the
    /// minor. Each entry a tool step or a confirm gate calls must exist,
    /// belong to a module `needs` names, and have a class within `reach`.
    /// A result action whose entry is missing is not a reason: the card
    /// leaves the button out (a headless host has no `view`).
    pub fn availability(&self, catalog: &dyn Catalog) -> Vec<String> {
        let mut why = Vec::new();
        for need in &self.needs {
            let wanted = format!("{} {}.{}", need.module, need.major, need.minor);
            match catalog.module_version(&need.module) {
                None => why.push(format!(
                    "needs {wanted}; this host has no {} module",
                    need.module
                )),
                Some((major, minor)) if major != need.major || minor < need.minor => {
                    why.push(format!(
                        "needs {wanted}; this host has {} {major}.{minor}",
                        need.module
                    ))
                }
                Some(_) => {}
            }
        }
        let mut calls: Vec<(&str, bool)> = Vec::new();
        for step in &self.steps {
            match step.kind {
                StepKind::Tool => calls.extend(step.call.as_deref().map(|c| (c, true))),
                StepKind::Review => {
                    calls.extend(step.then.as_ref().map(|t| (t.call.as_str(), true)))
                }
                StepKind::Result => {
                    calls.extend(step.actions.iter().map(|a| (a.call.as_str(), false)))
                }
                StepKind::Input | StepKind::Model => {}
            }
        }
        calls.dedup();
        for (call, required) in calls {
            let Some((module, class)) = catalog.entry(call) else {
                if required {
                    why.push(format!("{call} is not an entry of this host"));
                }
                continue;
            };
            if required && !self.needs.iter().any(|n| n.module == module) {
                why.push(format!(
                    "{call} is in the {module} module, which needs does not name"
                ));
            }
            match self.reach.get(&module) {
                None => why.push(format!(
                    "{call} is in the {module} module, which reach leaves out"
                )),
                Some(ceiling) if class > *ceiling => why.push(format!(
                    "{call} is {}; reach allows {module} up to {}",
                    class.name(),
                    ceiling.name()
                )),
                Some(_) => {}
            }
        }
        why
    }
}
