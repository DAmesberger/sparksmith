//! The files of a KiCad project's folder, read for the importer
//! (`sparksmith_kicad`, which reads only the bytes it is given): the board's
//! folder, however its path is written, and the schematic files the sheets
//! name in it (`sparksmith_kicad::schematic::collect` asks for them by
//! name). Read only; nothing in the folder is written, and nothing outside
//! it is read.
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};

/// Largest schematic file read, bytes.
pub const MAX_SCHEMATIC: u64 = 64 * 1024 * 1024;
/// Most schematic files read from one project.
pub const MAX_FILES: usize = 512;
/// Most schematic bytes read from one project.
pub const MAX_TOTAL: u64 = 256 * 1024 * 1024;
/// Most folders a schematic file lies under, the project's not counted.
pub const MAX_DEPTH: usize = 8;

/// The folder `board` lies in: the working folder for a bare file name.
pub fn folder(board: &Path) -> &Path {
    match board.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    }
}

/// The name of the root schematic beside `board` (the board's stem), or
/// `None` when there is none.
pub fn root_schematic(board: &Path) -> Option<String> {
    let root = board.with_extension("kicad_sch");
    let name = root.file_name()?.to_str()?.to_owned();
    root.is_file().then_some(name)
}

/// The schematic files directly in a board's folder, by file name.
pub type SchematicFiles = (String, BTreeMap<String, Vec<u8>>);

/// Every schematic file directly in `board`'s folder with the root's name,
/// whether or not a sheet names it, or `None` without a root schematic:
/// what `cargo xtask kicad-compare` reads. The product's import reads by
/// reference ([`SheetReader`]).
pub fn schematic_files(board: &Path) -> std::io::Result<Option<SchematicFiles>> {
    let Some(root) = root_schematic(board) else {
        return Ok(None);
    };
    let mut sheets = SheetReader::new(board)?;
    let mut files = BTreeMap::new();
    for entry in std::fs::read_dir(folder(board))? {
        let name = entry?.file_name();
        let Some(name) = name.to_str().filter(|n| !n.starts_with(['.', '_'])) else {
            continue;
        };
        if let Some(bytes) = sheets.read(name)? {
            files.insert(name.to_owned(), bytes);
        }
    }
    Ok(Some((root, files)))
}

/// Reads a project's schematic files by their names in its folder, within
/// one budget.
pub struct SheetReader {
    root: cap_std::fs::Dir,
    files: usize,
    bytes: u64,
}

impl SheetReader {
    /// A reader of `board`'s folder.
    pub fn new(board: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: cap_std::fs::Dir::open_ambient_dir(folder(board), cap_std::ambient_authority())?,
            files: 0,
            bytes: 0,
        })
    }

    /// The file `name` (`/`-separated, inside the folder), or `None` when
    /// it is no schematic file there: missing, or a link out of the folder.
    pub fn read(&mut self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        let path: PathBuf = name.split('/').collect();
        let inside = path.components().all(|c| matches!(c, Component::Normal(_)));
        if !inside || !name.ends_with(".kicad_sch") || name.split('/').count() > MAX_DEPTH + 1 {
            return Ok(None);
        }
        let absent = |error: &std::io::Error| {
            // Leaving the folder is refused as a permission.
            matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::PermissionDenied
            )
        };
        match self.root.metadata(&path) {
            Ok(found) if found.is_file() => {}
            Ok(_) => return Ok(None),
            Err(error) if absent(&error) => return Ok(None),
            Err(error) => return Err(error),
        }
        let file = self.root.open(&path)?;
        let limit = || {
            std::io::Error::other("kicad.schematic_limit: too many or too large schematic files")
        };
        let allowed = MAX_SCHEMATIC.min(MAX_TOTAL.saturating_sub(self.bytes));
        if self.files >= MAX_FILES || file.metadata()?.len() > allowed {
            return Err(limit());
        }
        let mut bytes = Vec::new();
        file.take(allowed + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > allowed {
            return Err(limit());
        }
        self.files += 1;
        self.bytes += bytes.len() as u64;
        Ok(Some(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_file_name_lies_in_the_working_folder() {
        assert_eq!(folder(Path::new("board.kicad_pcb")), Path::new("."));
        assert_eq!(folder(Path::new("./board.kicad_pcb")), Path::new("."));
        assert_eq!(folder(Path::new("a/board.kicad_pcb")), Path::new("a"));
        assert!(SheetReader::new(Path::new("board.kicad_pcb")).is_ok());
        assert!(schematic_files(Path::new("no-such.kicad_pcb")).is_ok_and(|f| f.is_none()));
    }

    #[test]
    fn only_schematic_files_inside_the_folder_are_read() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join("sub"))?;
        std::fs::write(project.join("sub/a.kicad_sch"), b"a")?;
        std::fs::write(project.join("notes.txt"), b"n")?;
        std::fs::write(dir.path().join("out.kicad_sch"), b"outside")?;
        let mut reader = SheetReader::new(&project.join("board.kicad_pcb"))?;
        assert_eq!(reader.read("sub/a.kicad_sch")?.as_deref(), Some(&b"a"[..]));
        let absolute = dir.path().join("out.kicad_sch");
        for name in [
            "notes.txt",
            "missing.kicad_sch",
            "../out.kicad_sch",
            "sub/../../out.kicad_sch",
            "notes.txt/a.kicad_sch",
            "",
            &absolute.to_string_lossy(),
            "1/2/3/4/5/6/7/8/9/a.kicad_sch",
        ] {
            assert_eq!(reader.read(name)?, None, "{name}");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&absolute, project.join("link.kicad_sch"))?;
            std::os::unix::fs::symlink(dir.path(), project.join("up"))?;
            assert_eq!(reader.read("link.kicad_sch")?, None);
            assert_eq!(reader.read("up/out.kicad_sch")?, None);
        }
        Ok(())
    }

    #[test]
    fn one_budget_covers_every_file() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("a.kicad_sch"), b"12")?;
        let mut reader = SheetReader::new(&dir.path().join("b.kicad_pcb"))?;
        reader.bytes = MAX_TOTAL - 3;
        assert!(reader.read("a.kicad_sch")?.is_some());
        assert!(reader.read("a.kicad_sch").is_err());
        reader.bytes = 0;
        reader.files = MAX_FILES;
        assert!(reader.read("a.kicad_sch").is_err());
        Ok(())
    }
}
