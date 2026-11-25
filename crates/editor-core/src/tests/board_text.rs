//! Board text through public commands and queries: added, edited and
//! deleted as one undo step each, printed through the one legend (Gerber
//! legend files, checks) on its side, mirrored on the bottom, and checked
//! against the silkscreen text height rule and the board edge.
use super::text_placement::gerber_points;
use crate::fabrication::preflight;
use crate::fabrication::{self, FabricationOptions};
use crate::ids::Id;
use crate::layout::{self, BoardDrawing, GraphicOwner, LayoutCommand, PlacedItem};
use crate::library::graphics::Rotation;
use crate::model::{BoardText, BoardTextSpec, Layer};
use crate::units::{Nm, PointNm};
use crate::{Engine, ObjectRef, Operation, Outcome};
use std::collections::BTreeSet;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn spec(text: &str, at: [f64; 2], height_mm: f64, side: Layer) -> Fallible<BoardTextSpec> {
    Ok(BoardTextSpec {
        text: text.into(),
        position: PointNm::from_mm_f64_rounded(at)?,
        height_nm: Nm::from_mm_f64_rounded(height_mm)?,
        rotation: Rotation::R0,
        side,
    })
}

fn add(engine: &mut Engine, spec: BoardTextSpec) -> Fallible<Id> {
    match engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::AddText {
            text: Box::new(spec),
        }),
    )? {
        Outcome::TextAdded { text } => Ok(text),
        other => Err(format!("not a text: {other:?}").into()),
    }
}

/// The middle of the showcase board.
fn middle(engine: &Engine) -> [f64; 2] {
    let [w, h] = engine.project().board_size;
    [w / 2.0, h / 2.0]
}

#[test]
fn board_text_is_added_edited_and_deleted_one_undo_step_each() -> Fallible {
    let mut engine = Engine::showcase()?;
    let at = middle(&engine);
    let before = engine.revision();
    let id = add(&mut engine, spec("Sensor node rev A", at, 1.0, Layer::Top)?)?;
    assert_eq!(engine.revision(), before + 1);
    let text = engine.project().board_text(id).cloned().ok_or("no text")?;
    assert_eq!(text.text, "Sensor node rev A");
    let edited = BoardText {
        text: "Sensor node\nrev B".into(),
        side: Layer::Bottom,
        rotation: Rotation::R90,
        ..text.clone()
    };
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditText {
            text: Box::new(edited.clone()),
        }),
    )?;
    assert_eq!(engine.project().board_text(id), Some(&edited));
    engine.undo()?;
    assert_eq!(engine.project().board_text(id), Some(&text));
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::DeleteText { id }),
    )?;
    assert!(engine.project().board_texts.is_empty());
    engine.undo()?;
    assert_eq!(engine.project().board_text(id), Some(&text));
    // Refusals change nothing.
    let held = engine.revision();
    for refused in [
        spec("  ", at, 1.0, Layer::Top)?,
        spec("tiny", at, 0.05, Layer::Top)?,
        spec("huge", at, 60.0, Layer::Top)?,
    ] {
        let result = engine.apply(
            engine.revision(),
            Operation::Layout(LayoutCommand::AddText {
                text: Box::new(refused),
            }),
        );
        assert!(result.is_err(), "{result:?}");
    }
    let gone = BoardText {
        id: Id::MAX,
        ..text.clone()
    };
    assert!(
        engine
            .apply(
                engine.revision(),
                Operation::Layout(LayoutCommand::EditText {
                    text: Box::new(gone),
                }),
            )
            .is_err()
    );
    assert!(
        engine
            .apply(
                engine.revision(),
                Operation::Layout(LayoutCommand::DeleteText { id: Id::MAX }),
            )
            .is_err()
    );
    assert_eq!(engine.revision(), held);
    assert_eq!(engine.project().board_texts.len(), 1);
    Ok(())
}

/// The drawn coordinates of the legend file of `side`.
fn legend(engine: &Engine, side: &str) -> Fallible<BTreeSet<(i64, i64)>> {
    let set = fabrication::generate(
        &fabrication::ProductionSnapshot::resolve(engine.project(), None)?,
        &FabricationOptions::default(),
        &Default::default(),
    )?;
    let file = set
        .files
        .iter()
        .find(|f| f.name.ends_with(&format!("-legend-{side}.gbr")))
        .ok_or("no legend file")?;
    Ok(gerber_points(std::str::from_utf8(&file.bytes)?))
}

