//! A command that needs a generator waits off the window's thread, and
//! Stop ends the wait.
use super::ax_harness::{Ax, Fallible};
use super::*;
use crate::generators::{self, MakeRequest};
use sparksmith_automation::session::generators::Deferred;
use sparksmith_core::library::generated::GeneratorScope;

/// The window on its own runtime, and a command that makes a part, not yet
/// made there.
fn window() -> Fallible<(Ax, Operation)> {
    let mut ax = Ax::new(Workspace::Schematic)?;
    let request = MakeRequest {
        generator: "chip".into(),
        preset: Some("resistor-0603".into()),
        params: Default::default(),
        name: None,
        values: Vec::new(),
    };
    let into = generators::Into::Part(GeneratorScope::Component);
    let operation = generators::command(&ax.app.engine, &request, into)?;
    let runtime = ax.app.generators.runtime.clone();
    ax.app.engine.set_generators(runtime);
    Ok((ax, operation))
}

#[test]
fn a_command_waits_for_its_generator_and_then_runs() -> Fallible {
    let (mut ax, operation) = window()?;
    let ctx = ax.ctx.clone();
    let before = ax.app.engine.revision();
    ax.app.apply(&ctx, Origin::Command, Some(operation));
    assert_eq!(ax.app.engine.revision(), before);
    assert_eq!(ax.app.stoppable(), Some("Waiting for a generator\u{2026}"));
    // A call may meet a second generator call: it waits again.
    while ax.app.generators.is_waiting() {
        ax.app.serve_generator_wait(&ctx);
        std::thread::yield_now();
    }
    assert_eq!(ax.app.engine.revision(), before + 1);
    assert_eq!(ax.app.stoppable(), None);
    Ok(())
}

#[test]
fn stop_ends_the_wait_and_the_next_command_runs() -> Fallible {
    let (mut ax, operation) = window()?;
    let ctx = ax.ctx.clone();
    let document = ax.app.engine.document().clone();
    let before = ax.app.engine.revision();
    ax.app.apply(&ctx, Origin::Command, Some(operation.clone()));
    ax.app.stop_work(&ctx);
    assert!(!ax.app.generators.is_waiting());
    assert_eq!(Deferred::waiting(&ax.app.generators.runtime), 0);
    assert_eq!(ax.app.engine.revision(), before);
    assert_eq!(*ax.app.engine.document(), document);
    ax.app.apply(&ctx, Origin::Command, Some(operation));
    while ax.app.generators.is_waiting() {
        ax.app.serve_generator_wait(&ctx);
        std::thread::yield_now();
    }
    assert_eq!(ax.app.engine.revision(), before + 1);
    Ok(())
}
