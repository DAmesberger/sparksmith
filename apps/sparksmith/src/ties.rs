//! Shared tie properties for the schematic and board inspectors.
use crate::kit;
use sparksmith_core::ids::TerminalId;
use sparksmith_core::instances::ties::TieReport;
use sparksmith_core::library::tie::{JumperState, TieForm, TiePurpose};
use sparksmith_core::{
    ComponentInstance, Engine, Id, InstanceCommand, ObjectRef, Operation, Project, Query,
    QueryResult,
};
use std::sync::Arc;

fn rows(ui: &egui::Ui, engine: &Engine) -> Option<Arc<Vec<TieReport>>> {
    let id = egui::Id::new("net-tie-report");
    type Cached = (sparksmith_core::ReportKey, Arc<Vec<TieReport>>);
    let key = engine.report_key();
    if let Some((_, rows)) = ui
        .ctx()
        .data(|d| d.get_temp::<Cached>(id))
        .filter(|(k, _)| *k == key)
    {
        return Some(rows);
    }
    let QueryResult::Ties(rows) = engine.query(Query::Ties {}).ok()? else {
        return None;
    };
    let rows = Arc::new(rows);
    ui.ctx()
        .data_mut(|d| d.insert_temp(id, (key, rows.clone())));
    Some(rows)
}

pub fn card(ui: &mut egui::Ui, engine: &Engine, instance: &ComponentInstance) -> Option<Operation> {
    instance.tie.as_ref()?;
    let rows = rows(ui, engine)?;
    let row = rows.iter().find(|t| t.tie.instance == instance.id)?;
    let p = engine.project();
    let mut operation = None;
    kit::section(ui, "tie-properties", "Net tie", true, None, |ui| {
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
        if let Ok(report) = engine.issue_report() {
            for issue in report.rows.iter().filter(|i| {
                i.finding.code.starts_with("tie.")
                    && i.objects()
                        .any(|o| *o == ObjectRef::Instance { id: instance.id })
            }) {
                kit::value_row(ui, "Finding", &issue.finding.message);
            }
        }
    });
    operation
}

pub fn anchor_card(
    ui: &mut egui::Ui,
    p: &Project,
    i: &ComponentInstance,
    pick: &mut Option<(Id, TerminalId)>,
) -> Option<Operation> {
    let usage = i
        .tie
        .as_ref()
        .filter(|t| t.purpose == TiePurpose::KelvinSense)?;
    let spec = p.library.component(i.component)?.definition.tie.as_ref()?;
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
                *pick = Some((i.id, *end));
            }
            if usage.anchors.contains_key(end)
                && kit::summary_row(ui, end_name, Some("Clear anchor"))
            {
                operation = Some(Operation::Instance(InstanceCommand::SetAnchor {
                    instance: i.id,
                    end: *end,
                    anchor: None,
                }));
            }
            if *pick == Some((i.id, *end)) {
                kit::value_row(ui, "Anchor", "Click a pad on the board. Esc cancels.");
            }
        }
    });
    operation
}
