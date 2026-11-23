//! Profile suggestions for the desktop: what the board needs of its fab
//! profile and which installed profiles allow it
//! (`sparksmith_core::fabrication::suggest`), computed off the UI thread,
//! and the switch to one. The Production dialog and the Issues card show
//! the same answer; the words and the order are core's.
use crate::refusal;
use sparksmith_core::fabrication::{self, ProfileSuggestions};
use sparksmith_core::model::Document;
use sparksmith_core::rules::{Fabricator, RuleProfile};
use sparksmith_core::{Engine, Operation, ReportKey};
use sparksmith_io::rule_profiles::ProfileLibrary;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

/// Suggestions with the installed profiles they were checked against (a
/// switch sends the one suggested).
#[derive(Clone, Debug)]
pub struct Suggested {
    pub result: Result<ProfileSuggestions, String>,
    pub profiles: Vec<RuleProfile>,
}

impl Suggested {
    /// The first suggestion: the one a row's "Switch to …" offers.
    pub fn first(&self) -> Option<&fabrication::Suggestion> {
        self.result.as_ref().ok()?.suggestions.first()
    }

    /// The rules edit that switches the board to the suggested profile
    /// `id`, as the production check resolved it.
    pub fn switch(&self, engine: &Engine, id: &str) -> Result<Operation, String> {
        let profile = self
            .profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| "That fab profile is no longer installed".to_owned())?;
        fabrication::snapshot::switch(engine.project(), profile)
            .map(Operation::Rules)
            .map_err(|e| refusal::shown_error(&e))
    }
}

/// The user's profile library (`fab-profiles/` in the config folder) over
/// the shipped one; `None` when the folders cannot be found.
pub fn user_library() -> Option<ProfileLibrary> {
    sparksmith_io::settings::UserDirs::from_env()
        .ok()
        .and_then(|dirs| ProfileLibrary::from_dirs(&dirs).ok())
}

/// The installed profiles and fabricator records `library` reads: the
/// fabricators are the records beside it in the same config folder, over
/// the shipped ones.
fn installed(library: Option<&ProfileLibrary>) -> (Vec<RuleProfile>, Vec<Fabricator>) {
    let Some(library) = library else {
        return Default::default();
    };
    let profiles = library
        .list()
        .map(|l| l.profiles.into_iter().chain(l.builtins).collect())
        .unwrap_or_default();
    let fabricators = library
        .dir()
        .parent()
        .map(|config| {
            sparksmith_io::fabricators::FabricatorLibrary::new(
                config.join(sparksmith_io::fabricators::FABRICATORS_DIR),
                library.shipped().clone(),
            )
        })
        .and_then(|f| f.list().ok())
        .map(|l| l.fabricators)
        .unwrap_or_default();
    (profiles, fabricators)
}

/// The suggestions of `document` checked under `profile` (the board's own
/// when `None`), against the profiles `library` holds. Blocking: call it
/// on a worker.
pub fn compute(
    document: &Document,
    profile: Option<&RuleProfile>,
    checks: &sparksmith_core::layout::CheckSettings,
    library: Option<&ProfileLibrary>,
) -> Suggested {
    let (profiles, fabricators) = installed(library);
    let result = fabrication::suggest::suggest(
        &document.project,
        &document.drafts,
        checks,
        profile,
        &profiles,
        &fabricators,
    )
    .map_err(|e| refusal::shown_error(&e));
    Suggested { result, profiles }
}

/// One worker's suggestions for the board's own profile, kept per check
/// state ([`Engine::report_key`]): started when asked for a state that has
/// none, dropped when the state moves on.
#[derive(Default)]
pub struct Cache {
    /// The library to read; `None` reads the user's ([`user_library`]).
    pub library: Option<ProfileLibrary>,
    done: Option<(ReportKey, Arc<Suggested>)>,
    running: Option<(ReportKey, Receiver<Suggested>)>,
}

impl Cache {
    /// The suggestions for `engine`'s current state, starting them when
    /// none run; `None` until they arrive.
    pub fn get(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Arc<Suggested>> {
        let key = engine.report_key();
        if let Some((pending, receiver)) = &self.running {
            match receiver.try_recv() {
                Ok(found) => {
                    let pending = *pending;
                    self.running = None;
                    if pending == key {
                        self.done = Some((key, Arc::new(found)));
                    }
                }
                Err(TryRecvError::Empty) if *pending == key => return None,
                Err(_) => self.running = None,
            }
        }
        if let Some((at, found)) = &self.done
            && *at == key
        {
            return Some(found.clone());
        }
        let document = engine.document().clone();
        let checks = *engine.check_settings();
        let library = self.library.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("sparksmith-profile-suggestions".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let library = library.or_else(user_library);
                let _ = sender.send(compute(&document, None, &checks, library.as_ref()));
                repaint.request_repaint();
            }));
        if spawned.is_ok() {
            self.running = Some((key, receiver));
        }
        None
    }

    /// Block until the suggestions for the current state arrive (tests).
    #[cfg(test)]
    pub fn wait(&mut self, ctx: &egui::Context, engine: &Engine) -> Option<Arc<Suggested>> {
        for _ in 0..4000 {
            if let Some(found) = self.get(ctx, engine) {
                return Some(found);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        None
    }
}
