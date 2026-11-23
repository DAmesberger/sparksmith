//! Offline reader in the inspector slot. The help index belongs to the engine
//! context. Window state holds navigation, effective bindings and live issue actions.
use super::{App, command_palette::PaletteRow, shortcut_sheet};
use crate::{
    commands::{Context, Registry},
    issues::Fix,
    kit,
};
use egui::{Id, Ui};
use sparksmith_core::{
    Engine, ReportKey,
    help::{self, Block, Facts, HelpEntry, HelpId, HelpKind, Index, Inline},
    units::LengthUnit,
};
use sparksmith_ui::ui_kit;

pub(super) const PALETTE_PREFIX: &str = "help:";
const PAGE_SIZE: usize = 12;

/// The input changes much less often than egui repaints. Keep the ids
/// for precisely this library context and search text across repaints.
#[derive(Default)]
struct Matches {
    library: std::sync::Weak<sparksmith_core::library::standards::Context>,
    query: String,
    ids: Vec<HelpId>,
}

impl Matches {
    fn ids(
        &mut self,
        context: &std::sync::Arc<sparksmith_core::library::standards::Context>,
        index: &Index,
        query: &str,
    ) -> &[HelpId] {
        let library = std::sync::Arc::downgrade(context);
        if !self.library.ptr_eq(&library) || self.query != query {
            self.library = library;
            query.clone_into(&mut self.query);
            self.ids = help::search(index, query, &[], usize::MAX)
                .into_iter()
                .filter(visible)
                .map(|e| e.id.clone())
                .collect();
        }
        &self.ids
    }
}

#[derive(Clone, PartialEq, Eq, Default)]
enum Location {
    #[default]
    Search,
    Topic(HelpId),
}

#[derive(Clone, Default)]
struct Page {
    location: Location,
    query: String,
    fix: Option<(ReportKey, Fix)>,
}

#[derive(Default)]
pub(super) struct HelpSheet {
    pub(super) open: bool,
    pub(super) pending_context: bool,
    page: Page,
    back: Vec<Page>,
    next: Vec<Page>,
    focus: bool,
    bounds: Option<(egui::LayerId, egui::Rect)>,
    pub(super) focused: bool,
    selected: usize,
    navigation: kit::SearchKeys,
    matches: Matches,
    shown: usize,
    unit: Option<LengthUnit>,
    media: media::Images,
}

impl HelpSheet {
    pub(super) fn palette_rows(&mut self, engine: &Engine, text: &str) -> Vec<PaletteRow> {
        if text.trim().is_empty() {
            return vec![];
        }
        let Some(context) = engine.context() else {
            return vec![];
        };
        let Ok(index) = context.help() else {
            return vec![];
        };
        let q = text
            .trim()
            .strip_prefix(PALETTE_PREFIX)
            .unwrap_or(text)
            .trim();
        self.matches
            .ids(context, index, q)
            .iter()
            .take(PAGE_SIZE)
            .filter_map(|id| index.get(id))
            .map(|e| PaletteRow {
                id: format!("{PALETTE_PREFIX}{}", e.id),
                title: e.title.clone(),
                icon: "list",
                detail: e.summary.clone(),
                shortcut: None,
                enabled: true,
                recent: false,
            })
            .collect()
    }

