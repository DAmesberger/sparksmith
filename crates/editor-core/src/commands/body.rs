//! `library.replace_body`: a placed part's 3D body made by a generator,
//! its footprint kept (docs/generator-plugins.md, "A body for an existing
//! part"). The generator's package-scope generation gives the body (its
//! part file, evaluated by the host, and its lead contacts); the leads are
//! fitted onto the pads of every footprint that lands the package
//! ([`package_fit::fit_leads`]) and each must land on its pad as the board
//! check's `package.lead_off_land` rule reads it ([`package_fit::lead_states`]).
//! Applied, the package's next revision holds only a new model, placement,
//! envelope and lead contacts; its footprints, bindings and components are
//! republished pinning it, unchanged otherwise, and every placed part that
//! used the package moves to them, as one undo step. Terminals, pads and
//! the electrical definition never change.
use super::Outcome;
use super::generated::{Generate, model_of, prepare, runtime_needed, supplied_model};
use crate::diagnostics::{Error, Result};
use crate::geometry::Point;
use crate::ids::{
    BindingRef, ComponentRef, DefinitionId, DefinitionRef, FootprintRef, Id, ModelRef, PackageRef,
    PackageTerminalId, Revision,
};
use crate::library::generated::{self, GeneratorRuntime, GeneratorScope};
use crate::library::package_fit::{self, FitRotation, LeadPair};
use crate::library::package_model::{self, ModelPlacement, PartFileModel};
use crate::library::{
    Definition, DefinitionMeta, FootprintDefinition, MAX_NOTE, PackageDefinition, PackageLead,
    PackageModel, PadKind, drafts,
};
use crate::model::Document;
use crate::units::{Length, Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// How `library.make_body` opens the provenance it records: what the review
/// check looks for.
pub const RECREATED: &str = "3D body recreated by AI from";

/// The largest miss a request may allow beyond the board check's rule.
pub const MAX_TOLERANCE_NM: i64 = 500_000;
/// Lead-to-pad mappings one request gives.
pub const MAX_PINS: usize = crate::library::MAX_PACKAGE_TERMINALS;
/// Bytes of one pin name in a mapping.
const MAX_PIN_NAME: usize = 64;

/// One of the body's leads and the pad it lands on.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LeadPin {
    /// The generator's package terminal (`1`, `EP`).
    pub lead: String,
    /// The existing package terminal's designation: the footprint's pad
    /// number (`1`, `A`).
    pub pad: String,
}

/// `library.replace_body`'s parameters.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReplaceBody {
    /// The package revision whose body changes: every placed part that uses
    /// it changes with it.
    pub package: PackageRef,
    /// The package-scope generation that makes the body, its part file
    /// evaluated by the host (`library.make_body` fills it).
    pub generate: Generate,
    /// Which pad each lead lands on. A lead not listed lands the pad of its
    /// own number.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<LeadPin>,
    /// How far, in mm, a lead contact may miss its land beyond the board
    /// check's rule (on the land, within the middle half across it); 0–0.5,
    /// default 0.
    #[serde(default)]
    pub tolerance: Length,
    /// Where the body's values come from and who made it, recorded with
    /// the package and its model (at most 4 KiB).
    pub provenance: String,
}

/// Where one lead lands.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LeadLanding {
    pub lead: String,
    pub pad: String,
    /// How far the contact misses its land past the rule, mm; ≤ 0 lands.
    pub miss_mm: f64,
    /// The contact's distance from its pad's centre after the fit, mm.
    pub off_centre_mm: f64,
}

/// A body: its model and envelope, for the before/after of a plan.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodySummary {
    pub model: String,
    /// Length, width and height, mm.
    pub envelope_mm: [f64; 3],
}

/// Why a plan does or does not apply.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodyFinding {
    pub code: String,
    pub message: String,
    pub blocks: bool,
}

