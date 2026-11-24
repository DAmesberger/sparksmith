//! The order notes: what the order form and its remarks must say that the
//! other files cannot, and the published charges whose condition the board
//! meets, each with its source. Written only when there is one.
//!
//! The words and the conditions are the fab profile's data. The lines a
//! reader (`readback`) takes back are the tool lines (two spaces, `T1`, the
//! finished size, the count) under "Fill and cap" and "Leave open", and the
//! charges (two spaces and a quoted sentence) under "Charges".
use super::drill::{DrillTable, DrillTool};
use super::snapshot::Identity;
use super::{FabFile, FileGroup, HoleFunction, drill};
use crate::model::{Project, ViaFill};
use crate::rules::drills::{Charge, FillRule, Verdict, through_fill};
use crate::rules::requirement::number;
use std::fmt::Write as _;

/// The charges of the board's profile that fall on it, one a sentence.
pub fn charges(project: &Project) -> Vec<&Charge> {
    let layers = project.stackup.copper.len();
    let mut out: Vec<&Charge> = Vec::new();
    for charge in project.rules.profile.iter().flat_map(|p| &p.charges) {
        let falls = project
            .vias
            .iter()
            .any(|v| charge.when.holds(layers, v.fill, v.drill, v.diameter));
        if falls && !out.iter().any(|c| c.text == charge.text) {
            out.push(charge);
        }
    }
    out
}

fn tool_line(s: &mut String, tool: &DrillTool) {
    let holes = if tool.count == 1 { "hole" } else { "holes" };
    let _ = writeln!(
        s,
        "  T{}  {:.3} mm  {} {holes}",
        tool.number, tool.diameter, tool.count
    );
}

/// What the profile says of filled and capped vias.
fn capped_rule(project: &Project) -> Option<(&FillRule, bool)> {
    let profile = project.rules.profile.as_ref()?;
    match through_fill(&profile.drill_rules, ViaFill::Capped) {
        Verdict::Made(Some(rule)) => Some((rule, true)),
        Verdict::NotStated(rule) => Some((rule, false)),
        Verdict::Made(None) | Verdict::NotMade(_) => None,
    }
}

/// The order notes of `project`, when there is anything to say.
pub fn file(project: &Project, identity: &Identity, stem: &str) -> Option<FabFile> {
    let table = drill::table(&drill::holes(project));
    let charges = charges(project);
    let vias = |t: &&DrillTool| t.function == HoleFunction::Via;
    let filled = table.tools.iter().any(|t| !t.fill.is_none());
    if !filled && charges.is_empty() {
        return None;
    }
    let profile = project.rules.profile.as_ref();
    let mut s = String::new();
    let _ = writeln!(s, "ORDER NOTES: {}", project.name);
    let _ = writeln!(s, "Release {}", identity.revision);
    let _ = writeln!(
        s,
        "{}{} copper layers, {} mm",
        profile.map_or_else(String::new, |p| format!("{}: ", p.name)),
        project.stackup.copper.len(),
        number(project.stackup.thickness_mm())
    );
    if filled {
        fill(&mut s, project, &table);
        let _ = writeln!(s, "\nLeave open");
        let open: Vec<&DrillTool> = table
            .tools
            .iter()
            .filter(vias)
            .filter(|t| t.fill.is_none())
            .collect();
        if open.is_empty() {
            s.push_str("  no via\n");
        }
        for tool in open {
            tool_line(&mut s, tool);
        }
    }
    if !charges.is_empty() {
        let _ = writeln!(s, "\nCharges");
        for charge in charges {
            let _ = writeln!(s, "  \"{}\"", charge.text);
            let _ = writeln!(s, "    {}", charge.source);
        }
    }
    Some(FabFile {
        name: format!("{stem}-order-notes.txt"),
        group: FileGroup::Order,
        description: "Order notes (what to choose and write on the order)".into(),
        bytes: s.into_bytes(),
    })
}

/// What to choose on the order form, and which tools to fill.
fn fill(s: &mut String, project: &Project, table: &DrillTable) {
    let rule = capped_rule(project);
    let order = rule.and_then(|(r, _)| r.order.as_ref());
    if let Some(order) = order.filter(|o| !o.form.is_empty()) {
        let _ = writeln!(s, "\nOn the order form");
        for choice in &order.form {
            let unverified = if choice.unverified {
                " (form label not verified)"
            } else {
                ""
            };
            let _ = writeln!(s, "  {}: {}{unverified}", choice.label, choice.choice);
        }
    }
    let _ = writeln!(s, "\nFill and cap");
    for tool in table.tools.iter().filter(|t| !t.fill.is_none()) {
        tool_line(s, tool);
    }
    let name = project.rules.profile.as_ref().map(|p| p.name.as_str());
    if let (Some((rule, false)), Some(name)) = (rule, name) {
        let _ = writeln!(
            s,
            "\n{name} does not state what it makes of these: confirm before ordering"
        );
        let _ = writeln!(s, "    {}", rule.source);
    }
    if let (Some(order), Some(name)) = (order, name) {
        if !order.asks.is_empty() {
            let _ = writeln!(s, "\n{name} asks: \"{}\"", order.asks);
        }
        if !order.source.is_empty() {
            let _ = writeln!(s, "    {}", order.source);
        }
    }
}
