//! Production files…: the docked dialog that checks the board for
//! fabrication and writes the production set (docs/fabrication-output.md).
//!
//! The dialog picks the fabrication profile to check against (the board's
//! own rules, a shipped profile or one of the user's), shows the
//! pre-flight report (`fabrication.preflight`) grouped by kind, and writes
//! Gerber, drill, job, assembly and drawing files as a folder or one ZIP
//! through `sparksmith_io::fabrication::export`, the call the CLI and
//! automation use too. Findings are ticked one by one, as a Shift-click
//! run or as a whole group, and waived with one optional reason (required
//! when Settings says so): the reason form
//! opens under the row or group where the waiving started, and one
//! `fabrication.waive` with every ticked key is one undo step. Restore
//! (`fabrication.unwaive`) undoes one waiver. Writing files is an external
//! effect and is not undone.
//!
//! The pre-flight check and the export run off the UI thread on a snapshot
//! of the document (shared handles, no copy). A report computed for an
//! older document, other check settings or another profile is dropped and
//! computed again; an
//! export keeps the snapshot it started from.
use super::PcbEditor;
use crate::kit::{
    self, PickResponse, PickRow, Tick,
    card::{CardResponse, Subject},
    dialog::{DialogAction, DialogPanel},
    severity::Level,
    toast,
};
use crate::refusal::{self, Origin};
use egui::Ui;
use sparksmith_core::fabrication::{
    self, BottomRotation, FabricationCommand, FabricationOptions, PlacementOrigin, PreflightItem,
    PreflightReport, ProfileChoice,
};
use sparksmith_core::issues::Severity;
use sparksmith_core::model::Document;
use sparksmith_core::model::production::accepted_words;
use sparksmith_core::rules::RuleProfile;
use sparksmith_core::{Engine, Operation, ReportKey};
use sparksmith_io::fabrication::{ExportReport, ExportRequest, Progress};
use sparksmith_io::rule_profiles::ProfileLibrary;
use sparksmith_ui::theme::Tokens;
use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

/// Why Export waits for a new project's name.
pub(super) const UNNAMED: &str = "Save or name the project first: the files take its name";

/// The production files' base name: the options' own, else the project's.
/// An Untitled project takes the name its first save gives it: its own
/// file's (one saved as Untitled), else the KiCad or recovery file it came
/// from. A new project neither saved nor named has none (`None`): Export
/// waits for a name rather than writing "Untitled" files.
pub(super) fn files_stem(
    project: &sparksmith_core::Project,
    options: &FabricationOptions,
    file: Option<&std::path::Path>,
    origin: Option<&std::path::Path>,
) -> Option<String> {
    if options.base_name.is_some() || project.name != sparksmith_core::UNTITLED {
        return Some(fabrication::base_name(project, options).unwrap_or_else(|_| "board".into()));
    }
    let stem = file.or(origin)?.file_stem()?.to_str()?;
    Some(fabrication::stem_of(stem))
}

/// Rows a group of findings shows before "Show all".
const LIST_ROWS: usize = kit::LIST_MAX_ROWS;

pub(super) fn dialog_id() -> egui::Id {
    egui::Id::unique("pcb-production")
}

/// The profile the board is checked against.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum Pick {
    /// The board's own rules (its profile snapshot, overrides and classes).
    Board,
    User(String),
}

/// Folder or one ZIP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Folder,
    Zip,
}

/// What a pre-flight report was computed for: the document state and check
/// settings ([`Engine::report_key`]), the pick and the picked profile's
/// content, so an edited profile file is never shown as checked.
type Wanted = (ReportKey, Pick, Option<RuleProfile>);

/// A pre-flight report and what it was computed for.
type Checked = (Wanted, Result<PreflightReport, String>);

use crate::fab_suggestions::Suggested;

/// Suggestions and what they were computed for.
type SuggestedFor = (Wanted, Suggested);

enum ExportMessage {
    Progress(Progress),
    Done(Result<ExportReport, String>),
}

/// The Production files dialog's state while it is open.
pub(super) struct ProductionDialog {
    /// The user's fab profile library, read again before an export.
    library: Option<ProfileLibrary>,
    /// The user's fab profiles as last read.
    pub(super) user_profiles: Vec<RuleProfile>,
    pub(super) pick: Pick,
    /// The picked profile's name, to name it once it is gone.
    picked_name: Option<String>,
    pub(super) format: Format,
    /// The folder chosen to hold the output folder, or the ZIP file.
    pub(super) location: Option<PathBuf>,
    pub(super) options: FabricationOptions,
    /// The current report and what it was computed for.
    pub(super) report: Option<Checked>,
    checking: Option<(Wanted, Receiver<Checked>)>,
    /// The profiles that allow what the current report needs, once
    /// checked, and the check running for them.
    pub(super) suggested: Option<SuggestedFor>,
    suggesting: Option<(Wanted, Receiver<SuggestedFor>)>,
    /// Ticked findings and the reason being typed.
    pub(super) waiving: Waiving,
    picking: Option<Receiver<Option<PathBuf>>>,
    exporting: Option<Receiver<ExportMessage>>,
    /// Asks the running export to stop; nothing is written then.
    stop: Arc<AtomicBool>,
    progress: Option<Progress>,
    pub(super) written: Option<ExportReport>,
    refusal: Option<String>,
    opening: bool,
}

impl ProductionDialog {
    /// A new dialog writing into `folder` (the project's kept production
    /// folder) when that folder still exists.
    pub(super) fn new(library: Option<ProfileLibrary>, folder: Option<PathBuf>) -> Self {
        let user_profiles = library
            .as_ref()
            .and_then(|l| l.list().ok())
            .map(|l| l.profiles)
            .unwrap_or_default();
        Self {
            library,
            user_profiles,
            pick: Pick::Board,
            picked_name: None,
            format: Format::Folder,
            location: folder.filter(|f| f.is_dir()),
            options: FabricationOptions::default(),
            report: None,
            checking: None,
            suggested: None,
            suggesting: None,
            waiving: Waiving::default(),
            picking: None,
            exporting: None,
            stop: Arc::default(),
            progress: None,
            written: None,
            refusal: None,
            opening: true,
        }
    }

