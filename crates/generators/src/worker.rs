//! Generation off the caller's thread, with a wall-time bound and
//! cancellation (slice GP1).
//!
//! Each call runs on a fresh worker thread under the host's fuel and memory
//! limits. The caller waits at most [`Worker::wall`] and checks its cancel
//! flag while it waits; a call that runs out of time or is cancelled is
//! refused at once and its result, if one ever comes, is dropped. The
//! worker thread itself stops after bounded module execution and host
//! decoding. At most four workers are active in the process, including
//! abandoned calls: a permit stays with the thread through result/runtime
//! cleanup. A timeout bounds the wait, not the thread's lifetime.
//!
//! When all four are busy an interactive call is refused at once with the
//! retryable `generator.busy`. Batch work that nobody is there to retry (a
//! KiCad import making every part's model) asks for a worker with
//! [`Worker::waiting_until`] instead: it waits for one up to its deadline,
//! still watching the cancel flag, and its wall time starts once admitted.
use crate::{Limits, WasmiRuntime};
use sparksmith_generator_abi::v1::{Description, Request};
use sparksmith_generator_abi::{Failure, Generated, GeneratorRuntime};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The wall-time target of one call (`generators/README.md`): fuel bounds
/// the work, this bounds how long a caller waits on a slow host.
pub const WALL: Duration = Duration::from_secs(2);

/// How often a waiting caller looks at its cancel flag.
const POLL: Duration = Duration::from_millis(10);

/// Most generator workers active in this process, including timed-out or
/// cancelled calls still executing or decoding. A new call is refused as
/// busy rather than queued when all permits are held.
pub const MAX_ACTIVE: usize = 4;

// A process resource counter, not host/library data. Fresh Worker values
// must share it: sessions also construct Workers for individual requests.
static SLOTS: Slots = Slots(AtomicUsize::new(0));

#[derive(Debug)]
struct Slots(AtomicUsize);

impl Slots {
    fn acquire(&self) -> Result<(), Failure> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < MAX_ACTIVE).then(|| active + 1)
            })
            .map_err(|_| {
                Failure::one(
                    "generator.busy",
                    format!("All {MAX_ACTIVE} generator workers are still busy. Try again when one finishes. It was not run."),
                )
            })?;
        Ok(())
    }
}

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

    fn acquire(&self) -> Result<Permit, Failure> {
        self.slots().acquire()?;
        Ok(Permit(self.clone()))
    }
}

struct Permit(Pool);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.slots().0.fetch_sub(1, Ordering::Release);
    }
}

/// How a call gets a worker when all of them are busy.
#[derive(Clone, Copy, Debug)]
enum Admission {
    /// Refuse at once with the retryable `generator.busy` (interactive).
    Refuse,
    /// Wait for a worker until this deadline (batch work).
    WaitUntil(Instant),
}

/// A [`GeneratorRuntime`] that runs every call on its own thread.
#[derive(Clone, Debug)]
pub struct Worker {
    limits: Limits,
    wall: Duration,
    cancel: Option<Arc<AtomicBool>>,
    pool: Pool,
    admission: Admission,
}

impl Default for Worker {
    fn default() -> Self {
        Self::new(Limits::DEFAULT, WALL)
    }
}

