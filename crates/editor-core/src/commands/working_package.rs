//! A working set's package and footprint drafts taking a generated
//! package (`library.package_from_generator`, slice D1): which current
//! terminals carry over, the footprint rebuilt from the generated one, and
//! the findings the change gives the binding drafts.
use super::Outcome;
use crate::diagnostics::FindingCode;
use crate::diagnostics::{Error, Finding, ObjectRef, Result, Stage};
use crate::ids::{DefinitionId, FootprintRef, PackageRef, PackageTerminalId, PadId, Revision};
use crate::library::package_fit;
use crate::library::{
    Definition, DraftEdit, FootprintDefinition, PackageDefinition, PackageTerminal, drafts,
};
use crate::model::Document;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// One explicit renumbering of `library.package_from_generator`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Renumber {
    /// The current terminal's number.
    pub from: String,
    /// The new form's number it becomes.
    pub to: String,
}

/// How `library.package_from_generator` matches the package's current
/// terminals to the terminals of the new form.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TerminalMatch {
    /// A current terminal is kept (identity, and so its mappings) when the
    /// new form gives a terminal its number (designation); every other
    /// new terminal is fresh and unmapped, every other current terminal is
    /// removed, and both are reported. Nothing is matched by list order.
    #[default]
    ByNumber,
    /// The explicit choice to reuse the current terminals in list order
    /// whatever their numbers, renumbered from the new form; reported as
    /// `binding.mapping_retained` for review.
    ByPosition,
}

impl TerminalMatch {
    pub fn label(self) -> &'static str {
        match self {
            Self::ByNumber => "by number",
            Self::ByPosition => "by position",
        }
    }
}

/// How the package's terminals were matched (`PackagePlan::terminals`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TerminalMatchReport {
    pub strategy: TerminalMatch,
    /// Current terminals kept with their identity.
    pub kept: u32,
    /// New terminals with fresh identities (unmapped).
    pub added: u32,
    /// Current terminals the new package removes.
    pub removed: u32,
    /// Kept terminals whose number changes (only by position).
    pub renumbered: u32,
    /// One line saying the strategy and the counts.
    pub summary: String,
}

/// What `library.package_from_generator` changed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackagePlan {
    pub terminals_before: u32,
    pub terminals_after: u32,
    pub pads_after: u32,
    /// The footprint draft that lands the package: the one already in the
    /// working set, or the one the command creates.
    #[serde(default)]
    pub footprint: Option<DefinitionId>,
    /// No footprint draft landed the package, so the command creates one.
    #[serde(default)]
    pub footprint_created: bool,
    /// The binding draft a created footprint is listed in: the one binding
    /// draft of the package. `None` with `binding.footprint_unbound` when
    /// there are none or several to choose from.
    #[serde(default)]
    pub footprint_bound: Option<DefinitionId>,
    /// How the current terminals carried over to the new form.
    #[serde(default)]
    pub terminals: TerminalMatchReport,
    pub findings: Vec<Finding>,
}

/// What `package_from_template` would change: the edits of existing
/// drafts (package first), the footprint draft it creates, if any, and
/// the plan.
struct Candidate {
    edits: Vec<DraftEdit>,
    created: Option<DraftEdit>,
    plan: PackagePlan,
}

/// The name of a footprint draft created for the package `name`, within
/// the 256-byte name bound.
fn footprint_name(name: &str) -> String {
    const SUFFIX: &str = " footprint";
    let mut end = name.len().min(256 - SUFFIX.len());
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{SUFFIX}", &name[..end])
}

/// The binding-draft edit that lists a created footprint, when exactly one
/// binding draft implements `package`; else the finding that says which
/// binding to edit (none, or several to choose from).
fn bind_created(
    document: &Document,
    package: DefinitionId,
    footprint: FootprintRef,
) -> std::result::Result<DraftEdit, Finding> {
    let bindings: Vec<_> = document
        .drafts
        .items
        .iter()
        .filter_map(|d| match &d.definition {
            Definition::Binding(b) if b.package.id == package => Some((d, b)),
            _ => None,
        })
        .collect();
    if let [(draft, binding)] = bindings.as_slice() {
        let mut body = (*binding).clone();
        body.footprints.push(footprint);
        return Ok(DraftEdit {
            id: draft.id,
            meta: draft.meta.clone(),
            definition: Definition::Binding(body),
        });
    }
    let mut objects = vec![ObjectRef::Definition { id: footprint.id }];
    let message = if bindings.is_empty() {
        "No binding draft implements the package; list the new footprint in the binding that does before publishing.".to_owned()
    } else {
        let names: Vec<&str> = bindings.iter().map(|(d, _)| d.meta.name.as_str()).collect();
        objects.extend(
            bindings
                .iter()
                .map(|(d, _)| ObjectRef::Definition { id: d.id }),
        );
        format!(
            "{} binding drafts implement the package ({}); list the new footprint in the ones that use it.",
            bindings.len(),
            names.join(", ")
        )
    };
    Err(Finding::new(
        FindingCode::BindingFootprintUnbound,
        message,
        objects,
    ))
}

