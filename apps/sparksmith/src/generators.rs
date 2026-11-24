//! The shipped generators for the window: what `library.generators` describes, read once off the UI thread,
//! and parts made with `library.generate_part`'s own code, also off the UI
//! thread. The catalog rows, the template gallery, the Place palette's
//! generic parts and the part editor's package panel all read the same
//! [`Shelf`] and make parts through [`make`].
//!
//! Making a part runs the module and evaluates its part file (seconds for a
//! large part) on a session over a fork of the window's engine; what comes
//! back is the typed command the request would apply (`library.generate`,
//! or `library.package_from_generator` for a working set's package), which
//! the window applies on its own engine as one undo step, as it applies
//! every other command.
use crate::document_version::DocumentVersion;
use sparksmith_automation::Session;
use sparksmith_automation::cancel::Cancel;
use sparksmith_automation::session::generators::v1;
use sparksmith_automation::session::generators::{
    GeneratePartParams, GeneratorSummary, is_transient,
};
use sparksmith_core::catalog::generic::GenericValue;
use sparksmith_core::catalog::generic::generators::{self as catalog, GeneratorCandidate};
use sparksmith_core::commands::PackageFromGenerator;
use sparksmith_core::commands::working_package::TerminalMatch;
use sparksmith_core::library::generated::{
    GeneratorParams, GeneratorScope, ParamProblem, Standing,
};
use sparksmith_core::{ComponentRef, DefinitionId, Engine, LibraryCommand, Operation};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SHELF_RETRY: Duration = Duration::from_millis(250);

/// One shipped generator: its bundle name (`library.generate_part`'s
/// `generator`), what it offers, and its description for the core's
/// checks.
#[derive(Clone, Debug)]
pub struct Offer {
    pub name: String,
    pub summary: GeneratorSummary,
    pub description: v1::Description,
}

impl Offer {
    /// The preset `key`, if the generator has it.
    pub fn preset(&self, key: &str) -> Option<&v1::Preset> {
        self.summary.presets.iter().find(|p| p.key == key)
    }
}

#[derive(Clone, Debug, Default)]
enum Loaded {
    #[default]
    Loading,
    Ready(Arc<Vec<Offer>>),
    Failed(String),
    /// Not read this time, and why: read again from `at` on.
    Retry {
        at: Instant,
        why: String,
    },
}

/// The shipped generators, loading or loaded: cheap to clone, shared by
/// every view that offers generated parts.
#[derive(Clone, Debug, Default)]
pub struct Shelf {
    loaded: Arc<Mutex<Loaded>>,
    repaint: Option<egui::Context>,
}

enum ReadFailure {
    /// The host could not read them just now: a retry can.
    Transient(String),
    Failed(String),
}

impl ReadFailure {
    #[cfg(test)]
    fn message(self) -> String {
        match self {
            Self::Transient(why) | Self::Failed(why) => why,
        }
    }
}

/// `library.generators`, on a session of its own. A module's own refusal
/// leaves that module out; a transient one refuses the incomplete read.
fn read_offers_once() -> Result<Vec<Offer>, ReadFailure> {
    let session = Session::lending();
    let report = session.generators_report().map_err(|e| {
        let why = crate::refusal::automation_error(&e);
        if is_transient(&e) {
            ReadFailure::Transient(why)
        } else {
            ReadFailure::Failed(why)
        }
    })?;
    if report.shipped_library.is_none() {
        return Err(ReadFailure::Failed(
            "The shipped library was not found, so no generator is offered".into(),
        ));
    }
    Ok(report
        .shipped
        .into_iter()
        .filter_map(|s| {
            let summary = s.description?;
            Some(Offer {
                name: s.name,
                description: summary.description(),
                summary,
            })
        })
        .collect())
}

impl Shelf {
    /// Read the shipped generators off the UI thread; `ctx` repaints when
    /// they are there.
    pub fn load(ctx: &egui::Context) -> Self {
        let shelf = Self {
            repaint: Some(ctx.clone()),
            ..Self::default()
        };
        shelf.start_read();
        shelf
    }

