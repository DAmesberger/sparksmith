//! The window as the front door reaches it (`docs/extension-surface.md`,
//! slice S5a): the `view` entries an assistant sends over live MCP
//! (`view.context`, `view.select`, `view.open`) and the busy rule.
//!
//! The person's own view commands run the same functions: Workspace, the
//! PCB's 2D/3D switch and the 3D commands that open it, and a Browser
//! sheet ([`Window::show_workspace`], [`Window::show_pcb`],
//! [`Window::show_sheet`]); `view.select` locates as an issue row does
//! ([`crate::issues::route`]). All of it is selection and view state: no
//! revision, no undo step and no rebuilt geometry.
use super::App;
use crate::issues;
use crate::kit::toast::Toasts;
use crate::part_editor::PartEditor;
use crate::pcb_editor::PcbEditor;
use crate::schematic_editor::SchematicEditor;
use crate::shell::Workspace;
use sparksmith_automation::WindowHost;
use sparksmith_automation::front::Error;
use sparksmith_automation::view::{self, Context, OpenParams, PcbView, Selection};
use sparksmith_core::{Engine, ObjectRef, SheetId};

/// The window's view state, borrowed from the app for one call or command.
pub(super) struct Window<'a> {
    pub workspace: &'a mut Workspace,
    pub part: &'a mut PartEditor,
    pub schematic: &'a mut SchematicEditor,
    pub pcb: &'a mut PcbEditor,
    pub toasts: &'a mut Toasts,
    /// Why the person holds the window now, for the busy rule.
    pub busy: Option<String>,
}

impl App {
    /// The window's view state, the person not counted as busy.
    pub(super) fn window(&mut self) -> Window<'_> {
        self.window_and_engine().0
    }

    /// The window's view state and the engine it shows.
    pub(super) fn window_and_engine(&mut self) -> (Window<'_>, &Engine) {
        let window = Window {
            workspace: &mut self.workspace,
            part: &mut self.part,
            schematic: &mut self.schematic,
            pcb: &mut self.pcb,
            toasts: &mut self.toasts,
            busy: None,
        };
        (window, &self.engine)
    }

    /// The window with `busy`, the engine and live MCP, for running an
    /// assistant's call on the window's engine.
    pub(super) fn window_for_live(
        &mut self,
        busy: Option<String>,
    ) -> (Window<'_>, &mut Engine, &mut super::live_mcp::LiveMcp) {
        let window = Window {
            workspace: &mut self.workspace,
            part: &mut self.part,
            schematic: &mut self.schematic,
            pcb: &mut self.pcb,
            toasts: &mut self.toasts,
            busy,
        };
        (window, &mut self.engine, &mut self.live)
    }

    /// The shown workspace's selection, as issue objects.
    pub(super) fn selection(&self) -> Vec<ObjectRef> {
        selection(
            self.workspace,
            &self.part,
            &self.schematic,
            &self.pcb,
            &self.engine,
        )
    }
}

impl Window<'_> {
    /// Show `workspace`; leaving one dismisses its error toasts.
    pub fn show_workspace(&mut self, workspace: Workspace) {
        if workspace != *self.workspace {
            self.toasts.dismiss_errors();
            *self.workspace = workspace;
        }
    }

    /// Show the PCB in its 3D view or its Layout view.
    pub fn show_pcb(&mut self, three_d: bool) {
        self.show_workspace(Workspace::Pcb);
        if self.pcb.is_3d() != three_d {
            self.pcb.set_3d(three_d);
        }
    }

    /// Show the schematic at `sheet`.
    pub fn show_sheet(&mut self, engine: &Engine, sheet: SheetId) {
        self.show_workspace(Workspace::Schematic);
        self.schematic.show_sheet(sheet, engine.project());
    }
}

/// The selection `workspace` shows, as issue objects.
fn selection(
    workspace: Workspace,
    part: &PartEditor,
    schematic: &SchematicEditor,
    pcb: &PcbEditor,
    engine: &Engine,
) -> Vec<ObjectRef> {
    match workspace {
        Workspace::Part => {
            issues::selection(workspace, part.lineage(), None, None, None, None, vec![])
        }
        Workspace::Schematic => issues::selection(
            workspace,
            None,
            schematic.selected_instance(),
            None,
            None,
            schematic.selected_net(engine),
            schematic.selected_item_refs(engine.project()),
        ),
        Workspace::Pcb => issues::selection(
            workspace,
            None,
            pcb.selected_instance(),
            pcb.selected_track(),
            pcb.selected_via(),
            pcb.highlighted_net(),
            vec![],
        ),
    }
}

fn wire(workspace: Workspace) -> view::Workspace {
    match workspace {
        Workspace::Part => view::Workspace::Component,
        Workspace::Schematic => view::Workspace::Schematic,
        Workspace::Pcb => view::Workspace::Pcb,
    }
}

impl WindowHost for Window<'_> {
    fn busy(&self) -> Option<String> {
        self.busy.clone()
    }

    fn context(&self, engine: &Engine) -> Context {
        let workspace = *self.workspace;
        let view = (workspace == Workspace::Pcb).then(|| {
            if self.pcb.is_3d() {
                PcbView::ThreeD
            } else {
                PcbView::Layout
            }
        });
        let sheet = (workspace == Workspace::Schematic).then(|| self.schematic.active_sheet());
        Context::new(
            engine,
            wire(workspace),
            view,
            sheet,
            selection(workspace, self.part, self.schematic, self.pcb, engine),
        )
    }

    fn select(&mut self, engine: &Engine, objects: &[ObjectRef]) -> Result<Selection, Box<Error>> {
        let mut editors = issues::Editors {
            part: &mut *self.part,
            schematic: &mut *self.schematic,
            pcb: &mut *self.pcb,
        };
        let shown =
            issues::route(&mut editors, engine, objects, *self.workspace).ok_or_else(|| {
                Box::new(sparksmith_automation::gate::refusal(
                    "view.not_shown",
                    "view.select",
                    "no workspace shows these objects. Nothing was selected.",
                    "Select objects a workspace draws: parts, tracks, vias, nets, wires.",
                ))
            })?;
        self.show_workspace(shown);
        Ok(Selection {
            selected: objects.len(),
            workspace: wire(shown),
        })
    }

    fn open(&mut self, engine: &Engine, params: &OpenParams) -> Result<Context, Box<Error>> {
        match params.workspace {
            view::Workspace::Pcb => {
                let three_d = params
                    .view
                    .map_or(self.pcb.is_3d(), |view| view == PcbView::ThreeD);
                self.show_pcb(three_d);
            }
            view::Workspace::Schematic => match params.sheet {
                Some(sheet) => self.show_sheet(engine, sheet),
                None => self.show_workspace(Workspace::Schematic),
            },
            view::Workspace::Component => {
                // The front door checked it is a draft, so opening it
                // drafts nothing.
                if let Some(definition) = params.definition {
                    self.part.open(definition, engine);
                }
                self.show_workspace(Workspace::Part);
            }
        }
        Ok(self.context(engine))
    }
}
