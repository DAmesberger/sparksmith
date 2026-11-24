//! Desktop file workflow. Native picker parenting/filtering follows Solidcraft
//! apps/solidcraft/src/main.rs::services; unsaved choices follow documents.rs.
//! Dialogs stay in the app. Reads, writes, native export and recovery run outside
//! the UI thread; every result retains its document epoch and captured content.
use crate::kit::severity::Level;
use sparksmith_automation::{FileChange, HostFile};
use sparksmith_core::{Document, Engine, FirstSave, Project, format::EditorFile};
use sparksmith_io::recovery::{self, RecoveryEntry, RecoveryRecord, RecoveryStore, Snapshot};
use sparksmith_io::{DiskIdentity, SavedDocument};
use sparksmith_ui::theme::Tokens;
use sparksmith_ui::ui_kit::{self, Button};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

type Outcome<T> = Result<T, String>;

/// Seconds without an edit before a recovery snapshot is taken.
const SNAPSHOT_IDLE: f64 = 2.0;
/// Seconds of continuous editing after which a snapshot is taken anyway:
/// every edit restarts the idle wait, so without this bound steady work
/// would never be kept (review: crash recovery cadence).
pub(crate) const MAX_SNAPSHOT_INTERVAL: f64 = 30.0;

/// Seconds a save or import may take before the app bar says what the
/// system may be holding it for.
const HELD_ACCESS: f64 = 3.0;

