//! Whether a document can still be saved (review finding 15): its saved
//! document, with the identity a first save gives it, within
//! [`CONTAINER_MAX_DOCUMENT_BYTES`], and its container within
//! [`CONTAINER_MAX_BYTES`]. Reading a container and committing an edit
//! both ask, so an accepted document always has a save and a recovery
//! snapshot, and a refused edit leaves the document and its history as
//! they were.
//!
//! An engine asks on every commit, so the measure is kept per
//! copy-on-write section ([`Room`]): an edit measures only the sections it
//! wrote and the small rest of the project, which bounds the document from
//! above. Only a bound past the limit measures the whole document exactly.
use super::{
    CONTAINER_MAX_BYTES, CONTAINER_MAX_DOCUMENT_BYTES, EditorFile, FILE_VERSION,
    container_document_len,
};
use crate::diagnostics::{Error, Result};
use crate::library::Drafts;
use crate::model::{Document, Project};
use serde::Serialize;
use std::sync::Arc;

/// An identity of the shape a first save gives ([`crate::first_save`]):
/// every identity is this long, so a document measured with it is measured
/// as its first save writes it.
const PLACEHOLDER_ID: &str = "00000000-0000-4000-8000-000000000000";
/// Bytes a section's key and separators can take beyond its value, in the
/// bound: a section left out of the skeleton when it is empty loses its key
/// there.
const SECTION_SLACK: usize = 64;
/// Bytes the identity a first save adds can take, in the bound.
pub(super) const IDENTITY_SLACK: usize = 128;

/// Refuse `file` when it could not be saved: [`check_saveable`] at the
/// format's own limits.
pub fn check_file(file: &EditorFile) -> Result<()> {
    check_saveable(file, CONTAINER_MAX_DOCUMENT_BYTES)
}

/// Refuse `file` when its saved document, with an identity when it has
/// none, would exceed `limit` bytes, or its container
/// [`CONTAINER_MAX_BYTES`].
pub fn check_saveable(file: &EditorFile, limit: usize) -> Result<()> {
    exact(file, limit).map(|_| ())
}

/// The saved document's length as its first save writes it, refused past
/// `limit`, with the container total checked.
fn exact(file: &EditorFile, limit: usize) -> Result<usize> {
    let identified;
    let file = if file.project.production.project_id.is_none() {
        let mut copy = file.clone();
        copy.project.production.project_id = Some(PLACEHOLDER_ID.to_owned());
        identified = copy;
        &identified
    } else {
        file
    };
    let document = container_document_len(file, limit).map_err(|_| too_large(limit))?;
    let assets = asset_bytes(&file.project);
    if (document as u64).saturating_add(assets) > CONTAINER_MAX_BYTES {
        return Err(Error::Limit(format!(
            "the project could not be saved: its file would exceed {} MiB",
            CONTAINER_MAX_BYTES / (1024 * 1024)
        )));
    }
    Ok(document)
}

fn too_large(limit: usize) -> Error {
    Error::Limit(format!(
        "the project could not be saved: its document would exceed {} MiB",
        limit / (1024 * 1024)
    ))
}

fn asset_bytes(project: &Project) -> u64 {
    project
        .library
        .assets
        .iter()
        .map(|a| a.data.len() as u64)
        .sum()
}

/// Bytes and line breaks of a value written pretty at depth 0: at depth
/// `d` each line break is followed by `2 d` more spaces.
#[derive(Clone, Copy, Default)]
struct Count {
    bytes: usize,
    lines: usize,
}

impl Count {
    fn of(value: &impl Serialize) -> Result<Self> {
        struct Writer(Count);
        impl std::io::Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.bytes = self.0.bytes.saturating_add(bytes.len());
                self.0.lines = self
                    .0
                    .lines
                    .saturating_add(bytes.iter().filter(|b| **b == b'\n').count());
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = Writer(Count::default());
        serde_json::to_writer_pretty(&mut writer, value)
            .map_err(|e| Error::Invalid(format!("the document cannot be written: {e}")))?;
        Ok(writer.0)
    }

    fn at(self, depth: usize) -> usize {
        self.bytes
            .saturating_add(self.lines.saturating_mul(2 * depth))
            .saturating_add(SECTION_SLACK)
    }
}

