//! Generation off the caller's thread, with cancellation (slice GP1).
//!
//! Each call runs on a fresh worker thread under the host's fuel, module
//! size, memory and decode limits, which end a runaway module after the
//! same work on every machine. There is no time limit: a busy machine is
//! no property of a module, and whoever waits ends the wait themselves.
//!
//! At most four workers are active in the process. A call made while all
//! are busy waits for one. A caller gives up with its [`Cancel`], the
//! worker's own ([`Worker::with_cancel`]) or the call's it runs for
//! ([`crate::cancel::current`]): waiting or running, it is refused at once
//! (`generator.cancelled`), nothing is changed and the next call works. Its
//! worker thread runs on to the end of its fuel and then drops its result;
//! a permit is held through result/runtime cleanup, and a caller that
//! takes its result has the permit back by then, so calls made one after
//! another hold one.
//!
//! A window's thread makes no call at all: it has a [`crate::deferred`]
//! runtime.
use crate::cancel::{self, Cancel};
use crate::{Limits, WasmiRuntime};
use sparksmith_generator_abi::v1::{Description, Request};
use sparksmith_generator_abi::{Failure, Generated, GeneratorRuntime};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Whether a refusal with this code is about the host at that moment (the
/// call was cancelled, waits for its worker or could not start) and not
/// about the module: calling again can succeed, so it is never kept as the
/// module's answer.
pub fn is_transient(code: &str) -> bool {
    matches!(
        code,
        "generator.cancelled" | "generator.pending" | "generator.not_started"
    )
}

/// How often a waiting caller looks at its cancellation and at the pool.
const POLL: Duration = Duration::from_millis(10);

/// Most generator workers active in this process, including cancelled
/// calls still executing or decoding. Further calls wait for one.
pub const MAX_ACTIVE: usize = 4;

// A process resource counter, not host/library data. Fresh Worker values
// must share it: sessions also construct Workers for individual requests.
static SLOTS: Slots = Slots(AtomicUsize::new(0));

#[derive(Debug)]
struct Slots(AtomicUsize);

#[derive(Clone, Debug)]
enum Pool {
    Process(&'static Slots),
    #[cfg(any(test, feature = "test-fixtures"))]
    Fixture(Arc<Slots>),
}

impl Pool {
    fn slots(&self) -> &Slots {
        match self {
            Self::Process(slots) => slots,
            #[cfg(any(test, feature = "test-fixtures"))]
            Self::Fixture(slots) => slots,
        }
    }

    /// A permit, when a worker is free.
    fn acquire(&self) -> Option<Permit> {
        self.slots()
            .0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE).then(|| active + 1)
            })
            .ok()?;
        Some(Permit(self.clone()))
    }
}

struct Permit(Pool);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.slots().0.fetch_sub(1, Ordering::Release);
    }
}

/// A [`GeneratorRuntime`] that runs every call on its own thread.
#[derive(Clone, Debug)]
pub struct Worker {
    limits: Limits,
    cancel: Option<Cancel>,
    pool: Pool,
}

impl Default for Worker {
    fn default() -> Self {
        Self::new(Limits::DEFAULT)
    }
}

