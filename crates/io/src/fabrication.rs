//! Fabrication export: the pre-flight gate, the core's file set, the PDF
//! drawings, the check of the files read back
//! (`sparksmith_core::fabrication::readback`), and an atomic write as a
//! folder or one ZIP. The desktop dialog, the CLI (`sparksmith-cli fab`) and
//! the automation request `project.export_fabrication` (MCP's too) all call
//! [`export`], so nothing is released without both checks.
use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sparksmith_core::fabrication::{
    self, FabFile, FabricationOptions, FileGroup, PreflightReport, ProductionSnapshot, readback,
};
use sparksmith_core::library::Drafts;
use sparksmith_core::rules::RuleProfile;
use sparksmith_core::{Project, fabrication::preflight};
use std::io::{Cursor, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// One export.
#[derive(Clone, Debug)]
pub struct ExportRequest {
    /// A new or empty folder, or with `zip` the `.zip` file to create.
    pub output: PathBuf,
    pub zip: bool,
    /// The fabrication fab profile to check against (the project's own
    /// rules when `None`).
    pub profile: Option<RuleProfile>,
    pub options: FabricationOptions,
}

/// One timed step of an export.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Timing {
    pub step: String,
    pub ms: f64,
}

/// What an export wrote.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ExportReport {
    pub preflight: PreflightReport,
    /// The folder or ZIP written.
    pub output: String,
    /// Every file, in order, with its size.
    pub files: Vec<WrittenFile>,
    pub timings: Vec<Timing>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct WrittenFile {
    pub name: String,
    pub group: FileGroup,
    pub description: String,
    pub bytes: usize,
}

/// Export progress: `step` of `total` named steps.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub step: usize,
    pub total: usize,
    pub label: &'static str,
}

const STEPS: usize = 5;