    /// The profile `pick` names; `Ok(None)` checks the board's own rules.
    /// A picked profile that is gone (its file deleted, renamed or no
    /// longer readable) is an error that names it, never the board's own
    /// rules: it blocks the check and the export until another is picked.
    fn profile(&self) -> Result<Option<RuleProfile>, String> {
        match &self.pick {
            Pick::Board => Ok(None),
            Pick::User(id) => self
                .user_profiles
                .iter()
                .find(|p| p.id == *id)
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    format!(
                        "‘{}’ is no longer among your fab profiles. Pick another profile",
                        self.picked_name.as_ref().unwrap_or(id)
                    )
                }),
        }
    }

    fn options_list(&self, engine: &Engine) -> Vec<(Pick, String)> {
        let mut out = vec![(
            Pick::Board,
            format!("{} · this board", engine.project().rules.profile_name()),
        )];
        out.extend(
            self.user_profiles
                .iter()
                .map(|p| (Pick::User(p.id.clone()), p.name.clone())),
        );
        if let (Pick::User(id), Err(_)) = (&self.pick, self.profile()) {
            let name = self.picked_name.as_ref().unwrap_or(id);
            out.push((self.pick.clone(), format!("{name} · missing")));
        }
        out
    }

    /// What the current report must have been computed for. A missing
    /// profile is a user pick without content, which no other state is.
    fn wanted(&self, engine: &Engine) -> Wanted {
        (
            engine.report_key(),
            self.pick.clone(),
            self.profile().ok().flatten(),
        )
    }

    /// Read the user's profiles again, so a file edited while the dialog
    /// is open is what the next check and export use. A library that no
    /// longer reads has none: what was read before is not what is there.
    fn reload_profiles(&mut self) {
        if let Some(library) = &self.library {
            self.user_profiles = library.list().map(|l| l.profiles).unwrap_or_default();
        }
    }

    /// Start or take the pre-flight report for the current document, check
    /// settings and profile content. A result for an older one is dropped.
    fn check(&mut self, ctx: &egui::Context, engine: &Engine) {
        let wanted = self.wanted(engine);
        if let Some((pending, receiver)) = &self.checking {
            match receiver.try_recv() {
                Ok(checked) => {
                    self.checking = None;
                    if checked.0 == wanted {
                        self.report = Some(checked);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.checking = None;
                    self.report = Some((
                        wanted.clone(),
                        Err("The production check stopped unexpectedly".into()),
                    ));
                }
                Err(TryRecvError::Empty) if *pending == wanted => return,
                Err(TryRecvError::Empty) => self.checking = None,
            }
        }
        let current = self.report.as_ref().is_some_and(|(w, _)| *w == wanted);
        if current || self.checking.is_some() {
            return;
        }
        let profile = match self.profile() {
            Ok(profile) => profile,
            Err(missing) => {
                self.report = Some((wanted, Err(missing)));
                return;
            }
        };
        let document = engine.document().clone();
        let checks = *engine.check_settings();
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let computed = wanted.clone();
        let spawned = std::thread::Builder::new()
            .name("sparksmith-preflight".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let report = preflight(&document, profile.as_ref(), &checks);
                let _ = sender.send((computed, report));
                repaint.request_repaint();
            }));
        match spawned {
            Ok(_) => self.checking = Some((wanted, receiver)),
            Err(error) => {
                self.report = Some((
                    wanted,
                    Err(format!("The production check could not start: {error}")),
                ));
            }
        }
    }

    /// Start or take the profile suggestions for the current report's
    /// needs, off the UI thread: the board is checked under every
    /// installed profile (the user's library, the shipped ones) as an
    /// export would check it. Nothing runs while no finding needs more of
    /// the profile; a result for another state is dropped.
    fn suggest(&mut self, ctx: &egui::Context, engine: &Engine) {
        let wanted = self.wanted(engine);
        if let Some((pending, receiver)) = &self.suggesting {
            match receiver.try_recv() {
                Ok(done) => {
                    self.suggesting = None;
                    if done.0 == wanted {
                        self.suggested = Some(done);
                    }
                }
                Err(TryRecvError::Disconnected) => self.suggesting = None,
                Err(TryRecvError::Empty) if *pending == wanted => return,
                Err(TryRecvError::Empty) => self.suggesting = None,
            }
        }
        let needs = matches!(
            self.current(engine),
            Some(Ok(report)) if report.items.iter().any(|i| i.needs.is_some())
        );
        let current = self.suggested.as_ref().is_some_and(|(w, _)| *w == wanted);
        if !needs || current || self.suggesting.is_some() {
            return;
        }
        let Ok(profile) = self.profile() else {
            return;
        };
        let document = engine.document().clone();
        let checks = *engine.check_settings();
        let library = self.library.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let computed = wanted.clone();
        let spawned = std::thread::Builder::new()
            .name("sparksmith-profile-suggestions".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let suggested = crate::fab_suggestions::compute(
                    &document,
                    profile.as_ref(),
                    &checks,
                    library.as_ref(),
                );
                let _ = sender.send((computed, suggested));
                repaint.request_repaint();
            }));
        if spawned.is_ok() {
            self.suggesting = Some((wanted, receiver));
        }
    }

    /// The suggestions for the current report, if computed.
    pub(super) fn current_suggestions(&self, engine: &Engine) -> Option<&Suggested> {
        let wanted = self.wanted(engine);
        self.suggested
            .as_ref()
            .filter(|(w, _)| *w == wanted)
            .map(|(_, s)| s)
    }

    /// Block until the suggestions for the current state arrive (tests).
    #[cfg(test)]
    pub(super) fn wait_suggestions(&mut self, ctx: &egui::Context, engine: &Engine) {
        for _ in 0..4000 {
            self.check(ctx, engine);
            self.suggest(ctx, engine);
            if self.current_suggestions(engine).is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The report for the current document, check settings and profile
    /// content, if computed.
    pub(super) fn current(&self, engine: &Engine) -> Option<&Result<PreflightReport, String>> {
        let wanted = self.wanted(engine);
        self.report
            .as_ref()
            .filter(|(w, _)| *w == wanted)
            .map(|(_, report)| report)
    }

    /// Take a picked location.
    /// Take the picked location; returns a picked folder (not a ZIP file),
    /// for the app to keep as the project's production folder.
    fn poll_picker(&mut self) -> Option<PathBuf> {
        let receiver = self.picking.as_ref()?;
        match receiver.try_recv() {
            Ok(path) => {
                self.picking = None;
                if path.is_some() {
                    self.location = path;
                    self.refusal = None;
                    if self.format == Format::Folder {
                        return self.location.clone();
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.picking = None,
        }
        None
    }

    /// Take export progress and the result.
    fn poll_export(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.exporting else {
            return;
        };
        loop {
            match receiver.try_recv() {
                Ok(ExportMessage::Progress(progress)) => self.progress = Some(progress),
                Ok(ExportMessage::Done(result)) => {
                    self.exporting = None;
                    self.progress = None;
                    match result {
                        Ok(report) => {
                            toast::post(
                                ctx,
                                Level::Ok,
                                format!("Wrote {} production files", report.files.len()),
                                Vec::new(),
                            );
                            self.written = Some(report);
                            self.refusal = None;
                        }
                        Err(error) => self.refusal = Some(error),
                    }
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.exporting = None;
                    self.progress = None;
                    self.refusal = Some("The export stopped unexpectedly".into());
                    return;
                }
            }
        }
    }

    /// Ask where to write, off the UI thread: a folder that will hold the
    /// output folder, or the ZIP file.
    fn start_pick(&mut self, ctx: &egui::Context, stem: &str) {
        if self.picking.is_some() {
            return;
        }
        let dialog = rfd::AsyncFileDialog::new();
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
        > = match self.format {
            Format::Folder => Box::pin(
                dialog
                    .set_title("Choose the folder for the production files")
                    .pick_folder(),
            ),
            Format::Zip => Box::pin(
                dialog
                    .set_title("Save the production files as a ZIP")
                    .add_filter("ZIP archive", &["zip"])
                    .set_file_name(format!("{stem}-production.zip"))
                    .save_file(),
            ),
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("sparksmith-production-picker".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let picked = pollster::block_on(future).map(|f| f.path().to_path_buf());
                let _ = sender.send(picked);
                repaint.request_repaint();
            }));
        if spawned.is_ok() {
            self.picking = Some(receiver);
        }
    }

    /// The folder or ZIP the export writes.
    pub(super) fn output(&self, stem: &str) -> Option<PathBuf> {
        let location = self.location.as_ref()?;
        Some(match self.format {
            Format::Folder => location.join(format!("{stem}-production")),
            Format::Zip => location.clone(),
        })
    }

    /// Write the set from `document` off the UI thread.
    /// The files are named `stem`, the name [`files_stem`] gave.
    pub(super) fn start_export(
        &mut self,
        ctx: &egui::Context,
        document: Document,
        checks: sparksmith_core::layout::CheckSettings,
        output: PathBuf,
        stem: String,
    ) {
        if self.exporting.is_some() {
            return;
        }
        // The profile file may have changed since it was checked: export
        // only what the shown check passed.
        let checked = self.profile();
        self.reload_profiles();
        let Ok(profile) = self.profile() else {
            // The dialog names the missing profile while it is picked.
            self.refusal = None;
            return;
        };
        if checked.as_ref() != Ok(&profile) {
            self.refusal = Some(
                "The fab profile's file changed since the check; check the new rules, then export"
                    .into(),
            );
            return;
        }
        let request = ExportRequest {
            output,
            zip: self.format == Format::Zip,
            profile,
            options: FabricationOptions {
                base_name: Some(stem),
                ..self.options.clone()
            },
        };
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let stop = Arc::new(AtomicBool::new(false));
        self.stop = Arc::clone(&stop);
        let spawned = std::thread::Builder::new()
            .name("sparksmith-production".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let mut shown = 0;
                let mut progress = |p: Progress| {
                    if stop.load(Ordering::Relaxed) {
                        return ControlFlow::Break(());
                    }
                    // Within a step it only asks whether to go on.
                    if p.step != shown {
                        shown = p.step;
                        let _ = sender.send(ExportMessage::Progress(p));
                        repaint.request_repaint();
                    }
                    ControlFlow::Continue(())
                };
                let result = sparksmith_io::fabrication::export(
                    &document.project,
                    &document.drafts,
                    &request,
                    &checks,
                    &mut progress,
                )
                .map_err(|e| export_error(&e));
                let _ = sender.send(ExportMessage::Done(result));
                repaint.request_repaint();
            }));
        match spawned {
            Ok(_) => {
                self.exporting = Some(receiver);
                self.written = None;
                self.refusal = None;
            }
            Err(error) => self.refusal = Some(format!("The export could not start: {error}")),
        }
    }

    /// Stop the running export: it ends at its next step, or within the
    /// read-back, and writes nothing.
    pub(super) fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Whether an export is running.
    pub(super) fn busy(&self) -> bool {
        self.exporting.is_some()
    }

    /// Block until the running export finishes (tests).
    #[cfg(test)]
    pub(super) fn wait(&mut self, ctx: &egui::Context) {
        while self.exporting.is_some() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            self.poll_export(ctx);
        }
    }

    /// Block until the pre-flight report for the current state arrives
    /// (tests).
    #[cfg(test)]
    pub(super) fn wait_check(&mut self, ctx: &egui::Context, engine: &Engine) {
        for _ in 0..2000 {
            self.check(ctx, engine);
            if self.current(engine).is_some() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// The pre-flight report of a snapshot.
fn preflight(
    document: &Document,
    profile: Option<&RuleProfile>,
    checks: &sparksmith_core::layout::CheckSettings,
) -> Result<PreflightReport, String> {
    fabrication::preflight::report(&document.project, &document.drafts, profile, checks)
        .map_err(|e| refusal::shown_error(&e))
}

/// One line for an export failure: the rule code is dropped. Files that do
/// not match the design are named with the place and both values.
fn export_error(error: &sparksmith_io::Error) -> String {
    match error {
        sparksmith_io::Error::Document(e) if e.rule() == Some(fabrication::readback::MISMATCH) => {
            refusal::clip(&e.to_string(), 600)
        }
        sparksmith_io::Error::Document(e) => refusal::shown_error(e),
        other => {
            let text = other.to_string();
            let text = match text.split_once(": ") {
                Some((code, rest)) if code.contains('.') && !code.contains(' ') => rest.to_owned(),
                _ => text,
            };
            refusal::clip(&text, 80)
        }
    }
}

/// "1.2 MB", "34.5 KB", "812 B".
pub fn size_text(bytes: usize) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else if b >= 1024.0 {
        format!("{:.1} KB", b / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

/// "2 errors · 1 warning · 1 waived", or "No issues".
pub fn report_summary(report: &PreflightReport) -> String {
    let mut parts = Vec::new();
    let unwaived = |s: Severity| {
        report
            .items
            .iter()
            .filter(|i| i.severity == s && i.waiver.is_none())
            .count()
    };
    for (severity, level) in [
        (Severity::Error, Level::Error),
        (Severity::Warning, Level::Warning),
        (Severity::ToDo, Level::ToDo),
    ] {
        let n = unwaived(severity);
        if n > 0 {
            parts.push(level.count_words(n));
        }
    }
    if report.waived > 0 {
        parts.push(format!("{} waived", report.waived));
    }
    if parts.is_empty() {
        "No issues".into()
    } else {
        parts.join(" · ")
    }
}

/// Why Export is disabled, or `None` when it can run.
pub(super) fn export_blocked(
    dialog: &ProductionDialog,
    report: Option<&Result<PreflightReport, String>>,
    named: bool,
    output: Option<&PathBuf>,
) -> Option<String> {
    if dialog.busy() {
        return Some("Writing the production files…".into());
    }
    match report {
        None => return Some("Checking the design…".into()),
        Some(Err(error)) => return Some(error.clone()),
        // Errors only a profile that allows the board resolves are not
        // waived: say so.
        Some(Ok(report))
            if report.blocking > 0
                && report
                    .items
                    .iter()
                    .filter(|i| i.blocks())
                    .all(|i| !i.offers_waiver()) =>
        {
            let blocking = || report.items.iter().filter(|i| i.blocks());
            // Only a finding with needs is resolved by another profile.
            return Some(if blocking().any(|i| i.code == "rules.no_fab_profile") {
                "Choose a fab profile first".into()
            } else if blocking().any(|i| i.needs.is_some()) {
                "Fix the design or switch to a fab profile that allows it first".into()
            } else if report.blocking == 1 {
                "Fix the error first; it cannot be waived".into()
            } else {
                format!(
                    "Fix the {} errors first; they cannot be waived",
                    report.blocking
                )
            });
        }
        Some(Ok(report)) if report.blocking == 1 => {
            return Some("Fix or waive the error first".into());
        }
        Some(Ok(report)) if report.blocking > 1 => {
            let fixed_only = report
                .items
                .iter()
                .filter(|i| i.blocks() && !i.offers_waiver())
                .count();
            let words = format!("Fix or waive the {} errors first", report.blocking);
            return Some(match fixed_only {
                0 => words,
                n => format!("{words}; {n} cannot be waived"),
            });
        }
        Some(Ok(_)) => {}
    }
    if !named {
        return Some(UNNAMED.into());
    }
    if output.is_none() {
        return Some("Choose where to save the files".into());
    }
    None
}

impl PcbEditor {
    /// Open the Production files dialog (the ribbon's Production files…).
    /// The user profile library is the config folder's `fab-profiles/`.
    pub(super) fn open_production(&mut self) {
        let library = sparksmith_io::settings::UserDirs::from_env()
            .ok()
            .and_then(|dirs| ProfileLibrary::from_dirs(&dirs).ok());
        self.open_production_in(library);
    }

    /// [`Self::open_production`] over a given library (tests).
    pub(super) fn open_production_in(&mut self, library: Option<ProfileLibrary>) {
        self.setup = None;
        self.stackup = None;
        self.net_classes = None;
        self.rules = None;
        self.variants = None;
        if self.production.is_none() {
            self.production = Some(ProductionDialog::new(
                library,
                self.production_folder.clone(),
            ));
        }
    }

    /// Whether the Production dialog is open.
    #[cfg(test)]
    pub(crate) fn production_open(&self) -> bool {
        self.production.is_some()
    }

    /// A production folder just chosen in the dialog, once, for the app
    /// to keep for this project.
    pub fn take_chosen_production_folder(&mut self) -> Option<PathBuf> {
        self.chosen_production_folder.take()
    }

    pub(super) fn production_subject(&self) -> Option<Subject> {
        self.production.as_ref().map(|_| Subject::Dialog)
    }

    /// The whole subject slot while the dialog is open.
    pub(super) fn production_slot(
        &mut self,
        ui: &mut Ui,
        engine: &Engine,
    ) -> CardResponse<Operation> {
        let Some(mut dialog) = self.production.take() else {
            return CardResponse::output(None);
        };
        let ctx = ui.ctx().clone();
        if let Some(text) = refusal::claim(&ctx, Origin::Dialog(dialog_id())) {
            dialog.refusal = Some(text);
        }
        dialog.check(&ctx, engine);
        dialog.suggest(&ctx, engine);
        if let Some(folder) = dialog.poll_picker() {
            self.chosen_production_folder = Some(folder);
        }
        dialog.poll_export(&ctx);
        let stem = files_stem(
            engine.project(),
            &dialog.options,
            self.project_path.as_deref(),
            self.project_origin.as_deref(),
        );
        let output = stem.as_deref().and_then(|s| dialog.output(s));
        let report = dialog.current(engine).cloned();
        let blocked = export_blocked(&dialog, report.as_ref(), stem.is_some(), output.as_ref());
        // The footer's status line says why errors keep Export disabled, as
        // a review's says its gate; the other reasons show in the body.
        let errors = matches!(&report, Some(Ok(report)) if report.blocking > 0);
        let status = dialog
            .progress
            .as_ref()
            .map(|p| format!("Step {} of {} · {}", p.step, p.total, p.label))
            .or_else(|| blocked.clone().filter(|_| errors));
        // A missing profile is named for as long as it is picked.
        let refusal_text = dialog.profile().err().or_else(|| dialog.refusal.clone());
        let panel = DialogPanel {
            kind: "Output",
            body_rows: 24.0,
            primary_disabled: blocked.as_deref(),
            refusal: refusal_text.as_deref(),
            reset_scroll: dialog.opening,
            status: status.as_deref(),
            ..DialogPanel::new(dialog_id(), "export", "Production files", "Export")
        };
        dialog.opening = false;
        let mut op = None;
        let action = panel.show(ui, |ui| {
            op = body(
                &mut dialog,
                ui,
                engine,
                report.as_ref(),
                output.as_ref(),
                stem.as_deref(),
            );
        });
        let mut close = false;
        match action {
            Some(DialogAction::Cancel) => {
                dialog.stop();
                close = true;
            }
            Some(DialogAction::Primary) => {
                if let (Some(output), Some(stem)) = (output, stem) {
                    dialog.start_export(
                        &ctx,
                        engine.document().clone(),
                        *engine.check_settings(),
                        output,
                        stem,
                    );
                }
            }
            None => {}
        }
        if op.is_some() {
            refusal::mark(&ctx, Origin::Dialog(dialog_id()));
        }
        if !close {
            self.production = Some(dialog);
        }
        CardResponse::output(op)
    }
}

/// What the dialog says of a check under the shipped default, which is no
/// fabricator's own profile: to choose one before ordering. It blocks
/// nothing.
fn no_fabricators(dialog: &ProductionDialog, engine: &Engine) -> Option<String> {
    let shipped = sparksmith_io::new_project::shipped().ok()?.fab_profile;
    let picked = dialog.profile().ok()?;
    let profile = picked
        .as_ref()
        .or(engine.project().rules.profile.as_ref())
        .filter(|p| p.id == shipped && p.fabricator.is_none())?;
    Some(format!(
        "Checked against {}; choose your fabricator's profile before ordering",
        profile.name
    ))
}

/// The dialog's body; returns a waiver edit.
fn body(
    dialog: &mut ProductionDialog,
    ui: &mut Ui,
    engine: &Engine,
    report: Option<&Result<PreflightReport, String>>,
    output: Option<&PathBuf>,
    stem: Option<&str>,
) -> Option<Operation> {
    let mut op = None;
    if let Some(p) = &dialog.progress
        && kit::summary_row(ui, p.label, Some("Stop"))
    {
        dialog.stop();
    }
    kit::section(
        ui,
        "pcb-production-check",
        "Production check",
        true,
        None,
        |ui| {
            let options = dialog.options_list(engine);
            let mut pick = dialog.pick.clone();
            if kit::choice_row(ui, "Fab profile", &mut pick, &options) && pick != dialog.pick {
                dialog.picked_name = options
                    .iter()
                    .find(|(p, _)| *p == pick)
                    .map(|(_, name)| name.clone());
                dialog.pick = pick;
                dialog.waiving = Waiving::default();
            }
            if let Some(text) = no_fabricators(dialog, engine) {
                kit::hint(
                    ui,
                    &text,
                    "It holds limits common to several fabricators. Yours can allow more or ask \
                     for more: choose its profile in Board › Rules.",
                );
            }
            match report {
                None => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(Tokens::get().font_small));
                        kit::note(ui, "Checking the design…");
                    });
                }
                Some(Err(_)) => {}
                Some(Ok(report)) => {
                    kit::value_row(ui, "Result", &report_summary(report));
                    op = findings(dialog, ui, report, engine.waiver_reason_required());
                    if let Some(switch) = needs(dialog, ui, engine, report) {
                        op = Some(switch);
                    }
                }
            }
        },
    );
    kit::section(ui, "pcb-production-output", "Output", true, None, |ui| {
        let mut format = dialog.format;
        let formats = [
            (Format::Folder, "Folder".to_owned()),
            (Format::Zip, "One ZIP file".to_owned()),
        ];
        if kit::choice_row(ui, "Format", &mut format, &formats) && format != dialog.format {
            dialog.format = format;
            dialog.location = None;
        }
        let shown = output.map_or_else(|| "Not chosen".to_owned(), |p| p.display().to_string());
        let mut choose = false;
        kit::row(ui, "Save to", |ui| {
            choose = kit::summary_row(ui, &shown, Some("Choose…"));
        });
        if stem.is_none() {
            // A new project's files would be "Untitled": say how to name them.
            kit::hint(ui, UNNAMED, "");
        } else if output.is_none() {
            // Why Export is grey, without hovering it.
            let what = match dialog.format {
                Format::Folder => "a folder",
                Format::Zip => "a ZIP file",
            };
            kit::hint(ui, &format!("Choose {what} to save to, then Export"), "");
        }
        if choose {
            dialog.start_pick(ui.ctx(), stem.unwrap_or(sparksmith_core::UNTITLED));
        }
    });
    kit::section(ui, "pcb-production-files", "Files", false, None, |ui| {
        let o = &mut dialog.options.outputs;
        kit::check_row(ui, "Gerber and job file", &mut o.gerber);
        kit::check_row(ui, "Drill and drill map", &mut o.drill);
        kit::check_row(ui, "Placement and BOM", &mut o.assembly);
        kit::check_row(ui, "PDF drawings", &mut o.drawings);
    });
    kit::section(
        ui,
        "pcb-production-placement",
        "Pick and place",
        false,
        None,
        |ui| {
            let p = &mut dialog.options.placement;
            let mut origin = match p.origin {
                PlacementOrigin::OutlineLowerLeft => 1,
                _ => 0,
            };
            let origins = [
                (0, "Board origin".to_owned()),
                (1, "Outline lower left".to_owned()),
            ];
            if kit::choice_row(ui, "Origin", &mut origin, &origins) {
                p.origin = if origin == 1 {
                    PlacementOrigin::OutlineLowerLeft
                } else {
                    PlacementOrigin::BoardOrigin
                };
            }
            let rotations = [
                (
                    BottomRotation::ViewedFromTop,
                    "Seen from the top".to_owned(),
                ),
                (
                    BottomRotation::ViewedFromBottom,
                    "Seen from below".to_owned(),
                ),
            ];
            kit::choice_row(ui, "Bottom", &mut p.bottom, &rotations);
            kit::check_row(ui, "Mirror bottom X", &mut p.mirror_bottom_x);
            kit::hint(ui, "Angles turn counter-clockwise from 0° as drawn", "");
        },
    );
    if let Some(written) = &dialog.written {
        let output = PathBuf::from(&written.output);
        kit::section(
            ui,
            "pcb-production-written",
            "Written",
            true,
            Some(written.files.len()),
            |ui| {
                kit::list_frame(ui, written.files.len(), usize::MAX, |ui, index| {
                    if let Some(file) = written.files.get(index) {
                        kit::summary_row(
                            ui,
                            &format!("{} · {}", file.name, size_text(file.bytes)),
                            None,
                        );
                    }
                });
                if kit::summary_row(
                    ui,
                    &output.display().to_string(),
                    Some(crate::library_sources::REVEAL),
                ) {
                    crate::library_sources::reveal(&output);
                }
            },
        );
    }
    op
}

/// Where the reason form opens: under a finding's row (its key) or a
/// group's header (its code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum At {
    Row(String),
    Group(String),
}

