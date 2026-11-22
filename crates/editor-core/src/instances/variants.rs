//! Assembly variants (docs/assembly-variants.md): which part each variant
//! fits on each footprint, and the commands that edit them.
//!
//! [`fitted`] is the one answer every consumer reads (the 3D view and the
//! exact export, the BOM, pick-and-place, fabrication output and the body
//! checks): whether a part is populated under the active variant, and the
//! Part and physical implementation fitted. The copper never changes
//! between variants: a substitute must sit on the same land pattern with
//! the same pad grouping ([`substitute_fit`]).
use super::{PhysicalView, designator_key, instance_mut, invalid, resolve};
use crate::diagnostics::{Error, Finding, ObjectRef, Result};
use crate::fields;
use crate::ids::{ComponentRef, FootprintRef, Id, PartRef, TerminalId};
use crate::library::{FootprintDefinition, Library, Pad, PartDefinition};
use crate::model::variants::{
    MAX_EXCEPTION_REASON, MAX_EXCEPTIONS, MAX_VARIANT_NAME, MAX_VARIANTS, RequirementException,
    SubstituteReview,
};
use crate::model::{
    AssemblyVariant, AssemblyVariants, ComponentInstance, Population, Project, VariantOverride,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Instances one `variant.set_population` names.
pub const MAX_POPULATION_EDIT: usize = 4096;

/// The finding of a substitute that no longer fits its footprint (a library
/// update changed the part's footprint or the substitute's binding).
pub const SUBSTITUTE_UNFIT: &str = "variant.substitute_unfit";

/// The finding of a substitute nobody has reviewed as electrically
/// equivalent to the part it replaces: it fits the lands, which says
/// nothing about what each pin does.
pub const SUBSTITUTE_UNREVIEWED: &str = "variant.substitute_unreviewed";

/// The finding of a substitute that does not meet, rate or compare with a
/// requirement of the part it replaces, and no review excepts it.
pub const SUBSTITUTE_UNMET: &str = "variant.substitute_unmet";

/// Edit assembly variants. Each changing command is one undo step; one
/// that changes nothing is none.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum VariantCommand {
    /// Add an assembly variant named `name` (unique, case-insensitively),
    /// empty or, with `copy_of`, a copy of that variant's overrides. The
    /// outcome names the new variant.
    #[serde(rename = "variant.add")]
    Add {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        copy_of: Option<Id>,
    },
    /// Rename a variant.
    #[serde(rename = "variant.rename")]
    Rename { variant: Id, name: String },
    /// Delete a variant and its overrides. When it was active, the default
    /// becomes active.
    #[serde(rename = "variant.remove")]
    Remove { variant: Id },
    /// Choose the variant the 3D view, BOM, pick-and-place and fabrication
    /// output follow; none (null) is the default.
    #[serde(rename = "variant.set_active")]
    SetActive {
        #[serde(default)]
        variant: Option<Id>,
    },
    /// Set whether parts are fitted: in the default (no `variant`) or in
    /// one variant. In a variant, a null population follows the default
    /// again. Refused whole when an instance is unknown.
    #[serde(rename = "variant.set_population")]
    SetPopulation {
        instances: Vec<Id>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variant: Option<Id>,
        population: Option<Population>,
    },
    /// Fit another manufacturer part on a part's footprint in one variant;
    /// null fits the default part again. The Part must be in the project
    /// library and sit on the same land pattern with the same pad grouping,
    /// so the copper and nets never change; anything else is refused.
    #[serde(rename = "variant.set_part")]
    SetPart {
        instance: Id,
        variant: Id,
        #[serde(default)]
        part: Option<PartRef>,
    },
    /// Record a person's review of a variant's substitute part. It says
    /// whether the substitute is electrically equivalent to the part it
    /// replaces (its pinout, beyond the land pattern and pad grouping that
    /// fitting checks), and which requirements it is deliberately accepted
    /// not to meet, each with its reason. Not equivalent and no exceptions
    /// clears the review. The review is bound to the substitute; fitting
    /// another Part voids it.
    #[serde(rename = "variant.review_substitute")]
    ReviewSubstitute {
        instance: Id,
        variant: Id,
        equivalent: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        exceptions: Vec<RequirementException>,
    },
}