    pub(super) fn close(&mut self) {
        self.open = false;
        self.focused = false;
        self.bounds = None;
        self.page.fix = None;
    }
    fn visit(&mut self, page: Page) {
        if self.page.location != page.location
            || self.page.query != page.query
            || self.page.fix.is_some()
            || page.fix.is_some()
        {
            if self.back.len() >= 128 {
                self.back.remove(0);
            }
            self.back.push(std::mem::replace(&mut self.page, page));
            self.next.clear();
            self.selected = 0;
            self.shown = PAGE_SIZE;
        }
        self.open = true;
    }
    pub(super) fn open_topic(&mut self, topic: &str) {
        if let Ok(id) = topic.parse::<HelpId>() {
            self.visit(Page {
                location: Location::Topic(id),
                ..Page::default()
            });
        }
        self.focus = true;
    }
    /// Consume navigation before the text field or canvas sees the key.
    pub(super) fn keys(&mut self, ctx: &egui::Context) {
        self.navigation = Default::default();
        self.focused = self.open
            && self.bounds.is_some_and(|(layer, bounds)| {
                ctx.memory(|m| m.focused())
                    .and_then(|id| ctx.read_response(id))
                    .is_some_and(|r| r.layer_id == layer && bounds.contains(r.rect.center()))
            });
        if !self.focused {
            return;
        }
        if crate::keys::pressed_navigation(ctx, "help.back") {
            self.previous();
        }
        if crate::keys::pressed_navigation(ctx, "help.forward") {
            self.forward();
        }
        if matches!(self.page.location, Location::Search)
            && ctx.memory(|m| m.has_focus(Id::unique("help-search")))
        {
            self.navigation = kit::search_keys(ctx);
            if self.navigation.escape {
                self.close();
            }
        }
    }

    pub(super) fn shortcuts(&mut self) {
        if self.open
            && matches!(&self.page.location, Location::Topic(id) if id.as_str() == "panel/keyboard-shortcuts")
        {
            self.close();
        } else {
            self.open_topic("panel/keyboard-shortcuts");
        }
    }
    pub(super) fn finding(
        &mut self,
        code: sparksmith_core::diagnostics::FindingCode,
        key: ReportKey,
        fix: Option<Fix>,
    ) {
        self.open_topic(&format!("finding/{code}"));
        self.page.fix = fix.map(|fix| (key, fix));
    }
    fn previous(&mut self) {
        if let Some(page) = self.back.pop() {
            self.next.push(std::mem::replace(&mut self.page, page));
            self.focus = true;
        }
    }
    fn forward(&mut self) {
        if let Some(page) = self.next.pop() {
            self.back.push(std::mem::replace(&mut self.page, page));
            self.focus = true;
        }
    }

