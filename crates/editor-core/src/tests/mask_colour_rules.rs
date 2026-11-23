//! The board's solder mask colour drives the fab profile's colour option
//! (`layout::mask_colour`): one place sets the colour, the rule reads it.
//! The profile here is the prototype fixture with an original colour
//! option, not a fabricator's numbers.
use crate::layout::{self, LayoutCommand, mask_colour};
use crate::model::{Fabrication, MaskColour};
use crate::rules::{self, ProfileChoice, RuleProfile, RulesCommand, fixtures};
use crate::{Engine, Operation, Query, QueryResult};
use std::collections::BTreeMap;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The prototype fixture offering a colour option: "light" (green, blue;
/// the base 0.1 mm web) and "dark" (black; 0.15 mm). Red, white, purple,
/// yellow and clear are not offered.
fn coloured() -> Fallible<RuleProfile> {
    let mut value: serde_json::Value = serde_json::from_str(fixtures::PROTOTYPE_JSON)?;
    let p = &mut value["fab_profile"];
    p["id"] = "test-coloured".into();
    p["name"] = "Test coloured".into();
    p["options"] = serde_json::json!([{
        "id": "mask", "name": "Mask colour", "default": "light", "follows": "mask_colour",
        "source": "Test fixture.",
        "choices": [
            {"id": "light", "name": "Light", "mask_colours": ["green", "blue"]},
            {"id": "dark", "name": "Dark", "mask_colours": ["black"]}
        ]
    }]);
    p["rules"]["mask_web_min"]["cases"] = serde_json::json!([
        {"when": {"mask": "dark"}, "value": 0.15, "source": "Test fixture: dark mask."}
    ]);
    Ok(fixtures::parse(&value.to_string())?)
}

fn web(engine: &Engine) -> f64 {
    engine.project().rules.effective().mask.web_min
}

fn chosen(engine: &Engine) -> Option<String> {
    engine
        .project()
        .rules
        .profile
        .as_ref()
        .and_then(|p| p.chosen().get("mask").cloned())
}

fn colour(engine: &mut Engine, mask_colour: MaskColour) -> Fallible<()> {
    let fabrication = Fabrication {
        mask_colour,
        ..engine.project().fabrication
    };
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetFabrication(fabrication)),
    )?;
    Ok(())
}

fn not_offered(engine: &Engine) -> Vec<String> {
    mask_colour::findings(engine.project())
        .into_iter()
        .map(|f| f.message)
        .collect()
}

#[test]
fn the_board_colour_chooses_the_mask_rule_in_the_same_undo_step() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    assert_eq!(engine.project().fabrication.mask_colour, MaskColour::Green);
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(coloured()?, false)),
    )?;
    assert_eq!(web(&engine), 0.1);
    assert_eq!(chosen(&engine).as_deref(), Some("light"));

    colour(&mut engine, MaskColour::Black)?;
    assert_eq!(web(&engine), 0.15);
    assert_eq!(chosen(&engine).as_deref(), Some("dark"));
    assert!(not_offered(&engine).is_empty());
    // The colour and the rule are one step.
    engine.undo()?;
    assert_eq!(engine.project().fabrication.mask_colour, MaskColour::Green);
    assert_eq!(web(&engine), 0.1);
    engine.redo()?;
    assert_eq!(web(&engine), 0.15);

    // No rules command chooses the colour option.
    let rev = engine.revision();
    let refused = engine
        .apply(
            rev,
            Operation::Rules(RulesCommand::SetOptions {
                options: BTreeMap::from([("mask".into(), "light".into())]),
            }),
        )
        .map_err(|e| e.to_string());
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.contains("follows the board's solder mask colour")),
        "{refused:?}"
    );
    let refused = engine.apply(
        rev,
        Operation::Rules(RulesCommand::SetProfile {
            profile: ProfileChoice::Profile(Box::new(coloured()?)),
            options: BTreeMap::from([("mask".into(), "light".into())]),
            keep_overrides: false,
        }),
    );
    assert!(refused.is_err());
    assert_eq!(engine.revision(), rev, "a refusal changes nothing");

    // Taking the profile again on a black board keeps the black rule.
    engine.apply(
        rev,
        Operation::Rules(RulesCommand::set_profile(coloured()?, false)),
    )?;
    assert_eq!(engine.revision(), rev, "the same profile is no step");
    assert_eq!(web(&engine), 0.15);
    Ok(())
}

