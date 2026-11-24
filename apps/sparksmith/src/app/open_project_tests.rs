//! Opening another project: nothing started, typed, ticked or chosen for
//! the project that was open reaches the new one, and a dialog's OK never
//! overwrites what changed under it.
use super::ax_harness::{Ax, Fallible};
use super::*;
use crate::commands::PcbDialog;
use egui::accesskit::Role;
use sparksmith_core::schematic::SchematicCommand;

/// The buttons that commit a dialog, card or review.
const COMMITS: [&str; 13] = [
    "OK",
    "Apply",
    "Use",
    "Use these limits",
    "Place",
    "Export",
    "Export…",
    "Choose",
    "Assign",
    "Promote",
    "Publish",
    "Save",
    "Link",
];

fn rename(name: &str) -> Operation {
    Operation::Schematic(SchematicCommand::RenameProject { name: name.into() })
}

/// The second project: the first one's twin (equal ids) under its own name.
fn second() -> Fallible<EditorFile> {
    let mut file = Engine::demo()?.to_file();
    file.project.name = "Second project".into();
    Ok(file)
}

fn open_second(ax: &mut Ax) -> Fallible<sparksmith_core::Document> {
    let path = ax.dir.path().join("second.sparksmith");
    ax.app.replace(second()?, Some(path), None, false);
    assert_eq!(ax.app.engine.project().name, "Second project");
    Ok(ax.app.engine.document().clone())
}

/// The enabled buttons named `name`.
fn enabled(ax: &Ax, name: &str) -> Vec<egui::Pos2> {
    ax.all(Role::Button, name)
        .into_iter()
        .filter(|n| !n.disabled)
        .map(super::ax_harness::Node::center)
        .collect()
}

/// Press every commit button shown that the second project opened by
/// itself does not show (the ribbon has a "Place" of its own), and the keys
/// that commit.
fn press_all(ax: &mut Ax, workspace: Workspace) -> Fallible {
    let mut plain = Ax::new(workspace)?;
    open_second(&mut plain)?;
    plain.settle();
    for name in COMMITS {
        let shown = enabled(ax, name);
        if shown.len() > enabled(&plain, name).len() {
            for at in shown {
                ax.click_at(at);
            }
        }
    }
    ax.key(egui::Key::Enter);
    ax.key_with(egui::Key::Enter, egui::Modifiers::COMMAND);
    ax.settle();
    Ok(())
}

fn unchanged(ax: &Ax, before: &sparksmith_core::Document, what: &str) {
    assert_eq!(ax.app.engine.document(), before, "{what}");
    assert!(!ax.app.engine.can_undo(), "{what}");
}

type Stage = fn(&mut Ax) -> Fallible;

/// Every dialog, sheet, card and review that can be open, opened (and
/// edited where it holds a draft) on the first project.
fn stages() -> Vec<(&'static str, Workspace, Stage)> {
    fn dialog(ax: &mut Ax, dialog: PcbDialog) -> Fallible {
        let project = ax.app.engine.project_arc().clone();
        ax.app.pcb.open_dialog(dialog, &project);
        Ok(())
    }
    vec![
        ("board setup", Workspace::Pcb, |ax| {
            let project = ax.app.engine.project_arc().clone();
            ax.app.pcb.stage_board_setup(&project);
            Ok(())
        }),
        ("stackup", Workspace::Pcb, |ax| {
            let project = ax.app.engine.project_arc().clone();
            ax.app.pcb.stage_stackup(&project);
            Ok(())
        }),
        ("rules", Workspace::Pcb, |ax| dialog(ax, PcbDialog::Rules)),
        ("net classes", Workspace::Pcb, |ax| {
            dialog(ax, PcbDialog::NetClasses)
        }),
        ("high speed", Workspace::Pcb, |ax| {
            dialog(ax, PcbDialog::HighSpeed)
        }),
        ("variants", Workspace::Pcb, |ax| {
            dialog(ax, PcbDialog::Variants)
        }),
        ("production", Workspace::Pcb, |ax| {
            let _ = ax.app.pcb.on_tool("production", &ax.app.engine);
            Ok(())
        }),
        ("update PCB review", Workspace::Pcb, |ax| {
            let _ = ax.app.pcb.on_tool("update", &ax.app.engine);
            Ok(())
        }),
        ("place 3D part", Workspace::Pcb, |ax| {
            let parts = sparksmith_automation::part_files::builtin_parts()?;
            let name = parts.all().first().ok_or("a part file")?.name.clone();
            ax.app.pcb.place_builtin_part(name);
            Ok(())
        }),
        ("place component", Workspace::Schematic, |ax| {
            let _ = ax.app.schematic.on_tool("place", &ax.app.engine);
            Ok(())
        }),
        ("choose part", Workspace::Schematic, |ax| {
            let instance = ax.app.engine.project().instances[0].id;
            ax.app
                .schematic
                .palette_mut()
                .open_assign(String::new(), vec![instance]);
            Ok(())
        }),
        ("export to SolidCraft", Workspace::Pcb, |ax| {
            ax.app.files.request(crate::file_ui::Action::Export);
            Ok(())
        }),
        ("promote", Workspace::Schematic, |ax| {
            let component = ax.app.engine.project().instances[0].component;
            ax.app.sources.begin_promote(component, "Resistor".into());
            Ok(())
        }),
        ("component", Workspace::Part, |ax| {
            let component = ax.app.engine.project().instances[0].component.id;
            ax.app.part.open(component, &ax.app.engine);
            Ok(())
        }),
    ]
}