// A new command chooses its permission class here.
declare_entries!(VariantCommand in project {
    Add "variant.add" Edit "Add an assembly variant" [since(7, 13)],
    Rename "variant.rename" Edit "Rename an assembly variant" [since(7, 13)],
    Remove "variant.remove" Edit "Delete an assembly variant" [since(7, 13)],
    SetActive "variant.set_active" Edit "Choose the active assembly variant" [since(7, 13)],
    SetPopulation "variant.set_population" Edit "Set whether parts are fitted" [since(7, 13)],
    SetPart "variant.set_part" Edit "Fit another part in a variant" [since(7, 13)],
    ReviewSubstitute "variant.review_substitute" Ask "Review a variant's substitute part" [since(7, 18)],
});

/// What one variant fits for one part.
#[derive(Clone, Copy)]
pub struct Fitted<'a> {
    pub population: Population,
    /// The Part fitted: the variant's substitute, else the instance's own.
    pub part: Option<PartRef>,
    /// The fitted component revision: the substitute's, else the instance's.
    pub component: ComponentRef,
    /// The fitted physical implementation: the substitute's, else the
    /// instance's own (`None` without a physical choice).
    pub physical: Option<PhysicalView<'a>>,
    /// A substitute is fitted instead of the default part.
    pub substituted: bool,
}

impl Fitted<'_> {
    pub fn populated(&self) -> bool {
        self.population.is_populated()
    }

    /// The exact footprint revision the fitted body is placed by.
    pub fn footprint(&self) -> Option<FootprintRef> {
        self.physical
            .map(|p| FootprintRef::new(p.footprint.id, p.footprint.revision))
    }
}

/// An instance's population in `variant` (`None`: the default).
pub fn population(instance: &ComponentInstance, variant: Option<&AssemblyVariant>) -> Population {
    variant
        .and_then(|v| v.overrides.get(&instance.id))
        .and_then(|o| o.population)
        .unwrap_or(instance.population)
}

/// What the active variant fits for `instance`.
pub fn fitted<'a>(project: &'a Project, instance: &ComponentInstance) -> Fitted<'a> {
    fitted_in(project, instance, project.variants.active())
}

/// What `variant` (`None`: the default) fits for `instance`. A substitute
/// that no longer fits its footprint is not fitted: the default part is,
/// and [`findings`] names it.
pub fn fitted_in<'a>(
    project: &'a Project,
    instance: &ComponentInstance,
    variant: Option<&AssemblyVariant>,
) -> Fitted<'a> {
    let library = &project.library;
    let own = resolve(library, instance).ok().and_then(|r| r.physical);
    let substitute = variant
        .and_then(|v| v.overrides.get(&instance.id))
        .and_then(|o| o.part)
        .and_then(|r| {
            let physical = substitute_fit(library, instance, r).ok()?;
            let part = library.part(r)?;
            Some((r, part.definition.component, physical))
        });
    match substitute {
        Some((part, component, physical)) => Fitted {
            population: population(instance, variant),
            part: Some(part),
            component,
            physical: Some(physical),
            substituted: true,
        },
        None => Fitted {
            population: population(instance, variant),
            part: instance.part,
            component: instance.component,
            physical: own,
            substituted: false,
        },
    }
}

/// The value text the fitted part prints in the BOM and pick-and-place: the
/// instance's own value ([`super::value_text`]) for the default part; for
/// a substitute, its rated specs through its component's value template,
/// else its MPN.
pub fn fitted_value(
    project: &Project,
    instance: &ComponentInstance,
    fitted: &Fitted<'_>,
) -> Result<Option<String>> {
    let library = &project.library;
    if !fitted.substituted {
        return super::value_text(library, instance);
    }
    let Some(part) = fitted.part.and_then(|r| library.part(r)) else {
        return Ok(None);
    };
    let text = part_value(library, &part.definition);
    Ok(text.or_else(|| Some(part.definition.mpn.trim().to_owned()).filter(|m| !m.is_empty())))
}

