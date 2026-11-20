//! Assembly variants on the canvases and cards (docs/assembly-variants.md):
//! a part's population in the default and in the active variant, as the
//! PCB Placement card, the schematic Instance card and both canvases' part
//! menus offer it. Every change is one `variant.set_population` step; what
//! is fitted is the core's answer (`instances::variants`), never worked
//! out here.
use crate::context_menu::Entry;
use crate::kit;
use sparksmith_core::instances::variants::{self, VariantCommand};
use sparksmith_core::model::Population;
use sparksmith_core::{ComponentInstance, Id, Operation, Project};

/// Where a population change lands, in the user's words: "in Lite", or
/// "in default" while no variant is active.
pub fn scope(project: &Project) -> String {
    match project.variants.active() {
        Some(variant) => format!("in {}", variant.name),
        None => "in default".to_owned(),
    }
}

/// Whether every one of `instances` is left off under the active variant;
/// `None` when none of them is a part of the project.
pub fn all_not_populated(project: &Project, instances: &[Id]) -> Option<bool> {
    let active = project.variants.active();
    let mut found = instances
        .iter()
        .filter_map(|id| project.instance(*id))
        .peekable();
    found.peek()?;
    Some(found.all(|i| !variants::population(i, active).is_populated()))
}

/// The one command that makes `instances` populated or not in the active
/// variant (the default while none is active). In a variant, a value
/// every instance's default already has clears the override, so the part
/// follows the default again.
pub fn set_population(project: &Project, instances: Vec<Id>, populated: bool) -> Option<Operation> {
    if instances.is_empty() {
        return None;
    }
    let population = if populated {
        Population::Populated
    } else {
        Population::NotPopulated
    };
    let variant = project.variants.active().map(|v| v.id);
    let follows_default = instances.iter().all(|id| {
        project
            .instance(*id)
            .is_some_and(|i| i.population == population)
    });
    let population = match variant {
        Some(_) if follows_default => None,
        _ => Some(population),
    };
    Some(Operation::Variant(VariantCommand::SetPopulation {
        instances,
        variant,
        population,
    }))
}

/// The part menu's check row: "Not populated in Lite", checked (a tick
/// for its icon) while every one of `instances` is left off, else the
/// leave-off icon. `None` without a part.
pub fn menu_entry<A>(
    project: &Project,
    instances: &[Id],
    act: A,
    enabled: bool,
) -> Option<Entry<A>> {
    let checked = all_not_populated(project, instances)?;
    Some(Entry::act(
        format!("Not populated {}", scope(project)),
        if checked { "finish" } else { "suppress" },
        act,
        enabled,
    ))
}

/// The operation the menu row sends: the opposite of its check.
pub fn toggle(project: &Project, instances: Vec<Id>) -> Option<Operation> {
    let off = all_not_populated(project, &instances)?;
    set_population(project, instances, off)
}

/// The ASSEMBLY section of a part's card: whether it is populated by
/// default and, while a variant is active, in that variant, each a check
/// committing one `variant.set_population`; and the Part the variant fits
/// instead of the default one, when it fits another.
pub fn assembly_section(
    ui: &mut egui::Ui,
    project: &Project,
    instance: &ComponentInstance,
) -> Option<Operation> {
    let active = project.variants.active();
    let mut operation = None;
    kit::section(ui, "part-assembly", "Assembly", true, None, |ui| {
        let label = if active.is_some() {
            "Populated by default"
        } else {
            "Populated"
        };
        let mut default = instance.population.is_populated();
        if kit::check_row(ui, label, &mut default) {
            crate::refusal::mark(ui.ctx(), crate::refusal::Origin::Command);
            operation = Some(Operation::Variant(VariantCommand::SetPopulation {
                instances: vec![instance.id],
                variant: None,
                population: Some(if default {
                    Population::Populated
                } else {
                    Population::NotPopulated
                }),
            }));
        }
        let Some(variant) = active else {
            return;
        };
        let mut fitted = variants::population(instance, Some(variant)).is_populated();
        if kit::check_row(ui, &format!("Populated in {}", variant.name), &mut fitted) {
            crate::refusal::mark(ui.ctx(), crate::refusal::Origin::Command);
            operation = set_population(project, vec![instance.id], fitted);
        }
        let shown = variants::fitted(project, instance);
        if shown.substituted
            && let Some(part) = shown.part.and_then(|r| project.library.part(r))
        {
            let part = &part.definition;
            kit::value_row(
                ui,
                "Fitted part",
                &format!("{} {}", part.manufacturer.trim(), part.mpn.trim()),
            );
        }
    });
    operation
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::Engine;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn add(engine: &mut Engine, name: &str) -> Result<Id, Box<dyn std::error::Error>> {
        match engine.apply(
            engine.revision(),
            Operation::Variant(VariantCommand::Add {
                name: name.into(),
                copy_of: None,
            }),
        )? {
            sparksmith_core::Outcome::VariantAdded { variant } => Ok(variant),
            other => Err(format!("unexpected {other:?}").into()),
        }
    }

    /// In the default the toggle sets the default; in a variant it sets
    /// the override, and setting it back to the default clears it.
    #[test]
    fn the_toggle_sets_the_active_scope_and_clears_a_restated_override() -> TestResult {
        let mut engine = Engine::demo()?;
        let r1 = engine
            .project()
            .instance_by_designator("R1")
            .ok_or("R1")?
            .id;
        assert_eq!(scope(engine.project()), "in default");
        let op = toggle(engine.project(), vec![r1]).ok_or("no toggle")?;
        engine.apply(engine.revision(), op)?;
        assert_eq!(all_not_populated(engine.project(), &[r1]), Some(true));
        let lite = add(&mut engine, "Lite")?;
        engine.apply(
            engine.revision(),
            Operation::Variant(VariantCommand::SetActive {
                variant: Some(lite),
            }),
        )?;
        assert_eq!(scope(engine.project()), "in Lite");
        // Lite follows the default: R1 is off there too; fitting it is an
        // override.
        let op = toggle(engine.project(), vec![r1]).ok_or("no toggle")?;
        assert!(
            matches!(
                &op,
                Operation::Variant(VariantCommand::SetPopulation {
                    variant: Some(v),
                    population: Some(Population::Populated),
                    ..
                }) if *v == lite
            ),
            "{op:?}"
        );
        engine.apply(engine.revision(), op)?;
        assert_eq!(all_not_populated(engine.project(), &[r1]), Some(false));
        // Off again: the default's value, so the override goes.
        let op = toggle(engine.project(), vec![r1]).ok_or("no toggle")?;
        assert!(
            matches!(
                &op,
                Operation::Variant(VariantCommand::SetPopulation {
                    population: None,
                    ..
                })
            ),
            "{op:?}"
        );
        engine.apply(engine.revision(), op)?;
        let variant = engine.project().variants.get(lite).ok_or("Lite")?;
        assert!(
            variant
                .overrides
                .get(&r1)
                .is_none_or(|o| o.population.is_none())
        );
        Ok(())
    }
}
