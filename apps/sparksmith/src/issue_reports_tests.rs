//! The one report of every issue: made once per state and check settings,
//! on the worker, never applied to a state it does not describe.
use super::*;
use sparksmith_core::Operation;
use sparksmith_core::layout::CheckSettings;
use sparksmith_core::layout::LayoutCommand;
use std::sync::Arc;
use std::time::Duration;

type Fallible = Result<(), Box<dyn std::error::Error>>;

const PATIENCE: Duration = Duration::from_secs(30);

fn tent_first_via(engine: &mut Engine, tented: bool) -> Fallible {
    let via = engine.project().vias.first().ok_or("a via")?.id;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetViaTented { id: via, tented }),
    )?;
    Ok(())
}

/// Asked again for one state, no second report is made; a new revision
/// and a check settings change (no revision) each make one.
#[test]
fn one_report_per_state_and_settings() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let mut reports = Reports::default();
    let first = reports.latest(&engine).ok_or("a report")?;
    assert!(first.report().is_some());
    reports.latest(&engine);
    reports.latest(&engine);
    assert_eq!(reports.computed, 1);
    let tented = engine.project().vias.first().ok_or("a via")?.tented;
    tent_first_via(&mut engine, !tented)?;
    reports.latest(&engine);
    assert_eq!(reports.computed, 2, "a new revision makes a new report");
    let revision = engine.revision();
    engine.set_check_settings(CheckSettings {
        impedance_tolerance: Some(7.0),
    })?;
    assert_eq!(engine.revision(), revision, "settings take no revision");
    assert!(
        !reports.is_current(&engine),
        "a settings change is a new state"
    );
    reports.latest(&engine);
    assert_eq!(reports.computed, 3, "a settings change makes a new report");
    Ok(())
}

/// The worker's report equals the query on the UI thread; until it
/// arrives the last report stays shown; a result for a state that moved on
/// is dropped, never shown for the current one.
#[test]
fn the_worker_report_is_applied_only_to_its_own_state() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let mut reports = Reports::start(egui::Context::default());
    assert!(reports.wait(&engine, PATIENCE), "the first report arrives");
    let expected = engine.query(sparksmith_core::Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    })?;
    let sparksmith_core::QueryResult::Issues(expected) = expected else {
        return Err("not an issue report".into());
    };
    let first = reports.latest(&engine).ok_or("a report")?;
    assert_eq!(first.report().map(|r| r.as_ref()), Some(&expected));

    // A new state: the last report is still shown while the next is made.
    let tented = engine.project().vias.first().ok_or("a via")?.tented;
    tent_first_via(&mut engine, !tented)?;
    let shown = reports.latest(&engine).ok_or("the last report stays")?;
    assert!(matches!(
        (&shown, &first),
        (Answer::Report(a), Answer::Report(b)) if Arc::ptr_eq(a, b)
    ));
    assert!(!reports.is_current(&engine));

    // Move on again before receiving: the report for the middle state,
    // finished or not, is never shown for the newest state.
    let middle = engine.report_key();
    tent_first_via(&mut engine, tented)?;
    assert!(reports.wait(&engine, PATIENCE), "the newest report arrives");
    assert_ne!(engine.report_key(), middle);
    let newest = reports.latest(&engine).ok_or("a report")?;
    let sparksmith_core::QueryResult::Issues(expected) =
        engine.query(sparksmith_core::Query::Issues {
            scope: sparksmith_core::issues::IssueScope::All,
        })?
    else {
        return Err("not an issue report".into());
    };
    assert_eq!(newest.report().map(|r| r.as_ref()), Some(&expected));
    Ok(())
}

/// Keep the previous counts readable and explicitly stale for the whole worker interval.
#[test]
fn visible_counts_mark_the_previous_revision_until_its_replacement_arrives() -> Fallible {
    use crate::{issues::Surface, shell::Workspace};
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let reports = SharedReports::default();
    let mut issues = Surface::with_reports(reports.clone());
    let first = issues.summary(&engine, Workspace::Pcb);
    assert!(first.counts().is_some() && !first.checking_changes());
    let (updates, done) = mpsc::channel();
    let (jobs, _requests) = mpsc::channel();
    reports.0.borrow_mut().worker = Some(Worker { jobs, done });
    let tented = engine.project().vias.first().ok_or("a via")?.tented;
    tent_first_via(&mut engine, !tented)?;
    let pending = issues.summary(&engine, Workspace::Pcb);
    assert_eq!(pending.counts(), first.counts());
    assert!(pending.checking_changes());
    assert!(issues.ribbon_toggle(&engine, Workspace::Pcb).checking);
    assert!(!issues.has_current_report(&engine));
    updates.send((engine.report_key(), compute(&engine)))?;
    assert!(!issues.summary(&engine, Workspace::Pcb).checking_changes());
    assert!(issues.has_current_report(&engine));
    Ok(())
}

/// A result the worker sends for an older state is dropped on receipt.
#[test]
fn a_stale_result_is_dropped() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let old = engine.report_key();
    let stale = Answer::Refused("stale".into());
    let (outbox, done) = mpsc::channel();
    let (jobs, _inbox) = mpsc::channel();
    let mut reports = Reports {
        worker: Some(Worker { jobs, done }),
        ..Reports::default()
    };
    let tented = engine.project().vias.first().ok_or("a via")?.tented;
    tent_first_via(&mut engine, !tented)?;
    outbox.send((
        old,
        Computed {
            issues: stale,
            ties: Default::default(),
        },
    ))?;
    assert!(
        reports.latest(&engine).is_none(),
        "the stale result is not shown"
    );
    assert!(!reports.is_current(&engine));
    // The current state's result is shown.
    outbox.send((
        engine.report_key(),
        Computed {
            issues: Answer::Refused("current".into()),
            ties: Default::default(),
        },
    ))?;
    let shown = reports.latest(&engine).ok_or("the current result")?;
    assert!(matches!(shown, Answer::Refused(ref why) if why == "current"));
    Ok(())
}

/// `current` answers for the current state at once, also with a worker,
/// and the views then show that report without asking the worker for it.
#[test]
fn current_checks_the_current_state_at_once() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let mut reports = Reports::start(egui::Context::default());
    let tented = engine.project().vias.first().ok_or("a via")?.tented;
    tent_first_via(&mut engine, !tented)?;
    let answer = reports.current(&engine);
    assert!(reports.is_current(&engine));
    let sparksmith_core::QueryResult::Issues(expected) =
        engine.query(sparksmith_core::Query::Issues {
            scope: sparksmith_core::issues::IssueScope::All,
        })?
    else {
        return Err("not an issue report".into());
    };
    assert_eq!(answer.report().map(|r| r.as_ref()), Some(&expected));
    let shown = reports.latest(&engine).ok_or("shown")?;
    assert!(matches!(
        (&shown, &answer),
        (Answer::Report(a), Answer::Report(b)) if Arc::ptr_eq(a, b)
    ));
    Ok(())
}

/// While the report of a project just opened is made, nothing stands in
/// for it: the last project's rows name its own objects and findings.
#[test]
fn another_projects_report_is_never_shown() -> Fallible {
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let mut engine = Engine::showcase()?;
    let mut reports = Reports::start(egui::Context::default());
    assert!(reports.wait(&engine, PATIENCE));
    engine.replace_document(Engine::demo()?.to_file())?;
    assert!(reports.latest(&engine).is_none());
    assert!(reports.wait(&engine, PATIENCE));
    Ok(())
}