/// The production check's waiving: ticked findings, the reason being
/// typed and where its form shows, and how the groups are shown.
#[derive(Default)]
pub(super) struct Waiving {
    /// Ticked finding keys.
    pub(super) ticked: BTreeSet<String>,
    /// Where the reason form is; `None` while it is closed.
    pub(super) at: Option<At>,
    pub(super) reason: String,
    /// The last row ticked, for a Shift-click run.
    last: Option<String>,
    /// Groups unfolded, and groups showing all their rows.
    pub(super) open: BTreeSet<String>,
    all: BTreeSet<String>,
    /// Focus the reason and scroll the form into view when next drawn.
    reveal: bool,
}

impl Waiving {
    /// Open the form at `at` and bring it into view.
    fn open_at(&mut self, at: At) {
        self.at = Some(at);
        self.reveal = true;
    }

    /// Close the form and drop the ticks; the reason is kept for the next.
    fn close(&mut self) {
        self.ticked.clear();
        self.at = None;
        self.last = None;
        self.reveal = false;
    }
}

/// A finding that can be waived now: an error or warning nobody waived,
/// of a kind core offers a waiver for (never an unrouted connection, copper
/// of two nets that touches or files that would not be the design, never a
/// finding the board needs more of its fab profile for).
fn waivable(item: &PreflightItem) -> bool {
    item.waiver.is_none()
        && matches!(item.severity, Severity::Error | Severity::Warning)
        && item.offers_waiver()
}