/// What `library.replace_body` would do: `library.body_fit`'s answer and
/// the outcome's plan.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodyPlan {
    /// Every lead lands and nothing blocks: the body may be applied.
    pub accepted: bool,
    pub package: PackageRef,
    /// The package's name.
    pub name: String,
    /// The placed parts that use the package, by designator.
    pub parts: Vec<String>,
    /// The body in the package's datum frame, as fitted.
    #[serde(with = "package_model::model_placement_mm")]
    #[schemars(with = "package_model::ModelPlacementParams")]
    pub placement: ModelPlacement,
    pub tolerance_mm: f64,
    /// Each lead on the footprint the fit used, worst first.
    pub leads: Vec<LeadLanding>,
    /// Copper pads no lead lands (mounting tabs, shields): reported only.
    pub unlanded: Vec<String>,
    pub before: BodySummary,
    pub after: BodySummary,
    pub findings: Vec<BodyFinding>,
    /// What applying it makes, by noun.
    pub change_counts: BTreeMap<String, u64>,
}

/// `library.replace_body`'s outcome.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BodyReplaced {
    /// The package's new revision.
    pub package: PackageRef,
    pub model: ModelRef,
    /// Every revision published, the model's first when it is new.
    pub revisions: Vec<DefinitionRef>,
    pub plan: BodyPlan,
}

/// A placed part that uses the package.
struct User {
    instance: Id,
    designator: String,
    component: ComponentRef,
    binding: BindingRef,
    footprint: FootprintRef,
}

/// Every placed part whose chosen implementation lands `package`.
fn users(document: &Document, package: PackageRef) -> Vec<User> {
    let project = &document.project;
    let library = &project.library;
    project
        .instances
        .iter()
        .filter_map(|instance| {
            let choice = instance.physical?;
            let component = library.component(instance.component)?;
            let view =
                crate::instances::resolve_physical(library, &component.definition, choice).ok()?;
            (view.binding.definition.package == package).then(|| User {
                instance: instance.id,
                designator: instance.designator.clone(),
                component: instance.component,
                binding: BindingRef::new(view.binding.id, view.binding.revision),
                footprint: FootprintRef::new(view.footprint.id, view.footprint.revision),
            })
        })
        .collect()
}

/// The placed parts that use `package`, by designator.
pub fn parts_using(document: &Document, package: PackageRef) -> Vec<String> {
    users(document, package)
        .into_iter()
        .map(|u| u.designator)
        .collect()
}

/// The package revision the placed part `designator` lands, for a host
/// that names the part as a person does.
pub fn package_of(document: &Document, designator: &str) -> Result<PackageRef> {
    let project = &document.project;
    let instance = project
        .instances
        .iter()
        .find(|i| i.designator == designator)
        .ok_or_else(|| Error::NotFound(format!("body.no_part: there is no part {designator}")))?;
    let choice = instance.physical.ok_or_else(|| {
        Error::Invalid(format!(
            "body.no_footprint: {designator} has no footprint chosen"
        ))
    })?;
    let component = project
        .library
        .component(instance.component)
        .ok_or_else(|| {
            Error::NotFound(format!(
                "body.no_part: the component of {designator} is not in the project"
            ))
        })?;
    let view = crate::instances::resolve_physical(&project.library, &component.definition, choice)?;
    Ok(view.binding.definition.package)
}

fn copper(kind: PadKind) -> bool {
    matches!(kind, PadKind::Smd | PadKind::PlatedThrough)
}

fn finding(code: &str, message: String) -> BodyFinding {
    BodyFinding {
        code: code.into(),
        message,
        blocks: true,
    }
}

/// The plan and the package revision it would publish.
struct Staged {
    plan: BodyPlan,
    package: PackageDefinition,
    meta: DefinitionMeta,
    users: Vec<User>,
}

/// What `request` would do to `document`, refused only when it cannot be
/// planned at all (no runtime, a package no part uses, a generation that
/// fails); a body that does not fit is a plan with blocking findings.
pub fn plan(
    document: &Document,
    request: &ReplaceBody,
    runtime: Option<&dyn GeneratorRuntime>,
) -> Result<BodyPlan> {
    stage(document, request, runtime).map(|s| s.plan)
}