/// The first local identity a package draft may allocate: above its
/// counter, its terminals and its base revision's counter, and never 0.
fn package_floor(
    document: &Document,
    draft: &crate::library::Draft,
    package: &PackageDefinition,
) -> u32 {
    let base = draft
        .base
        .and_then(|revision| {
            document
                .project
                .library
                .package(PackageRef::new(draft.id, revision))
        })
        .map_or(0, |p| p.definition.next_local);
    floor(
        package.next_local,
        base,
        package.terminals.iter().map(|t| t.id.0),
    )
}

/// The first local identity a footprint draft may allocate once it is
/// cleared: above its counter, every identity it holds now (they retire
/// with the cleared pads and drawings) and its base revision's counter,
/// and never 0.
fn footprint_floor(
    document: &Document,
    draft: &crate::library::Draft,
    footprint: &FootprintDefinition,
) -> u32 {
    let base = draft
        .base
        .and_then(|revision| {
            document
                .project
                .library
                .footprint(FootprintRef::new(draft.id, revision))
        })
        .map_or(0, |p| p.definition.next_local);
    let arrays = footprint
        .source
        .as_deref()
        .into_iter()
        .flat_map(|s| s.arrays.iter().map(|a| a.id.0));
    let used = footprint
        .pads
        .iter()
        .map(|p| p.id.0)
        .chain(footprint.graphics.iter().map(|g| g.id.0))
        .chain(arrays);
    floor(footprint.next_local, base, used)
}

/// The largest of the counter, the base counter, one past every used
/// identity and 1, saturating: an exhausted space then refuses at the
/// first allocation and the document stays unchanged.
fn floor(counter: u32, base: u32, used: impl Iterator<Item = u32>) -> u32 {
    used.map(|id| id.saturating_add(1))
        .fold(counter.max(base).max(1), u32::max)
}

/// The package's terminals after a form change, and how they matched.
struct Replaced {
    package: PackageDefinition,
    report: TerminalMatchReport,
    /// Numbers of the new form no current terminal carried (fresh
    /// terminals), in lead order.
    added: Vec<String>,
    /// Numbers of current terminals the new package removes.
    removed: Vec<String>,
}