/// What the board needs of its fab profile, from the report's findings,
/// and the installed profiles that allow it, each with "Switch to …":
/// one `rules.set_fab_profile` as the production check resolved it, so the
/// board is then made as it was checked. Returns that switch.
fn needs(
    dialog: &mut ProductionDialog,
    ui: &mut Ui,
    engine: &Engine,
    report: &PreflightReport,
) -> Option<Operation> {
    let requirements = sparksmith_core::rules::requirement::tightest(
        report
            .items
            .iter()
            .filter_map(|i| i.needs.as_ref().map(|n| &n.requirement)),
    );
    if requirements.is_empty() {
        return None;
    }
    // The words are core's, for the profile checked under: one line for
    // each rule's tightest need.
    for requirement in &requirements {
        if let Some(need) = report
            .items
            .iter()
            .filter_map(|i| i.needs.as_ref())
            .find(|n| n.requirement == *requirement)
        {
            kit::hint(ui, &need.text, &need_tip(need));
        }
    }
    let Some(suggested) = dialog.current_suggestions(engine).cloned() else {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new().size(Tokens::get().font_small));
            kit::note(ui, "Checking the installed fab profiles…");
        });
        return None;
    };
    let found = match &suggested.result {
        Err(error) => {
            kit::hint(ui, error, "");
            return None;
        }
        Ok(found) => found,
    };
    if found.suggestions.is_empty() {
        kit::hint(
            ui,
            "No installed fab profile allows this; add one that does",
            "",
        );
        return None;
    }
    let mut picked = None;
    kit::list_frame(ui, found.suggestions.len(), LIST_ROWS, |ui, index| {
        let Some(s) = found.suggestions.get(index) else {
            return;
        };
        let maker = s
            .fabricator_name
            .as_deref()
            .unwrap_or("No fabricator named");
        if kit::summary_row(ui, maker, Some(&format!("Switch to {}", s.name))) {
            picked = Some(s.id.clone());
        }
    });
    let id = picked?;
    match suggested.switch(engine, &id) {
        Ok(operation) => {
            // The board's own rules are now the ones to check.
            dialog.pick = Pick::Board;
            dialog.waiving = Waiving::default();
            Some(operation)
        }
        Err(error) => {
            dialog.refusal = Some(error);
            None
        }
    }
}

