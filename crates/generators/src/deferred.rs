//! A window's generator runtime: no call runs or waits on the window's
//! thread.
//!
//! A module's answer depends on its bytes and the request alone, so an
//! answer is an answer wherever it was made. A call on the window's thread
//! gets the answer when there is one; otherwise the call starts on a thread
//! of its own and the window's is refused with the transient
//! `generator.pending`, which changes nothing, like any refusal. The window
//! keeps the request that met it, shows that it waits, and runs it again
//! when the answer is there ([`Deferred::waiting`] is none) or drops it on
//! Stop ([`Deferred::stop`]). A call on any other thread runs through the
//! worker and waits there; its answer is kept for the window too, so a part
//! made in the background is applied without a second run.
use crate::cancel::Cancel;
use crate::worker::{self, Worker};
use sparksmith_generator_abi::v1::{Description, Request};
use sparksmith_generator_abi::{Failure, Generated, GeneratorRuntime, codec};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

/// Answers kept for the window, the newest: enough for one request that
/// describes every shipped module and makes a part.
const KEPT: usize = 256;

type Key = [u8; 32];

#[derive(Clone)]
enum Answer {
    Description(Result<Description, Failure>),
    Generated(Result<Generated, Failure>),
}

impl Answer {
    fn refusal(&self) -> Option<&Failure> {
        match self {
            Self::Description(answer) => answer.as_ref().err(),
            Self::Generated(answer) => answer.as_ref().err(),
        }
    }
}

#[derive(Default)]
struct State {
    /// The calls running for the window.
    running: BTreeMap<Key, Cancel>,
    answers: BTreeMap<Key, Answer>,
    /// The keys of `answers`, oldest first.
    order: VecDeque<Key>,
    /// The window's thread met a pending call since it last asked.
    met: bool,
}

impl State {
    /// Keep `answer`, unless it is about the host and not the module.
    fn keep(&mut self, key: Key, answer: &Answer) {
        let transient = answer
            .refusal()
            .and_then(Failure::code)
            .is_some_and(worker::is_transient);
        if transient || self.answers.insert(key, answer.clone()).is_some() {
            return;
        }
        self.order.push_back(key);
        if self.order.len() > KEPT
            && let Some(oldest) = self.order.pop_front()
        {
            self.answers.remove(&oldest);
        }
    }
}

struct Shared {
    worker: Worker,
    window: ThreadId,
    wake: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
}

/// A window's runtime, made on its thread; clones are the same one.
#[derive(Clone)]
pub struct Deferred(Arc<Shared>);

impl std::fmt::Debug for Deferred {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Deferred")
    }
}

impl Deferred {
    /// The runtime of the window whose thread this is, over `worker`;
    /// `wake` makes the window look when an answer came.
    pub fn new(worker: Worker, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(Shared {
            worker,
            window: std::thread::current().id(),
            wake: Box::new(wake),
            state: Mutex::default(),
        }))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.0.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Calls running for the window now.
    pub fn waiting(&self) -> usize {
        self.state().running.len()
    }

    /// Whether a call of the window's thread met `generator.pending` since
    /// this was last asked, whatever the caller made of the refusal.
    pub fn take_met(&self) -> bool {
        std::mem::take(&mut self.state().met)
    }

    /// Cancel every call running for the window.
    pub fn stop(&self) {
        let running = std::mem::take(&mut self.state().running);
        for cancel in running.into_values() {
            cancel.set();
        }
    }

    fn answer(
        &self,
        key: Key,
        call: impl FnOnce(&Worker) -> Answer + Send + 'static,
    ) -> Result<Answer, Failure> {
        if std::thread::current().id() != self.0.window {
            let answer = call(&self.0.worker);
            self.state().keep(key, &answer);
            return Ok(answer);
        }
        let mut state = self.state();
        if let Some(answer) = state.answers.get(&key) {
            return Ok(answer.clone());
        }
        state.met = true;
        if state.running.contains_key(&key) {
            return Err(pending());
        }
        let cancel = Cancel::default();
        let (own, shared) = (cancel.clone(), self.clone());
        std::thread::Builder::new()
            .name("sparksmith-window-generator".into())
            .spawn(move || {
                let answer = own.scope(|| call(&shared.0.worker));
                {
                    let mut state = shared.state();
                    // Stopped: the window no longer waits for it.
                    if !own.is_set() {
                        state.running.remove(&key);
                    }
                    state.keep(key, &answer);
                }
                (shared.0.wake)();
            })
            .map_err(|e| {
                Failure::one(
                    "generator.not_started",
                    format!("The generator could not be started ({e}). It was not run."),
                )
            })?;
        state.running.insert(key, cancel);
        Err(pending())
    }
}