impl Worker {
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            cancel: None,
            pool: Pool::Process(&SLOTS),
        }
    }

    /// A test fixture's own pool of four workers. Clones share that pool;
    /// unrelated fixtures cannot consume it. Production constructors always
    /// use the process pool, even when this fixture API is compiled in.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn fixture() -> Self {
        Self {
            pool: Pool::Fixture(Arc::new(Slots(AtomicUsize::new(0)))),
            ..Self::default()
        }
    }

    /// This worker with a cancellation of its own: once set, the call in
    /// progress and every later one is refused (`generator.cancelled`).
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Wait for a worker, reserve it before preparing owned inputs, then
    /// run the job and wait for it, watching the cancellations. The thread
    /// owns the permit until it hands it over with the result; abandoning
    /// the receiver cannot release it early.
    fn run<T: Send + 'static, J>(&self, prepare: impl FnOnce() -> J) -> Result<T, Failure>
    where
        J: FnOnce(&WasmiRuntime) -> Result<T, Failure> + Send + 'static,
    {
        self.run_with(prepare, |task| {
            std::thread::Builder::new()
                .name("sparksmith-generator".into())
                .spawn(task)
                .map(|_| ())
        })
    }

    fn run_with<T: Send + 'static, J>(
        &self,
        prepare: impl FnOnce() -> J,
        spawn: impl FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
    ) -> Result<T, Failure>
    where
        J: FnOnce(&WasmiRuntime) -> Result<T, Failure> + Send + 'static,
    {
        let call = cancel::current();
        let given_up = || {
            [&self.cancel, &call]
                .into_iter()
                .flatten()
                .any(Cancel::is_set)
        };
        let permit = loop {
            if given_up() {
                return Err(cancelled());
            }
            match self.pool.acquire() {
                Some(permit) => break permit,
                None => std::thread::sleep(POLL),
            }
        };
        let job = prepare();
        if given_up() {
            return Err(cancelled());
        }
        let (send, receive) = mpsc::sync_channel(1);
        let limits = self.limits;
        spawn(Box::new(move || {
            let runtime = WasmiRuntime::new(limits);
            let result = job(&runtime);
            drop(runtime);
            // The receiver may have given up; then the result is dropped,
            // and the permit after it.
            let _ = send.send((result, permit));
        }))
        .map_err(|e| {
            Failure::one(
                "generator.not_started",
                format!("The generator could not be started ({e}). It was not run."),
            )
        })?;
        loop {
            match receive.recv_timeout(POLL) {
                Ok((result, permit)) => {
                    drop(permit);
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Timeout) if given_up() => return Err(cancelled()),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(Failure::one(
                        "generator.worker",
                        "The generator stopped without an answer. Nothing was changed.".into(),
                    ));
                }
            }
        }
    }
}

fn cancelled() -> Failure {
    Failure::one(
        "generator.cancelled",
        "The generator was cancelled. Nothing was changed.".into(),
    )
}

impl GeneratorRuntime for Worker {
    fn describe(&self, module: &[u8]) -> Result<Description, Failure> {
        crate::check_module_size(module.len(), self.limits.module_bytes)?;
        self.run(|| {
            let module = module.to_vec();
            move |runtime: &WasmiRuntime| runtime.describe(&module)
        })
    }