/// A Part's rated specs as value text.
fn part_value(library: &Library, part: &PartDefinition) -> Option<String> {
    let texts: BTreeMap<crate::ids::DefinitionId, String> = part
        .specs
        .iter()
        .filter_map(|spec| {
            let view = library.field_view(spec.field)?;
            let text = fields::display(
                &spec.value,
                &view.field.definition.value_type,
                view.kind_body(),
            );
            Some((spec.field.id, text))
        })
        .collect();
    if let Some(template) = library
        .component(part.component)
        .and_then(|c| c.definition.value_template.as_ref())
    {
        return template.render(|lineage| texts.get(&lineage).cloned());
    }
    let joined: Vec<String> = part
        .specs
        .iter()
        .filter_map(|s| texts.get(&s.field.id))
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .collect();
    (!joined.is_empty()).then(|| joined.join(" "))
}

/// The physical implementation a substitute Part brings to `instance`'s
/// footprint, or why it does not fit: the Part must be in the project
/// library with its component and binding, and the binding must offer a
/// footprint with the instance's land pattern (the same pads, holes, mask,
/// paste and datum) whose pads group into terminals exactly as the
/// instance's do, so every pad keeps its net and no pads are joined or
/// split. Pin names are never compared.
pub fn substitute_fit<'a>(
    library: &'a Library,
    instance: &ComponentInstance,
    r: PartRef,
) -> Result<PhysicalView<'a>> {
    let name = &instance.designator;
    let own = resolve(library, instance)?.physical.ok_or_else(|| {
        Error::Conflict(format!(
            "{name} has no footprint, so no other part can be fitted on it."
        ))
    })?;
    let part = library.part(r).ok_or_else(|| {
        Error::NotFound(format!(
            "That Part (revision {}) is not in the project library; add it to the project first.",
            r.revision
        ))
    })?;
    let mpn = &part.definition.mpn;
    let component = library
        .component(part.definition.component)
        .ok_or_else(|| {
            invalid(format!(
                "{mpn}'s component is missing from the project library."
            ))
        })?;
    if component
        .definition
        .implementation(part.definition.binding.id)
        != Some(part.definition.binding)
    {
        return Err(Error::Conflict(format!(
            "{mpn}'s implementation is not one its component offers."
        )));
    }
    let binding = library.binding(part.definition.binding).ok_or_else(|| {
        invalid(format!(
            "{mpn}'s implementation is missing from the project library."
        ))
    })?;
    let target = &own.footprint.definition;
    let mut land_match = None;
    for f in &binding.definition.footprints {
        let Some(candidate) = library.footprint(*f) else {
            continue;
        };
        let exact =
            candidate.id == own.footprint.id && candidate.revision == own.footprint.revision;
        if exact || same_lands(target, &candidate.definition) {
            land_match = Some(candidate);
            if exact {
                break;
            }
        }
    }
    let footprint = land_match.ok_or_else(|| {
        Error::Conflict(format!(
            "{mpn} does not fit {name}: none of its footprints has {name}'s land pattern ({})",
            own.footprint.meta.name
        ))
    })?;
    let package = library.package(binding.definition.package).ok_or_else(|| {
        invalid(format!(
            "{mpn}'s package is missing from the project library."
        ))
    })?;
    let view = PhysicalView {
        binding,
        footprint,
        package,
    };
    let ours = pad_groups(&own, target, &own_order(target));
    let theirs = pad_groups(
        &view,
        &footprint.definition,
        &matched_order(target, &footprint.definition),
    );
    if ours != theirs {
        return Err(Error::Conflict(format!(
            "{mpn} does not fit {name}: its pins join or split {name}'s pads differently, \
             so the board's nets would change"
        )));
    }
    Ok(view)
}

