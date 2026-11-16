//! Where a run's calls go: the session they run on, here or in a window
//! (`docs/workflow-format.md`, the runner).
//!
//! A headless run (the CLI's `design-part`, the tests) owns its
//! [`Session`], which is its own desk. The desktop's chat runs a workflow
//! off the UI thread on the window's engine: its desk sends each call and
//! each piece of session work to the window, which runs it between frames
//! on the chat's session over the window's engine, with the window as the
//! call's `WindowHost` and the chat's gate questions as Ask cards. The
//! runner and the model loop see the same three operations either way, so
//! the run is one path wherever it runs.
use crate::verbs::Agent;
use serde_json::Value;
use sparksmith_automation::front::{Asker, Call, NoOne};
use sparksmith_automation::session::metadata;
use sparksmith_automation::{Grant, Session, WindowHost};

/// Work on a session's own state: attached documents, the open model step,
/// generators, its revision.
pub type Work = Box<dyn FnOnce(&mut Session) + Send>;

/// A verb call's answer: the transport envelope, and the revision before
/// and after it.
#[derive(Clone, Debug, PartialEq)]
pub struct Called {
    pub envelope: Value,
    pub before: u64,
    pub after: u64,
}

impl Called {
    /// An answer for a call that never ran.
    pub fn refused(envelope: Value) -> Self {
        Self {
            envelope,
            before: 0,
            after: 0,
        }
    }
}

/// Where a run's calls run. Every call is the caller's principal under
/// `grant`, in `turn` when one is open.
pub trait Desk {
    /// One of the model's verbs.
    fn verb(&mut self, grant: &Grant, turn: Option<&str>, verb: &str, input: Value) -> Called;
    /// One call of `entry` by the runner itself, at the session's current
    /// revision when the entry takes one.
    fn call(&mut self, grant: &Grant, turn: Option<&str>, entry: &str, params: Value) -> Value;
    /// Run `work` on the session; it has run (or never will) when this
    /// returns.
    fn session(&mut self, work: Work);
}

/// What `f` answers on `desk`'s session; `None` when it could not run
/// (the window that held the session is gone).
pub fn with<R: Send + 'static>(
    desk: &mut dyn Desk,
    f: impl FnOnce(&mut Session) -> R + Send + 'static,
) -> Option<R> {
    let (tx, rx) = std::sync::mpsc::channel();
    desk.session(Box::new(move |session| {
        let _ = tx.send(f(session));
    }));
    rx.try_recv().ok()
}

/// A verb call on `session`, with its window and asker.
pub fn verb_on(
    session: &mut Session,
    grant: &Grant,
    turn: Option<&str>,
    window: Option<&mut dyn WindowHost>,
    asker: &mut dyn Asker,
    verb: &str,
    input: Value,
) -> Called {
    let before = session.engine().revision();
    let envelope = Agent {
        session: &mut *session,
        grant,
        turn,
        window: window.map(|w| -> &mut dyn WindowHost { w }),
        asker,
    }
    .verb(verb, input);
    Called {
        envelope,
        before,
        after: session.engine().revision(),
    }
}

/// A call of `entry` on `session`, at its current revision when the entry
/// takes one.
pub fn call_on(
    session: &mut Session,
    grant: &Grant,
    turn: Option<&str>,
    window: Option<&mut dyn WindowHost>,
    asker: &mut dyn Asker,
    entry: &str,
    params: Value,
) -> Value {
    let expected_revision = metadata::find(entry)
        .filter(|meta| meta.revision)
        .map(|_| session.engine().revision());
    let call = Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    };
    Agent {
        session,
        grant,
        turn,
        window: window.map(|w| -> &mut dyn WindowHost { w }),
        asker,
    }
    .run(call)
}

/// A session is its own desk: no window, and no one to ask.
impl Desk for Session {
    fn verb(&mut self, grant: &Grant, turn: Option<&str>, verb: &str, input: Value) -> Called {
        verb_on(self, grant, turn, None, &mut NoOne, verb, input)
    }

    fn call(&mut self, grant: &Grant, turn: Option<&str>, entry: &str, params: Value) -> Value {
        call_on(self, grant, turn, None, &mut NoOne, entry, params)
    }

    fn session(&mut self, work: Work) {
        work(self);
    }
}
