//! The online library as the window's sessions reach it: a request of a
//! chat call or a live MCP call never waits for the network on the window's
//! thread. Asked there, it starts on a thread of its own and the call is
//! told it still runs; the window keeps the call and runs it again when the
//! answer is there (`app/generator_wait.rs`), or drops it on Stop, which
//! ends the request. On any other thread a request waits where it is asked.
use sparksmith_automation::cancel::Cancel;
use sparksmith_automation::session::online::{OnlineAccount, OnlineReport, OnlineSource, SignOut};
use sparksmith_core::ids::{LibraryId, Revision, SourceLineageId};
use sparksmith_core::library::bundle::Bundle;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

/// What a call is told of a request that still runs.
const RUNNING: &str = "the online library has not answered yet";

#[derive(Clone, PartialEq)]
enum Request {
    Search(String, usize),
    Bundle(LibraryId, SourceLineageId, Option<Revision>),
    SignOut,
}

#[derive(Clone)]
enum Answer {
    Search(Result<OnlineReport, String>),
    Bundle(Result<Option<Bundle>, String>),
    SignOut(Result<SignOut, String>),
}

#[derive(Default)]
struct State {
    running: Vec<(Request, Cancel)>,
    answers: Vec<(Request, Answer)>,
    /// The window's thread met a running request since it last asked.
    met: bool,
}

struct Shared {
    window: ThreadId,
    wake: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
}

/// The window's requests, made on its thread; clones are the same.
#[derive(Clone)]
pub struct OnlineCalls(Arc<Shared>);

impl OnlineCalls {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared {
            window: std::thread::current().id(),
            wake: Box::new(wake),
            state: Mutex::default(),
        }))
    }

    /// `source` as the window's sessions reach it.
    pub fn source(&self, source: Arc<dyn OnlineSource>) -> Arc<dyn OnlineSource> {
        Arc::new(WindowSource {
            source,
            calls: self.clone(),
        })
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Requests running now.
    pub fn waiting(&self) -> usize {
        self.state().running.len()
    }

    /// Whether a call met a running request since this was last asked.
    pub fn take_met(&self) -> bool {
        std::mem::take(&mut self.state().met)
    }

    /// End every request.
    pub fn stop(&self) {
        let mut state = self.state();
        for (_, cancel) in state.running.drain(..) {
            cancel.set();
        }
        state.answers.clear();
    }

    /// Forget the answers; for when no call waits for one.
    pub fn forget(&self) {
        self.state().answers.clear();
    }

    /// The answer to `request`, asked of `source` by `ask`; `None` on the
    /// window's thread while it runs.
    fn answer(
        &self,
        request: Request,
        source: &Arc<dyn OnlineSource>,
        ask: impl FnOnce(&dyn OnlineSource) -> Answer + Send + 'static,
    ) -> Result<Answer, String> {
        if std::thread::current().id() != self.0.window {
            return Ok(ask(source.as_ref()));
        }
        let mut state = self.state();
        if let Some((_, answer)) = state.answers.iter().find(|(r, _)| *r == request) {
            return Ok(answer.clone());
        }
        state.met = true;
        if state.running.iter().any(|(r, _)| *r == request) {
            return Err(RUNNING.into());
        }
        let cancel = Cancel::default();
        let (own, calls, source) = (cancel.clone(), self.clone(), Arc::clone(source));
        let asked = request.clone();
        std::thread::Builder::new()
            .name("online-call".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let answer = own.scope(|| ask(source.as_ref()));
                if !own.is_set() {
                    let mut state = calls.state();
                    state.running.retain(|(r, _)| *r != asked);
                    state.answers.push((asked, answer));
                }
                (calls.0.wake)();
            }))
            .map_err(|e| format!("the online library could not be asked: {e}"))?;
        state.running.push((request, cancel));
        Err(RUNNING.into())
    }
}