/// A pad with what identifies it in its own definition taken away: its
/// local id and package terminal.
fn land(pad: &Pad) -> Pad {
    Pad {
        id: Default::default(),
        terminal: None,
        ..pad.clone()
    }
}

fn land_order(footprint: &FootprintDefinition) -> Vec<usize> {
    let mut order: Vec<usize> = (0..footprint.pads.len()).collect();
    order.sort_by(|a, b| {
        let (pa, pb) = (footprint.pads.get(*a), footprint.pads.get(*b));
        let key = |p: Option<&Pad>| p.map(|p| (p.number.clone(), p.position.x_nm, p.position.y_nm));
        key(pa).cmp(&key(pb))
    });
    order
}

/// Whether two footprints have the same land pattern, pads compared by
/// number and position whatever order they are stored in.
fn same_lands(a: &FootprintDefinition, b: &FootprintDefinition) -> bool {
    if a.pads.len() != b.pads.len() || a.datum != b.datum {
        return false;
    }
    land_order(a).into_iter().zip(land_order(b)).all(|(i, j)| {
        match (a.pads.get(i), b.pads.get(j)) {
            (Some(p), Some(q)) => land(p) == land(q),
            _ => false,
        }
    })
}

/// `footprint`'s pads in stored order.
fn own_order(footprint: &FootprintDefinition) -> Vec<usize> {
    (0..footprint.pads.len()).collect()
}

/// For each pad of `target` in stored order, the index of the pad of
/// `other` with the same land.
fn matched_order(target: &FootprintDefinition, other: &FootprintDefinition) -> Vec<usize> {
    let mut by_land = vec![0; target.pads.len()];
    for (i, j) in land_order(target).into_iter().zip(land_order(other)) {
        if let Some(slot) = by_land.get_mut(i) {
            *slot = j;
        }
    }
    by_land
}

/// The pad grouping of a footprint under a binding, pads taken in `order`:
/// for each, `None` when it lands no electrical terminal, else the
/// position of the first pad of its terminal.
fn pad_groups(
    physical: &PhysicalView<'_>,
    footprint: &FootprintDefinition,
    order: &[usize],
) -> Vec<Option<usize>> {
    let mut first: BTreeMap<TerminalId, usize> = BTreeMap::new();
    order
        .iter()
        .enumerate()
        .map(|(position, index)| {
            let terminal = footprint
                .pads
                .get(*index)
                .and_then(|p| p.terminal)
                .and_then(|t| physical.binding.definition.terminal_of(t))?;
            Some(*first.entry(terminal).or_insert(position))
        })
        .collect()
}

/// Every Part of the project library that fits `instance`'s footprint
/// ([`substitute_fit`]), at its latest revision, the instance's own Part
/// left out.
pub fn substitutes(project: &Project, instance: Id) -> Result<Vec<Substitute>> {
    let instance = project
        .instance(instance)
        .ok_or_else(|| Error::NotFound("Unknown component instance.".into()))?;
    let library = &project.library;
    let mut out = Vec::new();
    for part in library.latest_parts() {
        let r = PartRef::new(part.id, part.revision);
        if instance.part.is_some_and(|own| own.id == r.id) {
            continue;
        }
        let Ok(physical) = substitute_fit(library, instance, r) else {
            continue;
        };
        out.push(Substitute {
            part: r,
            manufacturer: part.definition.manufacturer.clone(),
            mpn: part.definition.mpn.clone(),
            value: part_value(library, &part.definition),
            package: physical.package.meta.name.clone(),
        });
    }
    out.sort_by(|a, b| a.mpn.cmp(&b.mpn).then(a.part.id.cmp(&b.part.id)));
    Ok(out)
}

/// One Part that can be fitted on a footprint in a variant.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct Substitute {
    pub part: PartRef,
    pub manufacturer: String,
    pub mpn: String,
    /// Its rated values as value text.
    pub value: Option<String>,
    /// The package its body is.
    pub package: String,
}

