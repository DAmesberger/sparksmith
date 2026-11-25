//! What the card says of each kind of thing, from the document only.
use super::*;
use sparksmith_core::Engine;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn row<'a>(card: &'a Content, label: &str) -> Option<&'a str> {
    let found = card.rows.iter().find(|(l, _)| *l == label);
    found.map(|(_, value)| value.as_str())
}

/// The part named `designator` of the demo.
fn id(engine: &Engine, designator: &str) -> Result<Id, String> {
    engine
        .project()
        .instance_by_designator(designator)
        .map(|i| i.id)
        .ok_or_else(|| format!("no {designator}"))
}

/// A resistor: its value and package beside its name, what it is in words,
/// and where it is. Nothing is said that the library does not state.
#[test]
fn a_part_says_its_value_package_and_place() -> TestResult {
    let engine = Engine::demo()?;
    let project = engine.project();
    let r1 = id(&engine, "R1")?;
    let instance = project.instance(r1).ok_or("R1")?;
    let card = about(project, Subject::Part(r1)).ok_or("a card")?;
    let value = instances::value_text(&project.library, instance)?.ok_or("a value")?;
    let package = sparksmith_core::fabrication::assembly::package_name(project, instance)
        .ok_or("a package")?;
    assert_eq!(card.title, format!("R1 · {value} · {package}"));
    assert!(card.words.is_some(), "{card:?}");
    assert_eq!(row(&card, "Side"), Some("Top"));
    assert_eq!(row(&card, "Locked"), None);
    assert_eq!(row(&card, "Assembly"), None);
    for (label, value) in &card.rows {
        assert!(
            !value.trim().is_empty() && value != "—",
            "{label}: {value:?}"
        );
    }

    // Stripped of what the library says, the card only gets shorter.
    let mut bare = project.clone();
    bare.placements.clear();
    let instance = bare.instances.iter_mut().find(|i| i.id == r1).ok_or("R1")?;
    instance.physical = None;
    instance.properties.clear();
    let card = about(&bare, Subject::Part(r1)).ok_or("a card")?;
    assert!(card.title.starts_with("R1"), "{}", card.title);
    for label in ["Height", "Side", "Board", "Manufacturer"] {
        assert_eq!(row(&card, label), None, "{card:?}");
    }
    assert_eq!(about(project, Subject::Part(u64::MAX)), None);
    Ok(())
}

/// The chosen Part's number names it, and the value is a row; a locked part
/// and one the board's variant leaves off say so.
#[test]
fn a_chosen_part_a_lock_and_a_variant_are_said() -> TestResult {
    let mut engine = Engine::demo()?;
    let r1 = id(&engine, "R1")?;
    let part = crate::resolve_parts::demo_part(&mut engine, "EX-R0603-10K0-F")?;
    let mut project = engine.project().clone();
    for i in project.instances.iter_mut().filter(|i| i.id == r1) {
        i.part = Some(part);
        i.population = sparksmith_core::Population::NotPopulated;
    }
    for p in project.placements.iter_mut().filter(|p| p.instance == r1) {
        p.locked = true;
    }
    let card = about(&project, Subject::Part(r1)).ok_or("a card")?;
    assert!(
        card.title.starts_with("R1 · EX-R0603-10K0-F"),
        "{}",
        card.title
    );
    assert_eq!(row(&card, "Manufacturer"), Some("ExampleCo"));
    assert!(row(&card, "Value").is_some(), "{card:?}");
    assert_eq!(row(&card, "Locked"), Some("Yes"));
    assert_eq!(row(&card, "Assembly"), Some("Not fitted"));
    Ok(())
}

/// A pad and the pin it lands say the same net and function; both name the
/// part they are on.
#[test]
fn a_pad_and_its_pin_agree() -> TestResult {
    let engine = Engine::demo()?;
    let project = engine.project();
    let mut seen = 0;
    for placed in sparksmith_core::layout::placed_pads(project) {
        let Some(pin) = placed.terminal else { continue };
        let pad = about(
            project,
            Subject::Pad {
                instance: placed.instance,
                pad: placed.pad.id,
            },
        )
        .ok_or("a pad's card")?;
        let pin = about(project, Subject::Pin(pin)).ok_or("a pin's card")?;
        assert_eq!(pad.rows, pin.rows);
        assert_eq!(pad.words, pin.words);
        assert!(pad.title.contains(&placed.pad.number), "{}", pad.title);
        assert_eq!(
            row(&pad, "Net"),
            placed.net.map(|n| project.net_name(n)),
            "{pad:?}"
        );
        seen += 1;
    }
    assert!(seen > 0);
    Ok(())
}