    fn start_read(&self) {
        let loaded = Arc::downgrade(&self.loaded);
        let repaint = self.repaint.clone();
        let started = std::thread::Builder::new()
            .name("sparksmith-generators".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                // The result belongs only to this shelf/context. Replacing
                // or closing its last owner abandons the read and retries.
                if loaded.strong_count() == 0 {
                    return;
                }
                let read = read_offers_once();
                if let Some(loaded) = loaded.upgrade() {
                    finish_read(&loaded, read, Instant::now());
                    if let Some(repaint) = repaint {
                        repaint.request_repaint();
                        repaint.request_repaint_after(SHELF_RETRY);
                    }
                }
            }));
        if started.is_err()
            && let Ok(mut slot) = self.loaded.lock()
        {
            *slot = Loaded::Failed("The generators could not be read: no thread".into());
        }
    }

    fn begin_retry(&self, now: Instant) -> bool {
        if let Ok(mut slot) = self.loaded.lock()
            && matches!(&*slot, Loaded::Retry { at, .. } if now >= *at)
        {
            *slot = Loaded::Loading;
            return true;
        }
        false
    }

    fn retry_if_due(&self) {
        // No waiting thread and no timer loop: a view asking for this
        // shelf starts at most one background retry after the backoff.
        if self.begin_retry(Instant::now()) {
            self.start_read();
        }
    }

    /// A shelf with `offers`, loaded now (tests).
    #[cfg(test)]
    pub fn with(offers: Vec<Offer>) -> Self {
        Self {
            loaded: Arc::new(Mutex::new(Loaded::Ready(Arc::new(offers)))),
            repaint: None,
        }
    }

    /// The shipped generators, once read.
    pub fn offers(&self) -> Option<Arc<Vec<Offer>>> {
        self.retry_if_due();
        match &*self.loaded.lock().ok()? {
            Loaded::Ready(offers) => Some(Arc::clone(offers)),
            _ => None,
        }
    }

    /// Why no generator is offered, while none is.
    pub fn status(&self) -> Option<String> {
        self.retry_if_due();
        match &*self.loaded.lock().ok()? {
            Loaded::Loading => Some("Reading the shipped generators…".into()),
            Loaded::Failed(why) => Some(why.clone()),
            Loaded::Retry { why, .. } => Some(format!(
                "The generators are not read yet; reading them again. {why}"
            )),
            Loaded::Ready(_) => None,
        }
    }

    /// The descriptions the core's catalog searches with.
    pub fn descriptions(&self) -> Vec<v1::Description> {
        self.offers()
            .map(|o| o.iter().map(|o| o.description.clone()).collect())
            .unwrap_or_default()
    }

    /// The offer of generator `id`.
    pub fn offer(&self, id: &str) -> Option<Offer> {
        self.offers()?
            .iter()
            .find(|o| o.description.id == id)
            .cloned()
    }
}

fn finish_read(loaded: &Mutex<Loaded>, result: Result<Vec<Offer>, ReadFailure>, now: Instant) {
    if let Ok(mut slot) = loaded.lock() {
        *slot = match result {
            Ok(offers) => Loaded::Ready(Arc::new(offers)),
            Err(ReadFailure::Transient(why)) => Loaded::Retry {
                at: now + SHELF_RETRY,
                why,
            },
            Err(ReadFailure::Failed(why)) => Loaded::Failed(why),
        };
    }
}

/// The generic parts `query` reads as (`catalog/generic-families.json`),
/// each made by the generator its family names.
pub fn find_generic(engine: &Engine, shelf: &Shelf, query: &str) -> Vec<GeneratorCandidate> {
    let descriptions = shelf.descriptions();
    if descriptions.is_empty() {
        return Vec::new();
    }
    let _context = engine.enter_context();
    catalog::search(engine.document(), query, &descriptions).unwrap_or_default()
}

/// Presets a search found, and how many more match.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub hits: Vec<catalog::PresetHit>,
    pub more: usize,
}

/// The presets a search text finds (none for an empty text).
pub fn find_presets(engine: &Engine, shelf: &Shelf, query: &str, limit: usize) -> Found {
    let (hits, more) =
        catalog::find_presets(engine.document(), query, &shelf.descriptions(), limit);
    Found { hits, more }
}

/// A text that reads as a preset's part lists it once, as that part.
pub fn drop_covered(found: &mut Found, candidates: &[GeneratorCandidate]) {
    found.hits.retain(|hit| {
        !candidates.iter().any(|c| {
            c.generator == hit.generator && c.covers_preset.as_deref() == Some(hit.preset.as_str())
        })
    });
}

