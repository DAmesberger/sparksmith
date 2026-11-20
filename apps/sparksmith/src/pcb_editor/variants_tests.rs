//! The Assembly variants card, the part menu's population row and the
//! cross over a part the active variant leaves off, on a demo board with
//! an invented 0 Ω Part (`ExampleCo EX-R0603-0R`) of the demo resistor.
//!
//! With `SPARKSMITH_PICTURES` set, `variant_pictures` writes PNGs of the
//! card, the board and the Placement card there.
use super::super::raster_tests::picture_of;
use super::super::{PcbEditor, board_view, menu};
use super::*;
use crate::context_menu::Run;
use egui::{Event, PointerButton, Pos2, Rect, Shape, vec2};
use sparksmith_core::library::{
    Basis, Definition, DefinitionMeta, DistributorOffer, DraftEdit, PartDefinition, Provenance,
    Reference, ReferenceKind,
};
use sparksmith_core::{
    BindingRef, ComponentRef, DefinitionKind, LibraryCommand, Outcome, Revision,
};
use sparksmith_ui::fonts;

pub(crate) type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// The demo with a published 0 Ω Part of its resistor on the same lands.
pub(crate) struct Board {
    pub engine: Engine,
    pub r1: Id,
    pub r2: Id,
    pub zero: PartRef,
}

fn apply(engine: &mut Engine, operation: Operation) -> TestResult<Outcome> {
    Ok(engine.apply(engine.revision(), operation)?)
}

pub(crate) fn board() -> TestResult<Board> {
    let mut engine = Engine::demo()?;
    let draft = |kind: DefinitionKind| {
        engine
            .drafts()
            .items
            .iter()
            .find(|d| d.definition.kind() == kind)
            .map(|d| d.id)
            .ok_or(format!("no {kind:?} draft"))
    };
    let (component, binding) = (
        draft(DefinitionKind::Component)?,
        draft(DefinitionKind::Binding)?,
    );
    let id = |e: &Engine, d: &str| -> TestResult<Id> {
        Ok(e.project()
            .instance_by_designator(d)
            .ok_or(d.to_owned())?
            .id)
    };
    let (r1, r2) = (id(&engine, "R1")?, id(&engine, "R2")?);
    let Outcome::DraftCreated { id: draft } = apply(
        &mut engine,
        Operation::Library(LibraryCommand::NewDraft {
            meta: DefinitionMeta::default(),
            definition: Definition::empty(DefinitionKind::Part),
        }),
    )?
    else {
        return Err("no part draft".into());
    };
    let mpn = "EX-R0603-0R";
    let part = PartDefinition {
        next_local: 3,
        manufacturer: "ExampleCo".into(),
        mpn: mpn.into(),
        component: ComponentRef::new(component, Revision::FIRST),
        binding: BindingRef::new(binding, Revision::FIRST),
        specs: Vec::new(),
        references: vec![Reference {
            id: 1,
            kind: ReferenceKind::Datasheet,
            title: format!("{mpn} datasheet"),
            url: Some("https://example.com/exampleco/parts.pdf".into()),
            document_revision: Some("A".into()),
            asset: None,
            licence: None,
            provenance: Provenance::default(),
        }],
        offers: vec![DistributorOffer {
            id: 2,
            distributor: "Example Distribution".into(),
            sku: format!("EXD-{mpn}"),
            url: None,
            packaging: None,
        }],
        provenance: Provenance {
            basis: Basis::FromReference(1),
            ..Provenance::default()
        },
        top_mark: None,
    };
    apply(
        &mut engine,
        Operation::Library(LibraryCommand::Edit {
            edits: vec![DraftEdit {
                id: draft,
                meta: DefinitionMeta {
                    name: mpn.into(),
                    description: String::new(),
                    provenance: "Original synthetic test fixture".into(),
                },
                definition: Definition::Part(Box::new(part)),
            }],
        }),
    )?;
    let Outcome::Published { revisions } = apply(
        &mut engine,
        Operation::Library(LibraryCommand::Publish {
            drafts: vec![draft],
            close: false,
        }),
    )?
    else {
        return Err("not published".into());
    };
    let published = revisions.first().ok_or("nothing published")?;
    Ok(Board {
        engine,
        r1,
        r2,
        zero: PartRef::new(published.id, published.revision),
    })
}

