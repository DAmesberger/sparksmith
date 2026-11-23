//! The board's solder mask colour drives the fab profile's colour option.
//!
//! A fabricator can publish a limit that depends on the mask colour (a
//! wider mask web for black or white ink). A profile says so with an
//! option that follows the board's colour ([`FabOption::follows`]), each
//! choice listing the colours it stands for ([`FabChoice::mask_colours`]).
//! The colour has one place where it is set, Board › Fabrication
//! (`layout.set_fabrication`): the option's choice is always the one that
//! lists the board's colour, kept by [`follow`] in the step that changes
//! the colour or the profile, and no rules command chooses it. A colour no
//! choice lists is one the fabricator does not offer ([`NOT_OFFERED`]);
//! the option then stays at its default.
//!
//! [`FabOption::follows`]: crate::rules::FabOption::follows
//! [`FabChoice::mask_colours`]: crate::rules::FabChoice::mask_colours
use crate::diagnostics::{Error, Finding, ObjectRef, Requirement, Result};
use crate::model::{MaskColour, Project};
use crate::rules::{BoardSetting, ProjectRules, RuleProfile};

/// The board's solder mask colour is not one its fab profile offers.
pub const NOT_OFFERED: &str = crate::diagnostics::FindingCode::BoardMaskColourNotOffered.as_str();

/// `profile` with its colour option at the choice `colour` takes.
pub fn following(profile: &RuleProfile, colour: MaskColour) -> Result<RuleProfile> {
    profile.with_board_setting(BoardSetting::MaskColour, colour.id())
}

/// `rules`' profile with its colour option at the choice `colour` takes;
/// unchanged without a profile or such an option.
pub fn follow(rules: &mut ProjectRules, colour: MaskColour) -> Result<()> {
    let Some(profile) = &rules.profile else {
        return Ok(());
    };
    let next = following(profile, colour)?;
    if next != *profile {
        rules.profile = Some(next);
    }
    Ok(())
}

/// The profile's colour option names only solder mask colours, and its
/// choice is the one the board's colour takes: a file that says otherwise
/// is refused, never read with a choice that contradicts the colour.
pub fn check(project: &Project) -> Result<()> {
    let Some(profile) = &project.rules.profile else {
        return Ok(());
    };
    let Some(option) = profile.following(BoardSetting::MaskColour) else {
        return Ok(());
    };
    for name in option.choices.iter().flat_map(|c| &c.mask_colours) {
        if MaskColour::from_id(name).is_none() {
            let known: Vec<&str> = MaskColour::ALL.iter().map(|c| c.id()).collect();
            return Err(Error::Invalid(format!(
                "rules.fab_option: {} lists {name}, which is not a solder mask colour ({})",
                option.name,
                known.join(", ")
            )));
        }
    }
    let colour = project.fabrication.mask_colour;
    if following(profile, colour)? != *profile {
        return Err(Error::Invalid(format!(
            "rules.fab_option_follows_board: the fab profile's {} is {}, but the board's solder \
             mask is {}",
            option.name.to_lowercase(),
            profile.choice_name(&option.id),
            colour.id()
        )));
    }
    Ok(())
}

/// [`NOT_OFFERED`] when the board's profile lists the colours it offers and
/// the board's colour is not among them.
pub fn findings(project: &Project) -> Vec<Finding> {
    let Some(profile) = &project.rules.profile else {
        return Vec::new();
    };
    let colour = project.fabrication.mask_colour;
    let Some(None) = profile.board_choice(BoardSetting::MaskColour, colour.id()) else {
        return Vec::new();
    };
    let offered: Vec<&str> = profile
        .following(BoardSetting::MaskColour)
        .into_iter()
        .flat_map(|o| &o.choices)
        .flat_map(|c| c.mask_colours.iter().map(String::as_str))
        .collect();
    vec![
        Finding::new(
            NOT_OFFERED,
            format!(
                "{} does not offer a {} solder mask; it offers {}",
                profile.name,
                colour.id(),
                offered.join(", ")
            ),
            vec![ObjectRef::Board],
        )
        .requiring(Some(Requirement::MaskColour {
            colour: colour.id().into(),
        })),
    ]
}