/// A part to make: a generator, the preset its values start from, the
/// values, the name, and the values each placed instance takes.
#[derive(Clone, Debug, PartialEq)]
pub struct MakeRequest {
    pub generator: String,
    pub preset: Option<String>,
    pub params: GeneratorParams,
    pub name: Option<String>,
    pub values: Vec<GenericValue>,
}

impl MakeRequest {
    /// The request of a generic part a text read as.
    pub fn of_candidate(candidate: &GeneratorCandidate) -> Self {
        Self {
            generator: candidate.generator.clone(),
            preset: None,
            params: candidate.params.clone(),
            name: Some(candidate.name.clone()),
            values: candidate.values.clone(),
        }
    }

    /// The request of a preset as it is.
    pub fn of_preset(hit: &catalog::PresetHit) -> Self {
        Self {
            generator: hit.generator.clone(),
            preset: Some(hit.preset.clone()),
            params: GeneratorParams::new(),
            name: Some(hit.name.clone()),
            values: Vec::new(),
        }
    }

    /// The complete given parameters: the preset's, then the request's.
    pub fn given(&self, offer: &Offer) -> GeneratorParams {
        let mut params: GeneratorParams = self
            .preset
            .as_deref()
            .and_then(|key| offer.preset(key))
            .map(|p| {
                p.params
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            sparksmith_core::library::generated::ParamValue::from_wire(v),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        params.extend(self.params.clone());
        params
    }
}

/// Where the part `request` makes stands in the engine's project.
pub fn standing(
    engine: &Engine,
    offer: &Offer,
    scope: GeneratorScope,
    request: &MakeRequest,
) -> Standing {
    catalog::standing(
        engine.document(),
        &offer.description,
        scope,
        &request.given(offer),
    )
}

/// What a made part becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Into {
    /// A part of its own (`library.generate`).
    Part(GeneratorScope),
    /// The working set's package (`library.package_from_generator`).
    WorkingPackage { draft: DefinitionId, replace: bool },
}

/// A part being made off the UI thread.
pub struct Making {
    version: DocumentVersion,
    receiver: Receiver<Result<Operation, String>>,
    cancel: Cancel,
}

/// A part nobody waits for is not made.
impl Drop for Making {
    fn drop(&mut self) {
        self.cancel.set();
    }
}

impl std::fmt::Debug for Making {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Making")
    }
}

impl Making {
    /// Stop making it: [`Self::poll`] then says so.
    pub fn stop(&self) {
        self.cancel.set();
    }

    /// The command to apply once the part is made, or why it was not.
    pub fn poll(&mut self, engine: &Engine) -> Option<Result<Operation, String>> {
        if !self.version.is_current(engine) {
            return Some(Err(
                "The project changed. Review the part and try again.".into()
            ));
        }
        match self.receiver.try_recv() {
            Ok(done) => Some(done),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("Making the part stopped unexpectedly".into()))
            }
        }
    }
}

/// The command `request` makes on a session over `engine`'s document.
pub fn command(engine: &Engine, request: &MakeRequest, into: Into) -> Result<Operation, String> {
    let mut fork = engine.fork();
    let scope = match into {
        Into::Part(scope) => scope,
        Into::WorkingPackage { .. } => GeneratorScope::Package,
    };
    let params = GeneratePartParams {
        generator: Some(request.generator.clone()),
        module: None,
        scope,
        preset: request.preset.clone(),
        params: request.params.clone(),
        name: request.name.clone(),
        replaces: None,
        expected_revision: engine.revision(),
    };
    let mut session = Session::lending();
    let generate = session
        .lend(&mut fork, None, |s| s.generate_request(params))
        .map_err(|e| crate::refusal::automation_error(&e))?;
    Ok(match into {
        Into::Part(_) => Operation::Library(LibraryCommand::Generate(Box::new(generate))),
        Into::WorkingPackage { draft, replace } => Operation::Library(
            LibraryCommand::PackageFromGenerator(Box::new(PackageFromGenerator {
                draft,
                generate,
                replace,
                terminals: TerminalMatch::ByNumber,
                renumber: Vec::new(),
            })),
        ),
    })
}