/// One variant in `variant.list`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct VariantSummary {
    pub id: Id,
    pub name: String,
    /// Parts not fitted in it.
    pub not_populated: usize,
    /// Parts it fits another Part on.
    pub substituted: usize,
}

/// One part's state in one variant.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct VariantCell {
    pub variant: Id,
    pub population: Population,
    /// The variant sets the population rather than following the default.
    pub population_set: bool,
    /// The substitute Part, when the variant fits one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<PartRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
}

/// One part's row in `variant.list`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct VariantRow {
    pub instance: Id,
    pub designator: String,
    /// The default part's value text.
    pub value: Option<String>,
    /// The default part's MPN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mpn: Option<String>,
    /// The default population.
    pub population: Population,
    /// One cell per variant, in variant order.
    pub variants: Vec<VariantCell>,
}

/// `variant.list`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct VariantsReport {
    /// The active variant; `None` is the default.
    pub active: Option<Id>,
    pub variants: Vec<VariantSummary>,
    /// Every part that is bought (BOM parts), in designator order.
    pub parts: Vec<VariantRow>,
}

/// The variants and every part's state in each.
pub fn report(project: &Project) -> Result<VariantsReport> {
    let library = &project.library;
    let variants = &project.variants;
    let mut rows = Vec::new();
    let mut instances: Vec<&ComponentInstance> = project
        .instances
        .iter()
        .filter(|i| crate::library::mechanical::in_bom(library, i.component))
        .collect();
    instances.sort_by(|a, b| {
        designator_key(&a.designator)
            .cmp(&designator_key(&b.designator))
            .then(a.id.cmp(&b.id))
    });
    let mpn = |r: Option<PartRef>| {
        r.and_then(|r| library.part(r))
            .map(|p| p.definition.mpn.clone())
    };
    for instance in instances {
        rows.push(VariantRow {
            instance: instance.id,
            designator: instance.designator.clone(),
            value: super::value_text(library, instance)?,
            mpn: mpn(instance.part),
            population: instance.population,
            variants: variants
                .list
                .iter()
                .map(|v| {
                    let o = v.overrides.get(&instance.id).copied().unwrap_or_default();
                    VariantCell {
                        variant: v.id,
                        population: population(instance, Some(v)),
                        population_set: o.population.is_some(),
                        part: o.part,
                        mpn: mpn(o.part),
                    }
                })
                .collect(),
        });
    }
    let summaries = variants
        .list
        .iter()
        .enumerate()
        .map(|(k, v)| VariantSummary {
            id: v.id,
            name: v.name.clone(),
            not_populated: rows
                .iter()
                .filter(|r| {
                    r.variants
                        .get(k)
                        .is_some_and(|c| !c.population.is_populated())
                })
                .count(),
            substituted: rows
                .iter()
                .filter(|r| r.variants.get(k).is_some_and(|c| c.part.is_some()))
                .count(),
        })
        .collect();
    Ok(VariantsReport {
        active: variants.active,
        variants: summaries,
        parts: rows,
    })
}

/// A variant name trimmed, or why it cannot be used.
fn variant_name(variants: &AssemblyVariants, name: &str, own: Option<Id>) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_VARIANT_NAME || name.chars().any(char::is_control) {
        return Err(invalid(format!(
            "variant.name: a variant name is 1–{MAX_VARIANT_NAME} printable bytes"
        )));
    }
    if let Some(other) = variants.by_name(name)
        && Some(other.id) != own
    {
        return Err(Error::Conflict(format!(
            "There is already a variant named “{}”.",
            other.name
        )));
    }
    Ok(name.to_owned())
}

fn variant_mut(variants: &mut AssemblyVariants, id: Id) -> Result<&mut AssemblyVariant> {
    variants
        .get_mut(id)
        .ok_or_else(|| Error::NotFound("That assembly variant no longer exists.".into()))
}

