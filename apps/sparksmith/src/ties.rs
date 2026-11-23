//! Shared tie properties for the schematic and board inspectors.
use crate::issue_reports::SharedReports;
use crate::kit;
use sparksmith_core::ids::{TerminalId, TieId};
use sparksmith_core::library::tie::{JumperState, TieForm, TiePurpose};
use sparksmith_core::{
    ComponentInstance, Engine, Id, InstanceCommand, ObjectRef, Operation, Project,
};
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AnchorPick {
    pub instance: Id,
    pub group: TieId,
    pub end: TerminalId,
}
impl AnchorPick {
    pub fn place(self, land: Id, pad: sparksmith_core::PadId) -> Operation {
        Operation::Instance(InstanceCommand::SetAnchor {
            instance: self.instance,
            group: Some(self.group),
            end: self.end,
            anchor: Some(sparksmith_core::library::tie::Anchor {
                instance: land,
                pad,
            }),
        })
    }
}

/// Both inspectors edit the same stable group; a bridge pick supplies it directly.
pub fn card(
    ui: &mut egui::Ui,
    engine: &Engine,
    instance: &ComponentInstance,
    reports: &SharedReports,
    selection: &mut Option<TieId>,
    mut pick: Option<&mut Option<AnchorPick>>,
) -> Option<Operation> {
    if instance.ties.is_empty() {
        return None;
    }
    let Some(rows) = reports.ties(engine) else {
        kit::value_row(ui, "Net tie", "Checking joins…");
        return None;
    };
    let p = engine.project();
    let mut groups = rows.iter().filter(|r| r.tie.instance == instance.id);
    let first = groups.next()?;
    let rest = groups.collect::<Vec<_>>();
    let mut current = selection
        .filter(|id| instance.ties.contains_key(id))
        .unwrap_or(first.tie.group);
    let mut operation = None;
    kit::section(
        ui,
        ("tie-properties", instance.id),
        "Net tie",
        true,
        None,
        |ui| {
            if !rest.is_empty() {
                let options = std::iter::once(first)
                    .chain(rest.iter().copied())
                    .map(|r| {
                        let nets = r
                            .tie
                            .ends
                            .values()
                            .flatten()
                            .map(|n| p.net_name(*n))
                            .collect::<Vec<_>>()
                            .join(" ↔ ");
                        let group = r.tie.label(p);
                        let title = if nets.is_empty() {
                            group
                        } else {
                            format!("{group} · {nets}")
                        };
                        (r.tie.group, title)
                    })
                    .collect::<Vec<_>>();
                if kit::choice_row(ui, "Group", &mut current, &options)
                    && let Some(slot) = pick.as_deref_mut()
                {
                    *slot = None;
                }
            }
            *selection = Some(current);
            let row = rest
                .iter()
                .copied()
                .find(|r| r.tie.group == current)
                .unwrap_or(first);
            kit::value_row(
                ui,
                "Form",
                match row.tie.form {
                    TieForm::Copper => "Copper tie",
                    TieForm::ZeroOhm => "0 Ω link",
                    TieForm::SolderJumper { .. } => "Solder jumper",
                },
            );
            let choices = [
                (TiePurpose::None, "None"),
                (TiePurpose::StarGround, "Star ground"),
                (TiePurpose::KelvinSense, "Kelvin sense"),
                (TiePurpose::ShieldBond, "Shield bond"),
                (TiePurpose::SupplyJoin, "Supply join"),
            ]
            .map(|(k, l)| (k, l.to_owned()));
            let mut purpose = row.tie.purpose;
            if kit::choice_row(ui, "Purpose", &mut purpose, &choices) {
                operation = Some(Operation::Instance(InstanceCommand::SetPurpose {
                    instance: instance.id,
                    group: Some(current),
                    purpose,
                }));
            }
            for (end, net) in &row.tie.ends {
                let label = p
                    .library
                    .component(instance.component)
                    .and_then(|c| c.definition.terminal(*end))
                    .map_or("End", |e| e.name.as_str());
                kit::value_row(ui, label, net.map_or("Unconnected", |n| p.net_name(n)));
            }
            kit::value_row(
                ui,
                "State",
                match row.tie.state {
                    JumperState::Open => "Open",
                    JumperState::Closed => "Closed",
                },
            );
            for (label, value) in [
                ("Required current", row.current.required_a),
                ("Capacity", row.current.capacity_a),
                ("Margin", row.current.margin_a),
            ] {
                if let Some(value) = value {
                    kit::value_row(ui, label, &format!("{value:.3} A"));
                }
            }
            if let Some(source) = &row.current.source {
                kit::value_row(ui, "Source", source);
            }
            if let Some(crate::issues::Answer::Report(report)) = reports.latest(engine) {
                let findings = report.rows.iter().flat_map(|r| std::iter::once(&r.finding).chain(&r.also))
                    .filter(|f| {
                        f.code.starts_with("tie.")
                            && f.objects.contains(&ObjectRef::Instance { id: instance.id })
                            && (rest.is_empty() || f.objects.iter().any(|o| {
                                matches!(o, ObjectRef::Terminal { endpoint }
                                    if endpoint.instance == instance.id && row.tie.ends.contains_key(&endpoint.terminal))
                            }))
                    })
                    .collect::<Vec<_>>();
                if !findings.is_empty() {
                    let key = ui.make_persistent_id(("all-tie-findings", instance.id, current));
                    let limit = if ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(false) {
                        findings.len()
                    } else {
                        kit::LIST_MAX_ROWS
                    };
                    let list = kit::list_frame(ui, findings.len(), limit, |ui, idx| {
                        if let Some(finding) = findings.get(idx) {
                            kit::value_row(ui, "Finding", &finding.message);
                        }
                    });
                    if list.show_all {
                        ui.data_mut(|d| d.insert_temp(key, true));
                    }
                }
            }
        },
    );
    if let Some(pick) = pick {
        operation = operation.or_else(|| anchor_card(ui, p, instance, current, pick));
    }
    operation
}

