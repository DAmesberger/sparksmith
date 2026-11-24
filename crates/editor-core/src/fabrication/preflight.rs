//! The production check run before export, and the waiver commands.
//!
//! The report is the project's issues (design rules checked under the
//! chosen fabrication profile) plus production findings: unrouted nets,
//! unfilled or stale fills, parts without a footprint or placement, parts
//! shown with only an envelope model, legend over pads or off the board and
//! slots the drill file cannot route. Errors block export unless waived, and
//! those whose files would not be the design, or would short two nets, may
//! not be ([`NO_WAIVER`]); waivers are saved in the project
//! (`Project::production`).
use super::snapshot::ProductionSnapshot;
use crate::diagnostics::{Error, Measured, ObjectRef, Result, Unit};
use crate::geometry::{Bounds, Point, polyline_polygon_distance};
use crate::ids::{DefinitionId, Id};
use crate::issues::{IssueArea, IssueRow, Severity};
use crate::layout::board_edge::{BoardEdges, EdgeLoopKind, EdgeVerdict};
use crate::layout::{self, BoardDrawing, CopperRing, PlacedItem};
use crate::library::{Drafts, PackageModel};
use crate::model::board::CutoutKind;
use crate::model::production::{
    Acceptance, MAX_ACCEPTED, MAX_WAIVE_KEYS, MAX_WAIVER_TEXT, Production, Waiver,
};
use crate::model::{CopperLayer, Layer, Project, Track};
use crate::rules::RuleProfile;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Waive or restore pre-flight findings. Each is one undo step; a command
/// that changes nothing is not.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum FabricationCommand {
    /// Accept pre-flight findings for fabrication, by their keys, with one
    /// optional reason, as one undo step. Replaces earlier waivers of the
    /// same keys. What each finding measures now is kept: the waiver holds
    /// while it measures the same or better. `profile` is the fab profile
    /// the findings were read under (`fabrication.preflight`'s), the
    /// board's own when left out. Refused whole, changing nothing, when any
    /// key is empty, a key or the reason is too long or not printable, or
    /// the host requires a reason and none is given.
    #[serde(rename = "fabrication.waive")]
    Waive {
        keys: Vec<String>,
        #[serde(default)]
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<ProfileChoice>,
    },
    /// Remove the waiver of a key.
    #[serde(rename = "fabrication.unwaive")]
    Unwaive { key: String },
}

// A new command chooses its permission class here.
declare_entries!(FabricationCommand in checks {
    Waive "fabrication.waive" Ask "Waive a production finding",
    Unwaive "fabrication.unwaive" Edit "Withdraw a waiver",
});

/// Apply one waiver command; `reason_required` refuses a waiver without a
/// reason (a host setting), and `accepted` is what the findings of a
/// [`FabricationCommand::Waive`] measure ([`accepted`]). A refused command
/// leaves `production` unchanged.
pub fn apply(
    production: &mut Production,
    command: FabricationCommand,
    reason_required: bool,
    accepted: &BTreeMap<String, Vec<Measured>>,
) -> Result<()> {
    let waivers = &mut production.waivers;
    match command {
        FabricationCommand::Waive { keys, reason, .. } => {
            let reason = reason.trim().to_string();
            let keys: BTreeSet<String> = keys.iter().map(|k| k.trim().to_string()).collect();
            if keys.is_empty() || keys.contains("") {
                return Err(Error::Invalid(
                    "fabrication.waiver_key: a waiver needs a finding key".into(),
                ));
            }
            if let Some(k) = keys.iter().find(|k| unwaivable(k)) {
                let code = k.split_whitespace().next().unwrap_or_default();
                return Err(Error::Conflict(format!(
                    "fabrication.not_waivable: `{k}` cannot be waived; {}",
                    instead(code)
                )));
            }
            if reason_required && reason.is_empty() {
                return Err(Error::Invalid(
                    "fabrication.waiver_reason: a reason is required to waive findings (Settings)"
                        .into(),
                ));
            }
            if keys.len() > MAX_WAIVE_KEYS
                || reason.len() > MAX_WAIVER_TEXT
                || reason.chars().any(char::is_control)
                || keys
                    .iter()
                    .any(|k| k.len() > MAX_WAIVER_TEXT || k.chars().any(char::is_control))
            {
                return Err(Error::Invalid(format!(
                    "fabrication.waiver_bounds: keys and reasons are printable, at most {MAX_WAIVER_TEXT} bytes; at most {MAX_WAIVE_KEYS} keys a request"
                )));
            }
            if let Some(k) = keys
                .iter()
                .find(|k| crate::model::production::StoredKey::parse(k).is_none())
            {
                return Err(Error::Invalid(format!(
                    "fabrication.waiver_key: `{k}` is not a finding's key (its code, a space and the objects it names as JSON, as fabrication.preflight gives it)"
                )));
            }
            if let Some(k) = keys
                .iter()
                .find(|k| accepted.get(*k).is_some_and(|a| a.len() > MAX_ACCEPTED))
            {
                return Err(Error::Limit(format!(
                    "fabrication.waiver_bounds: `{k}` measures more than {MAX_ACCEPTED} numbers"
                )));
            }
            for key in keys {
                let waiver = Waiver {
                    accepted: accepted.get(&key).cloned().unwrap_or_default(),
                    reason: reason.clone(),
                    key,
                };
                match waivers.binary_search_by(|w| w.key.as_str().cmp(&waiver.key)) {
                    Ok(i) => waivers[i] = waiver,
                    Err(i) => waivers.insert(i, waiver),
                }
            }
        }
        FabricationCommand::Unwaive { key } => {
            let Ok(i) = waivers.binary_search_by(|w| w.key.as_str().cmp(key.trim())) else {
                return Err(Error::NotFound(
                    "fabrication.waiver_missing: that finding is not waived".into(),
                ));
            };
            waivers.remove(i);
        }
    }
    Ok(())
}