    fn generate(&self, module: &[u8], request: &Request) -> Result<Generated, Failure> {
        crate::check_module_size(module.len(), self.limits.module_bytes)?;
        self.run(|| {
            let module = module.to_vec();
            let request = request.clone();
            move |runtime: &WasmiRuntime| GeneratorRuntime::generate(runtime, &module, &request)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

    /// A test's watchdog against a hang, never a measure.
    const WATCHDOG: Duration = Duration::from_secs(120);

    fn code<T>(result: &Result<T, Failure>) -> Option<&str> {
        result.as_ref().err().and_then(Failure::code)
    }

    fn all(worker: &Worker) -> Vec<Permit> {
        (0..MAX_ACTIVE)
            .filter_map(|_| worker.pool.acquire())
            .collect()
    }

    fn active(worker: &Worker) -> usize {
        worker.pool.slots().0.load(Ordering::Acquire)
    }

    #[test]
    fn a_set_cancel_refuses_before_anything_runs() {
        let cancel = Cancel::default();
        cancel.set();
        let worker = Worker::fixture().with_cancel(cancel.clone());
        assert_eq!(
            code(&worker.describe(b"not a module")),
            Some("generator.cancelled")
        );
        let call = Worker::fixture();
        assert_eq!(
            code(&cancel.scope(|| call.describe(b"not a module"))),
            Some("generator.cancelled")
        );
        assert_eq!(
            code(&call.describe(b"not a module")),
            Some("generator.invalid_module"),
            "the next call works"
        );
    }

    #[test]
    fn transient_refusals_are_the_hosts_and_never_a_modules() {
        assert!(is_transient("generator.cancelled"));
        for verdict in [
            "generator.out_of_fuel",
            "generator.trapped",
            "generator.worker",
            "generator.invalid_module",
        ] {
            assert!(!is_transient(verdict), "{verdict}");
        }
    }

    /// With every worker busy a call waits; its cancel ends the wait, and
    /// the next call runs once a worker is free.
    #[test]
    fn a_wait_for_a_worker_is_ended_by_its_cancel() -> Fallible {
        let worker = Worker::fixture();
        let held = all(&worker);
        assert_eq!(held.len(), MAX_ACTIVE);
        let cancel = Cancel::default();
        let (waiter, stop) = (worker.clone(), cancel.clone());
        let caller = std::thread::spawn(move || {
            stop.scope(|| waiter.run(|| |_| Ok(())))
                .map_err(|f| f.to_string())
        });
        cancel.set();
        let ended = caller.join().map_err(|_| "caller panicked")?;
        assert!(ended.is_err());
        assert_eq!(active(&worker), MAX_ACTIVE, "it took no worker");

        let waiter = worker.clone();
        let caller = std::thread::spawn(move || waiter.run(|| |_| Ok(7)));
        drop(held);
        let ran = caller.join().map_err(|_| "caller panicked")?;
        assert_eq!(ran.map_err(|e| e.to_string())?, 7);
        assert_eq!(active(&worker), 0);
        Ok(())
    }

    /// A call that never ends is ended by its cancel. Its worker stays
    /// taken until the thread and its result are gone.
    #[test]
    fn a_running_call_is_ended_by_its_cancel_and_keeps_its_worker() -> Fallible {
        struct Dropped(Worker, mpsc::Sender<usize>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                let _ = self.1.send(active(&self.0));
            }
        }
        let worker = Worker::fixture();
        let cancel = Cancel::default();
        let (started, starts) = mpsc::channel();
        let (release, wait) = mpsc::channel::<()>();
        let (observed, drops) = mpsc::channel();
        let (inside, stop, runs) = (worker.clone(), cancel.clone(), worker.clone());
        let caller = std::thread::spawn(move || {
            stop.scope(|| {
                runs.run(|| {
                    move |_| {
                        let _ = started.send(());
                        let _ = wait.recv();
                        Ok(Dropped(inside, observed))
                    }
                })
            })
            .map(|_| ())
        });
        starts.recv_timeout(WATCHDOG)?;
        cancel.set();
        let ended = caller.join().map_err(|_| "caller panicked")?;
        assert_eq!(code(&ended), Some("generator.cancelled"));
        assert_eq!(active(&worker), 1);
        assert!(worker.run(|| |_| Ok(())).is_ok(), "the next call works");
        release.send(())?;
        assert_eq!(
            drops.recv_timeout(WATCHDOG)?,
            1,
            "dropped with its permit held"
        );
        Ok(())
    }

    /// A caller that takes its result has the permit back: one that calls
    /// again and again never waits for its own finished call.
    #[test]
    fn calls_one_after_another_hold_one_permit() -> Fallible {
        let worker = Worker::fixture();
        let _others: Vec<_> = (1..MAX_ACTIVE)
            .filter_map(|_| worker.pool.acquire())
            .collect();
        for _ in 0..200 {
            worker.run(|| |_| Ok(())).map_err(|e| e.to_string())?;
            assert_eq!(active(&worker), MAX_ACTIVE - 1);
        }
        Ok(())
    }

    /// Clones of a fixture share its pool; another fixture has its own, and
    /// production workers share the process's.
    #[test]
    fn pools_are_shared_by_clones_and_by_the_process() {
        let worker = Worker::fixture();
        let held = all(&worker);
        assert!(worker.clone().pool.acquire().is_none());
        assert!(Worker::fixture().run(|| |_| Ok(())).is_ok());
        drop(held);
        let (a, b) = (Worker::default(), Worker::new(Limits::DEFAULT));
        assert!(std::ptr::eq(a.pool.slots(), b.pool.slots()));
    }

    #[test]
    fn cancelling_during_preparation_runs_nothing_and_frees_the_worker() {
        let worker = Worker::fixture();
        let cancel = Cancel::default();
        let ended: Result<(), _> = cancel.scope(|| {
            worker.run(|| {
                cancel.set();
                |_| Ok(())
            })
        });
        assert_eq!(code(&ended), Some("generator.cancelled"));
        assert_eq!(active(&worker), 0);
    }

    #[test]
    fn a_failed_thread_spawn_releases_its_captured_permit() {
        let worker = Worker::fixture();
        let failure: Result<(), _> = worker.run_with(
            || |_: &WasmiRuntime| Ok(()),
            |task| {
                drop(task);
                Err(std::io::Error::other("scripted thread creation failure"))
            },
        );
        assert_eq!(code(&failure), Some("generator.not_started"));
        assert_eq!(active(&worker), 0);
        assert!(worker.run(|| |_| Ok(())).is_ok());
    }
}
