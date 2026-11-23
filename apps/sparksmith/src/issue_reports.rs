//! The one report of every issue (`project.issues` over every area), made
//! off the UI thread. It is keyed by the document state and the host's
//! check settings ([`Engine::report_key`]): a settings change (the
//! impedance tolerance) asks for a new report though it takes no revision.
//! Every view (the Issues bar and card, the Browser's dots, the PCB
//! inspector's issue slot) projects its scope from this one report
//! ([`IssueReport::scoped`]), so a selection change runs no check.
//!
//! The worker gets a fork of the engine (shared handles, no copy) and
//! answers with the key it was computed for. A result for a key that is no
//! longer current is dropped; the current key is already queued. Until the
//! current report arrives the views show the last one (`None` before the
//! first). Without a worker thread (tests, or when the thread cannot
//! start) the report is made on the calling thread, still once per key.
use crate::issues::Answer;
use sparksmith_core::{Engine, ReportKey};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};

use sparksmith_core::layout::connectivity::TieReport;
use std::sync::Arc;
struct Computed {
    issues: Answer,
    ties: Arc<[TieReport]>,
}

struct Worker {
    jobs: Sender<(ReportKey, Engine)>,
    done: Receiver<(ReportKey, Computed)>,
}

/// The report of every area and where the next one comes from.
#[derive(Default)]
pub struct Reports {
    /// The newest report applied, and the state it describes.
    shown: Option<(ReportKey, Computed)>,
    /// The key last sent to the worker.
    asked: Option<ReportKey>,
    worker: Option<Worker>,
    /// Reports computed (on the worker or here), for tests and benchmarks.
    #[cfg(test)]
    pub(crate) computed: usize,
}

/// The engine's report of every area ([`Engine::issue_report`]): made
/// from the last report it or a fork of it made, so a state that changed
/// only tracks and vias checks only what reads them.
fn compute(engine: &Engine) -> Computed {
    let issues = match engine.issue_report() {
        Ok(report) => Answer::Report(report),
        Err(error) => Answer::Refused(crate::refusal::shown_error(&error)),
    };
    let ties = if engine
        .project()
        .instances
        .iter()
        .any(|i| !i.ties.is_empty())
    {
        let _context = engine.enter_context();
        sparksmith_core::layout::connectivity::report(engine.project(), &engine.copper_graph())
    } else {
        vec![]
    };
    Computed {
        issues,
        ties: ties.into(),
    }
}