impl Worker {
    pub fn new(limits: Limits, wall: Duration) -> Self {
        Self {
            limits,
            wall,
            cancel: None,
            pool: Pool::Process(&SLOTS),
            admission: Admission::Refuse,
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

    /// This worker with a cancel flag: setting it refuses the call in
    /// progress (`generator.cancelled`) and every later one until cleared.
    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// This worker for batch work: a call made while all workers are busy
    /// waits for one until `deadline` (or until cancelled) instead of being
    /// refused at once; past the deadline it is refused as busy. Each
    /// call's wall time starts when it gets its worker.
    pub fn waiting_until(mut self, deadline: Instant) -> Self {
        self.admission = Admission::WaitUntil(deadline);
        self
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    pub fn wall(&self) -> Duration {
        self.wall
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
    }

    /// A permit from the pool, as this worker's admission says.
    fn admit(&self) -> Result<Permit, Failure> {
        loop {
            let busy = match self.pool.acquire() {
                Ok(permit) => return Ok(permit),
                Err(busy) => busy,
            };
            let Admission::WaitUntil(deadline) = self.admission else {
                return Err(busy);
            };
            if self.cancelled() {
                return Err(cancelled());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Failure::one(
                    "generator.busy",
                    format!(
                        "All {MAX_ACTIVE} generator workers stayed busy until this work's deadline. It was not run."
                    ),
                ));
            }
            std::thread::sleep(POLL.min(left));
        }
    }

    /// Reserve a worker before preparing owned inputs, then run the job
    /// within the caller's wall time, watching the cancel flag. The thread
    /// owns the permit; abandoning the receiver cannot release it early.
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
        if self.cancelled() {
            return Err(cancelled());
        }
        if self.wall.is_zero() {
            return Err(timed_out(self.wall));
        }
        let permit = self.admit()?;
        let started = Instant::now();
        let job = prepare();
        if self.cancelled() {
            return Err(cancelled());
        }
        if started.elapsed() >= self.wall {
            return Err(timed_out(self.wall));
        }
        let (send, receive) = mpsc::sync_channel(1);
        let limits = self.limits;
        spawn(Box::new(move || {
            let _permit = permit;
            let runtime = WasmiRuntime::new(limits);
            // The receiver may have given up; then the result is dropped.
            let _ = send.send(job(&runtime));
        }))
        .map_err(|e| {
            Failure::one(
                "generator.worker",
                format!("The generator could not be started ({e}). It was not run."),
            )
        })?;
        loop {
            if self.cancelled() {
                return Err(cancelled());
            }
            let remaining = self.wall.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(timed_out(self.wall));
            }
            match receive.recv_timeout(POLL.min(remaining)) {
                Ok(result) => return result,
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

fn timed_out(wall: Duration) -> Failure {
    Failure::one(
        "generator.timed_out",
        format!(
            "The generator did not finish within {} ms. Nothing was changed.",
            wall.as_millis()
        ),
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

    fn isolated(slots: &'static Slots, wall: Duration) -> Worker {
        Worker {
            pool: Pool::Process(slots),
            ..Worker::new(Limits::DEFAULT, wall)
        }
    }

    fn idle(slots: &Slots) -> Result<(), String> {
        // A watchdog, not a latency/performance assertion. The tests
        // release their blocked jobs explicitly before waiting here.
        let started = Instant::now();
        while slots.0.load(Ordering::Acquire) != 0 {
            if started.elapsed() > Duration::from_secs(10) {
                return Err("a finished worker did not release its permit".into());
            }
            std::thread::yield_now();
        }
        Ok(())
    }

    #[test]
    fn a_set_cancel_flag_refuses_before_anything_runs() {
        let flag = Arc::new(AtomicBool::new(true));
        let worker = Worker::default().with_cancel(flag);
        let failure = worker.describe(b"not a module").err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some("generator.cancelled")
        );
    }

    #[test]
    fn refusals_of_the_runtime_come_back_from_the_worker() {
        let failure = Worker::fixture().describe(b"not a module").err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some("generator.invalid_module")
        );
    }

    #[test]
    fn a_call_past_its_wall_time_is_refused() {
        let worker = Worker::new(Limits::DEFAULT, Duration::ZERO);
        let failure = worker.describe(b"not a module").err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some("generator.timed_out")
        );
    }

    fn capped(worker: Worker, contender: Worker) -> Result<(), String> {
        let (started, starts) = mpsc::channel();
        let mut releases = Vec::new();
        let mut callers = Vec::new();
        for _ in 0..MAX_ACTIVE {
            let worker = worker.clone();
            let started = started.clone();
            let (release, wait) = mpsc::channel();
            releases.push(release);
            callers.push(std::thread::spawn(move || {
                worker.run(|| {
                    move |_| {
                        let _ = started.send(());
                        let _ = wait.recv();
                        Ok(())
                    }
                })
            }));
        }
        for _ in 0..MAX_ACTIVE {
            starts
                .recv_timeout(Duration::from_secs(10))
                .map_err(|e| e.to_string())?;
        }
        let prepared = AtomicBool::new(false);
        // A fresh Worker sharing this process pool is capped too.
        let refused = contender.run(|| {
            prepared.store(true, Ordering::Relaxed);
            |_| Ok(())
        });
        assert_eq!(
            refused.err().as_ref().and_then(Failure::code),
            Some("generator.busy")
        );
        assert!(!prepared.load(Ordering::Relaxed));
        // A separately injected fixture owns an independent bounded pool.
        assert!(Worker::fixture().run(|| |_| Ok(())).is_ok());
        for release in releases {
            let _ = release.send(());
        }
        for caller in callers {
            caller
                .join()
                .map_err(|_| "caller panicked")?
                .map_err(|e| e.to_string())?;
        }
        idle(worker.pool.slots())?;
        assert!(worker.run(|| |_| Ok(())).is_ok());
        idle(worker.pool.slots())
    }

    #[test]
    fn production_workers_share_the_process_cap_before_preparing_another() -> Result<(), String> {
        capped(
            Worker::new(Limits::DEFAULT, Duration::from_secs(30)),
            Worker::new(Limits::DEFAULT, Duration::from_secs(30)),
        )
    }

    #[test]
    fn fixture_clones_share_their_own_four_worker_cap() -> Result<(), String> {
        let worker = Worker::fixture();
        capped(worker.clone(), worker)
    }

    /// Batch admission: with every worker of the pool held, a waiting call
    /// runs once one is released, a cancelled wait stops at once, and a
    /// wait past its deadline is refused as busy. Nothing prepares early.
    #[test]
    fn a_waiting_call_runs_when_a_worker_frees_and_stops_at_cancel_or_deadline()
    -> Result<(), String> {
        let worker = Worker::fixture();
        let mut held: Vec<_> = (0..MAX_ACTIVE)
            .map(|_| worker.pool.acquire())
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let busy = worker.run(|| |_| Ok(()));
        assert_eq!(
            busy.err().as_ref().and_then(Failure::code),
            Some("generator.busy")
        );

        let soon = Instant::now() + Duration::from_millis(50);
        let prepared = AtomicBool::new(false);
        let expired = worker.clone().waiting_until(soon).run(|| {
            prepared.store(true, Ordering::Relaxed);
            |_| Ok(())
        });
        assert_eq!(
            expired.err().as_ref().and_then(Failure::code),
            Some("generator.busy")
        );
        assert!(!prepared.load(Ordering::Relaxed));
        assert!(Instant::now() >= soon);

        let flag = Arc::new(AtomicBool::new(false));
        let later = Instant::now() + Duration::from_secs(60);
        let cancelling = worker
            .clone()
            .waiting_until(later)
            .with_cancel(flag.clone());
        let caller = std::thread::spawn(move || cancelling.run(|| |_| Ok(())));
        flag.store(true, Ordering::Relaxed);
        let cancelled = caller.join().map_err(|_| "caller panicked")?;
        assert_eq!(
            cancelled.err().as_ref().and_then(Failure::code),
            Some("generator.cancelled")
        );

        let (waiting, waits) = mpsc::channel();
        let patient = worker.clone().waiting_until(later);
        let caller = std::thread::spawn(move || {
            let _ = waiting.send(());
            patient.run(|| |_| Ok(7))
        });
        waits
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        // Let the caller find the pool full before one worker frees.
        std::thread::sleep(Duration::from_millis(30));
        held.pop();
        let ran = caller.join().map_err(|_| "caller panicked")?;
        assert_eq!(ran.map_err(|e| e.to_string())?, 7);
        drop(held);
        idle(worker.pool.slots())
    }

    struct Dropped {
        slots: &'static Slots,
        observed: mpsc::Sender<usize>,
    }

    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.observed.send(self.slots.0.load(Ordering::Acquire));
        }
    }

