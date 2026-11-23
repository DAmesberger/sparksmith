//! Which saved fab profiles make the board: computed, never written.
//!
//! A finding whose limit is the fab profile's own rule records what the
//! board measured ([`crate::diagnostics::Requirement`]). The board needs a
//! profile that allows every such measurement. [`suggest`] states what is
//! needed under the profile the board is checked under (the tightest
//! requirement of each rule, with the profile's limit and its source), then
//! checks the board under every installed profile the host lists, exactly
//! as an export would ([`ProductionSnapshot::resolve`], then
//! [`check`]): a profile qualifies when no profile-limited finding is left
//! and the design can take it. The ones that qualify are ordered by the
//! fabricator hierarchy: profiles of the board's own fabricator first, then
//! other fabricators', then profiles that name none, each by name. When
//! none qualifies the answer is only what is needed.
//!
//! Nothing here names a fabricator: the profiles and fabricator records are
//! data the host reads (`fab-profiles/`, `fabricators/`).
use super::preflight::check;
use super::snapshot::ProductionSnapshot;
use crate::diagnostics::Result;
use crate::layout::CheckSettings;
use crate::library::Drafts;
use crate::model::Project;
use crate::rules::fabricator::name_of;
use crate::rules::requirement::{Needed, describe, tightest};
use crate::rules::{Fabricator, RuleProfile};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Profiles [`suggest`] checks the board under, at most.
pub const MAX_CANDIDATES: usize = 512;

/// A profile under which every profile-limited finding clears.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Suggestion {
    /// The profile's id, as `rules.set_fab_profile` takes it.
    pub id: String,
    pub name: String,
    /// Its fabricator's record id, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabricator: Option<String>,
    /// Its fabricator's shown name (the record's, else the id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabricator_name: Option<String>,
    /// Whether it is a profile of the board's own fabricator.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub same_fabricator: bool,
}

/// `fabrication.profile_suggestions`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct ProfileSuggestions {
    /// The profile the board was checked under, by name.
    pub profile: String,
    /// Its fabricator's record id, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fabricator: Option<String>,
    /// What the board needs that the profile does not allow: the tightest
    /// measurement of each rule, with the profile's limit and source.
    /// Empty when no finding is profile-limited.
    pub needs: Vec<Needed>,
    /// The installed profiles under which every profile-limited finding
    /// clears: the board's own fabricator's first, then other
    /// fabricators', then those that name none. Empty when none does.
    pub suggestions: Vec<Suggestion>,
}

/// Whether `project` checked under `profile` leaves no profile-limited
/// finding: `false` when the design cannot take the profile.
fn clears(
    project: &Project,
    drafts: &Drafts,
    settings: &CheckSettings,
    profile: &RuleProfile,
) -> bool {
    let Ok(snapshot) = ProductionSnapshot::resolve(project, Some(profile)) else {
        return false;
    };
    check(&snapshot, drafts, settings).is_ok_and(|r| r.items.iter().all(|i| i.needs.is_none()))
}

/// What `project` needs of a fab profile under `profile` (the board's own
/// when `None`), and which of `candidates` (the host's installed profiles,
/// user and shipped) allow all of it, ordered by the fabricator hierarchy;
/// `fabricators` name the fabricators. A profile the design cannot take
/// is refused as for an export (`fabrication.profile_refused`).
pub fn suggest(
    project: &Project,
    drafts: &Drafts,
    settings: &CheckSettings,
    profile: Option<&RuleProfile>,
    candidates: &[RuleProfile],
    fabricators: &[Fabricator],
) -> Result<ProfileSuggestions> {
    let snapshot = ProductionSnapshot::resolve(project, profile)?;
    let current = snapshot.project().rules.profile.as_ref();
    let report = check(&snapshot, drafts, settings)?;
    let requirements = tightest(
        report
            .items
            .iter()
            .filter_map(|i| i.needs.as_ref().map(|n| &n.requirement)),
    );
    let needs = requirements
        .iter()
        .filter_map(|r| describe(r, current))
        .collect();
    let own = current.and_then(|p| p.fabricator.clone());
    let mut out = ProfileSuggestions {
        profile: snapshot.project().rules.profile_name(),
        fabricator: own.clone(),
        needs,
        suggestions: Vec::new(),
    };
    if requirements.is_empty() {
        return Ok(out);
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut fitting: Vec<(u8, &RuleProfile)> = candidates
        .iter()
        .take(MAX_CANDIDATES)
        .filter(|c| seen.insert(c.id.as_str()))
        .filter(|c| current != Some(*c))
        .filter(|c| clears(project, drafts, settings, c))
        .map(|c| {
            let rank = match (&c.fabricator, &own) {
                (Some(f), Some(o)) if f == o => 0,
                (Some(_), _) => 1,
                (None, _) => 2,
            };
            (rank, c)
        })
        .collect();
    fitting.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    out.suggestions = fitting
        .into_iter()
        .map(|(rank, c)| Suggestion {
            id: c.id.clone(),
            name: c.name.clone(),
            fabricator: c.fabricator.clone(),
            fabricator_name: c
                .fabricator
                .as_deref()
                .map(|f| name_of(fabricators, f).to_owned()),
            same_fabricator: rank == 0,
        })
        .collect();
    Ok(out)
}