fn stage(
    document: &Document,
    request: &ReplaceBody,
    runtime: Option<&dyn GeneratorRuntime>,
) -> Result<Staged> {
    let runtime = runtime.ok_or_else(runtime_needed)?;
    if request.generate.scope != GeneratorScope::Package {
        return Err(Error::Invalid(
            "body.scope: a body comes from a package-scope generation".into(),
        ));
    }
    if request.provenance.trim().is_empty() || request.provenance.len() > MAX_NOTE {
        return Err(Error::Invalid(format!(
            "body.provenance: say where the body's values come from, in 1–{MAX_NOTE} bytes"
        )));
    }
    let tolerance = request.tolerance.0;
    if tolerance < Nm::ZERO || tolerance.get() > MAX_TOLERANCE_NM {
        return Err(Error::Invalid(
            "body.tolerance: the tolerance is 0–0.5 mm".into(),
        ));
    }
    if request.pins.len() > MAX_PINS
        || request
            .pins
            .iter()
            .any(|p| p.lead.len() > MAX_PIN_NAME || p.pad.len() > MAX_PIN_NAME)
    {
        return Err(Error::Limit(format!(
            "body.pins: at most {MAX_PINS} mappings of names up to {MAX_PIN_NAME} bytes"
        )));
    }
    let library = &document.project.library;
    let published = library.package(request.package).ok_or_else(|| {
        Error::NotFound(format!(
            "body.package: {} is not in the project",
            library.describe(request.package.erased())
        ))
    })?;
    let users = users(document, request.package);
    let first = users.first().ok_or_else(|| {
        Error::Invalid(format!(
            "body.unused: no placed part uses the package ‘{}’ {}",
            published.meta.name, request.package.revision
        ))
    })?;
    let prepared = prepare(document, &request.generate, runtime)?;
    let made = runtime
        .generate(&prepared.bytes, &prepared.wire)
        .map_err(|f| generated::failure(&f))?;
    let generation = &made.generation;
    let supplied = supplied_model(&request.generate, generation)?;
    let old = &published.definition;
    let mut findings = Vec::new();

    // Each lead's pad: the mapping, else its own number.
    let keys: BTreeSet<&str> = generation
        .package
        .terminals
        .iter()
        .map(String::as_str)
        .collect();
    let mut mapped: BTreeMap<&str, &str> = BTreeMap::new();
    for pin in &request.pins {
        if !keys.contains(pin.lead.as_str()) {
            return Err(Error::Invalid(format!(
                "body.pins: the body has no lead {}; its leads are {}",
                pin.lead,
                keys.iter().copied().collect::<Vec<_>>().join(", ")
            )));
        }
        if mapped.insert(&pin.lead, &pin.pad).is_some() {
            return Err(Error::Invalid(format!(
                "body.pins: lead {} is mapped twice",
                pin.lead
            )));
        }
    }
    let pad_of = |key: &str| mapped.get(key).copied().unwrap_or(key).to_owned();
    let geometry = &generation.package.geometry;
    if geometry.leads.is_empty() {
        return Err(Error::Invalid(
            "body.no_leads: the generator gives no lead contacts to fit to the pads".into(),
        ));
    }
    let mut leads = Vec::with_capacity(geometry.leads.len());
    let mut named: BTreeMap<PackageTerminalId, String> = BTreeMap::new();
    for lead in &geometry.leads {
        let pad = pad_of(&lead.key);
        match old.terminals.iter().find(|t| t.designation == pad) {
            Some(terminal) => {
                leads.push(PackageLead {
                    terminal: terminal.id,
                    contact: PointNm::new(Nm::new(lead.contact[0])?, Nm::new(lead.contact[1])?),
                });
                if let Some(other) = named.insert(terminal.id, lead.key.clone())
                    && other != lead.key
                {
                    findings.push(finding(
                        "body.pin_twice",
                        format!("Leads {other} and {} both land pad {pad}.", lead.key),
                    ));
                }
            }
            None => findings.push(finding(
                "body.no_pad",
                format!(
                    "The body's lead {} lands pad {pad}, which the footprint does not have; map it with pins.",
                    lead.key
                ),
            )),
        }
    }

    // The package as it would be: the generator's body on the old
    // terminals, its lead bodies bound to the pads they land.
    let mut derived = supplied.derived.clone();
    derived.leads.retain_mut(|l| {
        let pad = pad_of(&l.terminal);
        let lands = old.terminals.iter().any(|t| t.designation == pad);
        l.terminal = pad;
        lands
    });
    derived.leads.sort();
    let [length, width, height] = geometry.envelope_nm;
    let mut package = old.clone();
    package.envelope_nm = [Nm::new(length)?, Nm::new(width)?, Nm::new(height)?];
    package.leads = leads;
    package.model = PackageModel::PartFile(Box::new(PartFileModel {
        model: ModelRef::new(DefinitionId(0), Revision::FIRST),
        values: supplied.values.clone(),
        derived,
    }));

    // The fit on the first part's footprint: each lead terminal's contacts'
    // centre onto its copper pads' centre, in the datum frame.
    let footprint = &library
        .footprint(first.footprint)
        .ok_or_else(|| Error::NotFound(format!("body.footprint: {} is missing", first.footprint)))?
        .definition;
    let pairs = pairs(&package, footprint, &mut findings)?;
    let placement = if pairs.is_empty() {
        ModelPlacement::default()
    } else {
        let fit = package_fit::fit_leads(&ModelPlacement::default(), &pairs, FitRotation::Free)?;
        if fit.mirrored_better {
            findings.push(finding(
                "body.mirrored",
                "The leads land better mirrored: the body's pin order runs the other way round the pads (a bottom view read as a top view?).".into(),
            ));
        }
        let mut placement = fit.placement;
        placement.translation_nm[2] = Nm::new(geometry.seating_nm)?;
        placement
    };
    package.placement = placement;

    // Every lead on its land, on every footprint the parts use.
    let mut checked = BTreeSet::new();
    let mut landings = Vec::new();
    let mut unlanded = Vec::new();
    for user in &users {
        if !checked.insert(user.footprint) {
            continue;
        }
        let Some(f) = library.footprint(user.footprint) else {
            continue;
        };
        let lands = land(
            &package,
            &f.definition,
            (tolerance, &f.meta.name),
            &named,
            &mut findings,
        );
        if user.footprint == first.footprint {
            landings = lands;
            unlanded = f
                .definition
                .pads
                .iter()
                .filter(|p| copper(p.kind))
                .filter(|p| {
                    p.terminal
                        .is_none_or(|t| !package.leads.iter().any(|l| l.terminal == t))
                })
                .map(|p| p.number.clone())
                .collect();
            unlanded.sort();
            unlanded.dedup();
        }
    }
    landings.sort_by(|a: &LeadLanding, b| b.miss_mm.total_cmp(&a.miss_mm));

    let footprints: BTreeSet<FootprintRef> = republished_footprints(document, &users);
    let bindings: BTreeSet<BindingRef> = users.iter().map(|u| u.binding).collect();
    let components: BTreeSet<ComponentRef> = users.iter().map(|u| u.component).collect();
    let (held, _) = model_of(document, supplied)?;
    let mut change_counts = BTreeMap::new();
    change_counts.insert("placed part".to_owned(), users.len() as u64);
    change_counts.insert("package revision".to_owned(), 1);
    change_counts.insert("footprint revision".to_owned(), footprints.len() as u64);
    change_counts.insert("binding revision".to_owned(), bindings.len() as u64);
    change_counts.insert("component revision".to_owned(), components.len() as u64);
    change_counts.insert("new 3D model".to_owned(), u64::from(held.is_none()));
    let title = prepared.description.title.clone();
    let preset = request
        .generate
        .preset
        .as_ref()
        .map(|p| format!(" ({p})"))
        .unwrap_or_default();
    let summary = |p: &PackageDefinition, model: String| BodySummary {
        model,
        envelope_mm: p.envelope(),
    };
    let plan = BodyPlan {
        accepted: !findings.iter().any(|f| f.blocks),
        package: request.package,
        name: published.meta.name.clone(),
        parts: users.iter().map(|u| u.designator.clone()).collect(),
        placement,
        tolerance_mm: tolerance.to_mm_f64(),
        leads: landings,
        unlanded,
        before: summary(old, old.model.describe()),
        after: summary(
            &package,
            format!("generated body: {title}{preset}, {}", supplied.name),
        ),
        findings,
        change_counts,
    };
    let mut meta = published.meta.clone();
    meta.provenance = super::generated::bounded(if meta.provenance.is_empty() {
        request.provenance.clone()
    } else {
        format!("{}\n{}", meta.provenance, request.provenance)
    });
    Ok(Staged {
        plan,
        package,
        meta,
        users,
    })
}