impl Reports {
    /// Reports made on a worker thread, which asks `ctx` to repaint when
    /// one is ready.
    pub fn start(ctx: egui::Context) -> Self {
        let (jobs, inbox) = mpsc::channel::<(ReportKey, Engine)>();
        let (outbox, done) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("sparksmith-issues".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                while let Ok(first) = inbox.recv() {
                    // Only the newest state matters.
                    let (key, engine) = inbox.try_iter().last().unwrap_or(first);
                    if outbox.send((key, compute(&engine))).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }));
        Self {
            worker: spawned.ok().map(|_| Worker { jobs, done }),
            ..Self::default()
        }
    }

    /// The newest report of every area: `engine`'s current one once it is
    /// ready, else the last one while the current one is made (`None`
    /// before the first). Asks for the current one when it has not yet.
    pub fn latest(&mut self, engine: &Engine) -> Option<Answer> {
        let key = engine.report_key();
        self.receive(&key);
        if self.shown.as_ref().is_some_and(|(at, _)| *at == key) {
            return self.shown.as_ref().map(|(_, answer)| answer.issues.clone());
        }
        match &self.worker {
            Some(worker) => {
                if self.asked != Some(key) {
                    if worker.jobs.send((key, engine.fork())).is_ok() {
                        self.asked = Some(key);
                    } else {
                        // The thread is gone: make reports here.
                        self.worker = None;
                        self.make(engine, key);
                    }
                }
            }
            None => self.make(engine, key),
        }
        self.shown.as_ref().map(|(_, answer)| answer.issues.clone())
    }

    /// `engine`'s current report, made here on the calling thread when it
    /// has not arrived: for a step that reports what it changed in the
    /// issues (a placement's toast), never for drawing. The views use it
    /// too, so no worker makes it again.
    pub fn current(&mut self, engine: &Engine) -> Answer {
        let key = engine.report_key();
        self.receive(&key);
        match &self.shown {
            Some((at, answer)) if *at == key => answer.issues.clone(),
            _ => {
                self.make(engine, key);
                self.asked = Some(key);
                self.shown
                    .as_ref()
                    .map_or(Answer::Pending, |(_, answer)| answer.issues.clone())
            }
        }
    }

    /// Whether the report shown describes `engine`'s current state.
    #[cfg(test)]
    pub fn is_current(&self, engine: &Engine) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|(at, _)| *at == engine.report_key())
    }

    fn make(&mut self, engine: &Engine, key: ReportKey) {
        self.shown = Some((key, compute(engine)));
        #[cfg(test)]
        {
            self.computed += 1;
        }
    }

    /// Take the worker's finished reports: the one for `current` is shown,
    /// any other is for a state that has moved on and is dropped.
    fn receive(&mut self, current: &ReportKey) {
        let Some(worker) = &self.worker else {
            return;
        };
        while let Ok((key, answer)) = worker.done.try_recv() {
            #[cfg(test)]
            {
                self.computed += 1;
            }
            if key == *current {
                self.shown = Some((key, answer));
            }
        }
    }

    /// Wait up to `patience` for the current report (tests and benchmarks).
    #[cfg(test)]
    pub(crate) fn wait(&mut self, engine: &Engine, patience: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + patience;
        loop {
            self.latest(engine);
            if self.is_current(engine) {
                return true;
            }
            if std::time::Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
}

/// What a view projected from the report of every area is kept for: that
/// report (its allocation) and the document state the view names objects
/// in (revision and project allocation).
pub type ViewKey = (usize, u64, usize);

/// The [`ViewKey`] of a view of `whole` on `engine`'s document.
pub fn view_key(
    whole: &std::sync::Arc<sparksmith_core::issues::IssueReport>,
    engine: &Engine,
) -> ViewKey {
    (
        std::sync::Arc::as_ptr(whole) as usize,
        engine.revision(),
        std::sync::Arc::as_ptr(engine.project_arc()) as usize,
    )
}

/// One [`Reports`] shared by the views of one window (the UI thread only).
#[derive(Clone, Default)]
pub struct SharedReports(Rc<RefCell<Reports>>);

impl SharedReports {
    pub fn start(ctx: egui::Context) -> Self {
        Self(Rc::new(RefCell::new(Reports::start(ctx))))
    }

    /// [`Reports::current`]; the query itself while another view holds
    /// the reports, which one thread never does across a call.
    pub fn current(&self, engine: &Engine) -> Answer {
        match self.0.try_borrow_mut() {
            Ok(mut reports) => reports.current(engine),
            Err(_) => compute(engine).issues,
        }
    }

    /// Board joins and margins computed off-thread for this exact document revision.
    pub fn ties(&self, engine: &Engine) -> Option<Arc<[TieReport]>> {
        let mut reports = self.0.try_borrow_mut().ok()?;
        let _ = reports.latest(engine);
        reports
            .shown
            .as_ref()
            .filter(|(key, _)| *key == engine.report_key())
            .map(|(_, v)| v.ties.clone())
    }

    /// Wait up to `patience` for the current report (tests).
    #[cfg(test)]
    pub(crate) fn wait(&self, engine: &Engine, patience: std::time::Duration) -> bool {
        self.0
            .try_borrow_mut()
            .is_ok_and(|mut reports| reports.wait(engine, patience))
    }

    /// Reports computed so far (tests).
    #[cfg(test)]
    pub(crate) fn computed(&self) -> usize {
        self.0.try_borrow().map_or(0, |r| r.computed)
    }

    /// [`Reports::latest`]; `None` too while another view holds it, which
    /// one thread never does across a call.
    pub fn latest(&self, engine: &Engine) -> Option<Answer> {
        self.0
            .try_borrow_mut()
            .ok()
            .and_then(|mut reports| reports.latest(engine))
    }
}

#[cfg(test)]
#[path = "issue_reports_tests.rs"]
mod tests;
