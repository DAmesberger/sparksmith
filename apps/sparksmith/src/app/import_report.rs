//! The KiCad import report: a system window over the canvas, shown when
//! File › Import KiCad project… finishes and reopened from the command
//! palette or Window › KiCad Import Report while the imported project is
//! open. It lays out what the import did (`sparksmith_kicad::Report`):
//! the counts, the rules not taken with both values, and every counted
//! case by how faithfully it mapped. The findings themselves (the pins an
//! import added, a stub a refused track left) stay in Issues.
use crate::kit;
use sparksmith_automation::kicad_import::{Fidelity, Report};
use sparksmith_ui::ui_kit;

/// The last import's report while its project is open.
#[derive(Default)]
pub struct ImportReport {
    shown: Option<Shown>,
    open: bool,
}

struct Shown {
    /// The board file's name.
    board: String,
    report: Report,
}

impl ImportReport {
    /// Show `report` of the import of `board`.
    pub fn set(&mut self, board: &std::path::Path, report: Report) {
        let board = board.file_name().map_or_else(
            || board.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        self.shown = Some(Shown { board, report });
        self.open = true;
    }

    /// Another project replaced the imported one: nothing to reopen.
    pub fn clear(&mut self) {
        self.shown = None;
        self.open = false;
    }

    pub fn available(&self) -> bool {
        self.shown.is_some()
    }

    pub fn is_open(&self) -> bool {
        self.open && self.shown.is_some()
    }

    pub fn open(&mut self) {
        self.open = self.shown.is_some();
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    /// The report as text, for Copy report.
    fn text(shown: &Shown) -> String {
        format!("{}\n{}", shown.board, shown.report.summary_named())
    }

    /// The window, while open.
    pub fn window(&mut self, ctx: &egui::Context) {
        if !self.is_open() {
            return;
        }
        let Some(shown) = &self.shown else {
            return;
        };
        let mut copy = false;
        let closed = ui_kit::Window::new(
            egui::Id::unique("kicad-import-report"),
            "list",
            "KiCad import report",
        )
        .show_body(ctx, |ui| body(ui, shown, &mut copy));
        if copy {
            ctx.copy_text(Self::text(shown));
        }
        if closed {
            self.open = false;
        }
    }
}

/// The report's rows, by how faithfully each case mapped.
const GROUPS: &[(Fidelity, &str, bool)] = &[
    (Fidelity::Refused, "Not imported", true),
    (Fidelity::Report, "Imported and listed", true),
    (Fidelity::Approximate, "Approximated", false),
    (Fidelity::Exact, "Exact", false),
];

fn body(ui: &mut egui::Ui, shown: &Shown, copy: &mut bool) {
    let report = &shown.report;
    let c = &report.counts;
    kit::actions(ui, |ui| {
        *copy = ui_kit::button(ui, "Copy report", ui_kit::Button::Normal).clicked();
    });
    kit::value_row(ui, "Board", &shown.board);
    kit::value_row(
        ui,
        "Footprints",
        &format!(
            "{} ({} on the back), {}",
            c.footprints,
            c.footprints_back,
            kit::counted(c.definitions, "definition", "definitions")
        ),
    );
    kit::value_row(
        ui,
        "Pads",
        &format!("{} of {} imported", c.pads_imported, c.pads),
    );
    kit::value_row(
        ui,
        "Copper",
        &format!(
            "{}, {}, {}, {}",
            kit::counted(c.nets, "net", "nets"),
            kit::counted(c.tracks, "track piece", "track pieces"),
            kit::counted(c.vias, "via", "vias"),
            kit::counted(c.fills, "fill", "fills")
        ),
    );
    if c.sheets > 0 {
        kit::value_row(
            ui,
            "Schematic",
            &format!(
                "{}; board nets equal {}, split {}, joined {}",
                kit::counted(c.sheets, "sheet", "sheets"),
                c.schematic_nets_equal,
                c.schematic_nets_split,
                c.schematic_nets_merged
            ),
        );
    }
    let looser = report.looser_rules();
    if !looser.is_empty() {
        kit::section(
            ui,
            "kicad-import-looser",
            "Rules not taken",
            true,
            Some(looser.len()),
            |ui| {
                kit::note(
                    ui,
                    "Looser in KiCad than the fab profile: the profile's value stays.",
                );
                for rule in looser {
                    kit::value_row(
                        ui,
                        &rule.rule,
                        &format!(
                            "KiCad {} mm, profile {} mm ({})",
                            kit::number_text(rule.kicad_mm),
                            kit::number_text(rule.profile_mm),
                            rule.kicad_setting
                        ),
                    );
                }
            },
        );
    }
    let rows = report.rows();
    for (fidelity, title, open) in GROUPS {
        let group: Vec<_> = rows.iter().filter(|r| r.fidelity == *fidelity).collect();
        if group.is_empty() {
            continue;
        }
        let total = group.iter().map(|r| r.count).sum();
        kit::section(
            ui,
            ("kicad-import-group", *title),
            title,
            *open,
            Some(total),
            |ui| {
                for row in group {
                    kit::value_row(ui, &row.count.to_string(), row.title);
                    if row.case == sparksmith_automation::kicad_import::Case::NetClassPatternRefused
                    {
                        for pattern in report.refused_patterns() {
                            kit::value_row(ui, &pattern.pattern, pattern.reason);
                        }
                    }
                    // Each net tie, mask side or part without a footprint,
                    // by name.
                    for named in report.named(row.case) {
                        kit::note(ui, &named);
                    }
                    // Each refused track piece or via, with the check that
                    // refused it.
                    let items: Vec<_> = report.refused_copper_of(row.case).collect();
                    let all_id = ui.make_persistent_id(("kicad-import-refused-all", row.title));
                    let all = ui.data(|d| d.get_temp::<bool>(all_id)).unwrap_or(false);
                    let max = if all { items.len() } else { kit::LIST_MAX_ROWS };
                    let listed = kit::list_frame(ui, items.len(), max, |ui, index| {
                        if let Some(item) = items.get(index) {
                            kit::value_row(ui, &item.what, &item.check);
                        }
                    });
                    if listed.show_all {
                        ui.data_mut(|d| d.insert_temp(all_id, true));
                    }
                }
            },
        );
    }
    if report.count(sparksmith_automation::kicad_import::Case::TrackRefused) > 0 {
        kit::note(
            ui,
            "Copper a refused track left is named in Issues: delete it or route on.",
        );
    }
}