/// Make `request` off the UI thread; `None` when no thread could start.
pub fn make(
    ctx: &egui::Context,
    engine: &Engine,
    request: MakeRequest,
    into: Into,
) -> Option<Making> {
    let fork = engine.fork();
    let (sender, receiver) = std::sync::mpsc::channel();
    let repaint = ctx.clone();
    let cancel = Cancel::default();
    let call = cancel.clone();
    std::thread::Builder::new()
        .name("sparksmith-make-part".into())
        .spawn(sparksmith_core::library::standards::carried(move || {
            let _ = sender.send(call.scope(|| command(&fork, &request, into)));
            repaint.request_repaint();
        }))
        .ok()?;
    Some(Making {
        receiver,
        version: DocumentVersion::of(engine),
        cancel,
    })
}

/// The module's refusal of `request`'s values, by field (empty when the
/// module makes the part): `request`'s parameters run through the module
/// on a session over a fork of `engine`, without its part file.
pub fn refusals(
    engine: &Engine,
    request: &MakeRequest,
    scope: GeneratorScope,
) -> Result<Vec<ParamProblem>, String> {
    let mut fork = engine.fork();
    let params = GeneratePartParams {
        generator: Some(request.generator.clone()),
        module: None,
        scope,
        preset: request.preset.clone(),
        params: request.params.clone(),
        name: None,
        replaces: None,
        expected_revision: engine.revision(),
    };
    let mut session = Session::lending();
    session
        .lend(&mut fork, None, |s| s.refusals(params))
        .map_err(|e| crate::refusal::automation_error(&e))
}

/// A check of a request's values by its module, off the UI thread (D1:
/// a value the module refuses is marked on its field as it is edited).
pub struct Checking {
    /// The request being checked.
    pub request: MakeRequest,
    receiver: Receiver<Vec<ParamProblem>>,
}

impl std::fmt::Debug for Checking {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Checking")
    }
}

impl Checking {
    /// The module's problems once the check is done. A check that could
    /// not run reports nothing: the make itself still refuses.
    pub fn poll(&mut self) -> Option<Vec<ParamProblem>> {
        match self.receiver.try_recv() {
            Ok(problems) => Some(problems),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Vec::new()),
        }
    }
}

/// Check `request` with its module off the UI thread; `None` when no
/// thread could start.
pub fn check(
    ctx: &egui::Context,
    engine: &Engine,
    request: MakeRequest,
    scope: GeneratorScope,
) -> Option<Checking> {
    let fork = engine.fork();
    let (sender, receiver) = std::sync::mpsc::channel();
    let repaint = ctx.clone();
    let checked = request.clone();
    std::thread::Builder::new()
        .name("sparksmith-check-part".into())
        .spawn(sparksmith_core::library::standards::carried(move || {
            let _ = sender.send(refusals(&fork, &checked, scope).unwrap_or_default());
            repaint.request_repaint();
        }))
        .ok()?;
    Some(Checking { request, receiver })
}

/// The component lineages the project has published.
pub fn published_components(engine: &Engine) -> Vec<DefinitionId> {
    engine
        .project()
        .library
        .components
        .iter()
        .map(|c| c.id)
        .collect()
}

/// The latest revision of a component lineage not in `before`.
pub fn new_component(engine: &Engine, before: &[DefinitionId]) -> Option<ComponentRef> {
    engine
        .project()
        .library
        .components
        .iter()
        .rfind(|c| !before.contains(&c.id))
        .map(|c| ComponentRef::new(c.id, c.revision))
}

/// The component the project holds from `request` now (after a make
/// applied), if it holds one.
pub fn made_component(
    engine: &Engine,
    shelf: &Shelf,
    request: &MakeRequest,
) -> Option<ComponentRef> {
    let offer = shelf.offer(&request.generator)?;
    catalog::component_of(&standing(
        engine,
        &offer,
        GeneratorScope::Component,
        request,
    ))
}

/// The shipped generators, read once for every test of the process.
#[cfg(test)]
pub fn test_shelf() -> Shelf {
    static OFFERS: Mutex<Option<Vec<Offer>>> = Mutex::new(None);
    match test_offers(&OFFERS, read_offers_once) {
        Ok(offers) => Shelf::with(offers),
        Err(why) => Shelf {
            loaded: Arc::new(Mutex::new(Loaded::Failed(why))),
            repaint: None,
        },
    }
}