/// The profile a `fabrication.preflight` query checks under.
pub use crate::rules::ProfileChoice;

/// One pre-flight finding.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PreflightItem {
    /// Stable identity for waivers: the finding code and the objects it
    /// names, encoded by [`key`].
    pub key: String,
    pub code: String,
    pub severity: Severity,
    /// One line naming objects by their user names.
    pub title: String,
    /// What resolves it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    pub objects: Vec<ObjectRef>,
    /// The saved waiver, when the user accepted this finding and it still
    /// measures what was accepted, or better.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiver: Option<Waiver>,
    /// The saved waiver that no longer accepts this finding, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lapsed: Option<Lapsed>,
    /// What the check measured against its limits; none for a finding that
    /// is a fact without a number, whose waiver goes by its key alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measured: Vec<Measured>,
    /// What the board needs of the fab profile for this finding to clear,
    /// when the limit it breaks is the profile's own rule: the measured
    /// value, the profile's limit and its source. Such a finding is
    /// resolved by a profile that allows it (`fabrication.profile_suggestions`)
    /// or by changing the design, never by a waiver
    /// ([`Self::offers_waiver`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs: Option<crate::rules::requirement::Needed>,
}

/// A waiver that counts for nothing: what it accepted, and why it no longer
/// holds ("Was accepted at 0.15 mm, is now 0.12 mm").
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Lapsed {
    pub waiver: Waiver,
    pub text: String,
}

impl PreflightItem {
    /// An error nobody waived: it blocks export.
    pub fn blocks(&self) -> bool {
        self.severity == Severity::Error && self.waiver.is_none()
    }

    /// Whether a host offers to waive this finding: not one that may not
    /// be waived ([`can_waive`]), and not one the board's geometry asks
    /// more of the fab profile for ([`Self::needs`]): a part drawn to its
    /// datasheet is made by a profile that allows it, not waived.
    pub fn offers_waiver(&self) -> bool {
        self.needs.is_none() && can_waive(&self.code)
    }
}

/// `fabrication.preflight`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PreflightReport {
    /// The fab profile the design was checked under.
    pub profile: String,
    /// Most severe first.
    pub items: Vec<PreflightItem>,
    pub errors: usize,
    pub warnings: usize,
    /// Waived findings (of any severity).
    pub waived: usize,
    /// Unwaived errors; export is refused while this is not 0.
    pub blocking: usize,
}

impl PreflightReport {
    pub fn ready(&self) -> bool {
        self.blocking == 0
    }
}

/// Characters of a legend text a finding quotes, so its title stays one line.
const MAX_LEGEND_NAME: usize = 16;