struct WindowSource {
    source: Arc<dyn OnlineSource>,
    calls: OnlineCalls,
}

impl OnlineSource for WindowSource {
    fn name(&self) -> String {
        self.source.name()
    }

    fn search(&self, text: &str, limit: usize) -> Result<OnlineReport, String> {
        let asked = text.to_owned();
        let request = Request::Search(asked.clone(), limit);
        let ask = move |s: &dyn OnlineSource| Answer::Search(s.search(&asked, limit));
        match self.calls.answer(request, &self.source, ask)? {
            Answer::Search(answer) => answer,
            _ => Err(RUNNING.into()),
        }
    }

    fn bundle(
        &self,
        library: LibraryId,
        lineage: SourceLineageId,
        revision: Option<Revision>,
    ) -> Result<Option<Bundle>, String> {
        let request = Request::Bundle(library, lineage, revision);
        let ask = move |s: &dyn OnlineSource| Answer::Bundle(s.bundle(library, lineage, revision));
        match self.calls.answer(request, &self.source, ask)? {
            Answer::Bundle(answer) => answer,
            _ => Err(RUNNING.into()),
        }
    }

    fn account(&self) -> Result<OnlineAccount, String> {
        self.source.account()
    }

    fn sign_out(&self) -> Result<SignOut, String> {
        let ask = |s: &dyn OnlineSource| Answer::SignOut(s.sign_out());
        match self.calls.answer(Request::SignOut, &self.source, ask)? {
            Answer::SignOut(answer) => answer,
            _ => Err(RUNNING.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// Answers a search of "now" at once and any other only when its call
    /// is cancelled.
    struct Slow;

    impl OnlineSource for Slow {
        fn name(&self) -> String {
            "slow".into()
        }
        fn search(&self, text: &str, _: usize) -> Result<OnlineReport, String> {
            if text != "now" {
                while !sparksmith_automation::cancel::requested() {
                    std::thread::yield_now();
                }
                return Err("stopped".into());
            }
            Ok(OnlineReport {
                total: 7,
                ..OnlineReport::default()
            })
        }
        fn bundle(
            &self,
            _: LibraryId,
            _: SourceLineageId,
            _: Option<Revision>,
        ) -> Result<Option<Bundle>, String> {
            Ok(None)
        }
        fn account(&self) -> Result<OnlineAccount, String> {
            Ok(OnlineAccount::default())
        }
        fn sign_out(&self) -> Result<SignOut, String> {
            Ok(SignOut::default())
        }
    }

    /// The window's thread never waits; a request that never ends is ended
    /// by Stop and holds up no other.
    #[test]
    fn a_window_call_never_waits_and_stop_ends_its_request()
    -> Result<(), Box<dyn std::error::Error>> {
        let (send, wakes) = mpsc::channel();
        let calls = OnlineCalls::new(move || {
            let _ = send.send(());
        });
        let source = calls.source(Arc::new(Slow));
        assert_eq!(source.search("never", 5).err().as_deref(), Some(RUNNING));
        assert_eq!(source.search("now", 5).err().as_deref(), Some(RUNNING));
        assert!(calls.take_met() && !calls.take_met());
        wakes.recv()?;
        assert_eq!(calls.waiting(), 1);
        assert_eq!(source.search("now", 5)?.total, 7);
        assert!(!calls.take_met());
        calls.stop();
        assert_eq!(calls.waiting(), 0);
        wakes.recv()?;
        assert!(calls.state().answers.is_empty(), "nothing is kept of it");
        // The next one works, and another thread waits where it asks.
        assert_eq!(source.search("now", 5).err().as_deref(), Some(RUNNING));
        wakes.recv()?;
        assert_eq!(source.search("now", 5)?.total, 7);
        let other = std::thread::spawn(move || source.search("now", 5).map(|r| r.total));
        assert_eq!(other.join().ok(), Some(Ok(7)));
        Ok(())
    }
}
