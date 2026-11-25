//! Slice D1: the catalog's generic parts through generators. Every example
//! the grammar declares (`library/catalog/generic-families.json`) reads as
//! a part of the generator its family names, and `library.generate_part`
//! makes it with the shipped bundle; read again, the same text finds the
//! part the project now holds.
use super::super::Session;
use serde_json::json;
use sparksmith_core::Engine;
use sparksmith_core::catalog::generic::generators::{self, find_presets};
use sparksmith_core::library::generated::Standing;
use sparksmith_io::shipped::ShippedLibrary;
use std::path::Path;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The repository's shipped library.
const LIBRARY: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library");

fn session() -> TestResult<Session> {
    let mut session = Session::fixture(Engine::default());
    session.set_shipped(ShippedLibrary::at(Path::new(LIBRARY))?);
    Ok(session)
}

#[test]
fn every_generic_example_builds_a_part_through_its_generator() -> TestResult {
    let probe = session()?;
    let descriptions: Vec<_> = probe
        .generators_report()?
        .shipped
        .iter()
        .filter_map(|s| s.description.as_ref().map(|d| d.description()))
        .collect();
    let grammar = {
        let _context = probe.engine().enter_context();
        sparksmith_core::library::standards::get()?.generic.clone()
    };
    let mut made = 0;
    for family in &grammar.families {
        assert!(
            descriptions.iter().any(|d| d.id == family.generator),
            "{:?} names the shipped generator {}",
            family.family,
            family.generator
        );
        for example in &family.examples {
            let mut session = session()?;
            let read = |session: &Session| {
                let _context = session.engine().enter_context();
                generators::search(session.engine().document(), example, &descriptions)
            };
            let found = read(&session)?;
            let candidate = found
                .iter()
                .find(|c| c.family == family.family)
                .ok_or_else(|| format!("{example}: no {:?} part in {found:?}", family.family))?;
            assert_eq!(
                candidate.standing,
                Standing::New,
                "{example}: {candidate:?}"
            );
            assert_eq!(candidate.generator, family.generator);
            let reply = session.dispatch(json!({
                "command": "library.generate_part",
                "params": {
                    "generator": candidate.generator,
                    "scope": "component",
                    "params": candidate.params,
                    "name": candidate.name,
                    "expected_revision": session.engine().revision(),
                },
            }));
            assert_eq!(reply["ok"], true, "{example}: {reply}");
            let report = &reply["result"]["outcome"]["report"];
            assert_eq!(report["created"], true, "{example}: {report}");
            // Read again, the text names the part the project holds.
            let again = read(&session)?;
            let held = again
                .iter()
                .find(|c| c.family == family.family)
                .ok_or_else(|| format!("{example}: not found again"))?;
            assert!(
                matches!(held.standing, Standing::Held { .. }),
                "{example}: {held:?}"
            );
            assert!(held.component().is_some(), "{example}");
            made += 1;
        }
    }
    assert_eq!(made, 25, "every example of the eleven families");
    Ok(())
}

#[test]
fn the_text_is_read_as_before_and_a_size_without_a_preset_is_refused() -> TestResult {
    let session = session()?;
    let descriptions: Vec<_> = session
        .generators_report()?
        .shipped
        .iter()
        .filter_map(|s| s.description.as_ref().map(|d| d.description()))
        .collect();
    let _context = session.engine().enter_context();
    let document = session.engine().document();
    let read = |text: &str| generators::search(document, text, &descriptions);
    // "10u 0603" is a capacitor and an inductor.
    let both = read("10u 0603")?;
    let families: Vec<_> = both.iter().map(|c| c.family).collect();
    assert_eq!(families.len(), 2, "{both:?}");
    // A preset the text names exactly is that preset.
    let resistor = read("resistor 0603")?;
    assert_eq!(resistor[0].covers_preset.as_deref(), Some("resistor-0603"));
    assert_eq!(resistor[0].name, "Chip resistor 0603");
    // A metric code.
    let metric = read("resistor 1608M")?;
    assert_eq!(metric[0].preset.as_deref(), Some("resistor-0603"));
    // An inductor is the ferrite bead's body with its own part.
    let inductor = read("inductor 0603")?;
    assert_eq!(inductor[0].preset.as_deref(), Some("ferrite-0603"));
    assert_eq!(inductor[0].covers_preset, None);
    // A header of a pitch with presets takes their proportions.
    let header = read("header 1x13 2mm")?;
    assert_eq!(header[0].preset.as_deref(), Some("header-1x2-2.0"));
    assert_eq!(header[0].name, "Pin header 1×13 2 mm");
    // A size no preset has is refused in plain words.
    let none = read("resistor 0604")?;
    assert!(
        none[0].refusal().is_some_and(|r| r.contains("0604")),
        "{none:?}"
    );
    // Presets are found by their names and aliases.
    let (qfn, _) = find_presets(document, "qfn-16", &descriptions, 10);
    assert!(qfn.iter().all(|h| h.generator == "no-lead") && !qfn.is_empty());
    // E2E run 9: a bare size code reads as imperial; the metric 0603 (the
    // imperial 0201) is found only when the text says metric.
    let keys = |text: &str| -> Vec<String> {
        find_presets(document, text, &descriptions, 50)
            .0
            .into_iter()
            .map(|h| h.preset)
            .collect()
    };
    let imperial = keys("resistor 0603");
    assert!(
        imperial.iter().any(|k| k == "resistor-0603"),
        "{imperial:?}"
    );
    assert!(
        !imperial.iter().any(|k| k == "resistor-0201"),
        "{imperial:?}"
    );
    let metric = keys("resistor 0603 metric");
    assert!(metric.iter().any(|k| k == "resistor-0201"), "{metric:?}");
    // VM run 8: every generator's presets are offered, the module's WROOM-3
    // among them, by its name and by the generator's words.
    for text in ["wroom", "module", "esp32"] {
        let (found, _) = find_presets(document, text, &descriptions, 50);
        assert!(
            found
                .iter()
                .any(|h| h.generator == "module" && h.name.contains("WROOM-3")),
            "{text}: {found:?}"
        );
    }
    Ok(())
}

/// Run 3, A2 and A3, over the shipped library: a word matches where a word
/// starts, so "fuse" finds no part by its maker's name (the library has no
/// fuse, so nothing at all); "button" also means "tactile" (the search
/// vocabulary: "tactile") and finds the tactile switches, not the reed switch; "sma" finds the SMA diode
/// outline.
#[test]
fn search_words_match_word_starts_and_the_vocabulary() -> TestResult {
    let session = session()?;
    let descriptions: Vec<_> = session
        .generators_report()?
        .shipped
        .iter()
        .filter_map(|s| s.description.as_ref().map(|d| d.description()))
        .collect();
    let _context = session.engine().enter_context();
    let document = session.engine().document();
    let names = |text: &str| -> Vec<String> {
        find_presets(document, text, &descriptions, 50)
            .0
            .into_iter()
            .map(|h| h.name)
            .collect()
    };
    let fuse = names("fuse");
    assert!(!fuse.iter().any(|n| n.contains("Littelfuse")), "{fuse:?}");
    assert!(
        names("mdsm").iter().any(|n| n.contains("MDSM")),
        "the maker's part is found by its own name"
    );
    let button = names("button");
    assert!(
        !button.is_empty() && !button.iter().any(|n| n.contains("MDSM")),
        "{button:?}"
    );
    assert!(names("sma").iter().any(|n| n.contains("SMA")));
    Ok(())
}
