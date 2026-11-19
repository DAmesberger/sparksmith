//! The grammar: every declared pattern parses, a bad one says why. Reading
//! texts as parts is `generators`' (its own tests and the session's
//! `catalog_tests.rs`, which build every example through its generator).
use super::*;
use crate::diagnostics::Result;
use crate::library::standards;

#[test]
fn every_declared_pattern_parses() -> Result<()> {
    let _context = crate::library::standards::Context::fixture().map(|c| c.enter());
    let standards = standards::get()?;
    let grammar = &standards.generic;
    check(grammar)?;
    assert_eq!(grammar.families.len(), 8, "every family declares patterns");
    for family in &grammar.families {
        assert!(!family.patterns.is_empty() && !family.examples.is_empty());
        for pattern in &family.patterns {
            let parsed = parse_pattern(grammar, pattern);
            assert!(parsed.is_ok(), "{pattern}: {parsed:?}");
        }
    }
    Ok(())
}

#[test]
fn a_bad_pattern_says_why() -> Result<()> {
    let _context = crate::library::standards::Context::fixture().map(|c| c.enter());
    let standards = standards::get()?;
    let grammar = &standards.generic;
    for bad in [
        "",
        "header {rows",
        "{a:nope}",
        "{a} {a}",
        "x{value:ohm}",
        "a b c d e f g",
    ] {
        assert!(parse_pattern(grammar, bad).is_err(), "{bad}");
    }
    // A grammar declaring one is refused as a whole.
    let mut broken = grammar.clone();
    broken.families[0].patterns.push("{a:nope}".into());
    assert!(check(&broken).is_err());
    let mut broken = grammar.clone();
    broken.header_pitch_mm = "wide".into();
    assert!(check(&broken).is_err());
    Ok(())
}

#[test]
fn generic_values_use_the_shared_quantity_parser() -> Result<()> {
    let _context = crate::library::standards::Context::fixture().map(|c| c.enter());
    let standards = standards::get()?;
    for (slot, cases) in [
        (
            "ohm",
            vec![
                ("4k7", "4.7 kΩ"),
                ("4.7k", "4.7 kΩ"),
                ("4K7", "4.7 kΩ"),
                ("1R0", "1 Ω"),
                ("1R0Ω", "1 Ω"),
                ("2M2", "2.2 MΩ"),
                ("0R1", "100 mΩ"),
                ("4K7ohm", "4.7 kΩ"),
            ],
        ),
        (
            "farad",
            vec![
                ("100n", "100 nF"),
                ("1u0", "1 µF"),
                ("4n7", "4.7 nF"),
                ("4n7F", "4.7 nF"),
            ],
        ),
    ] {
        let kind = standards
            .generic
            .value_slots
            .iter()
            .position(|v| v.slot == slot)
            .ok_or_else(|| Error::NotFound(slot.into()))?;
        for (text, expected) in cases {
            let Some(Bound::Value(value)) = read_value(&standards, text, kind) else {
                return Err(Error::Invalid(format!("{text} did not read as {slot}")));
            };
            assert_eq!(value.text, expected, "{text}");
        }
        for text in ["4k7k", "-4k7", "0603"] {
            assert!(read_value(&standards, text, kind).is_none(), "{text}");
        }
    }
    Ok(())
}