/// Check, generate, read back and write one fabrication set. The project and the
/// request's profile resolve once to a [`ProductionSnapshot`]; the check,
/// every Gerber, drill, job and assembly file and both drawings are made
/// from it, so every file describes the rules the check passed. Refused
/// (nothing written) when the design cannot take the request's profile
/// (`fabrication.profile_refused`, which no waiver accepts), while the
/// pre-flight report has unwaived errors, when
/// the output exists and is not an empty folder, on any generation
/// error, or when a file read back is not the design
/// (`fabrication.readback_mismatch`, which no waiver accepts): the files are
/// staged (a hidden folder beside the output, a ZIP in memory), read from
/// there, and only then moved or written to the output.
/// `checks` are the host's check settings ([`sparksmith_core::Engine::check_settings`]).
/// `progress` is called before each step and while the files are read back;
/// `Break` stops the export (`fabrication.stopped`) with nothing written; so
/// does cancelling the call it runs for (`sparksmith_generators::cancel`: an
/// agent's cancellation, a closed connection, Ctrl-C).
pub fn export(
    project: &Project,
    drafts: &Drafts,
    request: &ExportRequest,
    checks: &sparksmith_core::layout::CheckSettings,
    progress: &mut dyn FnMut(Progress) -> ControlFlow<()>,
) -> Result<ExportReport> {
    let mut timings = Vec::new();
    let mut timed = |step: &str, start: Instant| {
        timings.push(Timing {
            step: step.into(),
            ms: start.elapsed().as_secs_f64() * 1000.0,
        });
    };
    let mut step = |step: usize, label: &'static str| -> Result<()> {
        let total = STEPS;
        // The caller's stop, or the cancellation of the call this runs for.
        match progress(Progress { step, total, label }) {
            ControlFlow::Continue(()) if !sparksmith_generators::cancel::requested() => Ok(()),
            _ => Err(Error::Document(readback::stopped())),
        }
    };
    check_output(&request.output, request.zip)?;
    step(1, "Checking the design")?;
    let start = Instant::now();
    let snapshot = ProductionSnapshot::resolve(project, request.profile.as_ref())?;
    let report = preflight::check(&snapshot, drafts, checks)?;
    timed("preflight", start);
    if !report.ready() {
        return Err(Error::Document(sparksmith_core::Error::Conflict(format!(
            "fabrication.blocked: {} unwaived error{} in the production check; fix or waive {} before export",
            report.blocking,
            if report.blocking == 1 { "" } else { "s" },
            if report.blocking == 1 { "it" } else { "them" },
        ))));
    }
    step(2, "Making Gerber, drill and assembly files")?;
    let start = Instant::now();
    let mut set = fabrication::generate(&snapshot, &request.options, checks)?;
    timed("generate", start);
    let drawing = set.drawing.take();
    if let Some(data) = &drawing {
        step(3, "Drawing the fabrication and assembly PDFs")?;
        let start = Instant::now();
        let stem = fabrication::base_name(snapshot.project(), &request.options)?;
        for (suffix, description, bytes) in [
            (
                "fabrication.pdf",
                "Fabrication drawing",
                crate::fab_drawings::fabrication_pdf(data)?,
            ),
            (
                "assembly.pdf",
                "Assembly drawing",
                crate::fab_drawings::assembly_pdf(data)?,
            ),
        ] {
            if bytes.is_empty() {
                continue;
            }
            set.push(FabFile {
                name: format!("{stem}-{suffix}"),
                group: FileGroup::Drawing,
                description: description.into(),
                bytes,
            })?;
        }
        timed("drawings", start);
    }
    #[cfg(any(test, feature = "seeded-faults"))]
    seeded::apply(&request.output, &mut set.files);
    const READING: &str = "Reading the files back";
    step(4, READING)?;
    let start = Instant::now();
    let staged = Staged::of(&request.output, request.zip, &set.files)?;
    let written = staged.files()?;
    timed("stage", start);
    let start = Instant::now();
    let found = readback::check(
        &readback::Set {
            snapshot: &snapshot,
            options: &request.options,
            identity: &set.identity,
            drawing: drawing.as_ref(),
        },
        &written,
        &mut |_| step(4, READING).is_ok(),
    )?;
    timed("read back", start);
    if !found.is_empty() {
        return Err(Error::Document(readback::refusal(&found)));
    }
    step(STEPS, "Saving")?;
    let start = Instant::now();
    staged.release(&request.output)?;
    timed("write", start);
    Ok(ExportReport {
        preflight: report,
        output: request.output.display().to_string(),
        files: set
            .files
            .iter()
            .map(|f| WrittenFile {
                name: f.name.clone(),
                group: f.group,
                description: f.description.clone(),
                bytes: f.bytes.len(),
            })
            .collect(),
        timings,
    })
}

/// A ZIP must be a new `.zip` file; a folder must be new or empty.
fn check_output(output: &Path, zip: bool) -> Result<()> {
    if zip {
        if output.extension().and_then(|e| e.to_str()) != Some("zip") {
            return Err(Error::Invalid(
                "fabrication.output: a ZIP output needs a .zip file name".into(),
            ));
        }
        if output.try_exists()? {
            return Err(Error::Invalid(format!(
                "fabrication.output: {} exists; choose a new file",
                output.display()
            )));
        }
        return Ok(());
    }
    if output.try_exists()? && (!output.is_dir() || std::fs::read_dir(output)?.next().is_some()) {
        return Err(Error::Invalid(format!(
            "fabrication.output: {} exists and is not an empty folder; choose a new folder",
            output.display()
        )));
    }
    Ok(())
}

/// A set made and not released: dropped, it leaves nothing.
enum Staged {
    /// In a hidden folder beside the output, with the permissions the output
    /// folder it replaces has.
    Folder {
        folder: tempfile::TempDir,
        #[cfg(unix)]
        existing: Option<u32>,
    },
    Zip(Vec<u8>),
}

impl Staged {
    fn of(output: &Path, zip: bool, files: &[FabFile]) -> Result<Self> {
        if zip {
            return Ok(Self::Zip(zipped(files)?));
        }
        stage_folder(output, files)
    }