    fn abandoned_keeps_permit(slots: &'static Slots, cancel: bool) -> Result<(), String> {
        let reserved: Vec<_> = (1..MAX_ACTIVE)
            .map(|_| Pool::Process(slots).acquire())
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let flag = Arc::new(AtomicBool::new(false));
        let wall = if cancel {
            Duration::from_secs(30)
        } else {
            Duration::from_millis(20)
        };
        let worker = isolated(slots, wall).with_cancel(flag.clone());
        let (started, starts) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let (observed, drops) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            worker.run(|| {
                move |_| {
                    let _ = started.send(());
                    let _ = wait.recv();
                    Ok(Dropped { slots, observed })
                }
            })
        });
        starts
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?;
        if cancel {
            flag.store(true, Ordering::Relaxed);
        }
        let failure = caller.join().map_err(|_| "caller panicked")?.err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some(if cancel {
                "generator.cancelled"
            } else {
                "generator.timed_out"
            })
        );
        let another = isolated(slots, Duration::from_secs(30));
        for _ in 0..10 {
            assert_eq!(
                another
                    .run(|| |_| Ok(()))
                    .err()
                    .as_ref()
                    .and_then(Failure::code),
                Some("generator.busy")
            );
        }
        let _ = release.send(());
        // Even destruction of the abandoned result runs while its
        // permit is held; dropping the receiver released nothing early.
        assert_eq!(
            drops
                .recv_timeout(Duration::from_secs(10))
                .map_err(|e| e.to_string())?,
            MAX_ACTIVE
        );
        drop(reserved);
        idle(slots)?;
        assert!(another.run(|| |_| Ok(())).is_ok());
        idle(slots)
    }

    #[test]
    fn timed_out_work_keeps_its_permit_through_result_cleanup() -> Result<(), String> {
        static SLOTS: Slots = Slots(AtomicUsize::new(0));
        abandoned_keeps_permit(&SLOTS, false)
    }

    #[test]
    fn cancelled_work_keeps_its_permit_through_result_cleanup() -> Result<(), String> {
        static SLOTS: Slots = Slots(AtomicUsize::new(0));
        abandoned_keeps_permit(&SLOTS, true)
    }

    #[test]
    fn cancellation_during_preparation_releases_the_unstarted_permit() -> Result<(), String> {
        static SLOTS: Slots = Slots(AtomicUsize::new(0));
        let flag = Arc::new(AtomicBool::new(false));
        let worker = isolated(&SLOTS, Duration::from_secs(30)).with_cancel(flag.clone());
        let ran = Arc::new(AtomicBool::new(false));
        let in_job = ran.clone();
        let failure = worker
            .run(|| {
                flag.store(true, Ordering::Relaxed);
                move |_| {
                    in_job.store(true, Ordering::Relaxed);
                    Ok(())
                }
            })
            .err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some("generator.cancelled")
        );
        assert!(!ran.load(Ordering::Relaxed));
        idle(&SLOTS)
    }

    #[test]
    fn a_failed_thread_spawn_releases_its_captured_permit() -> Result<(), String> {
        static SLOTS: Slots = Slots(AtomicUsize::new(0));
        let worker = isolated(&SLOTS, Duration::from_secs(30));
        let ran = Arc::new(AtomicBool::new(false));
        let in_job = ran.clone();
        let failure = worker
            .run_with(
                || {
                    move |_| {
                        in_job.store(true, Ordering::Relaxed);
                        Ok(())
                    }
                },
                |task| {
                    drop(task);
                    Err(std::io::Error::other("scripted thread creation failure"))
                },
            )
            .err();
        assert_eq!(
            failure.as_ref().and_then(Failure::code),
            Some("generator.worker")
        );
        assert!(!ran.load(Ordering::Relaxed));
        idle(&SLOTS)?;
        assert!(worker.run(|| |_| Ok(())).is_ok());
        idle(&SLOTS)
    }
}
