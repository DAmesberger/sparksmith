//! The production snapshot: the one design state a fabrication job is
//! checked and written from.
//!
//! An export may name a fab profile other than the board's own. The
//! snapshot resolves it once, or not at all: the project with that profile
//! in place of its own, the overrides and net classes kept, its colour
//! option following the board's solder mask colour and its copper weight
//! the one that orders the board's outer foils; the stackup, the active
//! variant, the waivers and the computed fills as the project holds them.
//! The choices are carried and reconciled as a rules edit does it
//! ([`crate::layout::restack::reconcile_rules`]), except that the board is never changed to
//! fit the profile. A profile the design cannot take (a kept override it
//! conflicts with, no copper weight for the board's foils) gives no
//! snapshot: [`ProductionSnapshot::resolve`] refuses it
//! (`fabrication.profile_refused`), so no check can pass and no file can
//! be written under rules the board does not have, and no waiver can turn
//! the refusal into an export under the saved profile.
//!
//! The production check ([`super::preflight::check`]) and every writer
//! ([`super::generate`], [`super::drawing::data`]) read this one project, so
//! a job file, a drill file or a mask opening can never describe other rules
//! than the ones the check passed.
//!
//! Every file of a set states one release identity ([`Identity`]): the
//! project's stored identity and a content id of the files themselves
//! ([`super::generate`]).
use crate::diagnostics::{Error, Result};
use crate::model::Project;
use crate::rules::{self, ProfileChoice, RuleProfile, RulesCommand};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// One resolved fabrication state.
#[derive(Clone, Debug)]
pub struct ProductionSnapshot<'a> {
    project: Cow<'a, Project>,
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
    /// when `profile` is `None` or already the board's. Refused
    /// (`fabrication.profile_refused`) when the design cannot take the
    /// profile: a kept override conflicts with it, or none of its copper
    /// weights orders the board's outer foils; and when the board's own
    /// profile orders other foils than its stackup has.
    pub fn resolve(project: &'a Project, profile: Option<&RuleProfile>) -> Result<Self> {
        let Some(p) = profile.filter(|p| project.rules.profile.as_ref() != Some(*p)) else {
            crate::layout::restack::check_outer_copper(project, &project.stackup)
                .map_err(|e| refused(project.rules.profile_name().as_str(), &e.to_string()))?;
            return Ok(Self {
                project: Cow::Borrowed(project),
            });
        };
        let mut candidate = project.clone();
        let options = foil_choice(project, p).map_err(|reason| refused(&p.name, &reason))?;
        rules::apply(
            &mut candidate.rules,
            RulesCommand::SetProfile {
                profile: ProfileChoice::Profile(Box::new(p.clone())),
                options,
                keep_overrides: true,
            },
        )
        .and_then(|_| {
            crate::layout::restack::reconcile_rules(
                &mut candidate,
                crate::layout::restack::Foils::Keep,
            )
        })
        .map_err(|e| refused(&p.name, &e.to_string()))?;
        Ok(Self {
            project: Cow::Owned(candidate),
        })
    }

    /// The project every check and writer reads.
    pub fn project(&self) -> &Project {
        &self.project
    }
}

/// The refusal of a profile the design cannot take.
fn refused(profile: &str, reason: &str) -> Error {
    Error::Conflict(format!(
        "fabrication.profile_refused: the design cannot be made under {profile}: {reason}"
    ))
}

/// The choice of `profile`'s copper weight option that orders the board's
/// outer foils (none to choose when it has no such option); `Err` names
/// what it offers when no choice does.
fn foil_choice(
    project: &Project,
    profile: &RuleProfile,
) -> std::result::Result<BTreeMap<String, String>, String> {
    let (Some(option), Some((top, bottom))) =
        (profile.copper_option(), project.stackup.outer_copper_um())
    else {
        return Ok(BTreeMap::new());
    };
    let matching = option.choices.iter().find(|c| {
        c.outer_copper_um
            .is_some_and(|um| project.stackup.outer_copper_is(um))
    });
    if let Some(choice) = matching {
        return Ok(BTreeMap::from([(option.id.clone(), choice.id.clone())]));
    }
    let offered: Vec<String> = option
        .choices
        .iter()
        .filter_map(|c| c.outer_copper_um)
        .map(|um| format!("{um} µm"))
        .collect();
    let board = if (top - bottom).abs() < 1e-6 {
        format!("the board's outer copper is {top} µm")
    } else {
        format!("the board's top and bottom copper are {top} and {bottom} µm")
    };
    Err(format!(
        "it offers {} outer copper and {board}; export under a profile that offers it, or \
         choose another {} in Design rules first",
        offered.join(" or "),
        option.name.to_lowercase()
    ))
}