/// A need's hover: the profile's limit and where the profile says it
/// comes from.
fn need_tip(need: &sparksmith_core::rules::requirement::Needed) -> String {
    let mut tip = Vec::new();
    if let Some(limit) = &need.limit {
        tip.push(format!("{}: {limit}", need.profile));
    }
    if !need.source.is_empty() {
        tip.push(need.source.clone());
    }
    tip.join("\n")
}

/// The report's findings by code, in report order (most severe first).
fn groups(report: &PreflightReport) -> Vec<(&str, Vec<&PreflightItem>)> {
    let mut out: Vec<(&str, Vec<&PreflightItem>)> = Vec::new();
    for item in &report.items {
        match out.iter_mut().find(|(code, _)| *code == item.code) {
            Some((_, items)) => items.push(item),
            None => out.push((item.code.as_str(), vec![item])),
        }
    }
    out
}

/// One line of the findings list.
enum Entry<'a> {
    /// A group's header, by index.
    Header(usize),
    /// A finding, at an indent level.
    Row(&'a PreflightItem, usize),
    /// "n of m · Show all" of a group.
    More(usize),
    /// The reason form.
    Form,
}

/// The shown lines: a group of one is its row; a larger group is a
/// header, then (unfolded) its rows bounded at [`LIST_ROWS`] with "Show
/// all". The reason form follows the row or header it was opened from, or
/// that row's header while the row is hidden.
fn entries<'a>(waiving: &Waiving, groups: &[(&str, Vec<&'a PreflightItem>)]) -> Vec<Entry<'a>> {
    let mut out = Vec::new();
    let mut placed = false;
    for (g, (code, items)) in groups.iter().enumerate() {
        let here = match &waiving.at {
            Some(At::Group(c)) => c == code,
            Some(At::Row(key)) => items.iter().any(|i| i.key == *key),
            None => false,
        };
        if let [item] = items.as_slice() {
            out.push(Entry::Row(item, 0));
            if here {
                out.push(Entry::Form);
                placed = true;
            }
            continue;
        }
        out.push(Entry::Header(g));
        let open = waiving.open.contains(*code);
        let shown = if waiving.all.contains(*code) {
            items.len()
        } else {
            LIST_ROWS.min(items.len())
        };
        let row_shown = match &waiving.at {
            Some(At::Row(key)) => open && items.iter().take(shown).any(|i| i.key == *key),
            _ => false,
        };
        if here && !row_shown {
            out.push(Entry::Form);
            placed = true;
        }
        if open {
            for item in items.iter().take(shown) {
                out.push(Entry::Row(item, 1));
                if here && row_shown && waiving.at == Some(At::Row(item.key.clone())) {
                    out.push(Entry::Form);
                    placed = true;
                }
            }
            if shown < items.len() {
                out.push(Entry::More(g));
            }
        }
    }
    if !placed && waiving.at.is_some() {
        out.push(Entry::Form);
    }
    out
}