    /// Every file as it would be released, read from where it is staged.
    fn files(&self) -> Result<Vec<(String, Vec<u8>)>> {
        let mut out = Vec::new();
        match self {
            Self::Folder { folder, .. } => {
                for entry in std::fs::read_dir(folder.path())? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    out.push((name, std::fs::read(entry.path())?));
                }
                out.sort();
            }
            Self::Zip(bytes) => {
                let broken =
                    |e: zip::result::ZipError| Error::Invalid(format!("fabrication.zip: {e}"));
                let mut archive =
                    zip::ZipArchive::new(Cursor::new(bytes.as_slice())).map_err(broken)?;
                for k in 0..archive.len() {
                    let mut file = archive.by_index(k).map_err(broken)?;
                    let mut bytes = Vec::new();
                    file.read_to_end(&mut bytes)?;
                    out.push((file.name().to_owned(), bytes));
                }
            }
        }
        Ok(out)
    }

    /// Move the files to the output: complete or not at all.
    fn release(self, output: &Path) -> Result<()> {
        match self {
            Self::Zip(bytes) => write_zip(output, &bytes),
            Self::Folder {
                folder,
                #[cfg(unix)]
                existing,
            } => {
                #[cfg(unix)]
                if let Some(existing) = existing {
                    use std::os::unix::fs::PermissionsExt;
                    let mode = std::fs::Permissions::from_mode(existing);
                    std::fs::set_permissions(folder.path(), mode)?;
                }
                std::fs::create_dir_all(folder_of(output))?;
                let staged = folder.keep();
                if let Err(error) = move_into_place(&staged, output) {
                    let _ = std::fs::remove_dir_all(&staged);
                    return Err(error.into());
                }
                Ok(())
            }
        }
    }
}

/// The folder `path` is in.
fn folder_of(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Write every file into a hidden temporary folder.
fn stage_folder(output: &Path, files: &[FabFile]) -> Result<Staged> {
    // Beside the output, or above it where that folder is not made yet: a
    // refused set leaves no folder behind.
    let mut parent = folder_of(output);
    while !parent.try_exists()? && parent != Path::new(".") {
        parent = folder_of(parent);
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".sparksmith-fab-");
    // tempfile's folders are owner-only. The output folder gets the usual
    // permissions (0777 less the umask), or keeps those of the empty
    // folder it replaces.
    #[cfg(unix)]
    let existing = {
        use std::os::unix::fs::PermissionsExt;
        let existing = match std::fs::metadata(output) {
            Ok(meta) => Some(meta.permissions().mode() & 0o777),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        builder.permissions(std::fs::Permissions::from_mode(
            existing.map_or(0o777, |_| 0o700),
        ));
        existing
    };
    let staging = builder.tempdir_in(parent)?;
    for file in files {
        let path = staging.path().join(&file.name);
        let mut f = std::fs::File::create(&path)?;
        f.write_all(&file.bytes)?;
        f.sync_all()?;
    }
    Ok(Staged::Folder {
        folder: staging,
        #[cfg(unix)]
        existing,
    })
}

/// Rename `staged` to `output`. On Unix the rename itself replaces an
/// empty folder the user chose, so a failed rename leaves that folder as
/// it was. Windows refuses to rename over a folder: there the empty folder
/// is removed only after that refusal, and made again if the second rename
/// fails too.
fn move_into_place(staged: &Path, output: &Path) -> std::io::Result<()> {
    match std::fs::rename(staged, output) {
        Err(_)
            if cfg!(windows) && output.is_dir() && std::fs::read_dir(output)?.next().is_none() =>
        {
            std::fs::remove_dir(output)?;
            std::fs::rename(staged, output).inspect_err(|_| {
                let _ = std::fs::create_dir(output);
            })
        }
        other => other,
    }
}

/// One ZIP, entries in set order with a fixed timestamp, so the same set
/// gives the same archive.
fn zipped(files: &[FabFile]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644)
        .system(zip::System::Unix);
    for file in files {
        zip.start_file(file.name.as_str(), options)
            .map_err(|e| Error::Invalid(format!("fabrication.zip: {e}")))?;
        zip.write_all(&file.bytes)?;
    }
    let archive = zip
        .finish()
        .map_err(|e| Error::Invalid(format!("fabrication.zip: {e}")))?;
    Ok(archive.into_inner())
}

/// Write one ZIP atomically. The archive is created, never replaced: a file
/// that appears at `output` after the export's own check is kept and the
/// export is refused (review finding 9).
fn write_zip(output: &Path, archive: &[u8]) -> Result<()> {
    let created = crate::persistence::atomic_create_with(output, |target| {
        target.write_all(archive)?;
        Ok(())
    });
    match created {
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(Error::Invalid(format!(
                "fabrication.output: {} exists; choose a new file",
                output.display()
            )))
        }
        other => other,
    }
}