/// Apply one variant command to `project`. A refused command changes
/// nothing; a command that restates the current state leaves the project
/// untouched (no clone of its instances). Returns the variant
/// `variant.add` created.
pub fn apply(project: &mut Project, command: VariantCommand) -> Result<Option<Id>> {
    match command {
        VariantCommand::Add { name, copy_of } => {
            if project.variants.list.len() >= MAX_VARIANTS {
                return Err(Error::Limit(format!(
                    "A design keeps at most {MAX_VARIANTS} assembly variants."
                )));
            }
            let name = variant_name(&project.variants, &name, None)?;
            let (overrides, reviews) = match copy_of {
                Some(id) => {
                    let source = project.variants.get(id).ok_or_else(|| {
                        Error::NotFound("The variant to copy no longer exists.".into())
                    })?;
                    (source.overrides.clone(), source.reviews.clone())
                }
                None => (BTreeMap::new(), BTreeMap::new()),
            };
            let id = project.allocate()?;
            project.variants.list.push(AssemblyVariant {
                id,
                name,
                overrides,
                reviews,
            });
            Ok(Some(id))
        }
        VariantCommand::Rename { variant, name } => {
            let name = variant_name(&project.variants, &name, Some(variant))?;
            let v = variant_mut(&mut project.variants, variant)?;
            if v.name != name {
                v.name = name;
            }
            Ok(None)
        }
        VariantCommand::Remove { variant } => {
            if project.variants.get(variant).is_none() {
                return Err(Error::NotFound(
                    "That assembly variant no longer exists.".into(),
                ));
            }
            project.variants.list.retain(|v| v.id != variant);
            if project.variants.active == Some(variant) {
                project.variants.active = None;
            }
            Ok(None)
        }
        VariantCommand::SetActive { variant } => {
            if let Some(id) = variant
                && project.variants.get(id).is_none()
            {
                return Err(Error::NotFound(
                    "That assembly variant no longer exists.".into(),
                ));
            }
            if project.variants.active != variant {
                project.variants.active = variant;
            }
            Ok(None)
        }
        VariantCommand::SetPopulation {
            instances,
            variant,
            population,
        } => {
            set_population(project, &instances, variant, population)?;
            Ok(None)
        }
        VariantCommand::SetPart {
            instance,
            variant,
            part,
        } => {
            let found = project
                .instance(instance)
                .ok_or_else(|| Error::NotFound("Unknown component instance.".into()))?;
            if let Some(r) = part {
                substitute_fit(&project.library, found, r)?;
            }
            // The instance's own Part is the default, not a substitute.
            let part = part.filter(|r| found.part != Some(*r));
            let v = variant_mut(&mut project.variants, variant)?;
            let current = v.overrides.get(&instance).copied().unwrap_or_default();
            if current.part != part {
                let next = VariantOverride { part, ..current };
                if next.is_empty() {
                    v.overrides.remove(&instance);
                } else {
                    v.overrides.insert(instance, next);
                }
                // A review is of the substitute it reviewed.
                v.reviews.remove(&instance);
            }
            Ok(None)
        }
        VariantCommand::ReviewSubstitute {
            instance,
            variant,
            equivalent,
            exceptions,
        } => {
            if project.instance(instance).is_none() {
                return Err(Error::NotFound("Unknown component instance.".into()));
            }
            check_exceptions(&exceptions)?;
            let v = variant_mut(&mut project.variants, variant)?;
            let part = v
                .overrides
                .get(&instance)
                .and_then(|o| o.part)
                .ok_or_else(|| {
                    invalid(
                        "variant.review_substitute: the variant fits no substitute on this part; fit one first (variant.set_part)"
                            .into(),
                    )
                })?;
            let next = (equivalent || !exceptions.is_empty()).then_some(SubstituteReview {
                part,
                equivalent,
                exceptions,
            });
            if v.reviews.get(&instance) != next.as_ref() {
                match next {
                    Some(review) => {
                        v.reviews.insert(instance, review);
                    }
                    None => {
                        v.reviews.remove(&instance);
                    }
                }
            }
            Ok(None)
        }
    }
}