/// Each lead terminal's contacts' centre with its copper pads' centre on
/// `footprint`, in the datum frame; a terminal without a copper pad is a
/// blocking finding.
fn pairs(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    findings: &mut Vec<BodyFinding>,
) -> Result<Vec<LeadPair>> {
    let mut contacts: BTreeMap<PackageTerminalId, Vec<Point>> = BTreeMap::new();
    for lead in &package.leads {
        contacts
            .entry(lead.terminal)
            .or_default()
            .push(lead.contact.to_mm_f64());
    }
    let centre = |points: &[Point]| {
        let n = points.len().max(1) as f64;
        let [x, y] = points
            .iter()
            .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
        [x / n, y / n]
    };
    let mut out = Vec::new();
    for (terminal, points) in contacts {
        let pads: Vec<_> = footprint
            .pads
            .iter()
            .filter(|p| p.terminal == Some(terminal) && copper(p.kind))
            .collect();
        let Some(first) = pads.first() else {
            let name = package
                .terminal(terminal)
                .map(|t| t.designation.clone())
                .unwrap_or_default();
            findings.push(finding(
                "body.no_pad",
                format!("No copper pad of the footprint lands terminal {name}."),
            ));
            continue;
        };
        let at: Vec<Point> = pads
            .iter()
            .map(|p| package_fit::to_datum(&footprint.datum, p.position()))
            .collect();
        let [x, y] = centre(&points);
        out.push(LeadPair::from_mm(
            terminal,
            first.id,
            [x, y, 0.0],
            centre(&at),
        )?);
    }
    Ok(out)
}