/// What the list's rows asked for this frame.
enum Asked {
    /// A group header's tick box, fold or Waive all….
    Header(usize, PickResponse),
    /// A row's tick box or action.
    Row(String, PickResponse),
    More(usize),
}

/// The findings list; returns a waiver or restore edit.
fn findings(
    dialog: &mut ProductionDialog,
    ui: &mut Ui,
    report: &PreflightReport,
    required: bool,
) -> Option<Operation> {
    let under = dialog.profile().ok().flatten();
    let w = &mut dialog.waiving;
    // Keep only ticks of findings that can still be waived.
    let can: BTreeSet<&str> = report
        .items
        .iter()
        .filter(|i| waivable(i))
        .map(|i| i.key.as_str())
        .collect();
    w.ticked.retain(|k| can.contains(k.as_str()));
    if w.ticked.is_empty() {
        w.at = None;
    }
    let groups = groups(report);
    let entries = entries(w, &groups);
    let base = ui.scope_id().with("findings");
    let mut asked = None;
    let mut form = (false, false);
    let count = w.ticked.len();
    let reveal = std::mem::take(&mut w.reveal);
    let (ticked, open, reason) = (&w.ticked, &w.open, &mut w.reason);
    kit::list_frame(ui, entries.len(), usize::MAX, |ui, index| {
        match entries.get(index) {
            Some(Entry::Header(g)) => {
                let Some((code, items)) = groups.get(*g) else {
                    return;
                };
                let can: Vec<&&PreflightItem> = items.iter().filter(|i| waivable(i)).collect();
                let on = can.iter().filter(|i| ticked.contains(&i.key)).count();
                let tick = match (can.len(), on) {
                    (0, _) => None,
                    (_, 0) => Some(Tick::Off),
                    (n, m) if n == m => Some(Tick::All),
                    _ => Some(Tick::Some),
                };
                let level = items
                    .iter()
                    .find(|i| i.waiver.is_none())
                    .map_or(Level::Ok, |i| Level::from(i.severity));
                let title = sparksmith_core::issues::group_title(code);
                let response = kit::pick_row(
                    ui,
                    base.with(("group", *code)),
                    &PickRow {
                        level,
                        title: &title,
                        tip: "",
                        depth: 0,
                        fold: Some(open.contains(*code)),
                        count: Some(items.len()),
                        tick,
                        action: (!can.is_empty()).then_some("Waive all…"),
                    },
                );
                if response != PickResponse::default() {
                    asked = Some(Asked::Header(*g, response));
                }
            }
            Some(Entry::Row(item, depth)) => {
                let tip = row_tip(item);
                let title = row_title(item);
                let (level, tick, action) = if item.waiver.is_some() {
                    (Level::Ok, None, Some("Restore"))
                } else if waivable(item) {
                    let on = ticked.contains(&item.key);
                    (
                        Level::from(item.severity),
                        Some(if on { Tick::All } else { Tick::Off }),
                        Some("Waive…"),
                    )
                } else {
                    (Level::from(item.severity), None, None)
                };
                let response = kit::pick_row(
                    ui,
                    base.with(("row", &item.key)),
                    &PickRow {
                        level,
                        title: &title,
                        tip: &tip,
                        depth: *depth,
                        fold: None,
                        count: None,
                        tick,
                        action,
                    },
                );
                if response != PickResponse::default() {
                    asked = Some(Asked::Row(item.key.clone(), response));
                }
            }
            Some(Entry::More(g)) => {
                if let Some((_, items)) = groups.get(*g) {
                    let shown = LIST_ROWS.min(items.len());
                    if kit::summary_row(
                        ui,
                        &format!("{shown} of {}", items.len()),
                        Some("Show all \u{203a}"),
                    ) {
                        asked = Some(Asked::More(*g));
                    }
                }
            }
            Some(Entry::Form) => form = reason_form(ui, reason, count, reveal, required),
            None => {}
        }
    });
    let mut op = None;
    match asked {
        Some(Asked::Header(g, response)) => {
            if let Some((code, items)) = groups.get(g) {
                let code = (*code).to_owned();
                if response.folded && !w.open.remove(&code) {
                    w.open.insert(code.clone());
                }
                let keys: Vec<&String> = items
                    .iter()
                    .filter(|i| waivable(i))
                    .map(|i| &i.key)
                    .collect();
                let all_on = keys.iter().all(|k| w.ticked.contains(*k));
                if response.ticked && all_on {
                    for key in keys {
                        w.ticked.remove(key);
                    }
                    if w.ticked.is_empty() {
                        w.close();
                    }
                } else if response.ticked || response.action {
                    w.ticked.extend(keys.into_iter().cloned());
                    w.open_at(At::Group(code));
                }
            }
        }
        Some(Asked::Row(key, response)) => {
            let item = report.items.iter().find(|i| i.key == key);
            if response.action && item.is_some_and(|i| i.waiver.is_some()) {
                op = Some(Operation::Fabrication(FabricationCommand::Unwaive {
                    key: key.clone(),
                }));
            } else if response.action {
                w.ticked.insert(key.clone());
                w.last = Some(key.clone());
                w.open_at(At::Row(key));
            } else if response.ticked {
                tick_row(w, &groups, &key, response.shift);
            }
        }
        Some(Asked::More(g)) => {
            if let Some((code, _)) = groups.get(g) {
                w.all.insert((*code).to_owned());
            }
        }
        None => {}
    }
    match form {
        (true, _) => {
            let keys: Vec<String> = w.ticked.iter().cloned().collect();
            let reason = w.reason.trim().to_owned();
            if !keys.is_empty() && (!required || !reason.is_empty()) {
                op = Some(Operation::Fabrication(FabricationCommand::Waive {
                    keys,
                    reason,
                    // The numbers accepted are the ones shown.
                    profile: under.map(|p| ProfileChoice::Profile(Box::new(p))),
                }));
                w.close();
            }
        }
        (false, true) => w.close(),
        (false, false) => {}
    }
    op
}