fn anchor_card(
    ui: &mut egui::Ui,
    p: &Project,
    i: &ComponentInstance,
    group: TieId,
    pick: &mut Option<AnchorPick>,
) -> Option<Operation> {
    let usage = i
        .ties
        .get(&group)
        .filter(|t| t.purpose == TiePurpose::KelvinSense)?;
    let spec = p
        .library
        .component(i.component)?
        .definition
        .ties
        .iter()
        .find(|s| s.id == group)?;
    let mut operation = None;
    kit::section(ui, "tie-anchors", "Kelvin anchors", true, None, |ui| {
        for end in &spec.ends {
            let label = usage
                .anchors
                .get(end)
                .and_then(|a| {
                    let i = p.instance(a.instance)?;
                    let pad = sparksmith_core::instances::resolve(&p.library, i)
                        .ok()?
                        .physical?
                        .footprint
                        .definition
                        .pad(a.pad)?;
                    Some(format!("{} pad {}", i.designator, pad.number))
                })
                .unwrap_or_else(|| "Unassigned".into());
            let end_name = p
                .library
                .component(i.component)
                .and_then(|c| c.definition.terminal(*end))
                .map_or("End", |e| e.name.as_str());
            if kit::summary_row(ui, &format!("{end_name} · {label}"), Some("Pick pad…")) {
                *pick = Some(AnchorPick {
                    instance: i.id,
                    group,
                    end: *end,
                });
            }
            if usage.anchors.contains_key(end)
                && kit::summary_row(ui, end_name, Some("Clear anchor"))
            {
                operation = Some(Operation::Instance(InstanceCommand::SetAnchor {
                    instance: i.id,
                    group: Some(group),
                    end: *end,
                    anchor: None,
                }));
            }
            if *pick
                == Some(AnchorPick {
                    instance: i.id,
                    group,
                    end: *end,
                })
            {
                kit::value_row(ui, "Anchor", "Click a pad on the board. Esc cancels.");
            }
        }
    });
    operation
}

/// The nets to highlight and the places to mark are indexed once when
/// the board's checked tie report arrives. Selection only reads the index.
#[derive(Default)]
pub struct BoardTies {
    rows: std::sync::Arc<[sparksmith_core::layout::connectivity::TieReport]>,
    nets: std::collections::BTreeMap<Id, Vec<(Id, TieId)>>,
}
impl BoardTies {
    pub fn update(
        &mut self,
        rows: std::sync::Arc<[sparksmith_core::layout::connectivity::TieReport]>,
    ) {
        if std::sync::Arc::ptr_eq(&rows, &self.rows) {
            return;
        }
        self.nets.clear();
        for row in rows.iter() {
            for net in row.tie.ends.values().flatten() {
                self.nets
                    .entry(*net)
                    .or_default()
                    .push((row.tie.instance, row.tie.group));
            }
        }
        self.rows = rows;
    }
    pub fn net_groups(&self, net: Id) -> &[(Id, TieId)] {
        self.nets.get(&net).map_or(&[], Vec::as_slice)
    }
    pub fn rows(&self) -> &[sparksmith_core::layout::connectivity::TieReport] {
        &self.rows
    }
}

#[derive(Default)]
pub struct Focus {
    pub group: Option<(Id, TieId)>,
    pub nets: std::collections::BTreeSet<Id>,
}
impl Focus {
    /// When inspecting one group of a package, its sibling bridges and
    /// their pads keep their normal colour.
    pub fn includes(&self, source: &sparksmith_solidcraft::project::Source) -> bool {
        let Some((instance, group)) = self.group else {
            return true;
        };
        if source.instance != Some(instance) {
            return true;
        }
        if let Some(tie) = &source.tie {
            return tie.group == group;
        }
        source.pad.is_none() || source.net.is_some_and(|net| self.nets.contains(&net))
    }
}