/// Finding codes that may not be waived, and whether one may: they live
/// with the findings ([`crate::issues`]), which hide waived findings.
pub use crate::issues::{NO_WAIVER, SLOT_UNROUTABLE, can_waive};

/// Whether the finding a waiver key names may not be waived.
fn unwaivable(key: &str) -> bool {
    let code = key.split_whitespace().next().unwrap_or_default();
    !can_waive(code)
}

/// What the findings of `keys` measure on `project` (a snapshot's), for the
/// waivers that accept them: the numbers of every finding that shares a key,
/// but for those another accepts too. Refuses a key that names a finding
/// `project` needs more of its fab profile for (`fabrication.not_waivable`,
/// naming the need): a profile that allows it, or a change to the design,
/// resolves it. The engine runs this before [`apply`] of a waiver.
pub fn accepted(
    project: &Project,
    drafts: &Drafts,
    settings: &crate::layout::CheckSettings,
    keys: &[String],
) -> Result<BTreeMap<String, Vec<Measured>>> {
    let keys: BTreeSet<&str> = keys.iter().map(|k| k.trim()).collect();
    let mut out: BTreeMap<String, Vec<Measured>> = BTreeMap::new();
    let mut take = |k: String, measured: &[Measured]| {
        let all = out.entry(k).or_default();
        for m in measured {
            if !all.iter().any(|a| a.accepts(m)) {
                all.retain(|a| !m.accepts(a));
                all.push(Measured {
                    within: 0.0,
                    ..m.clone()
                });
            }
        }
    };
    for row in crate::issues::production_rows(project, drafts, settings)? {
        let k = key(&row.finding.code, &row.finding.objects);
        if !keys.contains(k.as_str()) {
            continue;
        }
        if let Some(requirement) = &row.finding.requirement {
            let need =
                crate::rules::requirement::describe(requirement, project.rules.profile.as_ref())
                    .map_or_else(String::new, |n| format!("{}; ", n.text));
            return Err(Error::Conflict(format!(
                "fabrication.not_waivable: `{k}` cannot be waived. {need}switch to a fab profile that allows it (fabrication.profile_suggestions) or change the design"
            )));
        }
        take(k, &row.finding.measured);
    }
    for (k, measured) in legend_measured(project) {
        if keys.contains(k.as_str()) {
            take(k, &measured);
        }
    }
    Ok(out)
}

/// The production findings that measure something, by key.
pub(super) fn legend_measured(project: &Project) -> Vec<(String, Vec<Measured>)> {
    let mut items = Vec::new();
    legend_against_the_board(project, &mut items);
    items.into_iter().map(|i| (i.key, i.measured)).collect()
}

/// What resolves a finding of `code` instead of a waiver, for the refusal.
fn instead(code: &str) -> &'static str {
    match code {
        "board.check_limit" => "the board checks must finish before fabrication",
        "board.unrouted" => "route the connection",
        "board.unplaced" => {
            "place the part (Update PCB); to leave it unassembled, mark it Not fitted in a variant"
        }
        "rules.no_fab_profile" => "choose a fab profile",
        crate::layout::fill::CLEARANCE_UNLISTED | "fill.stale" | "fabrication.fill_unfilled" => {
            "refill the fill"
        }
        "fill.overlap" => "refill the fill, or move the copper apart",
        "board.overlap" => "move the copper apart",
        "board.shape_unsolved" | layout::outline::SHAPE_OPEN | layout::outline::SHAPE_INVALID => {
            "make the board outline valid (Board shape)"
        }
        "fabrication.no_footprint" => {
            "choose a footprint for the part, or take it out of the design"
        }
        "instance.check_failed" => "restore what the part refers to",
        c if c.starts_with(SLOT_UNROUTABLE) => {
            "draw the plated slot as a straight slot with round ends, or make it unplated"
        }
        _ => "re-pick or acknowledge the reference",
    }
}

/// The waiver key of a finding: its code and the objects it names.
pub fn key(code: &str, objects: &[ObjectRef]) -> String {
    crate::model::production::waiver_key(code, objects)
}