/// A review's exceptions: bounded, one per field, each with a printable
/// reason.
fn check_exceptions(exceptions: &[RequirementException]) -> Result<()> {
    if exceptions.len() > MAX_EXCEPTIONS {
        return Err(invalid(format!(
            "a substitute review records at most {MAX_EXCEPTIONS} requirement exceptions"
        )));
    }
    let mut fields = BTreeSet::new();
    for e in exceptions {
        let reason = e.reason.trim();
        if reason.is_empty()
            || e.reason.len() > MAX_EXCEPTION_REASON
            || e.reason.chars().any(char::is_control)
        {
            return Err(invalid(format!(
                "a requirement exception needs a printable reason of 1–{MAX_EXCEPTION_REASON} bytes"
            )));
        }
        if !fields.insert(e.field) {
            return Err(invalid(
                "a requirement exception names each field once".into(),
            ));
        }
    }
    Ok(())
}

fn set_population(
    project: &mut Project,
    instances: &[Id],
    variant: Option<Id>,
    population: Option<Population>,
) -> Result<()> {
    let ids: BTreeSet<Id> = instances.iter().copied().collect();
    if ids.is_empty() || ids.len() > MAX_POPULATION_EDIT {
        return Err(invalid(format!(
            "variant.set_population names 1–{MAX_POPULATION_EDIT} parts"
        )));
    }
    if ids.iter().any(|id| project.instance(*id).is_none()) {
        return Err(Error::NotFound("Unknown component instance.".into()));
    }
    match variant {
        None => {
            let population = population.ok_or_else(|| {
                invalid(
                    "variant.population: the default needs a population (populated or not_populated)"
                        .into(),
                )
            })?;
            let changed: Vec<Id> = ids
                .into_iter()
                .filter(|id| {
                    project
                        .instance(*id)
                        .is_some_and(|i| i.population != population)
                })
                .collect();
            for id in changed {
                instance_mut(project, id)?.population = population;
            }
        }
        Some(variant) => {
            let v = variant_mut(&mut project.variants, variant)?;
            for id in ids {
                let current = v.overrides.get(&id).copied().unwrap_or_default();
                if current.population == population {
                    continue;
                }
                let next = VariantOverride {
                    population,
                    ..current
                };
                if next.is_empty() {
                    v.overrides.remove(&id);
                } else {
                    v.overrides.insert(id, next);
                }
            }
        }
    }
    Ok(())
}

/// Overrides of instances that no longer exist go with them. Run after
/// every edit; leaves the variants untouched when nothing is dropped.
pub fn follow_instances(project: &mut Project) {
    let dangling = project
        .variants
        .list
        .iter()
        .any(|v| v.overrides.keys().any(|id| project.instance(*id).is_none()));
    if !dangling {
        return;
    }
    let live: BTreeSet<Id> = project.instances.iter().map(|i| i.id).collect();
    for v in &mut project.variants.list {
        v.overrides.retain(|id, _| live.contains(id));
        v.reviews.retain(|id, _| live.contains(id));
    }
}

