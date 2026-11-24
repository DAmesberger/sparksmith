//! The window's help search: made on a worker thread at launch, from the
//! shipped help and its embedding model, and read by Help and the command
//! palette.
use sparksmith_core::library::standards::Context;
use sparksmith_help_search::{Model, Search};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::{Arc, Weak};

type Made = Result<Search, String>;

#[derive(Default)]
enum State {
    #[default]
    None,
    Making(Receiver<Made>),
    Made(Made),
}

/// The search over one library context's help.
#[derive(Default)]
pub(super) struct Searcher {
    library: Weak<Context>,
    state: State,
}

impl Searcher {
    /// Start making the search of `context`'s help when it is not the one
    /// held, and take a finished one. Call once per frame.
    pub(super) fn poll(&mut self, context: &Arc<Context>, ctx: &egui::Context) {
        if self.of(context) {
            if let State::Making(made) = &self.state {
                match made.try_recv() {
                    Ok(made) => self.state = State::Made(made),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => {
                        self.state = State::Made(Err("The help search stopped".to_owned()));
                    }
                }
            }
            return;
        }
        self.library = Arc::downgrade(context);
        // Tests read it in the frame that asks ([`Self::search`]).
        if cfg!(test) {
            self.state = State::None;
            return;
        }
        let (done, made) = channel();
        let (context, ctx) = (Arc::clone(context), ctx.clone());
        let spawned = std::thread::Builder::new()
            .name("sparksmith-help".into())
            .spawn(move || {
                // Without its model the search ranks by words alone.
                let _ = done.send(make(&context, shipped_model().as_ref()));
                ctx.request_repaint();
            });
        self.state = match spawned {
            Ok(_) => State::Making(made),
            Err(why) => State::Made(Err(why.to_string())),
        };
    }

    /// The search of `context`'s help: `None` while it is being made.
    pub(super) fn search(&mut self, context: &Arc<Context>) -> Option<&Made> {
        if cfg!(test) && (!self.of(context) || matches!(self.state, State::None)) {
            self.library = Arc::downgrade(context);
            self.state = State::Made(make(context, None));
        }
        match &self.state {
            State::Made(made) if self.of(context) => Some(made),
            _ => None,
        }
    }

    fn of(&self, context: &Arc<Context>) -> bool {
        self.library.ptr_eq(&Arc::downgrade(context))
    }
}

fn make(context: &Context, model: Option<&Model>) -> Made {
    Search::new(context.help().map_err(|e| e.to_string())?, model)
}

fn shipped_model() -> Option<Model> {
    let library = sparksmith_io::shipped::ShippedLibrary::locate().ok()?;
    let files = sparksmith_io::help::model(&library).ok()??;
    Model::read(&files.tokenizer, &files.weights, &files.config).ok()
}