/// Pre-flight severity of an issue row: unrouted connections, unplaced
/// parts, stale fills and an unsolved outline block production;
/// component-area findings (drafts and library definitions) and to-dos
/// (such as parts without a purchasable Part) are warnings; information
/// is left out.
fn severity(row: &IssueRow) -> Option<Severity> {
    let code = row.finding.code.as_str();
    if matches!(
        code,
        "board.unrouted"
            | "board.unplaced"
            | "fill.stale"
            | "board.shape_unsolved"
            | layout::outline::SHAPE_OPEN
            | layout::outline::SHAPE_INVALID
            | layout::dimensions::REFERENCE_LOST
            | layout::dimensions::REFERENCE_INEXACT
            | crate::interface::findings::ASSEMBLY_CYCLE
    ) {
        return Some(Severity::Error);
    }
    match (row.area, row.severity) {
        (_, Severity::Info) => None,
        (IssueArea::Component, Severity::Error) | (_, Severity::ToDo) => Some(Severity::Warning),
        (_, s) => Some(s),
    }
}

/// The pre-flight report of `project` checked under `profile` (the
/// project's own rules when `None`; overrides and net classes are kept):
/// [`check`] of the [`ProductionSnapshot`] they resolve to; a profile the
/// design cannot take is refused (`fabrication.profile_refused`), with no
/// report and nothing to waive. An export resolves the snapshot once and
/// checks and writes from it.
pub fn report(
    project: &Project,
    drafts: &Drafts,
    profile: Option<&RuleProfile>,
    settings: &crate::layout::CheckSettings,
) -> Result<PreflightReport> {
    check(
        &ProductionSnapshot::resolve(project, profile)?,
        drafts,
        settings,
    )
}

/// The pre-flight report of `snapshot`. Fill copper computed under other
/// rules is reported stale: refill under the fabrication profile before
/// export. `settings` are the host's check settings
/// ([`crate::Engine::check_settings`]).
pub fn check(
    snapshot: &ProductionSnapshot<'_>,
    drafts: &Drafts,
    settings: &crate::layout::CheckSettings,
) -> Result<PreflightReport> {
    let mut items = Vec::new();
    let p: &Project = snapshot.project();
    for row in &crate::issues::production_rows(p, drafts, settings)? {
        let Some(severity) = severity(row) else {
            continue;
        };
        let mut found = item(
            &row.finding.code,
            severity,
            row.title.clone(),
            row.detail.fix.clone(),
            row.finding.objects.clone(),
        );
        found.needs = row
            .finding
            .requirement
            .as_ref()
            .and_then(|r| crate::rules::requirement::describe(r, p.rules.profile.as_ref()));
        found.measured.clone_from(&row.finding.measured);
        items.push(found);
    }
    production_findings(p, drafts, &mut items);
    if p.rules.profile.is_none() {
        items.push(item(
            "rules.no_fab_profile",
            Severity::Error,
            "The board has no fab profile, so no manufacturing rule was checked".into(),
            Some("Choose the fabricator's fab profile (Board › Rules)".into()),
            Vec::new(),
        ));
    }
    for i in &mut items {
        // A saved waiver of a finding with needs is ignored, like one of an
        // unwaivable code: a profile that allows it or a design change clears it.
        if unwaivable(&i.key) || i.needs.is_some() {
            continue;
        }
        match p.production.acceptance(&i.key, &i.measured) {
            Acceptance::None => {}
            Acceptance::Holds(waiver) => i.waiver = Some(waiver.clone()),
            Acceptance::Lapsed(waiver, text) => {
                i.lapsed = Some(Lapsed {
                    waiver: waiver.clone(),
                    text,
                });
            }
        }
    }
    // Of findings that share a key, one nobody accepts stands for them.
    items.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then_with(|| a.code.cmp(&b.code))
            .then_with(|| a.key.cmp(&b.key))
            .then_with(|| a.waiver.is_some().cmp(&b.waiver.is_some()))
    });
    items.dedup_by(|a, b| a.key == b.key);
    let count = |s: Severity| items.iter().filter(|i| i.severity == s).count();
    Ok(PreflightReport {
        profile: p.rules.profile_name(),
        errors: count(Severity::Error),
        warnings: count(Severity::Warning),
        waived: items.iter().filter(|i| i.waiver.is_some()).count(),
        blocking: items.iter().filter(|i| i.blocks()).count(),
        items,
    })
}

