//! A fab profile's numbers hold only for the boards it is for: a
//! four-layer board under a two-layer profile, and a choice whose board
//! prerequisites (thickness, finish) the board does not meet, are errors
//! that block production instead of a clean pass.
use crate::Engine;
use crate::fabrication::preflight;
use crate::issues::Severity;
use crate::layout::profile_fit::{self, CHOICE_FINISH, CHOICE_THICKNESS, LAYERS};
use crate::model::{PadFinish, Project};
use crate::rules::{self, RuleProfile, RulesCommand, fixtures};
use std::collections::BTreeMap;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn codes(project: &Project) -> Vec<String> {
    profile_fit::findings(project)
        .into_iter()
        .map(|f| f.code)
        .collect()
}

fn under(mut project: Project, profile: &RuleProfile) -> Fallible<Project> {
    rules::apply(
        &mut project.rules,
        RulesCommand::set_profile(profile.clone(), true),
    )?;
    crate::layout::mask_colour::follow(&mut project.rules, project.fabrication.mask_colour)?;
    Ok(project)
}

/// The shipped JLCPCB 2-layer profile is for 2-layer boards.
#[test]
fn a_four_layer_board_does_not_pass_under_a_two_layer_profile() -> Fallible<()> {
    let showcase = Engine::showcase()?.project().clone();
    assert_eq!(showcase.stackup.copper.len(), 4);
    let jlc = fixtures::jlcpcb_example()?;
    assert_eq!(
        jlc.boards.as_ref().map(|b| b.copper_layers.clone()),
        Some(vec![2])
    );
    let checked = under(showcase.clone(), &jlc)?;
    assert_eq!(codes(&checked), [LAYERS]);
    let message = profile_fit::findings(&checked)
        .into_iter()
        .map(|f| f.message)
        .collect::<String>();
    assert_eq!(
        message,
        "JLCPCB 2-layer is for 2-layer boards; the board has 4 copper layers"
    );
    // The production check under that profile blocks on it, unwaived.
    let report = preflight::report(
        &showcase,
        &Default::default(),
        Some(&jlc),
        &Default::default(),
    )?;
    let item = report
        .items
        .iter()
        .find(|i| i.code == LAYERS)
        .ok_or("no layer finding")?;
    assert_eq!(item.severity, Severity::Error);
    assert!(item.blocks() && !report.ready());
    // A profile that says nothing about its boards checks none.
    assert!(codes(&showcase).is_empty());
    Ok(())
}

/// JLCPCB's 0.1 mm drilling needs a board 1 mm or thinner with an ENIG or
/// OSP finish; the profile says so as data, and the board is held to it.
#[test]
fn a_choice_holds_the_board_to_its_prerequisites() -> Fallible<()> {
    let mut showcase = Engine::showcase()?.project().clone();
    let jlc = fixtures::jlcpcb_example()?;
    let mut four = jlc.clone();
    four.boards = Some(crate::rules::ProfileBoards {
        copper_layers: vec![4],
        source: String::new(),
    });
    let microvia = four.with_choices(&BTreeMap::from([("drill".into(), "microvia".into())]))?;
    let thickness = showcase.stackup.thickness_mm();
    assert!(thickness > 1.0, "{thickness}");

    showcase.fabrication.pad_finish = PadFinish::Enig;
    assert!(
        codes(&under(showcase.clone(), &four)?).is_empty(),
        "standard drilling"
    );
    let checked = under(showcase.clone(), &microvia)?;
    assert_eq!(codes(&checked), [CHOICE_THICKNESS]);
    let message = &profile_fit::findings(&checked)[0].message;
    assert!(
        message.starts_with("Smallest drill: 0.1 mm (board 1 mm or thinner, ENIG or OSP finish) needs a board 1 mm or thinner; the board is"),
        "{message}"
    );

    showcase.fabrication.pad_finish = PadFinish::LeadFreeHasl;
    let checked = under(showcase.clone(), &microvia)?;
    assert_eq!(codes(&checked), [CHOICE_THICKNESS, CHOICE_FINISH]);
    assert!(
        profile_fit::findings(&checked)[1]
            .message
            .ends_with("needs ENIG or OSP; the board's finish is lead-free HASL"),
    );
    showcase.fabrication.pad_finish = PadFinish::Osp;
    assert_eq!(codes(&under(showcase, &microvia)?), [CHOICE_THICKNESS]);
    Ok(())
}

/// A requirement that names no surface finish is refused with the file,
/// never read as one no board meets; a layer list out of order is refused.
#[test]
fn malformed_board_requirements_are_refused() -> Fallible<()> {
    let text = fixtures::JLCPCB_EXAMPLE_JSON.replace("\"osp\"", "\"gold\"");
    let profile = fixtures::parse(&text)?;
    let project = under(Engine::showcase()?.project().clone(), &profile)?;
    assert!(crate::layout::validate(&project).is_err());
    for bad in ["[]", "[4, 2]", "[2, 2]", "[0]", "[66]"] {
        let text = fixtures::JLCPCB_EXAMPLE_JSON.replace(
            "\"copper_layers\": [\n        2\n      ]",
            &format!("\"copper_layers\": {bad}"),
        );
        assert_ne!(text, fixtures::JLCPCB_EXAMPLE_JSON);
        assert!(fixtures::parse(&text).is_err(), "{bad}");
    }
    Ok(())
}