/// The sections measured on their own, by field path and the depth their
/// value is written at in the saved document (`project` is depth 1, its
/// fields' values 2, the library's 3).
macro_rules! sections {
    ($mac:ident) => {
        $mac!(
            (instances; 2),
            (occurrences; 2),
            (wires; 2),
            (junctions; 2),
            (nets; 2),
            (placements; 2),
            (tracks; 2),
            (vias; 2),
            (part_dimensions; 2),
            (model_traces; 2),
            (fills; 2),
            (keepouts; 2),
            (blocks; 2),
            (labels; 2),
            (sheet_symbols; 2),
            (sheets.uses; 3),
            (sheets.locals; 3),
            (drawings; 2),
            (interface; 2),
            (library.components; 3),
            (library.symbols; 3),
            (library.packages; 3),
            (library.footprints; 3),
            (library.bindings; 3),
            (library.quantity_kinds; 3),
            (library.fields; 3),
            (library.parts; 3),
            (library.models; 3),
            (library.package_rows; 3),
            (library.origins; 3),
            (library.generated; 3),
            (library.reviews; 3),
        )
    };
}

macro_rules! count_sections {
    ($(($($field:ident).+; $depth:expr)),* $(,)?) => {
        [$(stringify!($($field).+)),*].len()
    };
}
const SECTIONS: usize = sections!(count_sections);

/// The last document an engine measured, its sections' lengths and the
/// rest of it, kept so the next commit measures only what changed.
#[derive(Clone)]
struct Measured {
    document: Document,
    sections: [usize; SECTIONS],
    assets: usize,
    drafts: usize,
    rest: usize,
}

/// An engine's measure of its document's saved size ([`Room::admit`]).
#[derive(Clone)]
pub struct Room {
    limit: usize,
    /// Off for an engine whose commits are steps of one batch, measured
    /// when the batch commits.
    checks: bool,
    measured: Option<Measured>,
}

impl Default for Room {
    fn default() -> Self {
        Self {
            limit: CONTAINER_MAX_DOCUMENT_BYTES,
            checks: true,
            measured: None,
        }
    }
}

impl Room {
    /// A room of `limit` document bytes, for tests that fill one quickly.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    /// A room that admits everything: a batch's scratch engine, whose end
    /// result the real engine measures.
    pub fn unchecked() -> Self {
        Self {
            checks: false,
            ..Self::default()
        }
    }

    /// Forget what was measured: a new document came in whole.
    pub fn forget(&mut self) {
        self.measured = None;
    }

    /// Refuse `candidate` when it could not be saved ([`check_saveable`]):
    /// the bound from the sections it changed first, the exact measure
    /// only when the bound passes the limit.
    pub fn admit(&mut self, candidate: &Document) -> Result<()> {
        if !self.checks {
            return Ok(());
        }
        let measured = self.measure(candidate)?;
        let mut bound = measured
            .sections
            .iter()
            .chain([&measured.assets, &measured.drafts, &measured.rest])
            .fold(0usize, |sum, n| sum.saturating_add(*n));
        if candidate.project.production.project_id.is_none() {
            bound = bound.saturating_add(IDENTITY_SLACK);
        }
        let assets = asset_bytes(&candidate.project);
        let fits =
            bound <= self.limit && (bound as u64).saturating_add(assets) <= CONTAINER_MAX_BYTES;
        if !fits {
            let file = EditorFile {
                version: FILE_VERSION,
                project: (*candidate.project).clone(),
                drafts: (*candidate.drafts).clone(),
            };
            exact(&file, self.limit).map_err(|error| {
                Error::Limit(format!(
                    "{error}. This change was not made, so the project stays one that saves; remove something first, or split the design"
                ))
            })?;
        }
        self.measured = Some(measured);
        Ok(())
    }