fn item(
    code: &str,
    severity: Severity,
    title: String,
    fix: Option<String>,
    objects: Vec<ObjectRef>,
) -> PreflightItem {
    PreflightItem {
        key: key(code, &objects),
        code: code.into(),
        severity,
        title,
        fix,
        objects,
        waiver: None,
        lapsed: None,
        measured: Vec::new(),
        needs: None,
    }
}

/// Findings only production needs: parts without a footprint or with an
/// empty value field (the BOM's Value would be blank), fills never
/// computed, parts with only an envelope 3D model, unpublished draft
/// changes of definitions in use, plated slots the drill file cannot route,
/// legend ink over pad openings and legend ink off the board.
fn production_findings(project: &Project, drafts: &Drafts, items: &mut Vec<PreflightItem>) {
    // Drafts whose content differs from the revision they edit (an open
    // draft identical to its base is no pending change).
    let drafted: BTreeSet<DefinitionId> = drafts
        .items
        .iter()
        .filter(|d| {
            let base = d.base.and_then(|revision| {
                project.library.definition(crate::ids::DefinitionRef {
                    kind: d.definition.kind(),
                    id: d.id,
                    revision,
                })
            });
            base.is_none_or(|(meta, definition)| meta != d.meta || definition != d.definition)
        })
        .map(|d| d.id)
        .collect();
    for instance in &project.instances {
        let objects = vec![ObjectRef::Instance { id: instance.id }];
        let unvalued =
            crate::instances::effective_fields(&project.library, instance).is_ok_and(|fields| {
                fields.iter().any(|f| {
                    f.value.is_none()
                        && crate::library::standard_fields::is_value_field(
                            &project.library,
                            f.field,
                        )
                })
            });
        if unvalued {
            items.push(item(
                "fabrication.no_value",
                Severity::Warning,
                format!("{} has no Value", instance.designator),
                Some("Fill in the part's value field".into()),
                objects.clone(),
            ));
        }
        let resolved = crate::instances::resolve(&project.library, instance).ok();
        match resolved.as_ref().and_then(|r| r.physical) {
            None => items.push(item(
                "fabrication.no_footprint",
                Severity::Error,
                format!("{} has no footprint", instance.designator),
                Some("Choose a footprint for the part, or remove the part".into()),
                objects,
            )),
            Some(physical) => {
                if matches!(
                    physical.package.definition.model,
                    PackageModel::Envelope { .. }
                ) {
                    items.push(item(
                        "fabrication.envelope_model",
                        Severity::Warning,
                        format!("{} has only an envelope 3D model", instance.designator),
                        Some("Generate or import a 3D model for the package".into()),
                        objects.clone(),
                    ));
                }
                let lineages = [
                    instance.component.id,
                    physical.footprint.id,
                    physical.binding.id,
                    physical.package.id,
                ];
                if lineages.iter().any(|l| drafted.contains(l)) {
                    items.push(item(
                        "fabrication.draft_pending",
                        Severity::Warning,
                        format!(
                            "{} uses a definition with unpublished draft changes",
                            instance.designator
                        ),
                        Some("Review and publish the draft, or discard it".into()),
                        objects,
                    ));
                }
            }
        }
    }
    for fill in &project.fills {
        if fill.computed.is_none() {
            items.push(item(
                "fabrication.fill_unfilled",
                Severity::Error,
                format!("Fill {} has never been filled", fill.name),
                Some("Refill the fills".into()),
                vec![ObjectRef::Fill { id: fill.id }],
            ));
        }
    }
    for cutout in &project.board().cutouts {
        if cutout.kind == (CutoutKind::Slot { plated: true })
            && super::drill::stadium(&cutout.outline).is_none()
        {
            items.push(item(
                &format!("{SLOT_UNROUTABLE}.{}", cutout.id.map_or(0, |id| id.0)),
                Severity::Error,
                "A plated board slot is not a straight slot the drill file can route".into(),
                Some("Draw the plated slot as a straight slot with round ends".into()),
                Vec::new(),
            ));
        }
    }
    silk_over_pads(project, items);
    legend_against_the_board(project, items);
}

/// How far a legend item is from staying on the board. Worse is greater.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LegendFault {
    /// Closer than the silkscreen edge clearance.
    Close { gap: f64, near: EdgeLoopKind },
    /// Over a cutout or slot.
    Inside(CutoutKind),
    /// Past the outer outline.
    Outside,
}