pub(crate) fn add(engine: &mut Engine, name: &str) -> TestResult<Id> {
    match apply(
        engine,
        Operation::Variant(VariantCommand::Add {
            name: name.into(),
            copy_of: None,
        }),
    )? {
        Outcome::VariantAdded { variant } => Ok(variant),
        other => Err(format!("unexpected {other:?}").into()),
    }
}

/// "Lite" leaves R2 off, "Alt" fits the 0 Ω Part on R1; Lite is active.
pub(crate) fn lite_and_alt(b: &mut Board) -> TestResult<(Id, Id)> {
    let lite = add(&mut b.engine, "Lite")?;
    let alt = add(&mut b.engine, "Alt")?;
    apply(
        &mut b.engine,
        Operation::Variant(VariantCommand::SetPopulation {
            instances: vec![b.r2],
            variant: Some(lite),
            population: Some(Population::NotPopulated),
        }),
    )?;
    apply(
        &mut b.engine,
        Operation::Variant(VariantCommand::SetPart {
            instance: b.r1,
            variant: alt,
            part: Some(b.zero),
        }),
    )?;
    apply(
        &mut b.engine,
        Operation::Variant(VariantCommand::SetActive {
            variant: Some(lite),
        }),
    )?;
    Ok((lite, alt))
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    fonts::setup_bundled_fonts(&ctx);
    ctx
}

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, egui::pos2(900.0, 700.0));

fn canvas(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    events: Vec<Event>,
) -> (egui::FullOutput, Option<Operation>) {
    let mut sent = None;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        },
        |ui| sent = editor.canvas(ui, engine),
    );
    output.textures_delta.clear();
    (output, sent)
}