/// A fabrication profile by id from the user's library
/// (`UserDirs::config/fab-profiles/`) or the shipped library.
pub fn resolve_profile(id: &str) -> Result<RuleProfile> {
    let dirs = crate::settings::UserDirs::from_env()?;
    crate::rule_profiles::ProfileLibrary::from_dirs(&dirs)?.get(id)
}

/// Every installed fab profile (the user's, then the shipped ones no user
/// profile replaces) and every fabricator record (the user's over the
/// shipped ones): what profile suggestions check a board under
/// (`fabrication.profile_suggestions`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Installed {
    pub profiles: Vec<RuleProfile>,
    pub fabricators: Vec<sparksmith_core::rules::Fabricator>,
}

/// [`Installed`] of the user folders `dirs` over the located shipped
/// library.
pub fn installed(dirs: &crate::settings::UserDirs) -> Result<Installed> {
    let listing = crate::rule_profiles::ProfileLibrary::from_dirs(dirs)?.list()?;
    Ok(Installed {
        profiles: listing
            .profiles
            .into_iter()
            .chain(listing.builtins)
            .collect(),
        fabricators: crate::fabricators::FabricatorLibrary::from_dirs(dirs)?
            .list()?
            .fabricators,
    })
}

/// [`installed`] of the user folders the environment names.
pub fn installed_from_env() -> Result<Installed> {
    installed(&crate::settings::UserDirs::from_env()?)
}

/// Faults seeded into a set before it is read back, to show in tests that
/// every way to export refuses them. Never in a shipped build: the feature
/// is enabled by dev-dependencies only.
#[cfg(any(test, feature = "seeded-faults"))]
pub mod seeded {
    use super::{FabFile, Path, PathBuf};
    use std::sync::{Mutex, PoisonError};

    /// What a fault does to the files.
    pub type Fault = Box<dyn Fn(&mut Vec<FabFile>) + Send>;

    static FAULTS: Mutex<Vec<(PathBuf, Fault)>> = Mutex::new(Vec::new());

    /// Seed `fault` into every later export to `output`.
    pub fn seed(output: &Path, fault: Fault) {
        FAULTS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((output.to_owned(), fault));
    }

