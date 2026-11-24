//! A KiCad project imported into a new project, for every host: the CLI's
//! `import-kicad` and the desktop's File › Import KiCad project… run this
//! one function. It reads the board (a `.kicad_pcb`, the board a
//! `.kicad_pro` names by its stem, or the one board in a folder), its
//! project file and its schematics, and builds the project through the one
//! command engine (`sparksmith_kicad::import`) on the fab profile library's
//! profile with SolidCraft's sketch engines. Nothing is written: the host
//! saves the project, and the report's detail where it chooses.
use sparksmith_core::Engine;
use std::path::{Path, PathBuf};

pub use sparksmith_kicad::Report;
pub use sparksmith_kicad::report::{Case, Fidelity, LooserRule};

/// Largest board file read, bytes (the reader's own bound).
const MAX_BOARD: u64 = sparksmith_kicad::sexpr::MAX_INPUT as u64;
/// Largest project file read, bytes (the reader's own bound).
const MAX_PROJECT: u64 = sparksmith_kicad::project::MAX_PROJECT as u64;

/// The extensions this import starts from, without the dot.
pub const EXTENSIONS: &[&str] = &["kicad_pro", "kicad_pcb"];

/// Whether `path` names a KiCad project or board file (any letter case).
pub fn is_kicad_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|k| e.eq_ignore_ascii_case(k)))
}

/// The new project and what the import did.
pub struct KicadImport {
    pub engine: Engine,
    pub report: Report,
    /// The board file read.
    pub board: PathBuf,
    /// Each source pad's board box, for difference reports.
    pub pads: Vec<sparksmith_kicad::PadRecord>,
}

/// The one board file of `source`: a `.kicad_pcb` itself, the board of a
/// `.kicad_pro` (its stem), or the only `.kicad_pcb` directly in a folder
/// (KiCad's backups and hidden files aside).
pub fn board_file(source: &Path) -> Result<PathBuf, String> {
    if !source.is_dir() {
        let is = |ext: &str| {
            source
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        };
        if is("kicad_pro") {
            let board = source.with_extension("kicad_pcb");
            return if board.is_file() {
                Ok(board)
            } else {
                Err("import-kicad: the project has no board beside it (its .kicad_pcb)".into())
            };
        }
        return Ok(source.to_path_buf());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(source).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_file() && name.ends_with(".kicad_pcb") && !name.starts_with(['.', '_']) {
            found.push(path);
        }
    }
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err("import-kicad: the folder holds no .kicad_pcb board".into()),
        more => Err(format!(
            "import-kicad: the folder holds {} boards; name the .kicad_pcb file",
            more.len()
        )),
    }
}

/// Import the KiCad project at `source` into a new project, on the fab
/// profile `profile` names (else the user's default).
pub fn import(source: &Path, profile: Option<&str>) -> Result<KicadImport, String> {
    ImportCatalog::default().import(source, profile)
}

/// Preset matching for successive imports on one host. Completed or
/// interrupted work may be reused while the generator manifests match.
#[derive(Default)]
pub struct ImportCatalog(crate::kicad_models::presets::Presets);

impl ImportCatalog {
    pub fn import(&mut self, source: &Path, profile: Option<&str>) -> Result<KicadImport, String> {
        self.import_on(source, &profile_for(profile)?)
    }

    /// [`Self::import`] on `profile`.
    pub fn import_on(
        &mut self,
        source: &Path,
        profile: &sparksmith_core::rules::RuleProfile,
    ) -> Result<KicadImport, String> {
        import_with(
            source,
            profile,
            Some(crate::session::generators::batch_runtime),
            &mut self.0,
        )
    }
}

/// The fab profile `id` names, else the one a new board takes.
fn profile_for(id: Option<&str>) -> Result<sparksmith_core::rules::RuleProfile, String> {
    sparksmith_io::rule_profiles::for_new_board_from_env(id).map_err(|e| e.to_string())
}

/// The generator runtime an import's models are made with, for its deadline.
pub(crate) type Runtime = fn(
    std::time::Instant,
) -> std::sync::Arc<
    dyn sparksmith_core::library::generated::GeneratorRuntime + Send + Sync,