/// `package` with terminals designated `designations`, in that order:
/// `strategy` says which current terminals carry over (their identities,
/// and so the binding mappings that name them), as
/// [`replaced_package`] describes; other new terminals get fresh
/// identities from `floor`.
fn matched_terminals(
    package: &PackageDefinition,
    designations: &[String],
    floor: u32,
    strategy: TerminalMatch,
    renumber: &[Renumber],
) -> Result<Replaced> {
    let count = designations.len();
    check_renumber(package, designations, strategy, renumber)?;
    // The current number a new number takes its terminal from: itself,
    // unless the request renumbers another one to it.
    let source = |designation: &str| -> String {
        renumber
            .iter()
            .find(|r| r.to.trim() == designation)
            .map_or_else(|| designation.to_owned(), |r| r.from.trim().to_owned())
    };
    let renamed = |current: &str| renumber.iter().any(|r| r.from.trim() == current);
    let mut next = package.clone();
    let mut local = floor;
    let mut fresh = || -> Result<PackageTerminalId> {
        let id = PackageTerminalId(local);
        local = local
            .checked_add(1)
            .ok_or_else(|| Error::Limit("package local identities exhausted".into()))?;
        Ok(id)
    };
    let mut terminals = Vec::with_capacity(count);
    let mut added = Vec::new();
    let mut renumbered = 0u32;
    let mut used = vec![false; package.terminals.len()];
    for (k, designation) in designations.iter().enumerate() {
        let found = match strategy {
            TerminalMatch::ByNumber => {
                let wanted = source(designation);
                package.terminals.iter().enumerate().position(|(j, t)| {
                    let current = t.designation.trim();
                    !used[j]
                        && !wanted.is_empty()
                        && current == wanted
                        // A renumbered terminal matches only its new number.
                        && (wanted != *designation || !renamed(current))
                })
            }
            TerminalMatch::ByPosition => (k < package.terminals.len()).then_some(k),
        };
        let id = match found.and_then(|j| package.terminals.get(j).map(|t| (j, t))) {
            Some((j, t)) => {
                used[j] = true;
                if t.designation.trim() != designation {
                    renumbered = renumbered.saturating_add(1);
                }
                t.id
            }
            None => {
                added.push(designation.clone());
                fresh()?
            }
        };
        terminals.push(PackageTerminal {
            id,
            designation: designation.clone(),
        });
    }
    next.next_local = local;
    let removed: Vec<String> = package
        .terminals
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(t, _)| t.designation.clone())
        .collect();
    let to_u32 = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let kept = to_u32(used.iter().filter(|u| **u).count());
    let mut summary = format!(
        "Terminals matched {}: {kept} kept, {} added, {} removed",
        strategy.label(),
        added.len(),
        removed.len()
    );
    match strategy {
        TerminalMatch::ByPosition => summary.push_str(&format!(", {renumbered} renumbered")),
        TerminalMatch::ByNumber if renumbered > 0 => {
            let pairs: Vec<String> = renumber
                .iter()
                .map(|r| format!("{} → {}", r.from.trim(), r.to.trim()))
                .collect();
            summary.push_str(&format!(
                ", {renumbered} renumbered as requested ({})",
                pairs.join(", ")
            ));
        }
        TerminalMatch::ByNumber => {}
    }
    summary.push('.');
    let report = TerminalMatchReport {
        strategy,
        kept,
        added: to_u32(added.len()),
        removed: to_u32(removed.len()),
        renumbered,
        summary,
    };
    next.terminals = terminals;
    Ok(Replaced {
        package: next,
        report,
        added,
        removed,
    })
}

/// Refuses a `renumber` list that is not an explicit one-to-one map from
/// current numbers to numbers of the new form, or that comes with
/// matching by position.
fn check_renumber(
    package: &PackageDefinition,
    designations: &[String],
    strategy: TerminalMatch,
    renumber: &[Renumber],
) -> Result<()> {
    if renumber.is_empty() {
        return Ok(());
    }
    if strategy == TerminalMatch::ByPosition {
        return Err(Error::Invalid(
            "template.renumber: renumber goes with terminals by_number".into(),
        ));
    }
    if renumber.len() > package.terminals.len() {
        return Err(Error::Invalid(
            "template.renumber: more entries than the package has terminals".into(),
        ));
    }
    for (k, r) in renumber.iter().enumerate() {
        let (from, to) = (r.from.trim(), r.to.trim());
        let current = package
            .terminals
            .iter()
            .filter(|t| t.designation.trim() == from)
            .count();
        if from.is_empty() || current != 1 {
            return Err(Error::Invalid(format!(
                "template.renumber: the package has no single terminal numbered ‘{from}’"
            )));
        }
        if !designations.iter().any(|d| d == to) {
            return Err(Error::Invalid(format!(
                "template.renumber: the new form has no terminal numbered ‘{to}’"
            )));
        }
        let repeated = renumber[..k]
            .iter()
            .any(|o| o.from.trim() == from || o.to.trim() == to);
        if repeated {
            return Err(Error::Invalid(format!(
                "template.renumber: ‘{from}’ or ‘{to}’ is named twice"
            )));
        }
    }
    Ok(())
}

/// At most this many numbers are listed in one finding.
const LISTED_NUMBERS: usize = 16;

/// `numbers` as a short list: the first [`LISTED_NUMBERS`], then a count.
fn number_list(numbers: &[String]) -> String {
    let shown: Vec<&str> = numbers
        .iter()
        .take(LISTED_NUMBERS)
        .map(|n| {
            if n.is_empty() {
                "(unnumbered)"
            } else {
                n.as_str()
            }
        })
        .collect();
    let mut out = shown.join(", ");
    if numbers.len() > LISTED_NUMBERS {
        out.push_str(&format!(" and {} more", numbers.len() - LISTED_NUMBERS));
    }
    out
}