/// Where each lead lands on `footprint` under the package's placement; a
/// lead past the rule and `tolerance` is a blocking finding.
fn land(
    package: &PackageDefinition,
    footprint: &FootprintDefinition,
    (tolerance, name): (Nm, &str),
    leads: &BTreeMap<PackageTerminalId, String>,
    findings: &mut Vec<BodyFinding>,
) -> Vec<LeadLanding> {
    let allowed = tolerance.to_mm_f64();
    let mut out = Vec::new();
    for state in package_fit::lead_states(package, footprint, &package.placement) {
        let pad = package
            .terminal(state.terminal)
            .map(|t| t.designation.clone())
            .unwrap_or_default();
        let lead = leads
            .get(&state.terminal)
            .cloned()
            .unwrap_or_else(|| pad.clone());
        let Some(miss) = state.miss_mm else {
            continue;
        };
        let at = state.at;
        let off_centre = footprint
            .pads
            .iter()
            .filter(|p| state.pads.contains(&p.id))
            .map(|p| {
                let c = package_fit::to_datum(&footprint.datum, p.position());
                (at[0] - c[0]).hypot(at[1] - c[1])
            })
            .fold(f64::INFINITY, f64::min);
        if miss > allowed + 1e-9 {
            findings.push(finding(
                "body.lead_off_land",
                format!(
                    "Lead {lead} misses pad {pad} of ‘{name}’ by {miss:.3} mm past the rule (the contact on the land, within the middle half across it), more than the {allowed:.3} mm allowed."
                ),
            ));
        }
        out.push(LeadLanding {
            lead,
            pad,
            miss_mm: miss,
            off_centre_mm: if off_centre.is_finite() {
                off_centre
            } else {
                0.0
            },
        });
    }
    out
}

/// Every footprint of the bindings the parts use: each pins the package,
/// so each is republished pinning its next revision.
fn republished_footprints(document: &Document, users: &[User]) -> BTreeSet<FootprintRef> {
    let library = &document.project.library;
    users
        .iter()
        .filter_map(|u| library.binding(u.binding))
        .flat_map(|b| b.definition.footprints.iter().copied())
        .collect()
}