#[test]
fn a_colour_the_profile_does_not_offer_is_a_finding() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(coloured()?, false)),
    )?;
    colour(&mut engine, MaskColour::Black)?;
    colour(&mut engine, MaskColour::Clear)?;
    // Not offered: the option is at its default, and the board says why.
    assert_eq!(chosen(&engine).as_deref(), Some("light"));
    assert_eq!(web(&engine), 0.1);
    assert_eq!(
        not_offered(&engine),
        ["Test coloured does not offer a clear solder mask; it offers green, blue, black"]
    );
    let codes: Vec<String> = layout::checks(engine.project(), &Default::default())
        .into_iter()
        .map(|f| f.code.to_string())
        .collect();
    assert!(
        codes.iter().any(|c| c == mask_colour::NOT_OFFERED),
        "{codes:?}"
    );
    colour(&mut engine, MaskColour::Blue)?;
    assert!(not_offered(&engine).is_empty());

    // A profile that does not say which colours it offers finds nothing.
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(fixtures::prototype()?, false)),
    )?;
    colour(&mut engine, MaskColour::Clear)?;
    assert!(not_offered(&engine).is_empty());
    Ok(())
}

#[test]
fn a_profile_switch_previews_the_rule_the_board_colour_takes() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::set_profile(fixtures::prototype()?, false)),
    )?;
    colour(&mut engine, MaskColour::Black)?;
    let QueryResult::ProfileImpact(impact) = engine.query(Query::PreviewProfile {
        profile: ProfileChoice::Profile(Box::new(coloured()?)),
        options: BTreeMap::new(),
        keep_overrides: false,
    })?
    else {
        return Err("not an impact".into());
    };
    let web = impact
        .changes
        .iter()
        .find(|c| c.key == rules::RuleKey::MaskWebMin)
        .ok_or("the mask web changes")?;
    assert_eq!(web.to, rules::RuleValue::Length(0.15), "{web:?}");
    assert!(impact.kept_choices.is_empty() && impact.reset_choices.is_empty());
    Ok(())
}

#[test]
fn a_file_whose_choice_contradicts_the_colour_is_refused() -> Fallible<()> {
    let mut project = Engine::demo()?.project().clone();
    rules::apply(
        &mut project.rules,
        RulesCommand::set_profile(coloured()?, false),
    )?;
    crate::validate::project(&project)?;
    project.fabrication.mask_colour = MaskColour::Black;
    let refused = crate::validate::project(&project).map_err(|e| e.to_string());
    assert!(
        refused.as_ref().is_err_and(
            |e| e.contains("mask colour is Light, but the board's solder mask is black")
        ),
        "{refused:?}"
    );
    mask_colour::follow(&mut project.rules, MaskColour::Black)?;
    crate::validate::project(&project)?;

    // A colour name that is no solder mask colour is refused too.
    let mut value: serde_json::Value = serde_json::to_value(coloured()?)?;
    value["options"][0]["choices"][1]["mask_colours"] = serde_json::json!(["ink"]);
    project.rules.profile = Some(serde_json::from_value(value)?);
    project.fabrication.mask_colour = MaskColour::Green;
    let refused = crate::validate::project(&project).map_err(|e| e.to_string());
    assert!(
        refused
            .as_ref()
            .is_err_and(|e| e.contains("lists ink, which is not a solder mask colour")),
        "{refused:?}"
    );
    Ok(())
}