/// The advisory finding naming the terminals a replacement left
/// unmatched: new numbers no current terminal carried (fresh and
/// unmapped) and current terminals the new package removes. `None` when
/// every terminal matched.
fn unmatched_finding(package: DefinitionId, replaced: &Replaced) -> Option<Finding> {
    if replaced.added.is_empty() && replaced.removed.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if !replaced.added.is_empty() {
        parts.push(format!(
            "new terminal(s) {} match no current terminal and are unmapped",
            number_list(&replaced.added)
        ));
    }
    if !replaced.removed.is_empty() {
        parts.push(format!(
            "current terminal(s) {} are not in the new form and are removed",
            number_list(&replaced.removed)
        ));
    }
    Some(Finding::new(
        FindingCode::PackageTerminalsUnmatched,
        format!(
            "Terminals matched {}: {}. Nothing is filled by position: if a removed and a new number are the same pad (an exposed pad the datasheet numbers), pass renumber [{{from, to}}]; else map them explicitly.",
            replaced.report.strategy.label(),
            parts.join("; ")
        ),
        vec![ObjectRef::Definition { id: package }],
    ))
}

/// Advice when the request chose [`TerminalMatch::ByPosition`] and the
/// package renumbers a kept terminal or changes family or lead count
/// while a binding draft maps kept terminals: those terminals are reused
/// by position (`binding.mapping_retained`), so each mapping should be
/// checked against the new pinout before publishing.
fn retained_findings(
    document: &Document,
    package: DefinitionId,
    before: &PackageDefinition,
    next: &PackageDefinition,
    renumbered: u32,
) -> Vec<Finding> {
    if renumbered == 0 && before.terminals.len() == next.terminals.len() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for draft in &document.drafts.items {
        let Definition::Binding(binding) = &draft.definition else {
            continue;
        };
        if binding.package.id != package {
            continue;
        }
        let kept = binding
            .mappings
            .iter()
            .filter(|m| next.terminal(m.package_terminal).is_some())
            .count();
        if kept > 0 {
            out.push(Finding::new(
                FindingCode::BindingMappingRetained,
                format!(
                    "{kept} mapping(s) of ‘{}’ keep package terminals by position, as requested, while the package changes from {} terminal(s) to {}; check each against the new pinout.",
                    draft.meta.name,
                    before.terminals.len(),
                    next.terminals.len(),
                ),
                vec![ObjectRef::Definition { id: draft.id }],
            ));
        }
    }
    out
}

/// Mapping findings the new terminals give each binding draft of the
/// package draft: terminals no mapping lands (`binding.mapping_missing`)
/// and mappings of removed terminals (`binding.mapping_unknown`).
fn mapping_findings(
    document: &Document,
    package: DefinitionId,
    next: &PackageDefinition,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for draft in &document.drafts.items {
        let Definition::Binding(binding) = &draft.definition else {
            continue;
        };
        if binding.package.id != package {
            continue;
        }
        let objects = vec![ObjectRef::Definition { id: draft.id }];
        let missing = next
            .terminals
            .iter()
            .filter(|t| !binding.mappings.iter().any(|m| m.package_terminal == t.id))
            .count();
        if missing > 0 {
            out.push(Finding::blocking(
                FindingCode::BindingMappingMissing,
                format!(
                    "{missing} package terminal(s) of the new package are not mapped by ‘{}’; map them explicitly.",
                    draft.meta.name
                ),
                objects.clone(),
                Stage::Publish,
            ));
        }
        let unknown = binding
            .mappings
            .iter()
            .filter(|m| next.terminal(m.package_terminal).is_none())
            .count();
        if unknown > 0 {
            out.push(Finding::blocking(
                FindingCode::BindingMappingUnknown,
                format!(
                    "{unknown} mapping(s) of ‘{}’ name package terminals the new package removes.",
                    draft.meta.name
                ),
                objects,
                Stage::Publish,
            ));
        }
    }
    out
}

/// The open working set's package and footprint drafts taking a generated
/// package's content (`library.package_from_generator`, slice D1): the
/// package's terminals (matched to the current ones as `strategy` says),
/// envelope, model and placement, and the footprint's pads, drawings,
/// polarity, keep-outs, defaults and datum, with fresh local identities
/// and pads landing the matched terminals. The drafts are the person's
/// own afterwards; the generated revisions stay in the library with their
/// record. One undo step with the generation, applied by the caller.
pub(crate) fn adopt_generated_package(
    document: &mut Document,
    draft: DefinitionId,
    replace: bool,
    strategy: TerminalMatch,
    renumber: &[Renumber],
    generated: (&PackageDefinition, &FootprintDefinition),
    provenance: String,
) -> Result<Outcome> {
    let fresh = DefinitionId(document.project.next_id);
    let Candidate {
        edits,
        created,
        plan,
    } = adopted(
        document,
        draft,
        replace,
        strategy,
        renumber,
        (generated.0, generated.1, provenance),
        fresh,
    )?;
    if let Some(created) = created {
        let project = Arc::make_mut(&mut document.project);
        let id = DefinitionId(project.allocate()?);
        if id != created.id {
            return Err(Error::Conflict(
                "template.footprint: the footprint identity changed while it was planned".into(),
            ));
        }
        drafts::create(
            Arc::make_mut(&mut document.drafts),
            &project.library,
            created.id,
            None,
            created.meta,
            created.definition,
        )?;
    }
    drafts::edit(Arc::make_mut(&mut document.drafts), edits)?;
    Ok(Outcome::PackageReplaced { plan })
}

