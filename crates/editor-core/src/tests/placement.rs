//! Where `component.place` puts a symbol: a position left out lands at the
//! next free spot inside the sheet's frame; the frame check names the
//! frame for a position outside it.
use super::support::*;
use crate::{Engine, Error, Id, Operation, Outcome, Result};

/// Two passive terminals, pins at x = ±5 mm of the unit origin.
fn two_pin(engine: &mut Engine) -> Result<Fixture> {
    component(
        engine,
        &Spec {
            name: "Two terminal",
            terminals: vec![(1, "A", passive()), (2, "B", passive())],
            units: vec![(1, "A", vec![(1, [-5.0, 0.0]), (2, [5.0, 0.0])])],
            package: vec![(1, "1"), (2, "2")],
            mapping: vec![(1, 1), (2, 2)],
            physical: true,
        },
    )
}

fn place(
    engine: &mut Engine,
    f: &Fixture,
    position: Option<[f64; 2]>,
    with_symbol: bool,
) -> Result<Id> {
    match apply(
        engine,
        Operation::Instance(crate::InstanceCommand::Place(
            crate::instances::NewInstance {
                sheet: None,
                component: f.r1(),
                designator: None,
                properties: vec![],
                representation: with_symbol.then_some(f.symbol),
                unit: with_symbol.then_some(crate::UnitId(1)),
                position: position.map(pt),
                physical: None,
            },
        )),
    )? {
        Outcome::InstancePlaced { occurrence, .. } => Ok(occurrence),
        other => Err(Error::Invalid(format!("unexpected {other:?}"))),
    }
}

fn missing(what: &str) -> Error {
    Error::Invalid(what.into())
}

/// With only the component: its first symbol and unit, inside the first
/// sheet's frame, on the grid; a second one does not land on the first.
#[test]
fn a_place_without_position_lands_free_inside_the_frame() -> Result<()> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let a = place(&mut engine, &f, None, false)?;
    let b = place(&mut engine, &f, None, false)?;
    let project = engine.project();
    let sheet = project
        .sheet(project.sheets.first())
        .ok_or_else(|| missing("no sheet"))?;
    let [left, top] = sheet.top_left();
    let [w, h] = sheet.size_mm();
    let m = crate::library::TextMetric::default();
    let mut boxes = Vec::new();
    for id in [a, b] {
        let occurrence = project.occurrence(id).ok_or_else(|| missing("gone"))?;
        assert_eq!(occurrence.unit, crate::UnitId(1));
        assert_eq!(occurrence.position.x_nm.get() % 2_540_000, 0);
        assert_eq!(occurrence.position.y_nm.get() % 2_540_000, 0);
        let drawn = crate::schematic::placed_unit(project, id)?
            .bounds(m)
            .ok_or_else(|| missing("nothing drawn"))?;
        assert!(drawn.min[0] > left && drawn.max[0] < left + w, "{drawn:?}");
        assert!(drawn.min[1] > top - h && drawn.max[1] < top, "{drawn:?}");
        boxes.push(drawn);
    }
    let (p, q) = (boxes[0], boxes[1]);
    let overlap =
        p.min[0] < q.max[0] && q.min[0] < p.max[0] && p.min[1] < q.max[1] && q.min[1] < p.max[1];
    assert!(!overlap, "{p:?} and {q:?} overlap");
    Ok(())
}

/// The frame check names the sheet and its frame for a position outside
/// it (the front door holds agents to it); the command itself places where
/// it is asked, as a person may draw beside the page.
#[test]
fn the_frame_check_names_the_frame_and_the_command_places_where_asked() -> Result<()> {
    let mut engine = Engine::default();
    let f = two_pin(&mut engine)?;
    let sheet = engine.project().sheets.first();
    let check =
        |at: [f64; 2]| crate::schematic::spot::check_on_page(engine.project(), sheet, pt(at));
    assert!(check([100.0, -80.0]).is_ok());
    let Err(error) = check([100.0, 80.0]) else {
        return Err(missing("outside the frame passed"));
    };
    let text = error.to_string();
    assert!(
        text.contains(
            "(100, 80) mm is outside Sheet 1 (A4 landscape): its frame spans x -30\u{2026}267 mm and y -180\u{2026}30 mm, y up"
        ),
        "{text}"
    );
    assert!(text.contains("leave position out"), "{text}");
    for at in [[100.0, -80.0], [100.0, 80.0]] {
        let placed = place(&mut engine, &f, Some(at), true)?;
        let got = engine
            .project()
            .occurrence(placed)
            .ok_or_else(|| missing("gone"))?
            .position
            .to_mm_f64();
        assert_eq!(got, at);
    }
    Ok(())
}