    pub(super) fn show(
        &mut self,
        ui: &mut Ui,
        engine: &Engine,
        registry: &Registry,
        active: &[Context],
    ) -> Option<Fix> {
        self.bounds = Some((ui.layer_id(), ui.max_rect()));
        ui.horizontal_wrapped(|ui| {
            if ui_kit::button(ui, "Back", ui_kit::Button::Enabled(!self.back.is_empty()))
                .on_hover_text(crate::keys::labelled(ui.ctx(), "Back in help", "help.back"))
                .clicked()
            {
                self.previous();
            }
            if ui_kit::button(
                ui,
                "Forward",
                ui_kit::Button::Enabled(!self.next.is_empty()),
            )
            .on_hover_text(crate::keys::labelled(
                ui.ctx(),
                "Forward in help",
                "help.forward",
            ))
            .clicked()
            {
                self.forward();
            }
            if ui_kit::button(ui, "Close help", ui_kit::Button::Small).clicked() {
                self.close();
            }
        });
        let shortcut_page = matches!(&self.page.location, Location::Topic(id) if id.as_str() == "panel/keyboard-shortcuts");
        let hint = if shortcut_page {
            "Filter by name or key"
        } else {
            "Search help"
        };
        let field_id = Id::unique("help-search");
        let mut query = self.page.query.clone();
        let field = kit::search_field(
            ui,
            field_id,
            &mut query,
            hint,
            false,
            std::mem::take(&mut self.focus),
            256,
        );
        field.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, hint));
        if field.changed() {
            if shortcut_page || matches!(self.page.location, Location::Search) {
                self.page.query = query;
            } else {
                self.visit(Page {
                    location: Location::Search,
                    query,
                    fix: None,
                });
            }
            self.shown = PAGE_SIZE;
            self.selected = 0;
        }
        let Some(context) = engine.context() else {
            kit::refusal_wrapped(ui, "The shipped library/help folder is unavailable.");
            return None;
        };
        let index = match context.help() {
            Ok(index) => index,
            Err(e) => {
                kit::refusal_wrapped(ui, &e.to_string());
                return None;
            }
        };
        let mut clicked_fix = None;
        egui::ScrollArea::vertical()
            .id_salt((
                "help-reader",
                &self.page.query,
                match &self.page.location {
                    Location::Topic(id) => id.as_str(),
                    Location::Search => "",
                }
                .to_owned(),
            ))
            .auto_shrink([false, false])
            .show(ui, |ui| match self.page.location.clone() {
                Location::Search => self.results(ui, context, index, field.has_focus()),
                Location::Topic(id) => {
                    let Some(entry) = index.get(&id) else {
                        kit::refusal_wrapped(
                            ui,
                            &format!("The installed library/help folder has no {id} topic."),
                        );
                        return;
                    };
                    heading(ui, &entry.title);
                    if let Some(introduction) = entry.introduction() {
                        ui.label(introduction);
                    }
                    if id.as_str() == "panel/keyboard-shortcuts" {
                        shortcuts(ui, index, registry, active, &self.page.query);
                    }
                    if let Some(to) = topic(
                        ui,
                        entry,
                        registry,
                        index,
                        engine,
                        &mut self.unit,
                        &mut self.media,
                    ) {
                        self.open_topic(to.as_str());
                    }
                    if let Some((key, fix)) = &self.page.fix
                        && *key == engine.report_key()
                        && kit::primary(ui, fix.label(), true).clicked()
                    {
                        clicked_fix = Some(fix.clone());
                    }
                }
            });
        clicked_fix
    }

    fn results(
        &mut self,
        ui: &mut Ui,
        context: &std::sync::Arc<sparksmith_core::library::standards::Context>,
        index: &Index,
        typing: bool,
    ) {
        let hits = self.matches.ids(context, index, &self.page.query);
        if hits.is_empty() {
            kit::note(ui, "No help topics match.");
            return;
        }
        // Keep the search rank inside each kind. The keyboard follows the
        // same grouped order as the visible links.
        let limit = self.shown.max(PAGE_SIZE).min(hits.len());
        let mut rows: Vec<_> = hits[..limit]
            .iter()
            .filter_map(|id| index.get(id))
            .collect();
        rows.sort_by_key(|e| e.id.kind());
        let keys = typing.then(|| std::mem::take(&mut self.navigation));
        if let Some(keys) = &keys {
            if keys.down {
                self.selected = self.selected.saturating_add(1).min(limit - 1);
            }
            if keys.up {
                self.selected = self.selected.saturating_sub(1);
            }
        }
        let mut kind = None;
        let mut picked = None;
        for (i, entry) in rows.iter().enumerate() {
            if kind != Some(entry.id.kind()) {
                heading(ui, kind_label(entry.id.kind()));
                kind = Some(entry.id.kind());
            }
            let response = ui
                .selectable_label(typing && self.selected == i, &entry.title)
                .on_hover_text(&entry.summary);
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_role(egui::accesskit::Role::Link);
                node.set_label(entry.title.as_str());
            });
            if response.clicked() || (keys.as_ref().is_some_and(|k| k.enter) && self.selected == i)
            {
                picked = Some(entry.id.clone());
            }
            if typing && self.selected == i && keys.as_ref().is_some_and(|k| k.up || k.down) {
                response.scroll_to_me(None);
            }
        }
        if limit < hits.len()
            && kit::action_row(
                ui,
                &format!("Show more ({} remaining)", hits.len() - limit),
                "Show the next group of matches",
            )
        {
            self.shown = limit.saturating_add(PAGE_SIZE);
        }
        if let Some(id) = picked {
            self.open_topic(id.as_str());
        }
    }
}