fn click(
    editor: &mut PcbEditor,
    engine: &Engine,
    ctx: &egui::Context,
    at: Pos2,
    button: PointerButton,
) -> (egui::FullOutput, Vec<Operation>) {
    let event = |pressed| Event::PointerButton {
        pos: at,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let mut sent = Vec::new();
    let mut last = None;
    for events in [
        vec![Event::PointerMoved(at)],
        vec![event(true)],
        vec![event(false)],
        vec![],
    ] {
        let (output, op) = canvas(editor, engine, ctx, events);
        sent.extend(op);
        last = Some(output);
    }
    (last.unwrap_or_default(), sent)
}

fn on_screen(editor: &PcbEditor, engine: &Engine, board: sparksmith_core::Point) -> Pos2 {
    board_view(
        SCREEN,
        engine.project().board_size,
        editor.zoom,
        editor.pan,
        false,
    )
    .screen(board)
}

fn placed(engine: &Engine, instance: Id) -> TestResult<sparksmith_core::Placement> {
    Ok(engine
        .project()
        .placements
        .iter()
        .find(|p| p.instance == instance)
        .ok_or("not placed")?
        .clone())
}

/// A right-click on a part offers "Not populated in default"; choosing it
/// sends one `variant.set_population`, after which the row is ticked and
/// undo takes it back. With a variant active it names the variant, and a
/// group's row sets every part of it in one step.
#[test]
fn the_part_menu_toggles_the_population() -> TestResult {
    let mut b = board()?;
    let ctx = context();
    let mut editor = PcbEditor::default();
    canvas(&mut editor, &b.engine, &ctx, vec![]);
    let at = on_screen(&editor, &b.engine, placed(&b.engine, b.r1)?.position);
    let (output, _) = click(&mut editor, &b.engine, &ctx, at, PointerButton::Secondary);
    let row = crate::kit::text::tests::painted(&output)
        .into_iter()
        .find(|(t, ..)| t == "Not populated in default")
        .map(|(_, rect, ..)| rect.center())
        .ok_or("the row is not in the menu")?;
    let (_, sent) = click(&mut editor, &b.engine, &ctx, row, PointerButton::Primary);
    let [operation] = sent.as_slice() else {
        return Err(format!("sent {sent:?}").into());
    };
    assert!(
        matches!(
            operation,
            Operation::Variant(VariantCommand::SetPopulation {
                instances,
                variant: None,
                population: Some(Population::NotPopulated),
            }) if *instances == [b.r1]
        ),
        "{operation:?}"
    );
    apply(&mut b.engine, operation.clone())?;
    let rows = editor.menu_rows(&ctx, &b.engine, menu::Target::Part(b.r1));
    let entry = rows
        .iter()
        .flatten()
        .find(|e| e.label == "Not populated in default")
        .ok_or("no row")?;
    assert_eq!(entry.icon, "finish", "ticked once left off");
    b.engine.undo()?;
    assert!(
        b.engine
            .project()
            .instance(b.r1)
            .ok_or("R1")?
            .population
            .is_populated(),
        "one undo step takes it back"
    );

    // In a variant, a group: both parts in one command, in Lite.
    let lite = add(&mut b.engine, "Lite")?;
    apply(
        &mut b.engine,
        Operation::Variant(VariantCommand::SetActive {
            variant: Some(lite),
        }),
    )?;
    editor.group.parts = [b.r1, b.r2].into_iter().collect();
    let rows = editor.menu_rows(&ctx, &b.engine, menu::Target::Group);
    assert!(
        rows.iter()
            .flatten()
            .any(|e| e.label == "Not populated in Lite"
                && e.run == Run::Act(menu::Act::GroupPopulation)),
        "{:?}",
        rows.iter().flatten().map(|e| &e.label).collect::<Vec<_>>()
    );
    let operation = editor
        .menu_act(&b.engine, menu::Act::GroupPopulation)
        .ok_or("no operation")?;
    assert!(
        matches!(
            &operation,
            Operation::Variant(VariantCommand::SetPopulation {
                instances,
                variant: Some(v),
                population: Some(Population::NotPopulated),
            }) if instances.len() == 2 && *v == lite
        ),
        "{operation:?}"
    );
    Ok(())
}

/// A part the active variant leaves off is crossed corner to corner of its
/// courtyard in the theme's dim ink; a fitted part is not, and the
/// default (R2 fitted there) draws no cross at all.
#[test]
fn the_board_crosses_out_a_part_the_active_variant_leaves_off() -> TestResult {
    fn segments(shape: &Shape, out: &mut Vec<([Pos2; 2], egui::Color32)>) {
        match shape {
            Shape::Vec(shapes) => shapes.iter().for_each(|s| segments(s, out)),
            Shape::LineSegment { points, stroke } => out.push((*points, stroke.color)),
            _ => {}
        }
    }
    let mut b = board()?;
    lite_and_alt(&mut b)?;
    let ctx = context();
    let mut editor = PcbEditor::default();
    let (output, _) = canvas(&mut editor, &b.engine, &ctx, vec![]);
    let mut lines = Vec::new();
    for clipped in &output.shapes {
        segments(&clipped.shape, &mut lines);
    }
    let corners = |editor: &PcbEditor, engine: &Engine, instance: Id| -> TestResult<[Pos2; 4]> {
        let p = placed(engine, instance)?;
        let corners = editor
            .cross_corners(engine.project(), &p)
            .ok_or("no outline")?;
        Ok(corners.map(|c| on_screen(editor, engine, c)))
    };
    let ink = Tokens::get().text_dim;
    let crossed = |lines: &[([Pos2; 2], egui::Color32)], c: [Pos2; 4]| {
        let near = |a: Pos2, b: Pos2| a.distance(b) < 0.01;
        let has = |a: Pos2, z: Pos2| {
            lines
                .iter()
                .any(|(p, color)| *color == ink && near(p[0], a) && near(p[1], z))
        };
        has(c[0], c[2]) && has(c[1], c[3])
    };
    assert!(
        crossed(&lines, corners(&editor, &b.engine, b.r2)?),
        "R2 is left off in Lite"
    );
    assert!(
        !crossed(&lines, corners(&editor, &b.engine, b.r1)?),
        "R1 is fitted in Lite"
    );
    apply(
        &mut b.engine,
        Operation::Variant(VariantCommand::SetActive { variant: None }),
    )?;
    let (output, _) = canvas(&mut editor, &b.engine, &ctx, vec![]);
    lines.clear();
    for clipped in &output.shapes {
        segments(&clipped.shape, &mut lines);
    }
    assert!(
        !crossed(&lines, corners(&editor, &b.engine, b.r2)?),
        "fitted by default"
    );
    Ok(())
}

/// The card lists the default and both variants with what each changes,
/// names the active one, and shows Lite's parts with R2 unticked.
#[test]
fn the_card_lists_the_variants_and_the_chosen_variants_parts() -> TestResult {
    let mut b = board()?;
    let (lite, _) = lite_and_alt(&mut b)?;
    let mut editor = PcbEditor::default();
    editor.open_variants();
    assert_eq!(editor.subject(), kit::card::Subject::Dialog);
    if let Some(card) = editor.variants.as_mut() {
        card.selected = Some(lite);
    }
    let ctx = context();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(420.0, 1400.0))),
            ..Default::default()
        },
        |ui| {
            let _ = editor.subject_slot(ui, &b.engine);
        },
    );
    output.textures_delta.clear();
    let painted: Vec<String> = crate::kit::text::tests::painted(&output)
        .into_iter()
        .map(|(t, ..)| t)
        .collect();
    for text in [
        "Default",
        "Lite",
        "Alt",
        "Active · 1 not populated",
        "1 substitute",
        "In Lite",
    ] {
        assert!(
            painted.iter().any(|t| t.eq_ignore_ascii_case(text)),
            "{text}: {painted:?}"
        );
    }
    // A cut text (a long MPN, a narrow column) shows whole on hover.
    let (_, missing) = crate::kit::text::tests::cuts_without_hover(&ctx, |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(320.0, 1400.0))),
                events,
                ..Default::default()
            },
            |ui| {
                let _ = editor.subject_slot(ui, &b.engine);
            },
        );
        output.textures_delta.clear();
        output
    });
    assert!(missing.is_empty(), "cut without hover: {missing:?}");
    Ok(())
}