/// Whatever was open on the first project, opening a second one closes it:
/// no commit still shown, and no key, changes the second project.
#[test]
fn no_dialog_reaches_the_project_opened_next() -> Fallible {
    for (what, workspace, stage) in stages() {
        let mut ax = Ax::new(workspace)?;
        let plain: Vec<usize> = COMMITS.iter().map(|n| enabled(&ax, n).len()).collect();
        let nodes = ax.dump();
        stage(&mut ax)?;
        ax.settle();
        // It is open: what follows is not a test of nothing.
        let now: Vec<usize> = COMMITS.iter().map(|n| enabled(&ax, n).len()).collect();
        assert!(now != plain || ax.dump() != nodes, "{what} did not open");
        let before = open_second(&mut ax)?;
        ax.settle();
        assert_eq!(ax.app.pcb.open_dialog_now(), None, "{what}");
        assert!(!ax.app.pcb.production_open(), "{what}");
        assert!(!ax.app.pcb.review_mut().is_open(), "{what}");
        assert!(!ax.app.schematic.palette_mut().is_open(), "{what}");
        assert_eq!(ax.app.part.lineage(), None, "{what}");
        press_all(&mut ax, workspace)?;
        unchanged(&ax, &before, what);
    }
    Ok(())
}

/// Background work and waiting calls, each started on the first project
/// and answered once the second is open: nothing of the second changes.
#[test]
fn no_background_result_reaches_the_project_opened_next() -> Fallible {
    let jobs: Vec<(&'static str, Stage)> = vec![
        ("a linked design read", |ax| {
            ax.app.pcb.stage_link(rename("Linked"));
            Ok(())
        }),
        ("an accepted workflow card", |ax| {
            ax.app
                .workflow_cards
                .outbox
                .push(crate::workflow_cards::CardAction::Start {
                    card: "card-1".into(),
                    workflow: "datasheet-to-component".into(),
                    inputs: Default::default(),
                });
            Ok(())
        }),
        ("a command held for a field", |ax| {
            let ctx = ax.ctx.clone();
            ax.app.after_commit.hold(&ctx, "pcb.select-all".into());
            Ok(())
        }),
        ("an expanded sheet", |ax| {
            let (send, result) = std::sync::mpsc::channel();
            let _ = send.send(Err(sparksmith_core::Error::Invalid("old".into())));
            ax.app.sheet_work = Some(SheetWork {
                cancel: Default::default(),
                result,
                origin: Origin::Command,
            });
            Ok(())
        }),
        ("text being typed", |ax| {
            let ctx = ax.ctx.clone();
            ctx.data_mut(|d| d.insert_temp(egui::Id::unique("typed"), "42".to_owned()));
            Ok(())
        }),
    ];
    for (what, stage) in jobs {
        let mut ax = Ax::new(Workspace::Pcb)?;
        stage(&mut ax)?;
        let before = open_second(&mut ax)?;
        // Gone at once, before a frame could carry it out.
        assert!(ax.app.sheet_work.is_none(), "{what}");
        assert!(ax.app.after_commit.take().is_empty(), "{what}");
        assert!(ax.app.workflow_cards.outbox.is_empty(), "{what}");
        assert!(ax.app.assistant.notes.is_empty(), "{what}");
        assert!(!ax.app.generators.is_waiting(), "{what}");
        assert!(
            ax.ctx
                .data(|d| d.get_temp::<String>(egui::Id::unique("typed")))
                .is_none(),
            "{what}: typed text stayed"
        );
        ax.settle();
        press_all(&mut ax, Workspace::Pcb)?;
        unchanged(&ax, &before, what);
        assert!(
            !ax.app.messages.entries().any(|m| m.text.contains("old")),
            "{what}: the old project's refusal was shown"
        );
    }
    Ok(())
}