fn kind_label(kind: HelpKind) -> &'static str {
    match kind {
        HelpKind::Entry => "Surface entries",
        HelpKind::Command => "Commands",
        HelpKind::Finding => "Findings",
        HelpKind::Generator => "Generators",
        HelpKind::Guide => "Guides",
        HelpKind::Panel => "Panels",
        HelpKind::Rule => "Profile rules",
        HelpKind::Setting => "Settings",
        HelpKind::Term => "Glossary",
        HelpKind::Workflow => "Workflows",
    }
}

fn heading(ui: &mut Ui, text: &str) {
    let r = ui.heading(text);
    ui.ctx().accesskit_node_builder(r.id, |node| {
        node.set_role(egui::accesskit::Role::Heading);
        node.set_label(text);
    });
}

fn topic_link(ui: &mut Ui, index: &Index, id: &HelpId, target: &mut Option<HelpId>) {
    let label = index
        .get(id)
        .map_or(id.as_str(), |entry| entry.title.as_str());
    if ui.link(label).clicked() {
        *target = Some(id.clone());
    }
}

fn inlines(
    ui: &mut Ui,
    parts: &[Inline],
    registry: &Registry,
    index: &Index,
    target: &mut Option<HelpId>,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for part in parts {
            match part {
                Inline::Text(s) => {
                    ui.label(s);
                }
                Inline::Reference(id) => topic_link(ui, index, id, target),
                Inline::Key(id) => {
                    let key = registry.get(id.name()).and_then(|c| registry.shortcut(c));
                    crate::shell::key_cap(ui, key.as_deref().unwrap_or("Unbound"));
                }
            }
        }
    });
}

fn shortcuts(ui: &mut Ui, index: &Index, registry: &Registry, active: &[Context], query: &str) {
    kit::note(ui, crate::reveal::hint_line());
    let groups = shortcut_sheet::groups(index, registry, query, active);
    if groups.is_empty() {
        kit::note(ui, "No shortcut matches.");
    }
    for group in groups {
        let label = if group.here {
            format!("{} · here", group.context.label())
        } else {
            group.context.label().to_owned()
        };
        kit::section(
            ui,
            ("shortcut-list", group.context),
            &label,
            true,
            Some(group.rows.len()),
            |ui| {
                for (title, key) in &group.rows {
                    kit::value_row(ui, title, key);
                }
            },
        );
    }
}
mod media;

fn display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => "None".into(),
        other => other.to_string(),
    }
}

fn quantity(
    ui: &mut Ui,
    label: &str,
    value: &serde_json::Value,
    suffix: &str,
    unit: &mut Option<LengthUnit>,
) {
    if let Some(source_unit) = LengthUnit::from_suffix(suffix)
        && let Some(n) = value.as_f64()
        && let Ok(length) = sparksmith_core::units::parse_length(&n.to_string(), source_unit)
    {
        let shown = unit.get_or_insert(source_unit);
        let choices = LengthUnit::ALL.map(|u| (u, u.symbol().to_owned()));
        kit::choice_row(ui, "Length unit", shown, &choices);
        kit::value_row(
            ui,
            label,
            &sparksmith_core::units::format_length(length, *shown),
        );
    } else {
        kit::value_row(ui, label, format!("{} {suffix}", display(value)).trim_end());
    }
}

fn example(ui: &mut Ui, call: &help::CallExample, index: &Index, target: &mut Option<HelpId>) {
    topic_link(ui, index, &call.entry, target);
    if let Ok(text) = serde_json::to_string_pretty(&call.params) {
        ui.monospace(text);
    }
}