/// Evidence pictures: the card (Lite chosen, then Alt with its
/// substitute), Layout with R2 crossed out in Lite, and R2's Placement
/// card with its Assembly section.
#[test]
fn variant_pictures() -> TestResult {
    let Some(dir) = std::env::var_os("SPARKSMITH_PICTURES") else {
        return Ok(());
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir)?;
    let mut b = board()?;
    let (lite, alt) = lite_and_alt(&mut b)?;
    let background = Tokens::get().panel;
    for (name, chosen) in [("variants-card-lite", lite), ("variants-card-alt", alt)] {
        let mut editor = PcbEditor::default();
        editor.open_variants();
        if let Some(card) = editor.variants.as_mut() {
            card.selected = Some(chosen);
        }
        let picture = picture_of([400.0, 760.0], background, |ui| {
            let _ = editor.subject_slot(ui, &b.engine);
        });
        std::fs::write(dir.join(format!("{name}.png")), picture.png().ok_or("png")?)?;
    }
    let mut editor = PcbEditor::default();
    let picture = super::super::raster_tests::picture(&mut editor, &b.engine);
    std::fs::write(
        dir.join("variants-board-lite.png"),
        picture.png().ok_or("png")?,
    )?;
    let mut editor = PcbEditor::default();
    editor.set_selection(Some(b.r2), None, None);
    let picture = picture_of([400.0, 900.0], background, |ui| {
        let _ = editor.subject_slot(ui, &b.engine);
    });
    std::fs::write(
        dir.join("variants-properties-r2.png"),
        picture.png().ok_or("png")?,
    )?;
    Ok(())
}