    pub(super) fn apply(output: &Path, files: &mut Vec<FabFile>) {
        let faults = FAULTS.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, fault) in faults.iter().filter(|(path, _)| path == output) {
            fault(files);
        }
    }

    /// A fault that changes the text of the file whose name ends so.
    pub fn edit(suffix: &'static str, change: fn(&str) -> String) -> Fault {
        Box::new(move |files| {
            for file in files.iter_mut().filter(|f| f.name.ends_with(suffix)) {
                let text: String = file.bytes.iter().map(|b| char::from(*b)).collect();
                let changed = change(&text);
                file.bytes = changed
                    .chars()
                    .filter_map(|c| u8::try_from(u32::from(c)).ok())
                    .collect();
            }
        })
    }

    /// `line`'s number after `axis` (in the units it is written in) changed
    /// by `by`.
    pub fn moved(line: &str, axis: char, by: f64) -> String {
        let Some(at) = line.find(axis) else {
            return line.to_owned();
        };
        let rest = &line[at + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '-' || c == '.'))
            .unwrap_or(rest.len());
        let value: f64 = rest[..end].parse().unwrap_or(0.0);
        let value = if rest[..end].contains('.') {
            format!("{:.3}", value + by)
        } else {
            format!("{}", (value + by).round() as i64)
        };
        format!("{}{axis}{value}{}", &line[..at], &rest[end..])
    }

    /// The first line of `text` that `pick` takes, changed (`None` drops it).
    pub fn first(
        text: &str,
        pick: impl Fn(&str) -> bool,
        change: impl Fn(&str) -> Option<String>,
    ) -> String {
        let mut done = false;
        let mut out = String::new();
        for line in text.split_inclusive('\n') {
            let body = line.trim_end_matches('\n');
            if !done && pick(body) {
                done = true;
                if let Some(changed) = change(body) {
                    out.push_str(&changed);
                    out.push('\n');
                }
            } else {
                out.push_str(line);
            }
        }
        out
    }

    /// The last line of `text` that `pick` takes, changed.
    pub fn last(
        text: &str,
        pick: impl Fn(&str) -> bool,
        change: impl Fn(&str) -> String,
    ) -> String {
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        if let Some(line) = lines.iter_mut().rev().find(|l| pick(l)) {
            *line = change(line);
        }
        lines.join("\n") + "\n"
    }

    /// The faults of every kind of file: what each is, the file it is in.
    pub fn faults() -> Vec<(&'static str, &'static str, Fault)> {
        vec![
            (
                "a pad moved by 0.05 mm",
                "copper-l1-top.gbr",
                edit("copper-l1-top.gbr", |t| {
                    last(
                        t,
                        |l| l.starts_with('X') && l.ends_with("D03*"),
                        |l| moved(l, 'X', 50_000.0),
                    )
                }),
            ),
            (
                "an aperture of another size",
                "copper-l1-top.gbr",
                edit("copper-l1-top.gbr", |t| {
                    first(
                        t,
                        |l| l.starts_with("%ADD") && l.contains("C,"),
                        |l| Some(l.replacen("C,", "C,1", 1)),
                    )
                }),
            ),
            (
                "a track on another net",
                "copper-l1-top.gbr",
                edit("copper-l1-top.gbr", |t| {
                    first(
                        t,
                        |l| l.starts_with("%TO.N,"),
                        |_| Some("%TO.N,WRONG*%".into()),
                    )
                }),
            ),
            (
                "the outer layers swapped",
                "copper-l1-top.gbr",
                Box::new(|files| {
                    let at = |end: &str| files.iter().position(|f| f.name.ends_with(end));
                    if let (Some(i), Some(j)) = (at("copper-l1-top.gbr"), at("-bottom.gbr")) {
                        let bytes = files[i].bytes.clone();
                        files[i].bytes = files[j].bytes.clone();
                        files[j].bytes = bytes;
                    }
                }),
            ),
            (
                "a mask opening missing",
                "mask-top.gbr",
                edit("mask-top.gbr", |t| {
                    first(t, |l| l.ends_with("D03*"), |_| None)
                }),
            ),
            (
                "a paste opening moved by 0.05 mm",
                "paste-top.gbr",
                edit("paste-top.gbr", |t| {
                    first(
                        t,
                        |l| l.starts_with('X') && l.ends_with("D03*"),
                        |l| Some(moved(l, 'Y', 50_000.0)),
                    )
                }),
            ),
            (
                "a legend line missing",
                "legend-top.gbr",
                edit("legend-top.gbr", |t| {
                    first(t, |l| l.starts_with('X') && l.ends_with("D01*"), |_| None)
                }),
            ),
            (
                "the profile moved at one corner by 0.1 mm",
                "profile.gbr",
                edit("profile.gbr", |t| {
                    first(
                        t,
                        |l| l.starts_with('X') && l.ends_with("D01*"),
                        |l| Some(moved(l, 'X', 100_000.0)),
                    )
                }),
            ),
            (
                "a drill of another size",
                "-plated.drl",
                edit("-plated.drl", |t| {
                    first(
                        t,
                        |l| l.starts_with('T') && l.contains('C'),
                        |l| Some(moved(l, 'C', 0.05)),
                    )
                }),
            ),
            (
                "a hole missing",
                "-plated.drl",
                edit("-plated.drl", |t| {
                    first(t, |l| l.starts_with('X') && !l.contains("G85"), |_| None)
                }),
            ),
            (
                "a hole moved by 0.02 mm",
                "-plated.drl",
                edit("-plated.drl", |t| {
                    first(t, |l| l.starts_with('X'), |l| Some(moved(l, 'Y', 0.02)))
                }),
            ),
            (
                "plated holes called non-plated",
                "-plated.drl",
                edit("-plated.drl", |t| t.replace("Plated,1,", "NonPlated,1,")),
            ),
            (
                "a tool's count in the drill table",
                "drill-table.txt",
                edit("drill-table.txt", |t| {
                    first(
                        t,
                        |l| l.starts_with("T1 "),
                        |l| Some(l.replace(" plated ", " non-plated ")),
                    )
                }),
            ),
            (
                "a symbol missing on the drill map",
                "drill-map.gbr",
                Box::new(|files| {
                    for file in files
                        .iter_mut()
                        .filter(|f| f.name.ends_with("drill-map.gbr"))
                    {
                        // The last line drawn is of the last hole's symbol.
                        let text = String::from_utf8_lossy(&file.bytes).into_owned();
                        let mut lines: Vec<&str> = text.lines().collect();
                        if let Some(k) = lines.iter().rposition(|l| l.ends_with("D01*")) {
                            lines.remove(k);
                        }
                        file.bytes = (lines.join("\n") + "\n").into_bytes();
                    }
                }),
            ),
            (
                "another board thickness",
                ".gbrjob",
                edit(".gbrjob", |t| {
                    t.replacen("\"BoardThickness\": ", "\"BoardThickness\": 1", 1)
                }),
            ),
            (
                "another layer count",
                ".gbrjob",
                edit(".gbrjob", |t| {
                    t.replacen("\"LayerNumber\": ", "\"LayerNumber\": 1", 1)
                }),
            ),
            (
                "a part turned by a quarter",
                "pos-top.csv",
                edit("pos-top.csv", |t| {
                    first(
                        t,
                        |l| l.ends_with(",Top"),
                        |l| {
                            let (head, _) = l.rsplit_once(',')?;
                            let (head, angle) = head.rsplit_once(',')?;
                            let angle: f64 = angle.parse().ok()?;
                            Some(format!(
                                "{head},{:.3},Top",
                                (angle + 90.0).rem_euclid(360.0)
                            ))
                        },
                    )
                }),
            ),
            (
                "a part missing from the bill of materials",
                "bom-parts.csv",
                edit("bom-parts.csv", |t| {
                    first(t, |l| !l.starts_with("Designator,"), |_| None)
                }),
            ),
            (
                "a line of the bill of materials missing",
                "bom.csv",
                edit("bom.csv", |t| {
                    first(t, |l| !l.starts_with("Line,"), |_| None)
                }),
            ),
            (
                "another release on the drawing",
                "fabrication.pdf",
                Box::new(|files| {
                    for file in files
                        .iter_mut()
                        .filter(|f| f.name.ends_with("fabrication.pdf"))
                    {
                        // A run of sixteen hexadecimal digits: the release.
                        let hex = |w: &[u8]| {
                            w[0] == b'('
                                && w[17] == b')'
                                && w[1..17].iter().all(u8::is_ascii_hexdigit)
                        };
                        if let Some(k) = file.bytes.windows(18).position(hex) {
                            file.bytes[k + 1] = if file.bytes[k + 1] == b'0' {
                                b'1'
                            } else {
                                b'0'
                            };
                        }
                    }
                }),
            ),
            (
                "a file cut short",
                "assembly.pdf",
                Box::new(|files| {
                    for file in files
                        .iter_mut()
                        .filter(|f| f.name.ends_with("assembly.pdf"))
                    {
                        file.bytes.truncate(file.bytes.len() / 2);
                    }
                }),
            ),
            (
                "a layer missing",
                "profile.gbr",
                Box::new(|files| files.retain(|f| !f.name.ends_with("profile.gbr"))),
            ),
            (
                "a file that is not one of the board's",
                "copper-l9-inner8.gbr",
                Box::new(|files| {
                    if let Some(mut file) = files.first().cloned() {
                        file.name = file.name.replace("l1-top", "l9-inner8");
                        files.push(file);
                    }
                }),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(output: &Path, zip: bool) -> ExportRequest {
        ExportRequest {
            output: output.to_owned(),
            zip,
            profile: None,
            options: FabricationOptions {
                base_name: Some("b".into()),
                ..Default::default()
            },
        }
    }

    fn run(project: &Project, output: &Path, zip: bool) -> Result<ExportReport> {
        export(
            project,
            &Default::default(),
            &request(output, zip),
            &Default::default(),
            &mut |_| ControlFlow::Continue(()),
        )
    }

    /// Nothing is left of a refused export: no output, no staging folder.
    fn assert_nothing(dir: &Path, what: &str) -> Result<()> {
        let left: Vec<_> = std::fs::read_dir(dir)?.collect();
        assert!(left.is_empty(), "{what}: {left:?}");
        Ok(())
    }

    /// The read-back releases what is the design: a folder and a ZIP of the
    /// showcase, under every placement convention and without some outputs.
    #[test]
    fn a_set_that_is_the_design_is_released() -> Result<()> {
        use sparksmith_core::fabrication::{
            BottomRotation, Outputs, PlacementConvention, PlacementOrigin, RotationOffset,
        };
        let project = sparksmith_core::Engine::showcase()?.project().clone();
        let dir = tempfile::tempdir()?;
        // In folders that are not made yet.
        for (out, zip) in [("a/b/fab", false), ("c/d/fab.zip", true)] {
            let output = dir.path().join(out);
            if let Some(parent) = output.parent().filter(|_| zip) {
                std::fs::create_dir_all(parent)?;
            }
            let report = run(&project, &output, zip)?;
            assert!(report.timings.iter().any(|t| t.step == "read back"));
            assert!(output.exists());
        }
        let placement = PlacementConvention {
            origin: PlacementOrigin::OutlineLowerLeft,
            bottom: BottomRotation::ViewedFromBottom,
            mirror_bottom_x: true,
            rotation_offsets: vec![RotationOffset {
                package: "*".into(),
                degrees: 90.0,
            }],
        };
        let outputs = |gerber, drill, assembly, drawings| Outputs {
            gerber,
            drill,
            assembly,
            drawings,
        };
        for (k, outputs) in [
            outputs(true, true, true, true),
            outputs(true, false, false, false),
            outputs(false, true, false, false),
            outputs(false, false, true, false),
            outputs(false, false, false, true),
        ]
        .into_iter()
        .enumerate()
        {
            let mut request = request(&dir.path().join(format!("{k}")), false);
            request.options.placement = placement.clone();
            request.options.outputs = outputs;
            request.options.creation_date = Some("2026-11-24T00:00:00Z".into());
            export(
                &project,
                &Default::default(),
                &request,
                &Default::default(),
                &mut |_| ControlFlow::Continue(()),
            )?;
        }
        Ok(())
    }

    /// Every kind of fault in every kind of file is refused, as a folder and
    /// as a ZIP, naming the file, and nothing is written; no waiver releases
    /// it.
    #[test]
    fn a_fault_in_any_file_is_refused_and_nothing_is_written() -> Result<()> {
        use sparksmith_core::model::production::Waiver;
        let mut project = sparksmith_core::Engine::showcase()?.project().clone();
        project.production.waivers = vec![Waiver {
            key: format!("{} []", readback::MISMATCH),
            reason: "as it is".into(),
            accepted: Vec::new(),
        }];
        for zip in [false, true] {
            for (what, file, fault) in seeded::faults() {
                let dir = tempfile::tempdir()?;
                let output = dir.path().join(if zip { "fab.zip" } else { "fab" });
                seeded::seed(&output, fault);
                let refused = run(&project, &output, zip)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default();
                assert!(
                    refused.contains("do not match the design") && refused.contains(file),
                    "{what}: {refused}"
                );
                assert_nothing(dir.path(), what)?;
            }
        }
        Ok(())
    }

    /// What stands between a fault and its refusal is a difference named in
    /// plain words: the file, the place and both values.
    #[test]
    fn a_refusal_names_the_file_the_place_and_both_values() -> Result<()> {
        let project = sparksmith_core::Engine::showcase()?.project().clone();
        let dir = tempfile::tempdir()?;
        let output = dir.path().join("fab");
        seeded::seed(
            &output,
            seeded::edit("b-plated.drl", |t| {
                seeded::first(t, |l| l.starts_with('X'), |_| None)
            }),
        );
        let refused = run(&project, &output, false)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            refused.contains("b-plated.drl at (")
                && refused.contains("the file has no such hole")
                && refused.contains("the design has a plated hole of "),
            "{refused}"
        );
        Ok(())
    }

    /// Stop at any step, also within the read-back, writes nothing.
    #[test]
    fn a_stopped_export_writes_nothing() -> Result<()> {
        let project = sparksmith_core::Engine::showcase()?.project().clone();
        // The call that stops it; 0 is the one before the release.
        for stop_at in [1, 2, 4, 9, 0] {
            let dir = tempfile::tempdir()?;
            let mut calls = 0;
            let stopped = export(
                &project,
                &Default::default(),
                &request(&dir.path().join("fab"), false),
                &Default::default(),
                &mut |p| {
                    calls += 1;
                    if calls == stop_at || stop_at == 0 && p.step == STEPS {
                        ControlFlow::Break(())
                    } else {
                        ControlFlow::Continue(())
                    }
                },
            );
            match stopped {
                Err(Error::Document(e)) => assert_eq!(e.rule(), Some(readback::STOPPED)),
                other => return Err(Error::Invalid(format!("{stop_at}: {other:?}"))),
            }
            assert_nothing(dir.path(), "stopped")?;
        }
        Ok(())
    }

    /// The finished folder replaces the user's empty one; a move that
    /// fails leaves the user's folder where it was, never deleted.
    #[test]
    fn a_failed_move_keeps_the_users_empty_folder() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let output = dir.path().join("fab");
        std::fs::create_dir(&output)?;
        let missing = dir.path().join(".sparksmith-fab-gone");
        assert!(move_into_place(&missing, &output).is_err());
        assert!(output.is_dir(), "the user's folder is kept");

        let staged = dir.path().join(".sparksmith-fab-done");
        std::fs::create_dir(&staged)?;
        std::fs::write(staged.join("board.gbr"), "G04*")?;
        move_into_place(&staged, &output)?;
        assert_eq!(std::fs::read_to_string(output.join("board.gbr"))?, "G04*");
        assert!(!staged.exists());
        Ok(())
    }

    /// A file that appears at the archive's path after the export checked
    /// it is kept: the export is refused, never written over it.
    #[test]
    fn a_zip_never_replaces_a_file_created_meanwhile() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let output = dir.path().join("fab.zip");
        std::fs::write(&output, b"another program's file")?;
        let files = [FabFile {
            name: "board.gbr".into(),
            group: sparksmith_core::fabrication::FileGroup::Gerber,
            description: String::new(),
            bytes: b"G04*".to_vec(),
        }];
        let refused = write_zip(&output, &zipped(&files)?)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(refused.contains("exists; choose a new file"), "{refused}");
        assert_eq!(std::fs::read(&output)?, b"another program's file");
        assert_eq!(std::fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }

    /// A new output folder gets the usual permissions, like one made by
    /// `create_dir` (not tempfile's owner-only 0700); a chosen empty folder
    /// keeps its own.
    #[cfg(unix)]
    #[test]
    fn output_folders_get_usual_or_kept_permissions() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let mode =
            |p: &Path| -> Result<u32> { Ok(std::fs::metadata(p)?.permissions().mode() & 0o777) };
        let dir = tempfile::tempdir()?;
        let reference = dir.path().join("reference");
        std::fs::create_dir(&reference)?;
        let output = dir.path().join("fab");
        stage_folder(&output, &[])?.release(&output)?;
        assert_eq!(mode(&output)?, mode(&reference)?);

        let chosen = dir.path().join("chosen");
        std::fs::create_dir(&chosen)?;
        std::fs::set_permissions(&chosen, std::fs::Permissions::from_mode(0o750))?;
        stage_folder(&chosen, &[])?.release(&chosen)?;
        assert_eq!(mode(&chosen)?, 0o750);
        Ok(())
    }
}