fn pending() -> Failure {
    Failure::one(
        "generator.pending",
        "The generator is still running. Nothing was changed yet.".into(),
    )
}

fn key(call: &[u8], module: &[u8], request: &[u8]) -> Key {
    let parts = [call, &codec::sha256(module), &codec::sha256(request)].concat();
    codec::sha256(&parts)
}

fn mixed() -> Failure {
    Failure::one(
        "generator.worker",
        "The generator's answer is of another call. Nothing was changed.".into(),
    )
}

impl GeneratorRuntime for Deferred {
    fn describe(&self, module: &[u8]) -> Result<Description, Failure> {
        let bytes = module.to_vec();
        let answer = self.answer(key(b"describe", module, &[]), move |worker| {
            Answer::Description(worker.describe(&bytes))
        })?;
        match answer {
            Answer::Description(answer) => answer,
            Answer::Generated(_) => Err(mixed()),
        }
    }

    fn generate(&self, module: &[u8], request: &Request) -> Result<Generated, Failure> {
        let wire = codec::encode(request).map_err(|e| {
            Failure::one(
                "generator.request_refused",
                format!("The request could not be written ({e}). It was not run."),
            )
        })?;
        let (bytes, request) = (module.to_vec(), request.clone());
        let answer = self.answer(key(b"generate", module, &wire), move |worker| {
            Answer::Generated(worker.generate(&bytes, &request))
        })?;
        match answer {
            Answer::Generated(answer) => answer,
            Answer::Description(_) => Err(mixed()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cancel;
    use std::sync::mpsc;
    use std::time::Duration;

    type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const WATCHDOG: Duration = Duration::from_secs(120);

    fn code<T>(result: &Result<T, Failure>) -> Option<&str> {
        result.as_ref().err().and_then(Failure::code)
    }

    /// A runtime of this thread, and its wakes.
    fn deferred() -> (Deferred, mpsc::Receiver<()>) {
        let (send, wakes) = mpsc::channel();
        let wake = move || {
            let _ = send.send(());
        };
        (Deferred::new(Worker::fixture(), wake), wakes)
    }

    /// The window's call never runs on its thread: it is refused as
    /// pending, then answered by the same call once the answer came.
    #[test]
    fn a_windows_call_is_pending_and_then_answered() -> Fallible {
        let (runtime, wakes) = deferred();
        assert_eq!(
            code(&runtime.describe(b"not a module")),
            Some("generator.pending")
        );
        assert!(runtime.take_met() && !runtime.take_met());
        wakes.recv_timeout(WATCHDOG)?;
        assert_eq!(runtime.waiting(), 0);
        assert_eq!(
            code(&runtime.describe(b"not a module")),
            Some("generator.invalid_module")
        );
        assert!(!runtime.take_met());
        Ok(())
    }

    /// A background thread's answer is the window's too.
    #[test]
    fn a_background_answer_is_kept_for_the_window() -> Fallible {
        let (runtime, _wakes) = deferred();
        let other = runtime.clone();
        let made = std::thread::spawn(move || other.describe(b"not a module"))
            .join()
            .map_err(|_| "the thread panicked")?;
        assert_eq!(code(&made), Some("generator.invalid_module"));
        assert_eq!(
            code(&runtime.describe(b"not a module")),
            Some("generator.invalid_module")
        );
        Ok(())
    }

    /// Stop ends a wait that would never end, and the next call works.
    #[test]
    fn stop_ends_the_wait_and_the_next_call_works() -> Fallible {
        let (runtime, wakes) = deferred();
        let held = runtime.answer([7; 32], move |_| {
            // Until it is cancelled, as a worker waits.
            while !cancel::requested() {
                std::thread::yield_now();
            }
            Answer::Description(Err(Failure::one("generator.cancelled", "Stopped.".into())))
        });
        assert!(held.is_err());
        assert_eq!(runtime.waiting(), 1);
        runtime.stop();
        assert_eq!(runtime.waiting(), 0);
        wakes.recv_timeout(WATCHDOG)?;
        assert!(runtime.state().answers.is_empty(), "nothing is kept of it");
        assert_eq!(
            code(&runtime.describe(b"not a module")),
            Some("generator.pending")
        );
        wakes.recv_timeout(WATCHDOG)?;
        assert_eq!(
            code(&runtime.describe(b"not a module")),
            Some("generator.invalid_module")
        );
        Ok(())
    }
}