/// `work` ("Saving…") after `elapsed` seconds on `path`, and when the text
/// next changes: past [`HELD_ACCESS`] it names the folder the system may
/// be asking permission for.
fn activity_text(work: &str, path: &Path, elapsed: f64) -> (String, Option<Duration>) {
    if elapsed < HELD_ACCESS {
        let wake = Duration::try_from_secs_f64(HELD_ACCESS - elapsed.max(0.0)).ok();
        return (work.to_owned(), wake);
    }
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the folder".to_owned());
    let text =
        format!("{work} waiting for access to {folder}: check for a system permission prompt");
    (text, None)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    epoch: u64,
    revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Ticket {
    id: u64,
    stamp: Stamp,
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum Transition {
    New,
    Open(Option<PathBuf>),
    /// Import a KiCad project (picked when `None`) as a new project.
    Import(Option<PathBuf>),
    Close,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Save,
    Open,
    Export,
    /// Print every schematic sheet to a PDF.
    SchematicPdf,
    Restore,
    /// A KiCad project imported as a new project.
    Import,
    Close,
}
#[derive(Clone)]
struct Flight {
    ticket: Ticket,
    kind: Kind,
    continuation: Option<Transition>,
}

/// Pure lifecycle state: a successful write can save its captured version
/// without marking later edits saved or carrying on with a destructive action.
struct Controller {
    current: Stamp,
    saved: Option<Stamp>,
    path: Option<PathBuf>,
    /// What `path` held when it was opened or last saved here: a save
    /// replaces it only while it still does (review finding 9).
    disk: Option<DiskIdentity>,
    serial: u64,
    flight: Option<Flight>,
    prompt: Option<Transition>,
    recovery_flight: Option<Ticket>,
    recovered: Option<Stamp>,
}
impl Controller {
    fn new(revision: u64) -> Self {
        let current = Stamp { epoch: 0, revision };
        Self {
            saved: Some(current.clone()),
            current,
            path: None,
            disk: None,
            serial: 0,
            flight: None,
            prompt: None,
            recovery_flight: None,
            recovered: None,
        }
    }
    fn dirty(&self) -> bool {
        self.saved.as_ref() != Some(&self.current)
    }
    /// The engine revision this document's file (or new project) holds,
    /// if any.
    fn saved_revision(&self) -> Option<u64> {
        self.saved
            .as_ref()
            .filter(|saved| saved.epoch == self.current.epoch)
            .map(|saved| saved.revision)
    }
    /// The current revision holds the saved content again (an undo or redo
    /// back to it): nothing is unsaved.
    fn back_at_saved(&mut self) {
        self.saved = Some(self.current.clone());
    }
    fn observe(&mut self, revision: u64) -> bool {
        let next = Stamp {
            epoch: self.current.epoch,
            revision,
        };
        let changed = next != self.current;
        self.current = next;
        changed
    }
    fn allocate(&mut self) -> Outcome<Ticket> {
        self.serial = self
            .serial
            .checked_add(1)
            .ok_or("File request identity exhausted")?;
        Ok(Ticket {
            id: self.serial,
            stamp: self.current.clone(),
        })
    }
    fn start(&mut self, kind: Kind, continuation: Option<Transition>) -> Outcome<Ticket> {
        if self.flight.is_some() {
            return Err("Another file operation is still running".into());
        }
        let ticket = self.allocate()?;
        self.flight = Some(Flight {
            ticket: ticket.clone(),
            kind,
            continuation,
        });
        Ok(ticket)
    }
    fn request(&mut self, transition: Transition) -> Option<Transition> {
        if self.flight.is_some() || self.prompt.is_some() {
            return None;
        }
        if self.dirty() {
            self.prompt = Some(transition);
            None
        } else {
            Some(transition)
        }
    }
    fn take_flight(&mut self, ticket: &Ticket, kind: Kind) -> Option<Flight> {
        if self
            .flight
            .as_ref()
            .is_some_and(|f| f.ticket == *ticket && f.kind == kind)
        {
            self.flight.take()
        } else {
            None
        }
    }
    fn cancel(&mut self, ticket: &Ticket, kind: Kind) {
        if let Some(flight) = self.take_flight(ticket, kind) {
            self.prompt = flight.continuation;
        }
    }
    fn saved(
        &mut self,
        ticket: &Ticket,
        path: PathBuf,
        identity: DiskIdentity,
    ) -> Option<(bool, Option<Transition>)> {
        let flight = self.take_flight(ticket, Kind::Save)?;
        if ticket.stamp.epoch != self.current.epoch {
            return None;
        }
        self.path = Some(path);
        self.disk = Some(identity);
        self.saved = Some(ticket.stamp.clone());
        let current = ticket.stamp == self.current;
        if !current {
            self.prompt = flight.continuation;
            return Some((false, None));
        }
        Some((true, flight.continuation))
    }
    fn accept(&mut self, ticket: &Ticket, kind: Kind) -> bool {
        self.take_flight(ticket, kind).is_some() && ticket.stamp == self.current
    }
    fn adopt(
        &mut self,
        revision: u64,
        path: Option<PathBuf>,
        identity: Option<DiskIdentity>,
        recovered: bool,
    ) -> Outcome<()> {
        let epoch = self
            .current
            .epoch
            .checked_add(1)
            .ok_or("Document identity exhausted")?;
        self.current = Stamp { epoch, revision };
        self.saved = if recovered {
            None
        } else {
            Some(self.current.clone())
        };
        self.disk = identity.filter(|_| path.is_some());
        self.path = path;
        self.flight = None;
        self.prompt = None;
        self.recovered = None;
        Ok(())
    }
    fn finish_recovery(&mut self, ticket: &Ticket, success: bool, removed: bool) {
        if self.recovery_flight.as_ref() != Some(ticket) {
            return;
        }
        self.recovery_flight = None;
        if success && ticket.stamp == self.current {
            self.recovered = if removed {
                None
            } else {
                Some(ticket.stamp.clone())
            };
        }
    }
}

pub enum Action {
    New,
    Open,
    OpenPath(PathBuf),
    /// File › Import KiCad project…: pick a `.kicad_pro` or `.kicad_pcb`.
    ImportKicad,
    /// Import this KiCad project or board file (from Finder).
    ImportKicadPath(PathBuf),
    Save,
    SaveAs,
    Export,
    ExportSchematicPdf,
    Close,
}
pub enum Effect {
    Replace {
        document: Box<EditorFile>,
        path: Option<PathBuf>,
        /// What the opened file held, for its next save.
        identity: Option<DiskIdentity>,
        recovered: bool,
    },
    /// A save gave the project its name or identity
    /// (`sparksmith_core::first_save`): the engine takes them
    /// (`Engine::adopt_first_save`), then call [`FileUi::first_saved`].
    /// `saved` says the write saved the current revision.
    FirstSave {
        given: FirstSave,
        saved: bool,
    },
    /// A KiCad project imported from `source`: the new project, unsaved,
    /// and the import's report.
    Imported {
        document: Box<EditorFile>,
        source: PathBuf,
        report: Box<sparksmith_automation::kicad_import::Report>,
    },
    Close,
}

enum Job {
    Read(Ticket, PathBuf),
    Import(Ticket, PathBuf),
    /// Write the document; replace the file only while it holds the
    /// identity, when one is given. A first save names and identifies it
    /// here, off the UI thread: that reads the whole document.
    Write(Ticket, PathBuf, Document, Option<DiskIdentity>),
    SchematicPdf(Ticket, PathBuf, Arc<Project>),
    Recover(Ticket, Document, Option<PathBuf>),
    ClearRecovery(Ticket, bool),
    Restore(Ticket, Snapshot),
    DiscardRecovery(Snapshot),
    /// Discard the older sessions the listing named but did not read.
    DiscardOlder(Vec<Snapshot>),
}
enum Event {
    Startup(Outcome<recovery::Listing>),
    Picked(Ticket, Kind, Option<PathBuf>),
    Read(Ticket, PathBuf, Outcome<(Box<EditorFile>, DiskIdentity)>),
    Imported(
        Ticket,
        PathBuf,
        Outcome<
            Box<(
                EditorFile,
                sparksmith_automation::kicad_import::Report,
                Option<String>,
            )>,
        >,
    ),
    Written(Ticket, PathBuf, Outcome<Wrote>),
    PdfExported(Ticket, PathBuf, Outcome<usize>),
    Recovered(Ticket, bool, bool, Outcome<()>),
    Restored(Ticket, Outcome<Box<(RecoveryRecord, Snapshot)>>),
    Discarded(Snapshot, Outcome<()>),
    /// The older sessions that are gone, and why the rest are not.
    DiscardedOlder(Vec<PathBuf>, Outcome<()>),
}

/// What a save wrote.
enum Wrote {
    /// The file it was for, and what it holds now.
    Saved(Box<SavedDocument>),
    /// The file changed on disk since it was opened: it is kept, and this
    /// document went to a new file beside it.
    Kept(Box<sparksmith_io::Conflict>),
}

/// The file worker. Without a recovery folder (`directory` is the reason)
/// files still open and save, and startup reports that recovery is off.
fn worker(ctx: egui::Context, directory: Outcome<PathBuf>, events: Sender<Event>) -> Sender<Job> {
    let (sender, jobs) = mpsc::channel();
    std::thread::spawn(sparksmith_core::library::standards::carried(move || {
        let mut store = directory.as_ref().map_err(Clone::clone).and_then(|d| {
            RecoveryStore::new(d)
                .map_err(|e| format!("Recovery unavailable: cannot use {}: {e}", d.display()))
        });
        let startup = match (&directory, &store) {
            (Err(error), _) => Err(format!(
                "Recovery unavailable: {error}. Unsaved work is not kept if {product} stops.",
                product = sparksmith_core::product::NAME
            )),
            (_, Err(error)) => Err(format!(
                "{error}. Unsaved work is not kept if {product} stops.",
                product = sparksmith_core::product::NAME
            )),
            (Ok(directory), Ok(_)) => recovery::list(directory).map_err(|e| e.to_string()),
        };
        if events.send(Event::Startup(startup)).is_err() {
            return;
        }
        ctx.request_repaint();
        let mut import_catalog = sparksmith_automation::kicad_import::ImportCatalog::default();
        while let Ok(job) = jobs.recv() {
            let result = match job {
                Job::Read(ticket, path) => {
                    let value = sparksmith_io::open_project(&path)
                        .map(|(document, id)| (Box::new(document), id))
                        .map_err(|e| e.to_string());
                    Event::Read(ticket, path, value)
                }
                Job::Import(ticket, path) => {
                    // The CLI's import, off the UI thread; the window
                    // shows that it runs (`FileUi::importing`).
                    let value = sparksmith_io::rule_profiles::for_new_board_shown_from_env()
                        .map_err(|e| e.to_string())
                        .and_then(|(profile, said)| {
                            let imported = import_catalog.import_on(&path, &profile)?;
                            Ok(Box::new((imported.engine.to_file(), imported.report, said)))
                        });
                    Event::Imported(ticket, path, value)
                }
                Job::Write(ticket, path, document, expected) => {
                    // A random nonce: two new projects saved under one file
                    // name are still two projects.
                    let nonce = sparksmith_io::interface::random_hex().unwrap_or_default();
                    // The deep copy into the file form happens here, off the UI thread.
                    let result =
                        match sparksmith_io::save_document(&path, &document, expected, &nonce) {
                            Ok(saved) => Ok(Wrote::Saved(Box::new(saved))),
                            Err(sparksmith_io::Error::Conflict(conflict)) => {
                                Ok(Wrote::Kept(conflict))
                            }
                            Err(error) => Err(error.to_string()),
                        };
                    Event::Written(ticket, path, result)
                }
                Job::SchematicPdf(ticket, path, project) => {
                    let result =
                        sparksmith_io::schematic_pdf::export_schematic_pdf(&project, &[], &path)
                            .map_err(|e| e.to_string());
                    Event::PdfExported(ticket, path, result)
                }
                Job::Recover(ticket, document, origin) => {
                    let file = EditorFile::from_document(&document);
                    let result = store.as_mut().map_err(|e| e.clone()).and_then(|store| {
                        store
                            .write(&file, origin.as_deref())
                            .map_err(|e| e.to_string())
                    });
                    Event::Recovered(ticket, false, false, result)
                }
                Job::ClearRecovery(ticket, close) => {
                    let result = match &mut store {
                        Ok(store) => store.remove_own().map_err(|e| e.to_string()),
                        Err(_) => Ok(()),
                    };
                    Event::Recovered(ticket, true, close, result)
                }
                Job::Restore(ticket, snapshot) => {
                    let result = directory.as_ref().map_err(Clone::clone).and_then(|dir| {
                        recovery::restore(dir, &snapshot)
                            .map(Box::new)
                            .map_err(|e| e.to_string())
                    });
                    Event::Restored(ticket, result)
                }
                Job::DiscardRecovery(path) => {
                    let result = directory
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|d| recovery::remove_entry(d, &path).map_err(|e| e.to_string()));
                    Event::Discarded(path, result)
                }
                Job::DiscardOlder(paths) => {
                    let mut removed = Vec::new();
                    let mut result = Ok(());
                    for path in paths {
                        match directory.as_ref().map_err(Clone::clone).and_then(|d| {
                            recovery::remove_entry(d, &path).map_err(|e| e.to_string())
                        }) {
                            Ok(()) => removed.push(path.path().to_path_buf()),
                            Err(error) => {
                                result = Err(error);
                                break;
                            }
                        }
                    }
                    Event::DiscardedOlder(removed, result)
                }
            };
            if events.send(result).is_err() {
                break;
            }
            ctx.request_repaint();
        }
    }));
    sender
}

pub struct FileUi {
    controller: Controller,
    /// The current document; cloning shares its data.
    document: Document,
    captured: Option<Document>,
    /// The content last saved (or opened). Undo and redo restore shared
    /// snapshots, so returning to it is seen by identity, not by revision.
    saved_document: Option<Document>,
    picker: rfd::AsyncFileDialog,
    folder: PathBuf,
    sender: Sender<Job>,
    event_sender: Sender<Event>,
    receiver: Receiver<Event>,
    effects: Vec<Effect>,
    /// The latest file message and how it reads.
    status: Option<(Level, String)>,
    recoveries: Vec<RecoveryEntry>,
    /// Older session files past the listing's read limit, named so the
    /// recovery question can say so and offer to discard them.
    older_recoveries: Vec<Snapshot>,
    /// The recovery the dialog would restore or discard.
    recovery_selected: usize,
    /// A restored session file, removed once this session has saved its
    /// own recovery snapshot of the restored document (so the work is never
    /// in no file at all).
    restored_source: Option<(u64, Snapshot)>,
    /// The file a restored session was opened from, or the KiCad file an
    /// imported project came from: until the project has
    /// a path, the save panel proposes its name and folder.
    restored_origin: Option<PathBuf>,
    recovery_choice_pending: bool,
    changed_at: f64,
    /// When the first edit no snapshot was started for happened.
    unkept_since: Option<f64>,
    recovery_retry_at: f64,
    now: f64,
    /// Quitting was asked for while a file operation ran: ask once it ends.
    close_pending: bool,
    /// The save request whose file is being written (not while its save
    /// panel is open), for [`FileUi::saving`].
    writing: Option<u64>,
    /// The KiCad import running (not while its file panel is open), for
    /// [`FileUi::importing`].
    importing: Option<u64>,
    /// When the running save or import sent its file to the worker, and
    /// the file, for [`FileUi::activity`].
    access: (f64, PathBuf),
    /// The window is closing with the user's answer (or nothing unsaved):
    /// the exit keeps no recovery copy.
    closed: bool,
    /// Export to SolidCraft: its window and the export running on its own
    /// thread, which saving and recovery do not wait for.
    model_export: crate::model_export::ModelExport,
}
impl FileUi {
    pub fn new(cc: &eframe::CreationContext<'_>, engine: &Engine) -> Self {
        let picker = rfd::AsyncFileDialog::new().set_parent(cc);
        let directory =
            sparksmith_io::settings::UserDirs::recovery_from_env().map_err(|e| e.to_string());
        Self::with_recovery(&cc.egui_ctx, picker, engine, directory)
    }
    /// The file workflow with recovery snapshots in `directory` (tests).
    #[cfg(test)]
    pub(crate) fn with_directory(
        ctx: &egui::Context,
        picker: rfd::AsyncFileDialog,
        engine: &Engine,
        directory: PathBuf,
    ) -> Self {
        Self::with_recovery(ctx, picker, engine, Ok(directory))
    }
    /// The file workflow with recovery snapshots in `directory`, or none,
    /// said at startup, when there is no recovery folder.
    fn with_recovery(
        ctx: &egui::Context,
        picker: rfd::AsyncFileDialog,
        engine: &Engine,
        directory: Outcome<PathBuf>,
    ) -> Self {
        let (events, receiver) = mpsc::channel();
        let sender = worker(ctx.clone(), directory, events.clone());
        Self {
            controller: Controller::new(engine.revision()),
            document: engine.document().clone(),
            captured: None,
            saved_document: Some(engine.document().clone()),
            picker,
            folder: home_directory(),
            sender,
            event_sender: events,
            receiver,
            effects: vec![],
            status: None,
            recoveries: vec![],
            older_recoveries: vec![],
            recovery_selected: 0,
            restored_source: None,
            restored_origin: None,
            recovery_choice_pending: false,
            changed_at: 0.0,
            unkept_since: None,
            recovery_retry_at: 0.0,
            now: 0.0,
            close_pending: false,
            writing: None,
            importing: None,
            access: (0.0, PathBuf::new()),
            closed: false,
            model_export: crate::model_export::ModelExport::new(ctx),
        }
    }
    pub fn observe(&mut self, engine: &Engine, now: f64) {
        self.now = now;
        // A change without a revision (a hosted first save's name and identity) is
        // the engine's document too: the window saves and recovers it.
        if !same_document(&self.document, engine.document()) {
            self.document = engine.document().clone();
        }
        if self.controller.observe(engine.revision()) {
            self.document = engine.document().clone();
            self.changed_at = now;
            self.unkept_since.get_or_insert(now);
            if self.controller.dirty()
                && self.saved_document.as_ref().is_some_and(|saved| {
                    same_content(saved, &self.document) || only_unedited_drafts(saved, engine)
                })
            {
                self.controller.back_at_saved();
                self.clear_recovery(false);
            }
        }
    }
    /// The fill worker stored copper on top of revision `before` (no undo
    /// step). Fill copper is derived and never saved, so when `before` was
    /// the saved content the new revision is too: opening a file and
    /// letting it refill leaves nothing unsaved.
    pub fn fills_stored(&mut self, engine: &Engine, before: u64) {
        let saved = !self.controller.dirty() && self.controller.current.revision == before;
        self.observe(engine, self.now);
        if saved && self.controller.dirty() {
            self.controller.back_at_saved();
            self.saved_document = Some(self.document.clone());
            self.clear_recovery(false);
        }
    }
    /// The engine took what the first save wrote ([`Effect::FirstSave`]):
    /// when that save was current, the engine's document is the saved one.
    pub fn first_saved(&mut self, engine: &Engine, saved: bool) {
        self.observe(engine, self.now);
        if saved {
            self.controller.back_at_saved();
            self.saved_document = Some(self.document.clone());
        }
    }
    pub fn dirty(&self) -> bool {
        self.controller.dirty()
    }
    pub fn path(&self) -> Option<&Path> {
        self.controller.path.as_deref()
    }
    /// The file a project without a path of its own was imported or
    /// restored from: its first save proposes, and gives an Untitled
    /// project, this file's name.
    pub fn unsaved_origin(&self) -> Option<&Path> {
        self.restored_origin
            .as_deref()
            .filter(|_| self.controller.path.is_none())
    }
    pub fn busy(&self) -> bool {
        self.controller.flight.is_some() || self.recovery_choice_pending
    }
    /// A save is writing its file: from the request (or the save panel's
    /// answer) until the write ends, however long the system holds it.
    pub fn saving(&self) -> bool {
        self.controller
            .flight
            .as_ref()
            .is_some_and(|f| f.kind == Kind::Save && Some(f.ticket.id) == self.writing)
    }
    /// A KiCad import is running: from the file's choice until the new
    /// project is ready.
    pub fn importing(&self) -> bool {
        self.controller
            .flight
            .as_ref()
            .is_some_and(|f| f.kind == Kind::Import && Some(f.ticket.id) == self.importing)
    }
    /// The app bar's text for a running save or import, and when it next
    /// changes. One the system holds past [`HELD_ACCESS`] (a permission
    /// prompt behind the window, a slow disk) says what it waits for.
    pub fn activity(&self) -> Option<(String, Option<Duration>)> {
        let work = if self.saving() {
            "Saving\u{2026}"
        } else if self.importing() {
            "Importing KiCad project\u{2026}"
        } else {
            return None;
        };
        let (since, path) = &self.access;
        Some(activity_text(work, path, self.now - since))
    }
    pub fn prompting(&self) -> bool {
        self.controller.prompt.is_some() || !self.recoveries.is_empty()
    }
    /// The latest file message, with its severity said explicitly.
    pub fn take_status(&mut self) -> Option<(Level, String)> {
        self.status.take()
    }
    /// An imported project, from the KiCad file `source`: unsaved (a
    /// recovery snapshot keeps it, and quitting asks), and its first save
    /// proposes the KiCad project's folder and name.
    pub fn adopt_imported(&mut self, engine: &Engine, source: PathBuf) -> Outcome<()> {
        self.adopt(engine, None, None, true)?;
        self.restored_source = None;
        self.restored_origin = Some(source);
        Ok(())
    }
    /// The engine holds a new document: opened from `path` (whose bytes
    /// had `identity`), new, or `recovered`. Path, saved state, disk
    /// identity and the recovery snapshot change together.
    pub fn adopt(
        &mut self,
        engine: &Engine,
        path: Option<PathBuf>,
        identity: Option<DiskIdentity>,
        recovered: bool,
    ) -> Outcome<()> {
        self.controller
            .adopt(engine.revision(), path, identity, recovered)?;
        if !recovered {
            self.restored_source = None;
            self.restored_origin = None;
        }
        self.document = engine.document().clone();
        self.saved_document = (!recovered).then(|| self.document.clone());
        self.model_export.document_opened();
        self.changed_at = self.now;
        self.clear_recovery(false);
        Ok(())
    }
    /// The window's file, as the engine is lent with it
    /// ([`sparksmith_automation::Session::lend_file`]): a lent session may
    /// open or start a project only when nothing is unsaved, and none of
    /// the three while this workflow runs or asks about a file.
    pub fn host_file(&self) -> HostFile {
        HostFile {
            path: self.controller.path.clone(),
            identity: self.controller.disk,
            kept: self.controller.saved_revision(),
            busy: self.busy() || self.prompting(),
        }
    }

    /// Take on what a lent session did to the window's file, in order
    /// (review finding 8): this workflow stays the one owner of the file,
    /// its saved state and the recovery snapshot.
    pub fn hosted(&mut self, engine: &Engine, changes: Vec<FileChange>) -> Outcome<()> {
        for change in changes {
            match change {
                FileChange::Opened {
                    path,
                    identity,
                    revision,
                } => {
                    self.controller
                        .adopt(revision, Some(path), Some(identity), false)?;
                    self.adopted(engine, revision);
                }
                FileChange::New { revision } => {
                    self.controller.adopt(revision, None, None, false)?;
                    self.adopted(engine, revision);
                }
                FileChange::Saved {
                    path,
                    identity,
                    revision,
                } => {
                    let stamp = Stamp {
                        epoch: self.controller.current.epoch,
                        revision,
                    };
                    self.controller.path = Some(path);
                    self.controller.disk = Some(identity);
                    self.controller.saved = Some(stamp);
                    self.saved_document =
                        (revision == engine.revision()).then(|| engine.document().clone());
                    if revision == engine.revision() {
                        self.clear_recovery(false);
                    }
                }
            }
        }
        self.observe(engine, self.now);
        Ok(())
    }

    /// A lent session replaced the document, which the file at `revision`
    /// (or none, for a new project) holds.
    fn adopted(&mut self, engine: &Engine, revision: u64) {
        self.restored_origin = None;
        self.restored_source = None;
        self.document = engine.document().clone();
        self.saved_document = (revision == engine.revision()).then(|| self.document.clone());
        self.model_export.document_opened();
        self.changed_at = self.now;
        self.clear_recovery(false);
    }

    /// The app is ending without a Save or Don't Save answer (Dock › Quit,
    /// a logout, a signal): write unsaved work to this session's recovery
    /// snapshot now, waiting at most `wait`, so the next launch offers it.
    /// Returns whether the snapshot holds the current document.
    pub fn keep_for_recovery(&mut self, wait: Duration) -> bool {
        if self.closed || !self.controller.dirty() {
            return false;
        }
        if self.controller.recovered.as_ref() == Some(&self.controller.current) {
            return true;
        }
        let Ok(ticket) = self.controller.allocate() else {
            return false;
        };
        self.controller.recovery_flight = Some(ticket.clone());
        self.send(Job::Recover(
            ticket.clone(),
            self.document.clone(),
            self.controller.path.clone(),
        ));
        let deadline = std::time::Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
            let Ok(event) = self.receiver.recv_timeout(left) else {
                break;
            };
            let ours = matches!(&event, Event::Recovered(t, false, false, _) if *t == ticket);
            self.receive(event);
            if ours {
                break;
            }
        }
        self.controller.recovered.as_ref() == Some(&self.controller.current)
    }
    fn send(&mut self, job: Job) {
        if let Job::Write(ticket, path, ..) = &job {
            self.writing = Some(ticket.id);
            self.access = (self.now, path.clone());
        }
        if let Job::Import(ticket, path) = &job {
            self.importing = Some(ticket.id);
            self.access = (self.now, path.clone());
        }
        if let Err(error) = self.sender.send(job) {
            self.controller.flight = None;
            self.controller.recovery_flight = None;
            self.status = Some((Level::Error, error.to_string()));
        }
    }
    fn begin(&mut self, kind: Kind, continuation: Option<Transition>) -> Option<Ticket> {
        match self.controller.start(kind, continuation) {
            Ok(ticket) => {
                self.captured = Some(self.document.clone());
                Some(ticket)
            }
            Err(error) => {
                self.status = Some((Level::Error, error));
                None
            }
        }
    }
    pub fn request(&mut self, action: Action) {
        if matches!(action, Action::Close) && self.closed {
            return;
        }
        if matches!(action, Action::Close) && self.controller.prompt.is_none() {
            if self.busy() {
                // A save or a file panel is still open: quit once it ends.
                self.close_pending = true;
                return;
            }
            // The startup recovery question is kept on disk for the next
            // launch, as "Keep for later" does.
            self.recoveries.clear();
            self.older_recoveries.clear();
        }
        if self.busy() || self.controller.prompt.is_some() || !self.recoveries.is_empty() {
            return;
        }
        match action {
            Action::Save => self.save(false, None),
            Action::SaveAs => self.save(true, None),
            Action::Export => self.model_export.open(),
            Action::ExportSchematicPdf => {
                if let Some(ticket) = self.begin(Kind::SchematicPdf, None) {
                    self.pick(ticket, Kind::SchematicPdf);
                }
            }
            action => {
                let transition = match action {
                    Action::New => Transition::New,
                    Action::Open => Transition::Open(None),
                    Action::OpenPath(path) => Transition::Open(Some(path)),
                    Action::ImportKicad => Transition::Import(None),
                    Action::ImportKicadPath(path) => Transition::Import(Some(path)),
                    _ => Transition::Close,
                };
                if let Some(transition) = self.controller.request(transition) {
                    self.proceed(transition);
                }
            }
        }
    }
    fn proceed(&mut self, transition: Transition) {
        match transition {
            Transition::New => match new_project() {
                Ok((engine, said)) => {
                    if let Some(said) = said {
                        self.status = Some((Level::Warning, said));
                    }
                    self.effects.push(Effect::Replace {
                        document: Box::new(engine.to_file()),
                        path: None,
                        identity: None,
                        recovered: false,
                    });
                }
                Err(error) => self.status = Some((Level::Error, error)),
            },
            Transition::Open(path) => {
                if let Some(ticket) = self.begin(Kind::Open, None) {
                    if let Some(path) = path {
                        match absolute_path(path) {
                            Ok(path) => self.send(Job::Read(ticket, path)),
                            Err(error) => {
                                self.controller.cancel(&ticket, Kind::Open);
                                self.status = Some((Level::Error, error));
                            }
                        }
                    } else {
                        self.pick(ticket, Kind::Open);
                    }
                }
            }
            Transition::Import(path) => {
                if let Some(ticket) = self.begin(Kind::Import, None) {
                    if let Some(path) = path {
                        match absolute_path(path) {
                            Ok(path) => self.send(Job::Import(ticket, path)),
                            Err(error) => {
                                self.controller.cancel(&ticket, Kind::Import);
                                self.status = Some((Level::Error, error));
                            }
                        }
                    } else {
                        self.pick(ticket, Kind::Import);
                    }
                }
            }
            Transition::Close => self.clear_recovery(true),
        }
    }
    fn save(&mut self, save_as: bool, continuation: Option<Transition>) {
        let Some(ticket) = self.begin(Kind::Save, continuation) else {
            return;
        };
        if !save_as && let Some(path) = self.controller.path.clone() {
            let expected = self.controller.disk;
            self.write(ticket, path, expected);
        } else {
            self.pick(ticket, Kind::Save);
        }
    }
    /// Write the captured document to `path`.
    fn write(&mut self, ticket: Ticket, path: PathBuf, expected: Option<DiskIdentity>) {
        let Some(document) = self.captured.clone() else {
            return;
        };
        self.send(Job::Write(ticket, path, document, expected));
    }
    /// The folder and file name the project's save panel proposes: its
    /// own file's, else (a restored session) the file it was restored
    /// from, else the project's name in the last folder used.
    fn save_proposal(&self) -> (&Path, String) {
        let origin = self.unsaved_origin();
        let folder = origin.and_then(Path::parent).unwrap_or(&self.folder);
        let name = proposed_file_name(
            self.controller.path.as_deref().or(origin),
            &self.document.project.name,
        );
        (folder, name)
    }
    fn pick(&mut self, ticket: Ticket, kind: Kind) {
        let (folder, filename) = self.save_proposal();
        let dialog = self.picker.clone().set_directory(folder);
        let dialog = if kind == Kind::Import {
            self.picker
                .clone()
                .set_directory(&self.folder)
                .set_title("Import KiCad project")
                .add_filter(
                    "KiCad project or board",
                    &["kicad_pro", "kicad_pcb", "KICAD_PRO", "KICAD_PCB"],
                )
        } else if kind == Kind::Export {
            dialog
                .set_title("Export PCB to Solidcraft")
                .set_file_name("board.solidcraft")
                .add_filter("Solidcraft model", &["solidcraft"])
        } else if kind == Kind::SchematicPdf {
            dialog
                .set_title("Export schematic PDF")
                .set_file_name("schematic.pdf")
                .add_filter("PDF document", &["pdf"])
        } else {
            dialog
                .set_title(if kind == Kind::Open {
                    concat!("Open ", sparksmith_core::product_name!(), " project")
                } else {
                    concat!("Save ", sparksmith_core::product_name!(), " project")
                })
                .set_file_name(&filename)
                .add_filter(
                    concat!(sparksmith_core::product_name!(), " project"),
                    &["sparksmith", "SPARKSMITH"],
                )
        };
        // Construct the parented native future on the event thread, just as
        // Solidcraft services do. Awaiting it does not block egui or file I/O.
        type Picking =
            std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>>;
        let future: Picking = if matches!(kind, Kind::Open | Kind::Import) {
            Box::pin(dialog.pick_file())
        } else {
            Box::pin(dialog.save_file())
        };
        let events = self.event_sender.clone();
        std::thread::spawn(sparksmith_core::library::standards::carried(move || {
            let picked = pollster::block_on(future).map(|handle| handle.path().to_path_buf());
            let _ = events.send(Event::Picked(ticket, kind, picked));
        }));
    }
    /// Pick an SVG drawing to import, with the same parented native picker.
    /// The answer (None when cancelled) arrives on the returned channel;
    /// the caller reads the file off the UI thread.
    pub fn pick_svg(&self) -> Receiver<Option<PathBuf>> {
        let dialog = self
            .picker
            .clone()
            .set_directory(&self.folder)
            .set_title("Import SVG drawing")
            .add_filter("SVG drawing", &["svg", "SVG"]);
        let future = dialog.pick_file();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(sparksmith_core::library::standards::carried(move || {
            let picked = pollster::block_on(future).map(|handle| handle.path().to_path_buf());
            let _ = sender.send(picked);
        }));
        receiver
    }
    /// Pick PDF documents for a workflow's datasheet field, with the same
    /// parented native picker; the paths (none when cancelled) arrive on
    /// the returned channel.
    pub fn pick_pdfs(&self) -> Receiver<Vec<PathBuf>> {
        let dialog = self
            .picker
            .clone()
            .set_directory(&self.folder)
            .set_title("Choose datasheets")
            .add_filter("PDF document", &["pdf", "PDF"]);
        let future = dialog.pick_files();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(sparksmith_core::library::standards::carried(move || {
            let picked = pollster::block_on(future)
                .unwrap_or_default()
                .iter()
                .map(|handle| handle.path().to_path_buf())
                .collect();
            let _ = sender.send(picked);
        }));
        receiver
    }
    fn clear_recovery(&mut self, close: bool) {
        let ticket = if close {
            self.begin(Kind::Close, None)
        } else {
            self.controller.allocate().ok()
        };
        if let Some(ticket) = ticket {
            self.controller.recovery_flight = Some(ticket.clone());
            self.send(Job::ClearRecovery(ticket, close));
        }
    }
    pub fn tick(&mut self, ctx: &egui::Context) -> Vec<Effect> {
        while let Ok(event) = self.receiver.try_recv() {
            self.receive(event);
        }
        self.model_export.poll(self.now, &self.document.project);
        if let Some(status) = self.model_export.take_status() {
            self.status = Some(status);
        }
        if self.close_pending && !self.busy() {
            self.close_pending = false;
            self.request(Action::Close);
        }
        // Once closing (Don't Save), no new snapshot brings the work back.
        if self.controller.dirty()
            && !self.closed
            && !self.prompting()
            && self.controller.recovery_flight.is_none()
            && self.controller.recovered.as_ref() != Some(&self.controller.current)
            && self.now >= self.recovery_retry_at
        {
            // Two quiet seconds, or at most the maximum interval since the
            // first edit no snapshot holds.
            let idle = SNAPSHOT_IDLE - (self.now - self.changed_at);
            let bound = self.unkept_since.map_or(f64::INFINITY, |since| {
                MAX_SNAPSHOT_INTERVAL - (self.now - since)
            });
            let wait = idle.min(bound).max(0.0);
            if wait == 0.0 {
                if let Ok(ticket) = self.controller.allocate() {
                    self.unkept_since = None;
                    self.controller.recovery_flight = Some(ticket.clone());
                    self.send(Job::Recover(
                        ticket,
                        self.document.clone(),
                        self.controller.path.clone(),
                    ));
                }
            } else {
                ctx.request_repaint_after(Duration::from_secs_f64(wait));
            }
        }
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        std::mem::take(&mut self.effects)
    }
    fn receive(&mut self, event: Event) {
        match event {
            Event::Startup(result) => match result {
                Ok(listing) => {
                    self.recoveries = listing.entries;
                    self.older_recoveries = listing.older;
                }
                Err(error) => self.status = Some((Level::Error, error)),
            },
            Event::Picked(ticket, kind, path) => {
                if !self
                    .controller
                    .flight
                    .as_ref()
                    .is_some_and(|f| f.ticket == ticket && f.kind == kind)
                {
                    return;
                }
                let Some(path) = path else {
                    self.controller.cancel(&ticket, kind);
                    self.captured = None;
                    self.status = Some((Level::Info, "File selection cancelled".into()));
                    return;
                };
                let path = match absolute_path(path) {
                    Ok(path) => path,
                    Err(error) => {
                        self.controller.cancel(&ticket, kind);
                        self.status = Some((Level::Error, error));
                        return;
                    }
                };
                if let Some(parent) = path.parent() {
                    self.folder = parent.to_path_buf();
                }
                match kind {
                    // Save As: the person chose the file, and its panel
                    // asked before replacing one. The window's own file,
                    // under any spelling, is replaced only while it holds
                    // what this window opened or saved (review finding 16).
                    Kind::Save => {
                        let expected = self
                            .controller
                            .path
                            .as_deref()
                            .filter(|own| sparksmith_io::same_file(own, &path))
                            .and(self.controller.disk);
                        self.write(ticket, path, expected);
                    }
                    Kind::Open if ticket.stamp == self.controller.current => {
                        self.send(Job::Read(ticket, path))
                    }
                    Kind::Import if ticket.stamp == self.controller.current => {
                        self.send(Job::Import(ticket, path))
                    }
                    Kind::Export if ticket.stamp == self.controller.current => {
                        // The export runs on its own thread: the file
                        // workflow is free again at once.
                        let _ = self.controller.take_flight(&ticket, Kind::Export);
                        if let Some(document) = self.captured.take() {
                            self.model_export.start(document.project, path);
                        }
                    }
                    Kind::SchematicPdf if ticket.stamp == self.controller.current => {
                        if let Some(document) = self.captured.as_ref() {
                            let project = Arc::clone(&document.project);
                            self.send(Job::SchematicPdf(ticket, path, project));
                        }
                    }
                    _ => {
                        self.controller.cancel(&ticket, kind);
                        self.status = Some((
                            Level::Warning,
                            "The document changed while choosing a file. Please try again.".into(),
                        ));
                    }
                }
            }
            Event::Written(ticket, path, result) => match result {
                Ok(wrote) => {
                    // A conflict kept both versions: this document is in
                    // the new file, which is the window's file from now on.
                    // The first case's document is the one the file holds,
                    // with what a first save gave it.
                    let (path, identity, saved, conflict) = match wrote {
                        Wrote::Saved(saved) => (
                            path,
                            saved.identity,
                            Some((saved.document, saved.first)),
                            None,
                        ),
                        Wrote::Kept(conflict) => (
                            conflict.kept.clone(),
                            conflict.identity,
                            None,
                            Some(sparksmith_io::Error::Conflict(conflict).to_string()),
                        ),
                    };
                    if let Some((current, continuation)) =
                        self.controller.saved(&ticket, path.clone(), identity)
                    {
                        let captured = self.captured.take();
                        let given = match saved {
                            Some((document, given)) => {
                                self.saved_document = Some(document);
                                given
                            }
                            None => {
                                self.saved_document = captured;
                                FirstSave::default()
                            }
                        };
                        self.status = Some(match conflict {
                            Some(conflict) => (Level::Warning, conflict),
                            None if current => (Level::Ok, format!("Saved {}", path.display())),
                            None => (
                                Level::Ok,
                                format!(
                                    "Saved captured version to {}; newer edits remain unsaved",
                                    path.display()
                                ),
                            ),
                        });
                        if current {
                            self.clear_recovery(false);
                        }
                        if !given.is_empty() {
                            self.effects.push(Effect::FirstSave {
                                given,
                                saved: current,
                            });
                        }
                        if let Some(transition) = continuation {
                            self.proceed(transition);
                        }
                    }
                }
                Err(error) => {
                    self.controller.cancel(&ticket, Kind::Save);
                    self.status = Some((Level::Error, format!("Save failed: {error}")));
                }
            },
            Event::Read(ticket, path, result) => {
                if !self.controller.accept(&ticket, Kind::Open) {
                    self.status = Some((
                        Level::Warning,
                        "Open cancelled because the current document changed".into(),
                    ));
                    return;
                }
                match result {
                    Ok((document, identity)) => self.effects.push(Effect::Replace {
                        document,
                        path: Some(path),
                        identity: Some(identity),
                        recovered: false,
                    }),
                    Err(error) => {
                        self.status = Some((Level::Error, format!("Open failed: {error}")))
                    }
                }
            }
            Event::Imported(ticket, path, result) => {
                self.importing = None;
                if !self.controller.accept(&ticket, Kind::Import) {
                    self.status = Some((
                        Level::Warning,
                        "Import cancelled because the current document changed".into(),
                    ));
                    return;
                }
                match result.map(|imported| *imported) {
                    Ok((document, report, said)) => {
                        if let Some(said) = said {
                            self.status = Some((Level::Warning, said));
                        }
                        self.effects.push(Effect::Imported {
                            document: Box::new(document),
                            source: path,
                            report: Box::new(report),
                        });
                    }
                    Err(error) => {
                        self.status = Some((Level::Error, format!("Import failed: {error}")))
                    }
                }
            }
            Event::PdfExported(ticket, path, result) => {
                let current = self.controller.accept(&ticket, Kind::SchematicPdf);
                self.status = Some(match result {
                    Ok(pages) if current => (
                        Level::Ok,
                        format!(
                            "Printed {} to {}",
                            crate::kit::counted(pages, "sheet", "sheets"),
                            path.display()
                        ),
                    ),
                    Ok(pages) => (
                        Level::Warning,
                        format!(
                            "Printed {} to {}; the document has changed",
                            crate::kit::counted(pages, "captured sheet", "captured sheets"),
                            path.display()
                        ),
                    ),
                    Err(error) => (Level::Error, format!("PDF export failed: {error}")),
                });
            }
            Event::Recovered(ticket, removed, close, result) => {
                // This session now holds the restored work in its own
                // snapshot: the restored session file is no longer needed.
                if result.is_ok()
                    && !removed
                    && ticket.stamp == self.controller.current
                    && self
                        .restored_source
                        .as_ref()
                        .is_some_and(|(epoch, _)| *epoch == ticket.stamp.epoch)
                    && let Some((_, source)) = self.restored_source.take()
                {
                    self.send(Job::DiscardRecovery(source));
                }
                self.controller
                    .finish_recovery(&ticket, result.is_ok(), removed);
                if close {
                    let current = self.controller.accept(&ticket, Kind::Close);
                    if current && result.is_ok() {
                        self.closed = true;
                        // An export still running writes nothing.
                        self.model_export.stop();
                        self.effects.push(Effect::Close);
                    } else if result.is_ok() {
                        self.controller.prompt = Some(Transition::Close);
                    }
                }
                if let Err(error) = result {
                    self.status = Some((Level::Error, format!("Recovery failed: {error}")));
                    self.recovery_retry_at = self.now + 10.0;
                }
            }
            Event::Restored(ticket, result) => {
                self.recovery_choice_pending = false;
                if !self.controller.accept(&ticket, Kind::Restore) {
                    self.status = Some((
                        Level::Warning,
                        "Restore cancelled because the current document changed".into(),
                    ));
                    return;
                }
                match result {
                    Ok(restored) => {
                        let (record, snapshot) = *restored;
                        let Some(epoch) = self.controller.current.epoch.checked_add(1) else {
                            self.status =
                                Some((Level::Error, "Document identity exhausted".into()));
                            return;
                        };
                        self.restored_origin = record.original_path;
                        self.recoveries
                            .retain(|e| e.snapshot.path() != snapshot.path());
                        self.recovery_selected = 0;
                        self.restored_source = Some((epoch, snapshot));
                        self.effects.push(Effect::Replace {
                            document: Box::new(record.document),
                            path: None,
                            identity: None,
                            recovered: true,
                        });
                    }
                    Err(error) => {
                        self.status = Some((Level::Error, format!("Restore failed: {error}")))
                    }
                }
            }
            Event::Discarded(snapshot, result) => {
                self.recovery_choice_pending = false;
                match result {
                    Ok(()) => self.recoveries.retain(|e| e.snapshot != snapshot),
                    Err(error) => self.status = Some((Level::Error, error)),
                }
            }
            Event::DiscardedOlder(removed, result) => {
                self.recovery_choice_pending = false;
                let removed: std::collections::BTreeSet<_> = removed.into_iter().collect();
                self.older_recoveries
                    .retain(|p| !removed.contains(p.path()));
                if let Err(error) = result {
                    self.status = Some((Level::Error, error));
                }
            }
        }
    }
    /// Unsaved-changes and startup-recovery prompts, in Solidcraft's system
    /// window chrome.
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        if self.model_export.show(ctx) == Some(crate::model_export::Answer::Pick) {
            if self.busy() || self.prompting() {
                self.status = Some((
                    Level::Warning,
                    "Another file operation is still running".into(),
                ));
            } else if let Some(ticket) = self.begin(Kind::Export, None) {
                self.pick(ticket, Kind::Export);
            }
        }
        if let Some(transition) = &self.controller.prompt {
            let title = match transition {
                Transition::Close => "Save changes before quitting?",
                Transition::New | Transition::Open(_) | Transition::Import(_) => {
                    "Save changes before continuing?"
                }
            };
            let mut choice = None;
            ui_kit::Window::new(egui::Id::unique("file-unsaved"), "status_warning", title)
                .required()
                .show_body(ctx, |ui| {
                    dialog_body(ui, |ui| {
                        crate::kit::status(
                            ui,
                            "The project or component draft has unsaved changes.",
                            crate::kit::Tone::Dim,
                        );
                        if let Some((_, status)) = &self.status {
                            crate::kit::note(ui, status);
                        }
                        crate::kit::actions(ui, |ui| {
                            if crate::kit::primary(ui, "Save", true).clicked() {
                                choice = Some(Choice::Save);
                            }
                            if ui_kit::button(ui, DONT_SAVE, Button::Normal).clicked() {
                                choice = Some(Choice::Discard);
                            }
                            if ui_kit::button(ui, "Cancel", Button::Normal).clicked() {
                                choice = Some(Choice::Cancel);
                            }
                        });
                    });
                });
            match choice {
                Some(Choice::Save) => {
                    let next = self.controller.prompt.take();
                    self.save(false, next);
                }
                Some(Choice::Discard) => {
                    if let Some(next) = self.controller.prompt.take() {
                        self.proceed(next);
                    }
                }
                Some(Choice::Cancel) => {
                    self.controller.prompt = None;
                    self.close_pending = false;
                }
                None => {}
            }
            return;
        }
        if self.recoveries.is_empty() {
            return;
        }
        self.recovery_selected = self.recovery_selected.min(self.recoveries.len() - 1);
        let Some(entry) = self.recoveries.get(self.recovery_selected).cloned() else {
            return;
        };
        let busy = self.busy();
        let mut choice = None;
        let mut discard_older = false;
        let mut selected = self.recovery_selected;
        let count = self.recoveries.len();
        let title = if count == 1 {
            "Recover an earlier session?".to_owned()
        } else {
            format!("Recover one of {count} earlier sessions?")
        };
        ui_kit::Window::new(egui::Id::unique("startup-recovery"), "status_warning", &title)
            .required()
            .show_body(ctx, |ui| {
                dialog_body(ui, |ui| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
                    // First, so a long list never pushes it out of reach.
                    if !self.older_recoveries.is_empty() {
                        crate::kit::status(
                            ui,
                            &older_sessions_note(self.older_recoveries.len()),
                            crate::kit::Tone::Dim,
                        );
                        ui.add_enabled_ui(!busy, |ui| {
                            crate::kit::actions(ui, |ui| {
                                if ui_kit::button(ui, DISCARD_OLDER, Button::Normal).clicked() {
                                    discard_older = true;
                                }
                            });
                        });
                    }
                    for (at, candidate) in self.recoveries.iter().enumerate() {
                        let mut on = at == selected;
                        let label = recovery_label(candidate, now);
                        if crate::kit::check_row(ui, &label, &mut on) && on {
                            selected = at;
                        }
                    }
                    if let Some(path) = &entry.original_path {
                        crate::kit::value_row(ui, "File", &path.display().to_string());
                    }
                    if let Some(error) = &entry.error {
                        // The cause in plain words; the technical one on hover.
                        crate::kit::status(ui, UNREADABLE_NOTE, crate::kit::Tone::Error)
                            .on_hover_text(error);
                    }
                    crate::kit::note(
                        ui,
                        "Restore opens an unsaved copy of the checked session; once it is safe in this session, its old snapshot is removed. The original file is unchanged.",
                    );
                    ui.add_enabled_ui(!busy, |ui| {
                        crate::kit::actions(ui, |ui| {
                            if crate::kit::primary(ui, "Restore", entry.error.is_none()).clicked() {
                                choice = Some(Choice::Save);
                            }
                            if ui_kit::button(ui, "Keep for later", Button::Normal).clicked() {
                                choice = Some(Choice::Cancel);
                            }
                            if ui_kit::button(ui, "Discard", Button::Normal).clicked() {
                                choice = Some(Choice::Discard);
                            }
                        });
                    });
                });
            });
        self.recovery_selected = selected;
        if discard_older {
            self.recovery_choice_pending = true;
            self.send(Job::DiscardOlder(self.older_recoveries.clone()));
        }
        match choice {
            Some(Choice::Save) => {
                if let Some(ticket) = self.begin(Kind::Restore, None) {
                    self.recovery_choice_pending = true;
                    self.send(Job::Restore(ticket, entry.snapshot));
                }
            }
            Some(Choice::Cancel) => {
                // Kept on disk for the next launch; none is asked again now.
                self.recoveries.clear();
                self.older_recoveries.clear();
                self.recovery_selected = 0;
            }
            Some(Choice::Discard) => {
                self.recovery_choice_pending = true;
                self.send(Job::DiscardRecovery(entry.snapshot));
            }
            None => {}
        }
    }
}

