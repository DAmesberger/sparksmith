//! Mechanical parts in Layout (docs/mechanical-parts.md M5): Place 3D part
//! (`pcb.place-part`, M) opens the part-file chooser in Place mode and
//! places the pick at the view centre on the side being viewed, with the
//! core `layout.place_part` built by the automation builder; the placed
//! part is then selected and moves, turns and changes side like any
//! footprint. The Placement card of a mechanical part gains a 3D part
//! section: its source, its parameters (each committed edit one
//! `layout.set_part`), the pad use and the BOM and pick-and-place flags.
//!
//! Place component (`pcb.place`) opens the Place palette over the board
//! (the schematic's palette, with its generators and favourites). A part
//! without pins it returns (a fiducial, a mounting hole, a logo) is put on
//! the board as a board-only part with one `component.place_board_only`,
//! at the view centre on the side being viewed, and selected; a part with
//! pins is refused with where to place it instead.
use super::PcbEditor;
use crate::kit;
use crate::part_chooser::{ChooserMode, ChooserPick, Evaluator, PartSource, values_form};
use egui::Ui;
use sparksmith_automation::session::mechanical_parts::{
    self as parts, PartFileSource, PlacePartFileParams, SetPartFileValuesParams,
};
use sparksmith_core::layout::LayoutCommand;
use sparksmith_core::library::mechanical::PadUse;
use sparksmith_core::model::Layer;
use sparksmith_core::model::PhysicalChoice;
use sparksmith_core::{
    BoardPlace, ComponentRef, DefinitionId, Engine, Id, InstanceCommand, Operation, Point,
    Project,
};
use sparksmith_solidcraft::models::part_file::PartValues;

/// The 3D part section's state: the instance it shows, the values being
/// edited and the evaluation that lists the part's parameters.
#[derive(Default)]
pub(super) struct PartSection {
    instance: Option<(Id, u64)>,
    editing: PartValues,
    committed: PartValues,
    evaluator: Evaluator,
    refusal: Option<String>,
}

fn chooser_source(source: &PartSource) -> PartFileSource {
    match source {
        PartSource::Path(path) => PartFileSource::Path(path.to_string_lossy().into_owned()),
        PartSource::Builtin(name) => PartFileSource::Builtin(name.clone()),
        PartSource::Model(model) => PartFileSource::Model(*model),
    }
}

/// The core `layout.place_part` of a chooser pick at `at` on `layer`.
pub(super) fn place_operation(
    engine: &Engine,
    pick: &ChooserPick,
    at: Point,
    layer: Layer,
) -> Result<Operation, String> {
    let mut params = PlacePartFileParams::new(chooser_source(&pick.source), at, engine.revision());
    params.values = pick.values.clone();
    params.layer = Some(layer);
    params.licence = pick.licence.clone();
    parts::place_part_operation(engine, &params)
        .map(|(operation, _)| operation)
        .map_err(|e| crate::part_chooser::shown_message(&e.to_string()))
}

/// The symbol and footprint `component.place_board_only` takes for
/// `component`, when it can go on the board alone: it has a footprint and
/// no pins.
pub fn board_part(project: &Project, component: ComponentRef) -> Option<(DefinitionId, PhysicalChoice)> {
    let published = project.library.component(component)?;
    if !published.definition.terminals.is_empty() {
        return None;
    }
    let first = sparksmith_core::instances::first_choices(project, component)?;
    Some((first.representation, first.physical?))
}

impl PcbEditor {
    /// Place component was chosen since the last call: open the Place
    /// palette over the board.
    pub fn take_place_request(&mut self) -> bool {
        std::mem::take(&mut self.place_request)
    }

    /// The revision a just-created component's placement was sent at,
    /// once: the app joins it to the create, so one undo takes back both.
    pub fn take_place_join(&mut self) -> Option<u64> {
        self.place_join.take()
    }

    /// Put `component`, chosen in the Place palette, on the board at the
    /// view centre on the side being viewed (`created`: the palette has
    /// just made it); a part with pins is refused with a toast.
    pub fn place_board_part(
        &mut self,
        ctx: &egui::Context,
        engine: &Engine,
        component: ComponentRef,
        created: bool,
    ) -> Option<Operation> {
        let project = engine.project();
        let Some((representation, physical)) = board_part(project, component) else {
            let name = project
                .library
                .component(component)
                .map_or_else(|| "The part".to_owned(), |c| c.meta.name.clone());
            let actions = if created {
                vec![kit::toast::ToastAction::Undo]
            } else {
                Vec::new()
            };
            kit::toast::post(
                ctx,
                crate::kit::severity::Level::Warning,
                crate::place_palette::board_refusal(&name),
                actions,
            );
            return None;
        };
        self.end_route();
        let centre = self
            .view_centre
            .unwrap_or([project.board_size[0] / 2.0, project.board_size[1] / 2.0]);
        let layer = if self.from_bottom {
            Layer::Bottom
        } else {
            Layer::Top
        };
        self.select_placed = Some(engine.revision());
        self.place_join = created.then(|| engine.revision());
        Some(Operation::Instance(InstanceCommand::PlaceBoardOnly {
            component,
            representation,
            physical,
            designator: None,
            at: Some(BoardPlace {
                position: centre.map(|v| (v * 2.0).round() / 2.0),
                rotation_deg: 0.0,
                layer,
            }),
        }))
    }