impl LegendFault {
    fn rank(self) -> u8 {
        match self {
            Self::Close { .. } => 0,
            Self::Inside(_) => 1,
            Self::Outside => 2,
        }
    }
}

/// The name a legend finding gives an item: at most `MAX_LEGEND_NAME`
/// characters of its own text.
fn legend_name(text: &str, designator: &str) -> String {
    if text == designator {
        return format!("Designator {designator}");
    }
    let line = text.lines().next().unwrap_or_default();
    let short: String = line.chars().take(MAX_LEGEND_NAME).collect();
    let more = if short.len() < line.len() { "…" } else { "" };
    format!("Legend text {short}{more} of {designator}")
}

/// Legend text past the board's outline or over a cutout or slot: the
/// fabricator trims it, so the text can't be read. With a silkscreen edge
/// clearance in the rules, text closer than that to an edge is reported too.
/// One finding per text on the printed sides. Legend lines may run past an
/// edge: a part that overhangs the board, such as a module's antenna end,
/// draws its outline there, and a trimmed line loses nothing.
fn legend_against_the_board(project: &Project, items: &mut Vec<PreflightItem>) {
    let limit = project.rules.effective().silkscreen.edge_clearance;
    let edges = BoardEdges::of(project);
    let names: BTreeMap<Id, (&str, DefinitionId)> = project
        .instances
        .iter()
        .filter_map(|instance| {
            let footprint = crate::instances::resolve(&project.library, instance)
                .ok()?
                .physical?
                .footprint
                .id;
            Some((instance.id, (instance.designator.as_str(), footprint)))
        })
        .collect();
    for graphic in layout::placed_graphics(project) {
        let BoardDrawing::Silkscreen(side) = graphic.layer else {
            continue;
        };
        let printed = match side {
            Layer::Top => project.stackup.silkscreen_top,
            Layer::Bottom => project.stackup.silkscreen_bottom,
        };
        if !printed {
            continue;
        }
        let PlacedItem::Text(text) = &graphic.item else {
            continue;
        };
        let strokes = layout::legend::text_strokes(text, &project.rules.effective().silkscreen);
        let mut worst: Option<LegendFault> = None;
        for points in &strokes.strokes {
            let single = points.first().map(|p| [*p, *p]);
            let pairs = points
                .windows(2)
                .map(|w| [w[0], w[1]])
                .chain(single.filter(|_| points.len() == 1));
            for [start, end] in pairs {
                let ink = CopperRing::of_track(&Track {
                    id: Id::default(),
                    net: Id::default(),
                    start,
                    end,
                    width: strokes.width,
                    layer: CopperLayer::from(side),
                    locked: false,
                });
                let fault = match edges.verdict(&ink, limit) {
                    EdgeVerdict::Clear => continue,
                    EdgeVerdict::Outside => LegendFault::Outside,
                    EdgeVerdict::Inside(kind) => LegendFault::Inside(kind),
                    EdgeVerdict::TooClose { gap, near } => LegendFault::Close { gap, near },
                };
                let worse = match (worst, fault) {
                    (None, _) => true,
                    (
                        Some(LegendFault::Close { gap: a, .. }),
                        LegendFault::Close { gap: b, .. },
                    ) => b < a,
                    (Some(w), f) => f.rank() > w.rank(),
                };
                if worse {
                    worst = Some(fault);
                }
            }
        }
        let Some(fault) = worst else {
            continue;
        };
        // placed_graphics includes only instances with resolved footprints.
        let Some(&(designator, footprint)) = names.get(&graphic.instance) else {
            continue;
        };
        let name = legend_name(&text.text, designator);
        let (code, title) = match fault {
            LegendFault::Outside => (
                "fabrication.legend_outside",
                format!("{name} runs outside the board"),
            ),
            LegendFault::Inside(kind) => (
                "fabrication.legend_outside",
                format!("{name} runs into {}", EdgeLoopKind::Cutout(kind).describe()),
            ),
            LegendFault::Close { gap, near } => (
                "fabrication.legend_edge",
                format!(
                    "{name} is {gap:.2} mm from {}; needs {limit} mm",
                    near.describe()
                ),
            ),
        };
        let mut finding = item(
            code,
            Severity::Warning,
            title,
            Some("Move the text on the part's Placement card, or move the part".into()),
            vec![
                ObjectRef::Instance {
                    id: graphic.instance,
                },
                ObjectRef::LibraryObject {
                    definition: footprint,
                    local: graphic.graphic.0,
                },
            ],
        );
        if let LegendFault::Close { gap, .. } = fault {
            finding.measured = vec![Measured::at_least(gap, limit, Unit::Mm, 1e-8)];
        }
        items.push(finding);
    }
}