/// The unsaved-changes answer that goes on without saving. It keeps no
/// recovery copy either: the session's snapshot is removed before the
/// window closes, as in Solidcraft.
const DONT_SAVE: &str = "Don't Save";

/// The recovery question's button that discards the sessions past the
/// listing's read limit.
const DISCARD_OLDER: &str = "Discard older sessions";

/// What the recovery question says about the sessions it did not read.
fn older_sessions_note(count: usize) -> String {
    let sessions = if count == 1 {
        "session is"
    } else {
        "sessions are"
    };
    format!(
        "{count} older {sessions} not listed: only the newest {} are read.",
        recovery::MAX_ENTRIES
    )
}

/// Why an unreadable recovery cannot be restored. Another version's
/// session is rejected, never migrated.
const UNREADABLE_NOTE: &str =
    "Saved by another version or damaged, so this session cannot be restored.";

/// One recovery as the dialog lists it: name, age and size (an unreadable
/// one: name and file age).
fn recovery_label(entry: &RecoveryEntry, now_ms: u64) -> String {
    let minutes = now_ms.saturating_sub(entry.saved_unix_ms) / 60_000;
    let age = match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", minutes / 60),
        _ => format!(
            "{} ago",
            crate::kit::counted(
                usize::try_from(minutes / 1440).unwrap_or(usize::MAX),
                "day",
                "days"
            )
        ),
    };
    if entry.error.is_some() {
        return format!("{} · {age}", entry.project_name);
    }
    let instances = match entry.instances {
        1 => "1 instance".to_owned(),
        n => format!("{n} instances"),
    };
    format!("{} · {age} · {instances}", entry.project_name)
}