#[test]
fn board_text_prints_on_its_side_mirrored_on_the_bottom() -> Fallible {
    let mut engine = Engine::showcase()?;
    let at = middle(&engine);
    let (top_before, bottom_before) = (legend(&engine, "top")?, legend(&engine, "bottom")?);
    let id = add(&mut engine, spec("REV A", at, 1.5, Layer::Top)?)?;
    let graphic = layout::placed_graphics(engine.project())
        .into_iter()
        .find(|g| g.owner == GraphicOwner::BoardText { id })
        .ok_or("not placed")?;
    assert_eq!(graphic.layer, BoardDrawing::Silkscreen(Layer::Top));
    let PlacedItem::Text(top_text) = &graphic.item else {
        return Err("not text".into());
    };
    assert!(!top_text.mirrored);
    // Every stroke point the legend prints for it is in the top file.
    let strokes =
        layout::legend::text_strokes(top_text, &engine.project().rules.effective().silkscreen);
    let printed: BTreeSet<_> = legend(&engine, "top")?
        .difference(&top_before)
        .copied()
        .collect();
    assert!(!printed.is_empty());
    for p in strokes.strokes.iter().flatten() {
        let nm = (
            Nm::from_mm_f64_rounded(p[0])?.get(),
            Nm::from_mm_f64_rounded(p[1])?.get(),
        );
        assert!(printed.contains(&nm), "{p:?} not printed");
    }
    assert_eq!(legend(&engine, "bottom")?, bottom_before);
    // On the bottom: mirrored, reflected about its centre in x, and in
    // the bottom file only.
    let text = engine.project().board_text(id).cloned().ok_or("no text")?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditText {
            text: Box::new(BoardText {
                side: Layer::Bottom,
                ..text
            }),
        }),
    )?;
    assert_eq!(legend(&engine, "top")?, top_before);
    let printed: BTreeSet<_> = legend(&engine, "bottom")?
        .difference(&bottom_before)
        .copied()
        .collect();
    let centre = Nm::from_mm_f64_rounded(at[0])?.get();
    let reflected: BTreeSet<_> = strokes
        .strokes
        .iter()
        .flatten()
        .map(|p| -> Fallible<(i64, i64)> {
            Ok((
                2 * centre - Nm::from_mm_f64_rounded(p[0])?.get(),
                Nm::from_mm_f64_rounded(p[1])?.get(),
            ))
        })
        .collect::<Fallible<_>>()?;
    for p in &reflected {
        let near = printed
            .iter()
            .any(|q| (q.0 - p.0).abs() <= 1 && (q.1 - p.1).abs() <= 1);
        assert!(near, "{p:?} not printed mirrored");
    }
    Ok(())
}

#[test]
fn small_board_text_and_text_past_the_edge_are_reported() -> Fallible {
    let mut engine = Engine::showcase()?;
    let min = engine
        .project()
        .rules
        .effective()
        .silkscreen
        .text_height_min;
    assert!(min > 0.1, "the showcase's rules state a minimum: {min}");
    let at = middle(&engine);
    let id = add(&mut engine, spec("small", at, min - 0.1, Layer::Top)?)?;
    let findings = layout::checks(engine.project(), engine.check_settings());
    let small: Vec<_> = findings
        .iter()
        .filter(|f| f.code == "board.text_height")
        .collect();
    assert_eq!(small.len(), 1, "{findings:#?}");
    assert_eq!(small[0].objects, [ObjectRef::BoardText { id }]);
    assert!(
        small[0].message.starts_with("Board text small is "),
        "{}",
        small[0].message
    );
    // At the minimum it is not reported.
    let text = engine.project().board_text(id).cloned().ok_or("no text")?;
    let at_min = BoardText {
        height_nm: Nm::from_mm_f64_rounded(min)?,
        ..text.clone()
    };
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditText {
            text: Box::new(at_min.clone()),
        }),
    )?;
    let findings = layout::checks(engine.project(), engine.check_settings());
    assert!(!findings.iter().any(|f| f.code == "board.text_height"));
    // Centred on the left edge: half of it runs off the board.
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditText {
            text: Box::new(BoardText {
                position: PointNm::from_mm_f64_rounded([0.0, at[1]])?,
                ..at_min
            }),
        }),
    )?;
    let report = preflight::report(
        engine.project(),
        engine.drafts(),
        None,
        engine.check_settings(),
    )?;
    let outside = report
        .items
        .iter()
        .find(|i| i.code == "fabrication.legend_outside")
        .ok_or("no legend finding")?;
    assert_eq!(outside.title, "Board text small runs outside the board");
    assert_eq!(outside.objects, [ObjectRef::BoardText { id }]);
    Ok(())
}