/// Legend strokes that reach into a solder-mask opening of a pad on the
/// same side: the fabricator clips that ink.
fn silk_over_pads(project: &Project, items: &mut Vec<PreflightItem>) {
    let rules = project.rules.effective();
    let tol = 0.01;
    let mut openings: Vec<(Layer, Vec<Point>, Bounds)> = Vec::new();
    for pad in layout::placed_pads(project) {
        let Some(by) = pad.mask_expansion(rules.mask.expansion) else {
            continue;
        };
        // A hole in an opening (mask the growth left) is not looked for:
        // ink there is reported too.
        for piece in pad.pad.shape.offset_pieces(by, tol) {
            let ring = piece.outer.transformed(&pad.pad_transform()).flatten(tol);
            let Some(bounds) = Bounds::of(ring.iter().copied()) else {
                continue;
            };
            if pad.pad.kind.through_hole() {
                openings.push((Layer::Top, ring.clone(), bounds));
                openings.push((Layer::Bottom, ring, bounds));
            } else {
                openings.push((pad.layer, ring, bounds));
            }
        }
    }
    let graphics = layout::placed_graphics(project);
    // Openings near each stroke from a bounds grid, not every opening;
    // at most the copper checks' pair budget for this many shapes
    // (`layout::check_pair_budget`), which only ink piled on pads reaches.
    let grid = layout::grid::BoundsGrid::new(openings.iter().map(|o| Some(o.2)), 0.0);
    let budget = layout::check_pair_budget(openings.len().saturating_add(graphics.len()));
    let mut near = Vec::new();
    let mut flagged: BTreeSet<(Id, Layer)> = BTreeSet::new();
    let mut pairs = 0usize;
    for graphic in graphics {
        let BoardDrawing::Silkscreen(side) = graphic.layer else {
            continue;
        };
        if flagged.contains(&(graphic.instance, side)) {
            continue;
        }
        let strokes: Vec<(Vec<Point>, f64)> = match &graphic.item {
            PlacedItem::Shape { shape, width, .. } => vec![(shape.flatten(tol), *width)],
            PlacedItem::Text(text) => {
                let s = layout::legend::text_strokes(text, &rules.silkscreen);
                s.strokes.into_iter().map(|p| (p, s.width)).collect()
            }
        };
        'strokes: for (points, width) in strokes {
            let Some(bounds) = Bounds::of(points.iter().copied()) else {
                continue;
            };
            let bounds = bounds.inflated(width / 2.0);
            grid.near(&bounds, 0.0, &mut near);
            for (s, ring, b) in near.iter().filter_map(|k| openings.get(*k)) {
                if *s != side || !b.intersects(&bounds) {
                    continue;
                }
                pairs += 1;
                if pairs > budget {
                    items.push(item(
                        "fabrication.silk_check_limit",
                        Severity::Warning,
                        "The legend-over-pad check stopped at its limit".into(),
                        None,
                        Vec::new(),
                    ));
                    return;
                }
                let path: Vec<Point> = if points.len() == 1 {
                    vec![points[0], points[0]]
                } else {
                    points.clone()
                };
                let hits = polyline_polygon_distance(&path, ring) < width / 2.0;
                if hits {
                    flagged.insert((graphic.instance, side));
                    break 'strokes;
                }
            }
        }
    }
    let names: BTreeMap<Id, &str> = project
        .instances
        .iter()
        .map(|i| (i.id, i.designator.as_str()))
        .collect();
    for (instance, side) in flagged {
        items.push(item(
            "fabrication.silk_over_pad",
            Severity::Warning,
            format!(
                "Legend of {} runs into a pad opening on the {} side",
                names.get(&instance).copied().unwrap_or("a part"),
                match side {
                    Layer::Top => "top",
                    Layer::Bottom => "bottom",
                }
            ),
            Some("Move the legend clear of the pads; the fabricator clips ink in openings".into()),
            vec![ObjectRef::Instance { id: instance }],
        ));
    }
}