/// `library.replace_body`.
pub(crate) fn replace_body(
    document: &mut Document,
    request: ReplaceBody,
    runtime: Option<&dyn GeneratorRuntime>,
    evaluator: Option<&dyn crate::library::FootprintEvaluator>,
) -> Result<Outcome> {
    let Staged {
        plan,
        mut package,
        meta,
        users,
    } = stage(document, &request, runtime)?;
    if !plan.accepted {
        let why: Vec<&str> = plan
            .findings
            .iter()
            .filter(|f| f.blocks)
            .map(|f| f.message.as_str())
            .collect();
        return Err(Error::Invalid(format!(
            "body.misfit: the body does not fit the footprint: {} Nothing was changed.",
            why.join(" ")
        )));
    }
    // The lineages republished, each from the revision the parts use,
    // which must be its newest and have no draft.
    let mut wanted: Vec<DefinitionRef> = vec![request.package.erased()];
    wanted.extend(
        republished_footprints(document, &users)
            .into_iter()
            .map(|r| r.erased()),
    );
    let bindings: BTreeSet<BindingRef> = users.iter().map(|u| u.binding).collect();
    let components: BTreeSet<ComponentRef> = users.iter().map(|u| u.component).collect();
    wanted.extend(bindings.iter().map(|r| r.erased()));
    wanted.extend(components.iter().map(|r| r.erased()));
    let library = &document.project.library;
    for r in &wanted {
        let name = library.describe(*r);
        if document.drafts.get(r.id).is_some() {
            return Err(Error::Conflict(format!(
                "body.draft_open: {name} has a draft; publish or discard it first. Nothing was changed."
            )));
        }
        if library.latest(r.id).map(|(_, revision)| revision) != Some(r.revision) {
            return Err(Error::Conflict(format!(
                "body.not_newest: the parts use {name}, and a newer revision exists; update them first. Nothing was changed."
            )));
        }
    }
    let supplied = request.generate.model.as_ref().ok_or_else(|| {
        Error::Invalid("generated.model_unevaluated: the body's part file was not evaluated by the host. Nothing was changed.".into())
    })?;
    let (held, new_model) = model_of(document, supplied)?;
    let mut publish = Vec::with_capacity(wanted.len() + 1);
    let model = match held {
        Some(model) => model,
        None => {
            let (mut model_meta, definition, fresh) = new_model
                .ok_or_else(|| Error::Invalid("body.model: the body's model is missing".into()))?;
            let project = Arc::make_mut(&mut document.project);
            let id = DefinitionId(project.allocate()?);
            if project.library.assets.len() + fresh.len() > crate::library::bundle::MAX_ASSETS {
                return Err(Error::Limit(format!(
                    "a project packs at most {} assets",
                    crate::library::bundle::MAX_ASSETS
                )));
            }
            project.library.assets.extend(fresh);
            model_meta.provenance = super::generated::bounded(request.provenance.clone());
            drafts::create(
                Arc::make_mut(&mut document.drafts),
                &document.project.library,
                id,
                None,
                model_meta,
                definition,
            )?;
            publish.push(id);
            ModelRef::new(id, Revision::FIRST)
        }
    };
    if let PackageModel::PartFile(part) = &mut package.model {
        part.model = model;
    }
    for r in &wanted {
        let (meta, definition) = document
            .project
            .library
            .definition(*r)
            .ok_or_else(|| Error::NotFound(format!("body.lineage: {} is missing", r.id)))?;
        drafts::create(
            Arc::make_mut(&mut document.drafts),
            &document.project.library,
            r.id,
            Some(r.revision),
            meta,
            definition,
        )?;
        publish.push(r.id);
    }
    super::library::pin_pending(document, &wanted);
    drafts::edit(
        Arc::make_mut(&mut document.drafts),
        vec![crate::library::DraftEdit {
            id: request.package.id,
            meta,
            definition: Definition::Package(package),
        }],
    )?;
    let Outcome::Published { revisions } =
        super::library::publish_drafts(document, evaluator, publish, true)?
    else {
        return Err(Error::Invalid("body.publish: nothing was published".into()));
    };
    let next = |id: DefinitionId| {
        revisions
            .iter()
            .find(|r| r.id == id)
            .map(|r| r.revision)
            .ok_or_else(|| Error::Invalid(format!("body.publish: {id} was not published")))
    };
    let project = Arc::make_mut(&mut document.project);
    for component in &components {
        let target = ComponentRef::new(component.id, next(component.id)?);
        let ids: Vec<Id> = users
            .iter()
            .filter(|u| u.component == *component)
            .map(|u| u.instance)
            .collect();
        crate::updates::move_instances(project, &ids, target)?;
    }
    let package = PackageRef::new(request.package.id, next(request.package.id)?);
    Ok(Outcome::BodyReplaced {
        report: Box::new(BodyReplaced {
            package,
            model,
            revisions,
            plan,
        }),
    })
}