    /// Place 3D part…: open the chooser in Place mode.
    pub(super) fn open_part_chooser(&mut self) {
        self.end_route();
        self.part_chooser
            .open(ChooserMode::Place, None, PartValues::new());
    }

    /// Place 3D part… at the shipped part file `name`, as the Schematic
    /// part search hands one over.
    pub fn place_builtin_part(&mut self, name: String) {
        self.end_route();
        self.part_chooser.open(
            ChooserMode::Place,
            Some(crate::part_chooser::PartSource::Builtin(name)),
            PartValues::new(),
        );
    }

    /// Show the chooser while it is open and place its pick. The part the
    /// last placement made is selected once the engine has applied it.
    pub(super) fn part_chooser_frame(
        &mut self,
        ctx: &egui::Context,
        engine: &Engine,
    ) -> Option<Operation> {
        if let Some(before) = self.select_placed
            && engine.revision() > before
        {
            self.select_placed = None;
            if let Some(last) = engine.project().instances.last() {
                self.instance = Some(last.id);
            }
        }
        let pick = self.part_chooser.show(ctx, engine)?;
        let project = engine.project();
        let centre = self
            .view_centre
            .unwrap_or([project.board_size[0] / 2.0, project.board_size[1] / 2.0]);
        let at = centre.map(|v| (v * 2.0).round() / 2.0);
        let layer = if self.from_bottom {
            Layer::Bottom
        } else {
            Layer::Top
        };
        match place_operation(engine, &pick, at, layer) {
            Ok(operation) => {
                if let PartSource::Path(path) = &pick.source {
                    self.part_chooser.remember(path.clone());
                }
                self.part_chooser.close();
                self.select_placed = Some(engine.revision());
                Some(operation)
            }
            Err(message) => {
                self.part_chooser.refused(message);
                None
            }
        }
    }

    /// The 3D part section of a placed mechanical part; `None` for any
    /// other part.
    pub(super) fn part_section(
        &mut self,
        ui: &mut Ui,
        engine: &Engine,
        id: Id,
    ) -> Option<Operation> {
        let Ok(placed) = parts::placed_part(engine, id) else {
            return None;
        };
        let state = &mut self.part_section;
        let key = (id, engine.revision());
        if state.instance != Some(key) {
            if state.instance.map(|(i, _)| i) != Some(id) {
                state.refusal = None;
            }
            state.instance = Some(key);
            state.editing = placed.values.clone();
            state.committed = placed.values.clone();
        }
        let source = PartSource::Model(placed.model);
        state
            .evaluator
            .request(ui.ctx(), engine, &source, &state.committed);
        state.evaluator.poll();
        let mut values = None;
        let mut flags = None;
        let mut remove = false;
        kit::section(ui, "pcb-part", "3D part", true, None, |ui| {
            kit::summary_row(ui, &placed.record.source, None);
            match state.evaluator.last().map(|e| &e.result) {
                Some(Ok(checked)) => {
                    values = values_form(
                        ui,
                        "pcb-part",
                        &checked.report.parameters,
                        &mut state.editing,
                        &state.committed,
                    );
                }
                Some(Err(message)) => {
                    kit::note(ui, message);
                }
                None => {
                    kit::note(ui, "Reading the part file…");
                }
            }
            let mut terminals = placed.record.pads == PadUse::Terminals;
            let mut bom = placed.record.bom;
            let mut pnp = placed.record.pick_and_place;
            let has_copper = placed.lands.iter().any(|l| l.shape.copper());
            // Each copper land can get a terminal that joins a net through
            // the part's symbol on a sheet; a part without copper has none.
            let a = ui
                .add_enabled_ui(has_copper, |ui| {
                    kit::check_row(ui, "Pads land terminals", &mut terminals)
                })
                .inner;
            let b = kit::check_row(ui, "In BOM", &mut bom);
            let c = kit::check_row(ui, "In pick-and-place", &mut pnp);
            if a || b || c {
                flags = Some((
                    if terminals {
                        PadUse::Terminals
                    } else {
                        PadUse::Mechanical
                    },
                    bom,
                    pnp,
                ));
            }
            if let Some(message) = &state.refusal {
                kit::note(ui, message);
            }
            remove = ui
                .button("Remove from board")
                .on_hover_text("Remove this part and its placement (one undo step)")
                .clicked();
        });
        if remove {
            return Some(Operation::Layout(LayoutCommand::RemovePart {
                instance: id,
            }));
        }
        if values.is_none() && flags.is_none() {
            return None;
        }
        let mut params = SetPartFileValuesParams {
            instance: id,
            values: values.unwrap_or_else(|| state.committed.clone()),
            pads: None,
            bom: None,
            pick_and_place: None,
            expected_revision: engine.revision(),
        };
        if let Some((pads, bom, pnp)) = flags {
            params.pads = Some(pads);
            params.bom = Some(bom);
            params.pick_and_place = Some(pnp);
        }
        match parts::set_part_operation(engine, &params) {
            Ok((operation, _)) => {
                state.refusal = None;
                state.committed = params.values;
                Some(operation)
            }
            Err(error) => {
                state.refusal = Some(crate::part_chooser::shown_message(&error.to_string()));
                state.editing = state.committed.clone();
                None
            }
        }
    }
}