#[derive(Clone, Copy)]
enum Choice {
    Save,
    Discard,
    Cancel,
}

/// Both documents share the same data: an undo or redo restored the very
/// snapshot that was saved.
/// Whether `a` and `b` are one state: the same shared sections.
fn same_document(a: &Document, b: &Document) -> bool {
    Arc::ptr_eq(&a.project, &b.project) && Arc::ptr_eq(&a.drafts, &b.drafts)
}

fn same_content(a: &Document, b: &Document) -> bool {
    Arc::ptr_eq(&a.project, &b.project) && Arc::ptr_eq(&a.drafts, &b.drafts)
}

/// The name the project panel proposes: the file's own, else the
/// project's name ("USB sensor board.sparksmith") without
/// characters a file name cannot hold, else [`sparksmith_core::UNTITLED`]
/// ("Untitled.sparksmith").
fn proposed_file_name(path: Option<&Path>, project: &str) -> String {
    if let Some(stem) = path.and_then(|p| p.file_stem()).and_then(|n| n.to_str()) {
        return format!("{stem}.{}", sparksmith_core::format::EXTENSION);
    }
    let stem: String = project
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':') && !c.is_control())
        .collect();
    let stem = match stem.trim().trim_start_matches('.') {
        "" => sparksmith_core::UNTITLED,
        stem => stem,
    };
    format!("{stem}.{}", sparksmith_core::format::EXTENSION)
}

