//! The production snapshot: the one design state a fabrication job is
//! checked and written from.
//!
//! An export may name a fab profile other than the board's own. The
//! snapshot resolves it once: the project with that profile in place of its
//! own, its manufacturing choices following the board (the solder mask
//! colour), the overrides and net classes kept; the active variant, the
//! waivers, the stackup and the computed fills as the project holds them.
//! The production check ([`super::preflight::check`]) and every writer
//! ([`super::generate`], [`super::drawing::data`]) read this one project, so
//! a job file, a drill file or a mask opening can never describe other rules
//! than the ones the check passed.
//!
//! Every file of a set states one release identity ([`Identity`]): the
//! project's stored identity and a content id of the files themselves
//! ([`super::generate`]).
use super::preflight::{self, PreflightItem};
use crate::issues::Severity;
use crate::model::Project;
use crate::rules::{self, RuleProfile, RulesCommand};
use std::borrow::Cow;

/// One resolved fabrication state.
#[derive(Clone, Debug)]
pub struct ProductionSnapshot<'a> {
    project: Cow<'a, Project>,
    /// Findings of the resolution itself: a profile the design cannot take.
    findings: Vec<PreflightItem>,
}

/// Who a release is and which one, as every file of it states (Gerber
/// `.ProjectId`, the drill files' header, the job file's `ProjectId`, both
/// drawings' title blocks): the project's name, its identity and the
/// release's content id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    /// RFC 4122-shaped: the project's stored identity
    /// (`Production::project_id`), else (a project never saved) the
    /// release's content digest in that shape.
    pub guid: String,
    /// The release: the first 16 hex digits of the SHA-256 of every file
    /// of the set as written without an identity (the creation date left
    /// out). Two exports that write the same files carry the same
    /// revision; any change to what a file says changes it.
    pub revision: String,
}

impl<'a> ProductionSnapshot<'a> {
    /// `project` checked and written under `profile`: the project itself
    /// when `profile` is `None` or already the board's. A profile the
    /// design cannot take (an override conflicts with it) leaves the
    /// project as it is, with a `fabrication.profile_refused` error that
    /// blocks export.
    pub fn resolve(project: &'a Project, profile: Option<&RuleProfile>) -> Self {
        let Some(p) = profile.filter(|p| project.rules.profile.as_ref() != Some(*p)) else {
            return Self {
                project: Cow::Borrowed(project),
                findings: Vec::new(),
            };
        };
        let mut candidate = project.clone();
        let applied = rules::apply(
            &mut candidate.rules,
            RulesCommand::set_profile(p.clone(), true),
        )
        .and_then(|_| {
            crate::layout::mask_colour::follow(
                &mut candidate.rules,
                candidate.fabrication.mask_colour,
            )
        });
        match applied {
            Ok(_) => Self {
                project: Cow::Owned(candidate),
                findings: Vec::new(),
            },
            Err(e) => Self {
                project: Cow::Borrowed(project),
                findings: vec![preflight::item(
                    "fabrication.profile_refused",
                    Severity::Error,
                    format!("The design cannot use {}: {}", p.name, e),
                    Some("Clear the overrides that conflict with the profile".into()),
                    Vec::new(),
                )],
            },
        }
    }

    /// The project every check and writer reads.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Findings of the resolution (a refused profile).
    pub fn findings(&self) -> &[PreflightItem] {
        &self.findings
    }
}
