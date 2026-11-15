//! The shipped generators for the window (slice D1 of the part-generation
//! plan): what `library.generators` describes, read once off the UI thread,
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
use sparksmith_automation::Session;
use sparksmith_automation::session::generators::v1;
use sparksmith_automation::session::generators::{GeneratePartParams, GeneratorSummary};
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
}

/// The shipped generators, loading or loaded: cheap to clone, shared by
/// every view that offers generated parts.
#[derive(Clone, Debug, Default)]
pub struct Shelf {
    loaded: Arc<Mutex<Loaded>>,
}

/// `library.generators`, on a session of its own: the shipped modules
/// described, in name order. A module the host could not describe is left
/// out.
pub fn read_offers() -> Result<Vec<Offer>, String> {
    let session = Session::lending();
    let report = session
        .generators_report()
        .map_err(|e| crate::refusal::automation_error(&e))?;
    if report.shipped_library.is_none() {
        return Err("The shipped library was not found, so no generator is offered".into());
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
        let shelf = Self::default();
        let loaded = Arc::clone(&shelf.loaded);
        let repaint = ctx.clone();
        let started = std::thread::Builder::new()
            .name("sparksmith-generators".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let read = match read_offers() {
                    Ok(offers) => Loaded::Ready(Arc::new(offers)),
                    Err(why) => Loaded::Failed(why),
                };
                if let Ok(mut slot) = loaded.lock() {
                    *slot = read;
                }
                repaint.request_repaint();
            }));
        if started.is_err()
            && let Ok(mut slot) = shelf.loaded.lock()
        {
            *slot = Loaded::Failed("The generators could not be read: no thread".into());
        }
        shelf
    }

    /// A shelf with `offers`, loaded now (tests).
    #[cfg(test)]
    pub fn with(offers: Vec<Offer>) -> Self {
        Self {
            loaded: Arc::new(Mutex::new(Loaded::Ready(Arc::new(offers)))),
        }
    }

    /// The shipped generators, once read.
    pub fn offers(&self) -> Option<Arc<Vec<Offer>>> {
        match &*self.loaded.lock().ok()? {
            Loaded::Ready(offers) => Some(Arc::clone(offers)),
            _ => None,
        }
    }

    /// Why no generator is offered, while none is.
    pub fn status(&self) -> Option<String> {
        match &*self.loaded.lock().ok()? {
            Loaded::Loading => Some("Reading the shipped generators…".into()),
            Loaded::Failed(why) => Some(why.clone()),
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
    receiver: Receiver<Result<Operation, String>>,
}

impl std::fmt::Debug for Making {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Making")
    }
}

impl Making {
    /// The command to apply once the part is made, or why it was not.
    pub fn poll(&mut self) -> Option<Result<Operation, String>> {
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
    std::thread::Builder::new()
        .name("sparksmith-make-part".into())
        .spawn(sparksmith_core::library::standards::carried(move || {
            let _ = sender.send(command(&fork, &request, into));
            repaint.request_repaint();
        }))
        .ok()?;
    Some(Making { receiver })
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
    static OFFERS: std::sync::OnceLock<Result<Vec<Offer>, String>> = std::sync::OnceLock::new();
    match OFFERS.get_or_init(read_offers) {
        Ok(offers) => Shelf::with(offers.clone()),
        Err(why) => Shelf {
            loaded: Arc::new(Mutex::new(Loaded::Failed(why.clone()))),
        },
    }
}

/// Wait for a part being made (tests): the command or why not.
#[cfg(test)]
pub fn wait(making: &mut Making) -> Result<Operation, String> {
    for _ in 0..600 {
        if let Some(done) = making.poll() {
            return done;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("making the part took more than 30 s".into())
}