/// The package draft `draft` and the footprint draft landing it, if one;
/// refused when the draft is gone or not a package, or several footprint
/// drafts land it, or (without `replace`) its footprint has content.
#[allow(clippy::type_complexity)]
fn working_package(
    document: &Document,
    draft: DefinitionId,
    replace: bool,
) -> Result<(
    &crate::library::Draft,
    &PackageDefinition,
    Option<(&crate::library::Draft, &FootprintDefinition)>,
)> {
    let current = document
        .drafts
        .get(draft)
        .ok_or_else(|| Error::NotFound("That draft no longer exists".into()))?;
    let Definition::Package(package) = &current.definition else {
        return Err(Error::Invalid(format!(
            "The draft ‘{}’ is not a package",
            current.meta.name
        )));
    };
    let mut landing = document
        .drafts
        .items
        .iter()
        .filter_map(|d| match &d.definition {
            Definition::Footprint(f) if f.package.id == draft => Some((d, f)),
            _ => None,
        });
    let landing = match (landing.next(), landing.next()) {
        (None, _) => None,
        (Some(one), None) => Some(one),
        (Some(_), Some(_)) => {
            return Err(Error::Invalid(format!(
                "package.footprint_ambiguous: several footprint drafts land the package ‘{}’; open one working set",
                current.meta.name
            )));
        }
    };
    if let Some((footprint_draft, footprint)) = landing
        && !replace
        && (!footprint.pads.is_empty() || footprint.source.is_some())
    {
        return Err(Error::Conflict(format!(
            "footprint.template_not_empty: the footprint ‘{}’ already has pads; pass replace to rebuild it from the generator",
            footprint_draft.meta.name
        )));
    }
    Ok((current, package, landing))
}

/// Whether the working set's package draft can take a generated package:
/// what [`adopt_generated_package`] checks before anything is generated.
pub(crate) fn check_working_package(
    document: &Document,
    draft: DefinitionId,
    replace: bool,
) -> Result<()> {
    working_package(document, draft, replace).map(|_| ())
}

