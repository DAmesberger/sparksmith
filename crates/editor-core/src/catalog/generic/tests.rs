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
    assert_eq!(grammar.families.len(), 6, "every family declares patterns");
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