/// The variants' invariants: bounded, uniquely named, overrides of
/// existing instances, substitutes in the library, the active variant
/// listed.
pub fn check(project: &Project) -> Result<()> {
    let variants = &project.variants;
    if variants.list.len() > MAX_VARIANTS {
        return Err(invalid(format!(
            "a design keeps at most {MAX_VARIANTS} assembly variants"
        )));
    }
    let mut names = BTreeSet::new();
    for v in &variants.list {
        let name = v.name.trim();
        if name != v.name
            || name.is_empty()
            || name.len() > MAX_VARIANT_NAME
            || name.chars().any(char::is_control)
            || !names.insert(name.to_lowercase())
        {
            return Err(invalid(
                "assembly variant names are unique, trimmed and printable".into(),
            ));
        }
        for (id, o) in &v.overrides {
            if project.instance(*id).is_none() || o.is_empty() {
                return Err(invalid(format!(
                    "variant “{name}” overrides a part that is not in the design"
                )));
            }
            if let Some(r) = o.part
                && project.library.part(r).is_none()
            {
                return Err(invalid(format!(
                    "variant “{name}” fits a Part that is missing from the project library"
                )));
            }
        }
        for (id, review) in &v.reviews {
            if v.overrides.get(id).and_then(|o| o.part) != Some(review.part)
                || (!review.equivalent && review.exceptions.is_empty())
            {
                return Err(invalid(format!(
                    "variant “{name}” keeps a review of a substitute it does not fit"
                )));
            }
            check_exceptions(&review.exceptions)?;
        }
    }
    if let Some(active) = variants.active
        && variants.get(active).is_none()
    {
        return Err(invalid("the active assembly variant is not listed".into()));
    }
    Ok(())
}

/// The ids variants take from the project's identity allocator.
pub fn ids(project: &Project) -> impl Iterator<Item = Id> + '_ {
    project.variants.list.iter().map(|v| v.id)
}

/// What stands between a fitted substitute and a resolved purchase, beyond
/// fitting: no review of its electrical equivalence
/// ([`SUBSTITUTE_UNREVIEWED`]), and each requirement of the instance it
/// does not rate, meet or compare with that its review does not except
/// ([`SUBSTITUTE_UNMET`], the comparison of `instances::rating_findings`).
/// Empty when the substitute is resolved.
pub fn substitute_findings(
    library: &Library,
    variant: &AssemblyVariant,
    instance: &ComponentInstance,
    r: PartRef,
) -> Vec<Finding> {
    let review = variant
        .reviews
        .get(&instance.id)
        .filter(|review| review.part == r);
    let Some(part) = library.part(r) else {
        return Vec::new();
    };
    let mpn = &part.definition.mpn;
    let objects = vec![
        ObjectRef::Instance { id: instance.id },
        ObjectRef::Definition { id: r.id },
    ];
    let mut out = Vec::new();
    if !review.is_some_and(|review| review.equivalent) {
        out.push(Finding::new(
            SUBSTITUTE_UNREVIEWED,
            format!(
                "Variant “{}”: {mpn} fits {}'s lands, but nobody has confirmed that its pins do the job of the part it replaces.",
                variant.name, instance.designator
            ),
            objects,
        ));
    }
    let excepted = |f: &Finding| {
        f.objects.iter().any(|o| match o {
            ObjectRef::Property { field, .. } => {
                review.is_some_and(|review| review.exceptions.iter().any(|e| e.field == *field))
            }
            _ => false,
        })
    };
    for f in super::rating_findings(library, instance, r, &part.definition) {
        if !excepted(&f) {
            let mut objects = vec![ObjectRef::Instance { id: instance.id }];
            objects.extend(f.objects.iter().copied());
            out.push(Finding {
                code: SUBSTITUTE_UNMET.into(),
                message: format!("Variant “{}”: {}", variant.name, f.message),
                objects,
                ..f
            });
        }
    }
    out
}

/// `variant.substitute_unfit`: a substitute that no longer fits its
/// footprint (after a library update). The default part is fitted until it
/// is chosen again or removed.
pub fn findings(project: &Project) -> Vec<Finding> {
    let mut out = Vec::new();
    for v in &project.variants.list {
        for (id, o) in &v.overrides {
            let (Some(r), Some(instance)) = (o.part, project.instance(*id)) else {
                continue;
            };
            if let Err(error) = substitute_fit(&project.library, instance, r) {
                out.push(Finding::new(
                    SUBSTITUTE_UNFIT,
                    format!("Variant “{}”: {error}", v.name),
                    vec![
                        ObjectRef::Instance { id: *id },
                        ObjectRef::Definition { id: r.id },
                    ],
                ));
            } else {
                out.extend(substitute_findings(&project.library, v, instance, r));
            }
        }
    }
    out
}