/// A row's title; of a finding whose waiver no longer holds, with why.
fn row_title(item: &PreflightItem) -> String {
    match &item.lapsed {
        Some(lapsed) => format!("{} \u{b7} {}", item.title, lapsed.text),
        None => item.title.clone(),
    }
}

/// A row's hover after its title: what it needs of the fab profile, what
/// fixes it, and its waiver.
fn row_tip(item: &PreflightItem) -> String {
    let mut tip = Vec::new();
    if let Some(need) = &item.needs {
        tip.push(need.text.clone());
        let more = need_tip(need);
        if !more.is_empty() {
            tip.push(more);
        }
    }
    if let Some(unstated) = &item.not_stated {
        tip.push(unstated.text.clone());
        tip.push(unstated.source.clone());
    }
    if let Some(waiver) = &item.waiver {
        tip.push(if waiver.reason.is_empty() {
            "Waived".to_owned()
        } else {
            format!("Waived: {}", waiver.reason)
        });
        tip.extend(accepted_words(&waiver.accepted).map(|a| format!("Accepted: {a}")));
    }
    if let Some(lapsed) = &item.lapsed {
        tip.push(lapsed.text.clone());
    }
    if let Some(fix) = &item.fix {
        tip.push(fix.clone());
    }
    if item.blocks() && item.needs.is_none() && !item.offers_waiver() {
        tip.push("Cannot be waived".to_owned());
    }
    tip.join("\n")
}

