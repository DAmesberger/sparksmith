//! Whether the board is one its fab profile's numbers are for.
//!
//! A profile's limits come from a fabricator's rows for some boards: a
//! two-layer page does not describe a four-layer board. A profile that
//! names its copper layer counts ([`ProfileBoards`]) holds the board to
//! them ([`LAYERS`]), and a manufacturing choice that needs something of
//! the board ([`ChoiceRequires`]: a thin board and a gold or OSP finish for
//! the smallest drill) holds the board to that ([`CHOICE_THICKNESS`],
//! [`CHOICE_FINISH`]). All are
//! errors: the checks passed under numbers the fabricator does not publish
//! for this board.
//!
//! [`ProfileBoards`]: crate::rules::ProfileBoards
//! [`ChoiceRequires`]: crate::rules::ChoiceRequires
use crate::diagnostics::{Error, Finding, ObjectRef, Requirement, Result};
use crate::model::{PadFinish, Project};

/// The board's copper layer count is not one its fab profile is for.
pub const LAYERS: &str = crate::diagnostics::FindingCode::BoardProfileLayers.as_str();
/// A chosen manufacturing option needs a thinner board.
pub const CHOICE_THICKNESS: &str =
    crate::diagnostics::FindingCode::BoardFabChoiceThickness.as_str();
/// A chosen manufacturing option needs another surface finish.
pub const CHOICE_FINISH: &str = crate::diagnostics::FindingCode::BoardFabChoiceFinish.as_str();

/// A profile's finish requirements name surface finishes: a file that
/// names another word is refused, never read as a requirement no board
/// can meet.
pub fn check(project: &Project) -> Result<()> {
    let Some(profile) = &project.rules.profile else {
        return Ok(());
    };
    for option in profile.options() {
        for choice in &option.choices {
            let finishes = choice.requires.iter().flat_map(|r| &r.finishes);
            for name in finishes {
                if PadFinish::from_id(name).is_none() {
                    let known: Vec<&str> = PadFinish::ALL.iter().map(|f| f.id()).collect();
                    return Err(Error::Invalid(format!(
                        "rules.fab_option: {} requires {name}, which is not a surface finish ({})",
                        choice.name,
                        known.join(", ")
                    )));
                }
            }
        }
    }
    Ok(())
}

/// [`LAYERS`], [`CHOICE_THICKNESS`] and [`CHOICE_FINISH`] for the board
/// under its profile: one finding of each at most, naming every choice
/// that the board does not meet.
pub fn findings(project: &Project) -> Vec<Finding> {
    let Some(profile) = &project.rules.profile else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let layers = project.stackup.copper.len();
    if let Some(boards) = &profile.boards
        && !boards
            .copper_layers
            .iter()
            .any(|n| usize::try_from(*n).is_ok_and(|n| n == layers))
    {
        let counts: Vec<String> = boards
            .copper_layers
            .iter()
            .map(ToString::to_string)
            .collect();
        out.push(
            Finding::new(
                LAYERS,
                format!(
                    "{} is for {}-layer boards; the board has {layers} copper layers",
                    profile.name,
                    or_list(&counts)
                ),
                vec![ObjectRef::Board],
            )
            .requiring(
                u32::try_from(layers)
                    .ok()
                    .map(|count| Requirement::CopperLayers { count }),
            ),
        );
    }
    let thickness = project.stackup.thickness_mm();
    let finish = project.fabrication.pad_finish;
    let (mut thin, mut finished) = (Vec::new(), Vec::new());
    for option in profile.options() {
        let chosen = profile.chosen().get(&option.id);
        let Some(choice) = option.choices.iter().find(|c| chosen == Some(&c.id)) else {
            continue;
        };
        let Some(requires) = &choice.requires else {
            continue;
        };
        if let Some(max) = requires.board_thickness_max_mm
            && thickness > max + 1e-9
        {
            thin.push(format!(
                "{}: {} needs a board {} mm or thinner; the board is {} mm",
                option.name,
                choice.name,
                super::limits::mm(max),
                super::limits::mm(thickness)
            ));
        }
        if !requires.finishes.is_empty() && !requires.finishes.iter().any(|f| f == finish.id()) {
            let words: Vec<String> = requires
                .finishes
                .iter()
                .map(|f| PadFinish::from_id(f).map_or_else(|| f.clone(), |p| p.label().to_owned()))
                .collect();
            finished.push(format!(
                "{}: {} needs {}; the board's finish is {}",
                option.name,
                choice.name,
                or_list(&words),
                finish.label()
            ));
        }
    }
    for (code, messages) in [
        (FindingCode::BoardFabChoiceThickness, thin),
        (FindingCode::BoardFabChoiceFinish, finished),
    ] {
        if !messages.is_empty() {
            out.push(Finding::new(
                code,
                messages.join("; "),
                vec![ObjectRef::Board],
            ));
        }
    }
    out
}

/// "2", "2 or 4", "1, 2 or 4".
fn or_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}