/// Whether the engine's document differs from `saved` only by drafts opened
/// for editing and not yet changed: whole component working sets that still
/// equal their published revisions. Opening a component is not an edit.
fn only_unedited_drafts(saved: &Document, engine: &Engine) -> bool {
    use crate::part_editor::WorkingSet;
    use sparksmith_core::DefinitionKind;
    let now = engine.document();
    let (old, new) = (&saved.drafts, &now.drafts);
    if !Arc::ptr_eq(&saved.project, &now.project)
        || old.grid != new.grid
        || old.symbols != new.symbols
        || !old.items.iter().all(|d| new.items.contains(d))
    {
        return false;
    }
    let added: Vec<_> = new
        .items
        .iter()
        .filter(|d| !old.items.contains(d))
        .collect();
    let mut covered = std::collections::BTreeSet::new();
    for draft in added
        .iter()
        .filter(|d| d.definition.kind() == DefinitionKind::Component)
    {
        match WorkingSet::of(new, draft.id) {
            Some(set) if !set.unpublished(engine) => covered.extend(set.ids.all()),
            _ => return false,
        }
    }
    added.iter().all(|d| covered.contains(&d.id))
}

fn absolute_path(path: PathBuf) -> Outcome<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .map_err(|e| e.to_string())
    }
}
/// The project a launch without a file and File › New open: empty, with
/// no parts, library or drafts, on the shipped new board and the fab profile
/// a new board takes (`sparksmith_io::new_project`): the user's default
/// (Settings), else the shipped one; with what to say when that default does
/// not read and the board starts on the shipped one. A missing or broken
/// shipped library is an error naming the file.
pub(crate) fn new_project() -> Result<(Engine, Option<String>), String> {
    // The app's context, current on the window's thread for the program's
    // life, is every engine's.
    let context =
        sparksmith_core::library::standards::Context::current().map_err(|e| e.to_string())?;
    sparksmith_io::new_project::engine_shown(&context).map_err(|e| e.to_string())
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(std::env::temp_dir)
}