>;

/// [`import`] with its models made on a test fixture's own worker pool, so
/// parallel tests do not wait on each other's generators.
#[cfg(any(test, feature = "test-fixtures"))]
pub fn import_in_fixture_pool(source: &Path, profile: Option<&str>) -> Result<KicadImport, String> {
    import_with(
        source,
        &profile_for(profile)?,
        Some(crate::session::generators::fixture_batch_runtime),
        &mut Default::default(),
    )
}

/// [`import`] with no parts made by generators: every footprint a project
/// part of its lands. The reference a test compares generated parts'
/// copper with.
#[cfg(test)]
pub(crate) fn import_source_lands(source: &Path) -> Result<KicadImport, String> {
    import_with(source, &profile_for(None)?, None, &mut Default::default())
}

fn import_with(
    source: &Path,
    profile: &sparksmith_core::rules::RuleProfile,
    runtime: Option<Runtime>,
    presets: &mut crate::kicad_models::presets::Presets,
) -> Result<KicadImport, String> {
    let board = board_file(source)?;
    let size = |path: &Path| std::fs::metadata(path).map(|m| m.len());
    if size(&board).map_err(|e| e.to_string())? > MAX_BOARD {
        return Err("import-kicad: the board file is too large".into());
    }
    let bytes = std::fs::read(&board).map_err(|e| e.to_string())?;
    let project = board.with_extension("kicad_pro");
    let project = if project.is_file() {
        if size(&project).map_err(|e| e.to_string())? > MAX_PROJECT {
            return Err("import-kicad: the project file is too large".into());
        }
        Some(std::fs::read(&project).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let folder = sparksmith_io::kicad_files::folder(&board);
    let schematic = sparksmith_io::kicad_files::root_schematic(&board)
        .map(|root| {
            let mut sheets = sparksmith_io::kicad_files::SheetReader::new(&board)?;
            let files = sparksmith_kicad::schematic::collect(&root, |name| sheets.read(name))?;
            Ok::<_, std::io::Error>((root, files))
        })
        .transpose()
        .map_err(|e| e.to_string())?;
    let defaults = sparksmith_io::new_project::shipped().map_err(|e| e.to_string())?;
    let mut models = runtime
        .map(|runtime| crate::kicad_models::Models::new(folder, runtime, presets))
        .transpose()?;
    let imported = sparksmith_kicad::import::import_with_models(
        &bytes,
        &sparksmith_kicad::Setup {
            board: &defaults.board,
            presets: &defaults.presets,
            profile: Some(profile),
            evaluators: crate::session::hosted(),
            project_file: project.as_deref(),
            schematic: schematic
                .as_ref()
                .map(|(root, files)| sparksmith_kicad::schematic::SchematicFiles { root, files }),
        },
        models
            .as_mut()
            .map(|m| m as &mut dyn sparksmith_kicad::models::Provider),
    )
    .map_err(|e| e.to_string())?;
    Ok(KicadImport {
        engine: imported.engine,
        report: imported.report,
        board,
        pads: imported.pads,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kicad_files_are_known_by_extension() {
        assert!(is_kicad_file(Path::new("/a/board.kicad_pcb")));
        assert!(is_kicad_file(Path::new("/a/Board.KICAD_PRO")));
        assert!(!is_kicad_file(Path::new("/a/board.kicad_sch")));
        assert!(!is_kicad_file(Path::new("/a/board.sparksmith")));
    }

    #[test]
    fn a_project_file_names_its_board_by_stem() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let pro = dir.path().join("demo.kicad_pro");
        std::fs::write(&pro, b"{}")?;
        assert!(board_file(&pro).is_err());
        std::fs::write(dir.path().join("demo.kicad_pcb"), b"(kicad_pcb)")?;
        assert_eq!(board_file(&pro)?, dir.path().join("demo.kicad_pcb"));
        assert_eq!(board_file(dir.path())?, dir.path().join("demo.kicad_pcb"));
        Ok(())
    }
}