/// An operation made for a document that is no longer the open one is
/// refused, whatever path replaced the document.
#[test]
fn an_operation_of_another_document_is_refused() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    // Not through any path that resets the editors.
    ax.app.engine.replace_document(second()?)?;
    let before = ax.app.engine.document().clone();
    let ctx = ax.ctx.clone();
    ax.app.apply(&ctx, Origin::Command, Some(rename("Late")));
    unchanged(&ax, &before, "a late operation");
    ax.app.apply(&ctx, Origin::Command, Some(rename("New")));
    assert_eq!(ax.app.engine.project().name, "New");
    Ok(())
}

/// Undo, redo or another client's edit under an open Board setup or Stackup
/// dialog: OK does not put the whole old draft back over it.
#[test]
fn a_dialog_never_overwrites_what_changed_under_it() -> Fallible {
    type Open = fn(&mut pcb_editor::PcbEditor, &sparksmith_core::Project);
    let dialogs: [(&str, Open); 2] = [
        ("board setup", |pcb, project| pcb.stage_board_setup(project)),
        ("stackup", |pcb, project| pcb.stage_stackup(project)),
    ];
    for (what, open) in dialogs {
        let mut ax = Ax::new(Workspace::Pcb)?;
        let project = ax.app.engine.project_arc().clone();
        open(&mut ax.app.pcb, &project);
        ax.settle();
        // Another client's edit of both.
        let mut fabrication = project.fabrication;
        fabrication.mask_colour = sparksmith_core::MaskColour::Yellow;
        let mut stackup = project.stackup.clone();
        stackup.dielectrics[0].thickness_mm += 0.3;
        use sparksmith_core::layout::{LayoutCommand, restack::StackupEdit};
        for operation in [
            LayoutCommand::SetFabrication(fabrication),
            LayoutCommand::EditStackup(StackupEdit {
                stackup,
                moves: Vec::new(),
                removed: Vec::new(),
            }),
        ] {
            let revision = ax.app.engine.revision();
            ax.app
                .engine
                .apply(revision, Operation::Layout(operation))?;
        }
        let before = ax.app.engine.document().clone();
        ax.settle();
        ax.button("OK")?;
        ax.settle();
        assert_eq!(
            ax.app.engine.project().fabrication,
            before.project.fabrication,
            "{what}"
        );
        assert_eq!(
            ax.app.engine.project().stackup,
            before.project.stackup,
            "{what}"
        );
        assert!(
            ax.texts
                .iter()
                .any(|t| t.contains("changed while this was open")),
            "{what}: {:?}",
            ax.texts
        );
        // Opened again, it edits what is there now.
        ax.button("Cancel")?;
        let project = ax.app.engine.project_arc().clone();
        open(&mut ax.app.pcb, &project);
        ax.settle();
        ax.button("OK")?;
        ax.settle();
        assert_ne!(ax.app.engine.document(), &before, "{what}");
    }
    Ok(())
}

/// What the person set up for any project stays: the view, the snap and
/// the chat column; what they type to the assistant too.
#[test]
fn the_persons_view_and_message_stay() -> Fallible {
    let mut ax = Ax::new(Workspace::Pcb)?;
    ax.app.pcb.set_3d(true);
    ax.app.pcb.set_shows_interface(true);
    ax.app.assistant.open = true;
    ax.app.assistant.composer = "Route the".into();
    open_second(&mut ax)?;
    ax.settle();
    assert!(ax.app.pcb.is_3d() && ax.app.pcb.shows_interface());
    assert!(ax.app.assistant.shown());
    assert_eq!(ax.app.assistant.composer, "Route the");
    Ok(())
}