fn topic(
    ui: &mut Ui,
    entry: &HelpEntry,
    registry: &Registry,
    index: &Index,
    engine: &Engine,
    unit: &mut Option<LengthUnit>,
    media: &mut media::Images,
) -> Option<HelpId> {
    let mut target = None;
    match &entry.facts {
        Facts::Command {
            context, reaches, ..
        } => {
            kit::value_row(ui, "Context", context);
            let key = registry
                .get(entry.id.name())
                .and_then(|c| registry.shortcut(c));
            kit::value_row(ui, "Shortcut", key.as_deref().unwrap_or("Unbound"));
            if let Some(id) = reaches {
                topic_link(ui, index, id, &mut target);
            }
        }
        Facts::Finding {
            severity,
            blocks,
            cause,
            explanation,
            fix,
            fixed_by,
            checks,
            rule_key,
            ..
        } => {
            use sparksmith_core::issues::Severity;
            let level = match severity {
                Severity::Error => "× Error",
                Severity::Warning => "△ Warning",
                Severity::ToDo => "○ To do",
                Severity::Info => "ⓘ Information",
            };
            kit::value_row(ui, "Severity", level);
            if let Some(stage) = blocks {
                kit::value_row(
                    ui,
                    "Blocks",
                    match stage {
                        sparksmith_core::Stage::Publish => "Publishing",
                        sparksmith_core::Stage::BoardPlaceable => "Board placement",
                        sparksmith_core::Stage::ElectricallyResolved => "Electrical resolution",
                    },
                );
            }
            heading(ui, "Cause");
            ui.label(cause);
            if let Some(text) = explanation {
                ui.label(text);
            }
            heading(ui, "Fix");
            ui.label(fix);
            if let Some(profile) = &engine.project().rules.profile
                && let Some(key) = rule_key
                    .as_deref()
                    .and_then(sparksmith_core::rules::RuleKey::from_name)
                && let Some((_, row)) = profile.active(key)
            {
                heading(ui, &profile.name);
                let value = key.get(profile.rules());
                match value {
                    sparksmith_core::rules::RuleValue::Length(mm) => {
                        quantity(ui, &key.label(), &mm.into(), "mm", unit)
                    }
                    other => {
                        kit::value_row(ui, &key.label(), &crate::pcb_editor::rule_text(key, other))
                    }
                }
                if !row.is_empty() {
                    ui.label(row);
                }
                source_link(ui, &profile.source.document, &profile.source.url);
                let id = format!("rule/{}/{}", profile.id, key.name())
                    .parse::<HelpId>()
                    .ok();
                if let Some(id) = id.filter(|id| index.contains_key(id)) {
                    topic_link(ui, index, &id, &mut target);
                }
            }
            if let Some(id) = checks {
                topic_link(ui, index, id, &mut target);
            }
            for id in fixed_by {
                topic_link(ui, index, id, &mut target);
            }
        }
        Facts::Rule {
            value,
            unit: suffix,
            row,
            source,
            author_choice,
        } => {
            quantity(
                ui,
                if *author_choice {
                    "Author’s choice"
                } else {
                    "Limit"
                },
                value,
                suffix,
                unit,
            );
            if row != &entry.summary {
                ui.label(row);
            }
            source_link(ui, &source.document, &source.url);
            kit::value_row(ui, "Publisher", &source.publisher);
            kit::value_row(ui, "Read on", &source.date);
        }
        Facts::Setting {
            schema,
            unit: suffix,
            default,
            source,
        } => {
            quantity(
                ui,
                "Default",
                default,
                suffix.as_deref().unwrap_or(""),
                unit,
            );
            for (key, label) in [
                ("type", "Type"),
                ("minimum", "Minimum"),
                ("maximum", "Maximum"),
            ] {
                if let Some(value) = schema.get(key) {
                    kit::value_row(ui, label, &display(value));
                }
            }
            heading(ui, "Default source");
            ui.label(source);
        }
        Facts::Entry {
            module,
            class,
            schema_digest,
            examples,
            entry_kind,
        } => {
            kit::value_row(ui, "Module", module);
            if let Some(kind) = entry_kind.and_then(|kind| serde_json::to_value(kind).ok()) {
                kit::value_row(ui, "Kind", &display(&kind));
            }
            kit::value_row(ui, "Class", class.name());
            kit::value_row(ui, "Schema", schema_digest);
            for call in examples {
                example(ui, call, index, &mut target);
            }
        }
        Facts::Term {
            definition, avoid, ..
        } => {
            if definition != &entry.summary {
                ui.label(definition);
            }
            if !avoid.is_empty() {
                kit::value_row(ui, "Words to avoid", &avoid.join(", "));
            }
        }
        Facts::Generator {
            parameters,
            presets,
        } => {
            for param in parameters {
                let base = entry
                    .id
                    .as_str()
                    .split('#')
                    .next()
                    .unwrap_or(entry.id.as_str());
                let id: Option<HelpId> = format!("{base}#{}", param.key).parse().ok();
                if let Some(id) = id.filter(|id| *id != entry.id && index.contains_key(id)) {
                    topic_link(ui, index, &id, &mut target);
                } else {
                    heading(ui, &param.label);
                }
                kit::value_row(ui, "Type", &help::parameter_type(&param.ty));
                if let Ok(value) = help::parameter_value(&param.ty, &param.default) {
                    kit::value_row(ui, "Default", &value);
                }
                if let Some(note) = &param.help {
                    ui.label(note);
                }
            }
            if !presets.is_empty() {
                heading(ui, "Presets");
            }
            for preset in presets {
                kit::value_row(ui, &preset.name, &preset.description);
            }
        }
        Facts::Guide | Facts::Workflow | Facts::Panel => {}
    }
    for (n, block) in entry.body.iter().enumerate() {
        match block {
            Block::Paragraph(parts) => inlines(ui, parts, registry, index, &mut target),
            Block::Note(parts) => {
                ui.group(|ui| inlines(ui, parts, registry, index, &mut target));
            }
            Block::List(items) => {
                let list = ui.vertical(|ui| {
                    for parts in items {
                        let row = ui.horizontal_top(|ui| {
                            ui.label("•");
                            inlines(ui, parts, registry, index, &mut target);
                        });
                        ui.ctx().accesskit_node_builder(row.response.id, |node| {
                            node.set_role(egui::accesskit::Role::ListItem)
                        });
                    }
                });
                ui.ctx().accesskit_node_builder(list.response.id, |node| {
                    node.set_role(egui::accesskit::Role::List)
                });
            }
            Block::Table(rows) => {
                egui::ScrollArea::horizontal()
                    .id_salt(("help-table", n))
                    .show(ui, |ui| {
                        egui::Grid::new(("help-grid", n))
                            .striped(true)
                            .show(ui, |ui| {
                                for row in rows {
                                    for cell in row {
                                        inlines(ui, cell, registry, index, &mut target);
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            }
            Block::Example(call) => example(ui, call, index, &mut target),
            Block::Image { file, alt, .. } => media.show(ui, file, alt),
        }
    }
    if !entry.related.is_empty() {
        heading(ui, "Related");
    }
    for id in &entry.related {
        topic_link(ui, index, id, &mut target);
    }
    target
}

fn source_link(ui: &mut Ui, label: &str, url: &str) {
    if url.starts_with("https://") || url.starts_with("http://") {
        ui.hyperlink_to(label, url);
    } else {
        ui.label(label);
    }
}

fn visible(entry: &&HelpEntry) -> bool {
    !matches!(
        &entry.facts,
        Facts::Term {
            developer_only: true,
            ..
        }
    )
}

/// The most specific panel or field containing this frame's pointer or
/// keyboard focus. Stored for this pass only; closed panels lose their target.
pub(crate) fn region(ui: &Ui, rect: egui::Rect, topic: &str, specificity: u8) {
    let focus = ui
        .memory(|m| m.focused())
        .and_then(|id| ui.ctx().read_response(id));
    let focused =
        focus.is_some_and(|r| r.layer_id == ui.layer_id() && rect.contains(r.rect.center()));
    if !focused && !ui.rect_contains_pointer(rect) {
        return;
    }
    let pass = ui.ctx().cumulative_pass_nr();
    let id = Id::unique("context-help-topic");
    ui.data_mut(|d| {
        if d.get_temp::<(u64, u8, String)>(id)
            .is_none_or(|(at, pri, _)| at != pass || pri <= specificity)
        {
            d.insert_temp(id, (pass, specificity, topic.to_owned()));
        }
    });
}

/// A Settings control and its question button share one id and one row.
pub(super) fn setting<R>(
    ui: &mut Ui,
    topic: &str,
    label: &str,
    draw: impl FnOnce(&mut Ui) -> R,
) -> R {
    let row = ui.push_id(topic, |ui| {
        ui.horizontal(|ui| {
            let t = sparksmith_ui::theme::Tokens::get();
            let width = (ui.available_width() - t.control_height - t.space_8).max(0.0);
            let value = ui
                .allocate_ui(egui::vec2(width, t.control_height), draw)
                .inner;
            let button = ui_kit::button(ui, "?", ui_kit::Button::Small)
                .on_hover_text(format!("Help for {label}"));
            button.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Help for {label}"))
            });
            if button.clicked() {
                ui.data_mut(|d| d.insert_temp(Id::unique("open-help-topic"), topic.to_owned()));
            }
            value
        })
        .inner
    });
    region(ui, row.response.rect, topic, 3);
    row.inner
}

impl App {
    pub(super) fn resolve_context_help(&mut self, ctx: &egui::Context) {
        let clicked = ctx.data_mut(|d| d.remove_temp::<String>(Id::unique("open-help-topic")));
        if clicked.is_none() && !std::mem::take(&mut self.help.pending_context) {
            return;
        }
        self.help.pending_context = false;
        let pass = ctx.cumulative_pass_nr();
        let panel = ctx
            .data(|d| d.get_temp::<(u64, u8, String)>(Id::unique("context-help-topic")))
            .filter(|(at, _, _)| *at == pass);
        let mut controls = vec![];
        crate::reveal::copy_targets(ctx, &mut controls);
        let focused = ctx
            .memory(|m| m.focused())
            .and_then(|id| ctx.read_response(id));
        let command = ctx
            .pointer_hover_pos()
            .into_iter()
            .chain(focused.map(|r| r.rect.center()))
            .find_map(|at| {
                controls
                    .iter()
                    .rev()
                    .find(|(_, rect)| rect.contains(at))
                    .and_then(|(target, _)| {
                        super::key_reveal::command_of(&self.registry, self.workspace, *target)
                    })
            })
            .map(|c| format!("command/{}", c.id));
        let id = clicked
            .or_else(|| {
                panel
                    .as_ref()
                    .filter(|(_, priority, _)| *priority >= 2)
                    .map(|(_, _, id)| id.clone())
            })
            .or(command)
            .or_else(|| panel.map(|(_, _, id)| id))
            .or_else(|| {
                self.settings_window
                    .is_open()
                    .then(|| "panel/settings".into())
            })
            .or_else(|| self.messages_open.then(|| "panel/messages".into()))
            .or_else(|| self.palette.is_open().then(|| "panel/palette".into()));
        if let Some(id) = id {
            self.help.open_topic(&id);
        } else if let Some((code, fix)) =
            self.issues.selection_help(&self.engine, &self.selection())
        {
            self.help.finding(code, self.engine.report_key(), fix);
        } else if self.workspace == crate::shell::Workspace::Pcb && self.pcb.is_3d() {
            self.help.open_topic("panel/model-3d");
        } else {
            self.help
                .open_topic(crate::commands::workspace_page(self.workspace));
        }
        self.palette.close();
        self.settings_window.close();
        self.messages_open = false;
        ctx.request_repaint();
    }
}