    fn measure(&self, candidate: &Document) -> Result<Measured> {
        let before = self.measured.as_ref();
        let new = &candidate.project;
        let mut sections = [0usize; SECTIONS];
        let mut skeleton = (**new).clone();
        let previous = before.map_or([0usize; SECTIONS], |b| b.sections);
        let mut slots = sections.iter_mut().zip(previous);
        macro_rules! measure_sections {
            ($(($($field:ident).+; $depth:expr)),* $(,)?) => {
                $(
                    let (slot, kept) = slots
                        .next()
                        .ok_or_else(|| Error::Invalid("a section is not counted".into()))?;
                    let unchanged = before.is_some_and(|b| {
                        same_section(&*b.document.project.$($field).+, &*new.$($field).+)
                    });
                    *slot = if unchanged {
                        kept
                    } else {
                        Count::of(&new.$($field).+)?.at($depth)
                    };
                    skeleton.$($field).+ = Default::default();
                )*
            };
        }
        sections!(measure_sections);
        let assets = match before {
            Some(before)
                if same_section(
                    &*before.document.project.library.assets,
                    &*new.library.assets,
                ) =>
            {
                before.assets
            }
            _ => {
                let stripped: Vec<_> = new
                    .library
                    .assets
                    .iter()
                    .map(|a| {
                        let mut a = a.clone();
                        a.data = Default::default();
                        a
                    })
                    .collect();
                Count::of(&stripped)?.at(3)
            }
        };
        skeleton.library.assets = Default::default();
        let drafts = match before {
            Some(before) if Arc::ptr_eq(&before.document.drafts, &candidate.drafts) => {
                before.drafts
            }
            _ => Count::of(candidate.drafts.as_ref())?.at(1),
        };
        let rest = match before {
            Some(before) if Arc::ptr_eq(&before.document.project, &candidate.project) => {
                before.rest
            }
            _ => Count::of(&Skeleton {
                version: FILE_VERSION,
                project: &skeleton,
                drafts: &Drafts::default(),
            })?
            .at(0),
        };
        Ok(Measured {
            document: candidate.clone(),
            sections,
            assets,
            drafts,
            rest,
        })
    }
}

/// Whether two copy-on-write sections are one, unchanged: a section's
/// value lives in its one shared allocation.
fn same_section<T>(a: &T, b: &T) -> bool {
    std::ptr::eq(a, b)
}

/// The saved form's outline, with the sections measured on their own left
/// empty.
#[derive(Serialize)]
struct Skeleton<'a> {
    version: u32,
    project: &'a Project,
    drafts: &'a Drafts,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SheetDrawing, SheetItem};

    /// The bound is never below the exact saved length, for a document
    /// with something in many sections, before and after an edit.
    #[test]
    fn the_bound_is_never_below_the_saved_length() -> Result<()> {
        let engine = crate::Engine::demo()?;
        let document = engine.document().clone();
        let mut room = Room::default();
        let measured = room.measure(&document)?;
        let bound: usize = measured.sections.iter().sum::<usize>()
            + measured.assets
            + measured.drafts
            + measured.rest
            + IDENTITY_SLACK;
        let exact = exact(&EditorFile::from_document(&document), usize::MAX)?;
        assert!(bound >= exact, "bound {bound} below {exact}");
        // Above it by the slack per section and the skeleton's empty
        // placeholders: a few KiB, nothing that grows with the document.
        assert!(
            bound - exact <= 4096,
            "bound {bound} exact {exact} sections {:?} assets {} drafts {} rest {}",
            measured.sections,
            measured.assets,
            measured.drafts,
            measured.rest
        );
        room.admit(&document)?;
        Ok(())
    }

    /// A refused candidate leaves the room measuring the document it had.
    #[test]
    fn a_note_past_the_limit_is_refused() -> Result<()> {
        let engine = crate::Engine::demo()?;
        let document = engine.document().clone();
        let exact = exact(&EditorFile::from_document(&document), usize::MAX)?;
        let mut room = Room::with_limit(exact + 1024);
        room.admit(&document)?;
        let mut bigger = document.clone();
        let project = Arc::make_mut(&mut bigger.project);
        let id = project.next_id;
        project.drawings.push(SheetDrawing {
            id,
            sheet: Default::default(),
            item: SheetItem::Text {
                at: Default::default(),
                text: "x".repeat(4096),
                height_nm: crate::units::Nm::new(2_000_000)?,
                rotation: Default::default(),
            },
        });
        let refused = room.admit(&bigger).err().map(|e| e.to_string());
        assert!(
            refused
                .as_deref()
                .is_some_and(|m| m.contains("could not be saved")),
            "{refused:?}"
        );
        Ok(())
    }
}
