//! Workflows: the contract ([`sparksmith_automation::workflow`]: the
//! format, its load check, the controls, the `assist.*` entries and the
//! run's events) and the one generic runner ([`runner`]) that drives a
//! workflow's steps on a session, its model steps on any backend.
pub use sparksmith_automation::workflow::*;

pub mod runner;
