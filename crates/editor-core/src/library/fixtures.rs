//! Test fixtures, never in the product: the sample parts
//! ([`super::sample_parts`]) and the preset parts the retired templates
//! built, frozen ([`presets`]). Other crates' tests reach them through the
//! `test-fixtures` feature.
use super::DraftEdit;
pub use super::part_refs::SampleRefs;
pub use super::sample_parts::{Kind, redraw, resistance_field, two_terminal, two_terminal_header};
use crate::diagnostics::Result;

/// The preset parts the retired templates built (an 0603 and an 0805
/// chip, SOIC-8, TQFP-32, QFN-16, DFN-8, headers 2×5 and 1×2), as frozen
/// draft definitions with a box envelope as their model, converted to
/// format 5 (`fixtures/presets.json`, the showcase parts' format).
pub fn presets() -> Result<super::showcase_parts::ShowcaseParts> {
    super::showcase_parts::ShowcaseParts::parse(include_str!("fixtures/presets.json"))
}

/// The preset part labelled `label` ("SOIC-8"), its edits naming `refs`.
pub fn preset(label: &str, refs: SampleRefs) -> Result<Vec<DraftEdit>> {
    let presets = presets()?;
    let part = presets
        .parts
        .iter()
        .find(|p| p.name == label)
        .ok_or_else(|| crate::diagnostics::Error::NotFound(format!("no preset part {label}")))?;
    super::showcase_parts::remap(&part.edits, refs)
}