fn adopted(
    document: &Document,
    draft: DefinitionId,
    replace: bool,
    strategy: TerminalMatch,
    renumber: &[Renumber],
    (generated, generated_footprint, provenance): (
        &PackageDefinition,
        &FootprintDefinition,
        String,
    ),
    fresh: DefinitionId,
) -> Result<Candidate> {
    let (current, package, landing) = working_package(document, draft, replace)?;
    let floor = package_floor(document, current, package);
    let designations: Vec<String> = generated
        .terminals
        .iter()
        .map(|t| t.designation.clone())
        .collect();
    let replaced = matched_terminals(package, &designations, floor, strategy, renumber)?;
    let unmatched = if package.terminals.is_empty() {
        None
    } else {
        unmatched_finding(draft, &replaced)
    };
    let terminals = replaced.report.clone();
    // The generated terminal each new terminal is, by position.
    let ids: Vec<(PackageTerminalId, PackageTerminalId)> = generated
        .terminals
        .iter()
        .zip(&replaced.package.terminals)
        .map(|(g, n)| (g.id, n.id))
        .collect();
    let mut next = replaced.package;
    next.envelope_nm = generated.envelope_nm;
    next.model = generated.model.clone();
    next.placement = generated.placement;
    let mut meta = current.meta.clone();
    meta.provenance = provenance;
    let (footprint_id, footprint_meta, footprint, floor, create) = match landing {
        Some((footprint_draft, footprint)) => (
            footprint_draft.id,
            footprint_draft.meta.clone(),
            footprint.clone(),
            footprint_floor(document, footprint_draft, footprint),
            false,
        ),
        None => {
            let revision = current.pending().ok_or_else(|| {
                Error::Limit(format!(
                    "the package ‘{}’ has no next revision",
                    current.meta.name
                ))
            })?;
            let footprint = FootprintDefinition {
                package: PackageRef::new(draft, revision),
                next_local: 1,
                ..FootprintDefinition::default()
            };
            let mut footprint_meta = meta.clone();
            footprint_meta.name = footprint_name(&current.meta.name);
            (fresh, footprint_meta, footprint, 1, true)
        }
    };
    let rebuilt = adopted_footprint(&footprint, generated_footprint, &ids, floor)?;
    let pads_after = u32::try_from(rebuilt.pads.len()).unwrap_or(u32::MAX);
    let mut findings = package_fit::placement_findings(&next, &rebuilt, footprint_id);
    let footprint_edit = DraftEdit {
        id: footprint_id,
        meta: footprint_meta,
        definition: Definition::Footprint(rebuilt),
    };
    let mut edits = Vec::new();
    let mut bound = None;
    let created = if create {
        let reference = FootprintRef::new(footprint_id, Revision::FIRST);
        match bind_created(document, draft, reference) {
            Ok(edit) => {
                bound = Some(edit.id);
                edits.push(edit);
            }
            Err(finding) => findings.push(finding),
        }
        Some(footprint_edit)
    } else {
        edits.push(footprint_edit);
        None
    };
    findings.extend(unmatched);
    findings.extend(mapping_findings(document, draft, &next));
    if terminals.strategy == TerminalMatch::ByPosition {
        findings.extend(retained_findings(
            document,
            draft,
            package,
            &next,
            terminals.renumbered,
        ));
    }
    let plan = PackagePlan {
        terminals_before: u32::try_from(package.terminals.len()).unwrap_or(u32::MAX),
        terminals_after: u32::try_from(next.terminals.len()).unwrap_or(u32::MAX),
        pads_after,
        footprint: Some(footprint_id),
        footprint_created: created.is_some(),
        footprint_bound: bound,
        terminals,
        findings,
    };
    edits.insert(
        0,
        DraftEdit {
            id: draft,
            meta,
            definition: Definition::Package(next),
        },
    );
    Ok(Candidate {
        edits,
        created,
        plan,
    })
}

/// `footprint` (the working set's, keeping its lineage and landing
/// reference) with `generated`'s content: pads, drawings, polarity,
/// keep-outs, defaults and datum, every pad and drawing given a fresh
/// identity from `floor` and each pad landing the terminal `ids` maps its
/// generated terminal to.
fn adopted_footprint(
    footprint: &FootprintDefinition,
    generated: &FootprintDefinition,
    ids: &[(PackageTerminalId, PackageTerminalId)],
    floor: u32,
) -> Result<FootprintDefinition> {
    let mut next_local = floor;
    let mut fresh = || -> Result<u32> {
        let id = next_local;
        next_local = next_local
            .checked_add(1)
            .ok_or_else(|| Error::Limit("footprint local identities exhausted".into()))?;
        Ok(id)
    };
    let mut pads = Vec::with_capacity(generated.pads.len());
    let mut pad_ids = Vec::with_capacity(generated.pads.len());
    for pad in &generated.pads {
        let mut pad = pad.clone();
        let id = PadId(fresh()?);
        pad_ids.push((pad.id, id));
        pad.id = id;
        pad.terminal = pad
            .terminal
            .and_then(|t| ids.iter().find(|(g, _)| *g == t).map(|(_, n)| *n));
        pads.push(pad);
    }
    let mut graphics = Vec::with_capacity(generated.graphics.len());
    let mut graphic_ids = Vec::with_capacity(generated.graphics.len());
    for graphic in &generated.graphics {
        let mut graphic = graphic.clone();
        let id = crate::ids::GraphicId(fresh()?);
        graphic_ids.push((graphic.id, id));
        graphic.id = id;
        graphics.push(graphic);
    }
    let polarity = generated.polarity.as_ref().and_then(|p| {
        let pad = pad_ids.iter().find(|(g, _)| *g == p.pad).map(|(_, n)| *n)?;
        Some(crate::library::Polarity {
            pad,
            markers: p
                .markers
                .iter()
                .filter_map(|m| graphic_ids.iter().find(|(g, _)| g == m).map(|(_, n)| *n))
                .collect(),
        })
    });
    Ok(FootprintDefinition {
        package: footprint.package,
        next_local,
        source: None,
        pads,
        graphics,
        polarity,
        datum: generated.datum,
        defaults: generated.defaults,
        keepouts: generated.keepouts.clone(),
    })
}
