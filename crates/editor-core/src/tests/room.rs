//! Every committed document saves (review finding 15): an edit that would
//! make the saved document too large is refused with document, revision
//! and history as they were, and every state the engine accepts measures
//! within the limit as its first save writes it.
use crate::format::room::{Room, check_saveable};
use crate::format::{EditorFile, container_document_len};
use crate::model::SheetItem;
use crate::schematic::SchematicCommand;
use crate::{Engine, Evaluators, Operation, Result};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn note(text: &str) -> Result<Operation> {
    Ok(Operation::Schematic(SchematicCommand::AddDrawing {
        sheet: None,
        item: SheetItem::Text {
            at: Default::default(),
            text: text.to_owned(),
            height_nm: crate::units::Nm::new(2_000_000)?,
            rotation: Default::default(),
        },
    }))
}

/// The saved document's exact length, with the identity a first save
/// gives it.
fn saved_len(engine: &Engine) -> Result<usize> {
    let mut file = engine.to_file();
    file.project.production.project_id = Some("00000000-0000-4000-8000-000000000000".into());
    container_document_len(&file, usize::MAX)
}

/// An engine whose room ends `spare` bytes past its document.
fn engine_with_room(spare: usize) -> Result<(Engine, usize)> {
    let mut engine = Engine::demo()?;
    let limit = saved_len(&engine)? + spare;
    engine.set_room(Room::with_limit(limit));
    Ok((engine, limit))
}

#[test]
fn an_edit_past_the_room_is_refused_and_history_stays() -> TestResult {
    let (mut engine, limit) = engine_with_room(20 * 1024)?;
    let text = "n".repeat(4096);
    let mut accepted = 0;
    let refused = loop {
        let before = (engine.to_file(), engine.revision(), engine.can_undo());
        match engine.apply(engine.revision(), note(&text)?) {
            Ok(_) => {
                accepted += 1;
                assert!(saved_len(&engine)? <= limit, "an accepted state fits");
                check_saveable(&engine.to_file(), limit)?;
            }
            Err(error) => {
                assert_eq!(
                    (engine.to_file(), engine.revision(), engine.can_undo()),
                    before,
                    "a refused edit changes nothing"
                );
                break error.to_string();
            }
        }
        assert!(accepted < 64, "the room never filled");
    };
    assert!(accepted >= 3, "{accepted} notes fit in 20 KiB");
    assert!(refused.contains("could not be saved"), "{refused}");
    assert!(refused.contains("This change was not made"), "{refused}");
    // The history is intact: every accepted note undoes, one at a time.
    for _ in 0..accepted {
        engine.undo()?;
    }
    assert_eq!(engine.project().drawings.len(), 0);
    // And a smaller edit still goes in at the full room.
    for _ in 0..accepted {
        engine.redo()?;
    }
    engine.apply(engine.revision(), note("short")?)?;
    Ok(())
}

/// A batch is measured as it commits: past the room, none of it is made.
#[test]
fn a_batch_past_the_room_is_refused_whole() -> TestResult {
    let (mut engine, _) = engine_with_room(10 * 1024)?;
    let before = (engine.to_file(), engine.revision());
    let text = "b".repeat(4096);
    let notes = (0..4).map(|_| note(&text)).collect::<Result<Vec<_>>>()?;
    let refused = engine.apply_all_hosted(engine.revision(), notes, Evaluators::default());
    assert!(refused.is_err());
    assert_eq!((engine.to_file(), engine.revision()), before);
    Ok(())
}

/// A document that comes in whole is measured too, and one too large to
/// save is refused with the previous document kept.
#[test]
fn a_replacement_too_large_to_save_is_refused() -> TestResult {
    let mut big = Engine::demo()?;
    for _ in 0..4 {
        big.apply(big.revision(), note(&"r".repeat(4096))?)?;
    }
    let (mut engine, _) = engine_with_room(4 * 1024)?;
    let before = engine.to_file();
    let refused = engine.replace_document(EditorFile::clone(&big.to_file()));
    assert!(refused.is_err());
    assert_eq!(engine.to_file(), before);
    Ok(())
}