/// Dialog content inset like the kit's cards, in the kit's form scope.
pub(crate) fn dialog_body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let t = Tokens::get();
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(t.space_12 as i8, t.space_8 as i8))
        .show(ui, |ui| {
            ui.set_min_width(360.0);
            crate::kit::form(ui, add);
        });
}

#[cfg(test)]
#[path = "file_ui_tests.rs"]
mod quit_tests;

#[cfg(test)]
mod tests {
    use super::*;

    /// Recoveries are told apart by age and size, not only by name.
    /// Some disk identity, for the controller's own bookkeeping.
    fn identity() -> DiskIdentity {
        DiskIdentity(sparksmith_core::ids::ContentHash([7; 32]))
    }

    #[test]
    fn recovery_rows_name_age_and_instances() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        RecoveryStore::new(dir.path())?.write(&Engine::demo()?.to_file(), None)?;
        let entry = RecoveryEntry {
            snapshot: recovery::list(dir.path())?
                .entries
                .pop()
                .ok_or("missing snapshot")?
                .snapshot,
            original_path: None,
            saved_unix_ms: 1_000_000,
            project_name: "Untitled".into(),
            instances: 4,
            error: None,
        };
        assert_eq!(
            recovery_label(&entry, 1_000_000 + 5 * 60_000),
            "Untitled · 5 min ago · 4 instances"
        );
        assert_eq!(
            recovery_label(&entry, 1_000_000 + 3 * 3_600_000),
            "Untitled · 3 h ago · 4 instances"
        );
        assert_eq!(
            recovery_label(&entry, 0),
            "Untitled · just now · 4 instances"
        );
        let unreadable = RecoveryEntry {
            project_name: recovery::UNREADABLE.into(),
            error: Some("unknown variant `x`".into()),
            ..entry
        };
        assert_eq!(
            recovery_label(&unreadable, 1_000_000 + 2 * 86_400_000),
            "Older or damaged recovery · 2 days ago"
        );
        Ok(())
    }

    /// Saving an untitled project proposes its name; a saved file keeps its
    /// own; once saved, the path (whose stem the title bar shows) is the
    /// chosen file.
    #[test]
    fn the_save_panel_proposes_the_project_name() -> Outcome<()> {
        assert_eq!(
            proposed_file_name(None, "USB sensor board"),
            "USB sensor board.sparksmith"
        );
        assert_eq!(proposed_file_name(None, " a/b:c "), "abc.sparksmith");
        assert_eq!(
            proposed_file_name(None, "  "),
            format!("{}.sparksmith", sparksmith_core::UNTITLED)
        );
        assert_eq!(
            proposed_file_name(Some(Path::new("/x/board.sparksmith")), "USB sensor board"),
            "board.sparksmith"
        );
        let mut controller = Controller::new(1);
        controller.observe(2);
        let ticket = controller.start(Kind::Save, None)?;
        let chosen = PathBuf::from("/x/USB sensor board.sparksmith");
        assert!(
            controller
                .saved(&ticket, chosen.clone(), identity())
                .is_some()
        );
        assert_eq!(controller.path.as_deref(), Some(chosen.as_path()));
        assert_eq!(
            chosen.file_stem().and_then(|s| s.to_str()),
            Some("USB sensor board")
        );
        Ok(())
    }

    #[test]
    fn a_save_marks_only_its_captured_revision_saved() -> Outcome<()> {
        let mut controller = Controller::new(1);
        assert!(!controller.dirty());
        controller.observe(2);
        assert!(controller.dirty());
        let ticket = controller.start(Kind::Save, None)?;
        controller.observe(3);
        let saved = controller.saved(&ticket, PathBuf::from("/tmp/x.sparksmith"), identity());
        assert_eq!(saved.map(|(current, _)| current), Some(false));
        assert!(controller.dirty(), "the newer edit stays unsaved");
        Ok(())
    }

    /// Fill copper the worker stores after opening is never saved, so it
    /// leaves nothing unsaved; on top of an edit, the edit stays unsaved.
    #[test]
    fn stored_fill_copper_leaves_nothing_unsaved() -> Outcome<()> {
        let showcase = Engine::showcase().map_err(|e| e.to_string())?;
        let opened = sparksmith_core::format::decode(
            &sparksmith_core::format::encode(&showcase.to_file()).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let mut engine = Engine::from_file(opened).map_err(|e| e.to_string())?;
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let mut files = FileUi::with_directory(
            &egui::Context::default(),
            rfd::AsyncFileDialog::new(),
            &engine,
            dir.path().to_path_buf(),
        );
        let before = engine.revision();
        assert!(engine.refill_stale().map_err(|e| e.to_string())? > 0);
        files.fills_stored(&engine, before);
        assert!(!files.controller.dirty());
        // An edit, then copper for it: the edit is still unsaved.
        let track = engine.project().tracks[0].id;
        engine
            .apply(
                engine.revision(),
                sparksmith_core::Operation::Layout(
                    sparksmith_core::layout::LayoutCommand::DeleteTrack { id: track },
                ),
            )
            .map_err(|e| e.to_string())?;
        files.observe(&engine, 1.0);
        let before = engine.revision();
        engine.refill_stale().map_err(|e| e.to_string())?;
        files.fills_stored(&engine, before);
        assert!(files.controller.dirty());
        Ok(())
    }

    #[test]
    fn destructive_transitions_wait_for_a_choice_when_dirty() -> Outcome<()> {
        let mut controller = Controller::new(1);
        assert_eq!(controller.request(Transition::New), Some(Transition::New));
        controller.observe(2);
        assert_eq!(controller.request(Transition::Close), None);
        assert_eq!(controller.prompt, Some(Transition::Close));
        controller.adopt(5, None, None, false)?;
        assert!(!controller.dirty());
        assert!(controller.prompt.is_none());
        Ok(())
    }
}