/// Tick or untick the row `key`; with Shift, tick the run of its group
/// from the last row ticked to it. The form follows the row.
fn tick_row(w: &mut Waiving, groups: &[(&str, Vec<&PreflightItem>)], key: &str, shift: bool) {
    let run = shift
        .then(|| {
            let last = w.last.as_deref()?;
            let (_, items) = groups
                .iter()
                .find(|(_, items)| items.iter().any(|i| i.key == key))?;
            let a = items.iter().position(|i| i.key == last)?;
            let b = items.iter().position(|i| i.key == key)?;
            let (from, to) = (a.min(b), a.max(b));
            Some(
                items
                    .get(from..=to)?
                    .iter()
                    .filter(|i| waivable(i))
                    .map(|i| i.key.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .flatten();
    match run {
        Some(keys) => w.ticked.extend(keys),
        None if w.ticked.remove(key) => {}
        None => {
            w.ticked.insert(key.to_owned());
        }
    }
    w.last = Some(key.to_owned());
    if w.ticked.is_empty() {
        w.close();
    } else if w.ticked.contains(key) {
        w.open_at(At::Row(key.to_owned()));
    }
}

/// The reason form under the row or group it was opened from: the reason,
/// then Cancel and "Waive n". Returns (waive, cancel).
fn reason_form(
    ui: &mut Ui,
    reason: &mut String,
    count: usize,
    reveal: bool,
    required: bool,
) -> (bool, bool) {
    let t = Tokens::get();
    let top = ui.cursor().top();
    ui.add_space(t.space_4);
    let field = kit::row(ui, "Reason", |ui| {
        kit::text_field(
            ui,
            "waive-reason",
            reason,
            if required { "Required" } else { "Optional" },
            false,
        )
    });
    let ready = !required || !reason.trim().is_empty();
    let mut out = (false, false);
    let label = if count == 1 {
        "Waive 1 finding".to_owned()
    } else {
        format!("Waive {count} findings")
    };
    kit::actions(ui, |ui| {
        if kit::primary(ui, &label, ready)
            .on_disabled_hover_text("Say why this is acceptable for this build")
            .clicked()
        {
            out.0 = true;
        }
        if sparksmith_ui::ui_kit::button(ui, "Cancel", sparksmith_ui::ui_kit::Button::Enabled(true))
            .clicked()
        {
            out.1 = true;
        }
    });
    ui.add_space(t.space_4);
    if reveal {
        field.request_focus();
        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
        ui.scroll_to_rect(rect, None);
    }
    out
}

#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