/// Only fixture initialization is shared, as with the previous OnceLock;
/// tests run concurrently after loading. A transient refusal is retried
/// within a watchdog and successes only are cached. Neither a transient refusal nor a failed read poisons later tests.
#[cfg(test)]
fn test_offers(
    cache: &Mutex<Option<Vec<Offer>>>,
    mut read: impl FnMut() -> Result<Vec<Offer>, ReadFailure>,
) -> Result<Vec<Offer>, String> {
    let mut saved = cache
        .lock()
        .map_err(|_| "The test shelf lock was poisoned")?;
    if let Some(offers) = &*saved {
        return Ok(offers.clone());
    }
    let started = Instant::now();
    loop {
        match read() {
            Ok(offers) => {
                *saved = Some(offers.clone());
                return Ok(offers);
            }
            Err(ReadFailure::Transient(_)) if started.elapsed() < Duration::from_secs(30) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(failure) => return Err(failure.message()),
        }
    }
}

/// How long a test waits for a part being made: a watchdog against a hang
/// on a loaded machine, not a measure.
#[cfg(test)]
pub const TEST_PATIENCE: Duration = Duration::from_secs(120);

/// Wait for a part being made (tests): the command or why not.
#[cfg(test)]
pub fn wait(making: &mut Making, engine: &Engine) -> Result<Operation, String> {
    let started = Instant::now();
    while started.elapsed() < TEST_PATIENCE {
        if let Some(done) = making.poll(engine) {
            return done;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(format!("making the part took more than {TEST_PATIENCE:?}"))
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn test_shelf_cache_retries_a_transient_failure_and_never_caches_failures() {
        let cache = Mutex::new(None);
        assert!(test_offers(&cache, || Err(ReadFailure::Failed("unavailable".into()))).is_err());
        let mut calls = 0;
        let loaded = test_offers(&cache, || {
            calls += 1;
            if calls == 1 {
                Err(ReadFailure::Transient("cancelled".into()))
            } else {
                Ok(Vec::new())
            }
        });
        assert!(loaded.is_ok());
        assert_eq!(calls, 2);
        assert!(
            test_offers(&cache, || Err(ReadFailure::Failed(
                "must not read again".into()
            )))
            .is_ok()
        );
    }

    #[test]
    fn an_interrupted_shelf_read_retries_once_when_due_then_keeps_the_complete_result() {
        let shelf = Shelf::default();
        let now = Instant::now() + Duration::from_secs(3600);
        let interrupted = || Err(ReadFailure::Transient("It was cancelled.".into()));
        finish_read(&shelf.loaded, interrupted(), now);
        assert!(shelf.offers().is_none());
        assert_eq!(
            shelf.status().as_deref(),
            Some("The generators are not read yet; reading them again. It was cancelled.")
        );
        assert!(!shelf.begin_retry(now));
        assert!(shelf.begin_retry(now + SHELF_RETRY));
        assert!(!shelf.clone().begin_retry(now + SHELF_RETRY));
        let description = v1::Description {
            abi: 1,
            id: "fixture".into(),
            version: 1,
            title: "Fixture".into(),
            licence: "MIT".into(),
            deterministic: true,
            capabilities: Vec::new(),
            needs: Vec::new(),
        };
        finish_read(
            &shelf.loaded,
            Ok(vec![Offer {
                name: "fixture".into(),
                summary: GeneratorSummary::of(&description),
                description,
            }]),
            now,
        );
        assert_eq!(
            shelf
                .offers()
                .map(|offers| offers.iter().map(|o| o.name.clone()).collect::<Vec<_>>()),
            Some(vec!["fixture".into()])
        );
        assert!(shelf.status().is_none());
        assert!(!shelf.begin_retry(now + SHELF_RETRY));
    }

    #[test]
    fn replacing_the_shelf_abandons_old_results_and_retry_state() {
        let old = Shelf::default();
        let pending = Arc::downgrade(&old.loaded);
        let cancelled = Err(ReadFailure::Transient("cancelled".into()));
        finish_read(&old.loaded, cancelled, Instant::now());
        let current = Shelf::with(Vec::new());
        drop(old);
        assert!(pending.upgrade().is_none());
        assert!(current.offers().is_some());
        assert!(current.status().is_none());
    }

    #[test]
    fn permanent_load_failure_remains_visible_and_does_not_retry() {
        let shelf = Shelf::default();
        finish_read(
            &shelf.loaded,
            Err(ReadFailure::Failed("The shipped library is missing".into())),
            Instant::now(),
        );
        assert_eq!(
            shelf.status().as_deref(),
            Some("The shipped library is missing")
        );
        assert!(!shelf.begin_retry(Instant::now() + Duration::from_secs(3600)));
    }
}
