//! Fabricators' stackups on original data: what a board takes from one,
//! what a broken file is refused for, and what taking one does.
use super::*;
use crate::layout::restack::LayerMove;
use crate::layout::{LayoutCommand, RouteRequest};
use crate::model::{CopperLayer, Project};
use crate::rules::{ClassLayer, NetClass, NetClassId, NetClassSpec, ProfileBoards};
use crate::{Engine, Operation};

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn layer(role: FabLayerRole, mm: f64) -> FabLayer {
    FabLayer {
        role,
        name: None,
        glass: None,
        brand: None,
        material: None,
        material_english: None,
        thickness_mm: mm,
        after_lamination_mm: None,
        dk: None,
        foil_text: None,
        plated_to_oz: None,
        plated_thickness_mm: None,
        listed_with_copper_mm: None,
        listed_with_copper_text: None,
        bare_board: false,
        source: "Test row".into(),
    }
}

fn copper(mm: f64) -> FabLayer {
    layer(FabLayerRole::Copper, mm)
}

fn prepreg(mm: f64, glass: &str, dk: f64) -> FabLayer {
    FabLayer {
        glass: Some(glass.into()),
        material: Some(format!("{glass} RC50%")),
        dk: Some(dk),
        ..layer(FabLayerRole::Prepreg, mm)
    }
}

fn core(mm: f64, dk: f64) -> FabLayer {
    FabLayer {
        dk: Some(dk),
        ..layer(FabLayerRole::Core, mm)
    }
}

/// A stackup of `layers`, its sums as they add up.
fn stackup(layers: Vec<FabLayer>) -> FabStackup {
    let sum: f64 = layers.iter().map(|l| l.thickness_mm).sum();
    let keys = |l: &FabLayer| (l.role, l.thickness_mm.to_bits(), l.glass.clone());
    let listed: Vec<_> = layers.iter().map(keys).collect();
    FabStackup {
        id: "test-4".into(),
        name: "Test 4".into(),
        fabricator: "test-fab".into(),
        fab_profiles: vec!["test-profile".into()],
        copper_layers: layers
            .iter()
            .filter(|l| l.role == FabLayerRole::Copper)
            .count(),
        outer_copper_oz: 1.0,
        inner_copper_oz: Some(0.5),
        source: ProfileSource {
            publisher: "Test fab".into(),
            document: "Original test data".into(),
            url: "https://fab.example".into(),
            date: "2026-11-24".into(),
            notes: String::new(),
        },
        nominal: FabNominal {
            board_mm: 1.6,
            basis: "Test".into(),
        },
        stated_finished: None,
        stated_after_lamination: None,
        inner_layer_residual_copper_ratio_pct: None,
        symmetric: listed.iter().eq(listed.iter().rev()),
        layers,
        layers_sum_mm: sum,
        layers_sum_basis: "Added".into(),
        layers_sum_minus_nominal_mm: sum - 1.6,
        layers_sum_minus_stated_finished_mm: None,
        layers_sum_after_lamination_mm: None,
        layers_sum_after_lamination_basis: None,
        layers_sum_after_lamination_minus_stated_mm: None,
        mask: None,
        flags: None,
        disagreements: Vec::new(),
        notes: Vec::new(),
        on_impedance_page: None,
    }
}

/// Foil, 0.2 mm 2116, a 1.1 mm core, 0.2 mm 2116, foil.
fn plain() -> FabStackup {
    stackup(vec![
        copper(0.035),
        prepreg(0.2, "2116", 4.2),
        copper(0.0152),
        core(1.1, 4.4),
        copper(0.0152),
        prepreg(0.2, "2116", 4.2),
        copper(0.035),
    ])
}

/// [`plain`] with two plies of different Dk under the top foil.
fn mixed() -> FabStackup {
    let mut layers = plain().layers;
    layers.insert(1, prepreg(0.1, "1080", 3.9));
    stackup(layers)
}

fn four_layers() -> Fallible<Engine> {
    let mut engine = Engine::demo()?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetStackup(Stackup::preset(4)?)),
    )?;
    Ok(engine)
}

fn refusal(stackup: &FabStackup) -> String {
    stackup
        .check()
        .err()
        .map_or_else(String::new, |e| e.to_string())
}

#[test]
fn a_board_takes_the_listed_layers() -> Fallible<()> {
    let fab = plain();
    fab.check()?;
    let taken = fab.stackup(&Stackup::preset(4)?, false)?;
    let um: Vec<f64> = taken.copper.iter().map(|c| c.thickness_um).collect();
    assert_eq!(um, [35.0, 15.2, 15.2, 35.0]);
    let rows: Vec<_> = taken
        .dielectrics
        .iter()
        .map(|d| (d.kind, d.thickness_mm, d.glass, d.dk))
        .collect();
    assert_eq!(
        rows,
        [
            (
                DielectricKind::Prepreg,
                0.2,
                GlassStyle::Style2116,
                Some(4.2)
            ),
            // A core of no named glass has none and shows none.
            (
                DielectricKind::Core,
                1.1,
                GlassStyle::Unspecified,
                Some(4.4)
            ),
            (
                DielectricKind::Prepreg,
                0.2,
                GlassStyle::Style2116,
                Some(4.2)
            ),
        ]
    );
    assert!(fab.is_build_of(&taken));
    assert!(!fab.is_build_of(&Stackup::preset(4)?));
    Ok(())
}

/// Plies of several Dk: 0.1 mm of 3.9 and 0.2 mm of 4.2 average to
/// (0.39 + 0.84) / 0.3 = 4.1. Unless asked for, the layer has none, and no
/// glass estimate stands in for it.
#[test]
fn plies_of_several_dk_give_no_dk_unless_their_average_is_asked_for() -> Fallible<()> {
    let fab = mixed();
    fab.check()?;
    let board = Stackup::preset(4)?;
    let taken = fab.stackup(&board, false)?;
    let first = taken.dielectrics[0];
    assert!((first.thickness_mm - 0.3).abs() < 1e-9);
    assert_eq!(first.glass, GlassStyle::Unspecified);
    assert_eq!(taken.permittivity(&first), None);
    assert_eq!(fab.dielectrics()[0].dk_text(), "3.9 and 4.2");
    let averaged = fab.stackup(&board, true)?;
    assert_eq!(averaged.dielectrics[0].dk, Some(4.1));
    // The other layers keep their one Dk either way.
    assert_eq!(averaged.dielectrics[2].dk, Some(4.2));
    Ok(())
}

/// The board takes the pressed prepregs, and the file keeps both sums.
#[test]
fn a_board_takes_pressed_thicknesses() -> Fallible<()> {
    let mut fab = plain();
    for layer in &mut fab.layers {
        if layer.role == FabLayerRole::Prepreg {
            layer.after_lamination_mm = Some(0.18);
        }
    }
    fab.layers_sum_after_lamination_mm = Some(fab.layers_sum_mm - 0.04);
    fab.check()?;
    let taken = fab.stackup(&Stackup::preset(4)?, false)?;
    assert_eq!(taken.dielectrics[0].thickness_mm, 0.18);
    let engine = four_layers()?;
    let pick = pick(engine.project(), &fab, &PickOptions::default())?;
    assert!(
        pick.open
            .iter()
            .any(|o| o.contains("before pressing") && o.contains("takes the pressed ones")),
        "{:?}",
        pick.open
    );
    Ok(())
}

/// A bare core between prepregs carries no foil: the dielectric is bonded.
#[test]
fn a_bare_core_between_prepregs_is_one_bonded_dielectric() -> Fallible<()> {
    let mut layers = plain().layers;
    layers.splice(
        3..4,
        [
            prepreg(0.2, "2116", 4.2),
            FabLayer {
                bare_board: true,
                ..core(0.6, 4.4)
            },
            prepreg(0.2, "2116", 4.2),
        ],
    );
    let fab = stackup(layers);
    fab.check()?;
    let dielectrics = fab.dielectrics();
    assert_eq!(dielectrics.len(), 3);
    assert_eq!(dielectrics[1].kind(), DielectricKind::Prepreg);
    assert!((dielectrics[1].thickness_mm() - 1.0).abs() < 1e-9);
    Ok(())
}

#[test]
fn a_bad_value_is_refused_naming_its_field() {
    type Break = Box<dyn Fn(&mut FabStackup)>;
    let cases: Vec<(&str, Break)> = vec![
        (
            "layers[1].thickness_mm",
            Box::new(|s| s.layers[1].thickness_mm = 0.0),
        ),
        (
            "layers[3].thickness_mm",
            Box::new(|s| s.layers[3].thickness_mm = f64::NAN),
        ),
        ("layers[1].dk", Box::new(|s| s.layers[1].dk = Some(0.5))),
        ("layers[0].dk", Box::new(|s| s.layers[0].dk = Some(4.0))),
        (
            "layers[1].glass",
            Box::new(|s| s.layers[1].glass = Some("S1000-2M".into())),
        ),
        (
            "layers[2].source",
            Box::new(|s| s.layers[2].source = " ".into()),
        ),
        ("layers_sum_mm", Box::new(|s| s.layers_sum_mm += 0.001)),
        (
            "layers_sum_minus_nominal_mm",
            Box::new(|s| s.nominal.board_mm = 1.5),
        ),
        ("symmetric", Box::new(|s| s.symmetric = false)),
        ("copper_layers", Box::new(|s| s.copper_layers = 3)),
        ("layers lists", Box::new(|s| s.copper_layers = 6)),
        ("outer_copper_oz", Box::new(|s| s.outer_copper_oz = 0.0)),
        ("id is", Box::new(|s| s.id = "Test 4".into())),
        ("source.url needs", Box::new(|s| s.source.url.clear())),
        ("fab_profiles", Box::new(|s| s.fab_profiles.clear())),
        (
            "disagreements[0]",
            Box::new(|s| s.disagreements.push(String::new())),
        ),
        (
            "mask.above_trace_mil",
            Box::new(|s| {
                s.mask = Some(FabMask {
                    source: "Test".into(),
                    above_substrate_mil: 1.2,
                    above_trace_mil: 0.0,
                    between_traces_mil: 1.2,
                    dk: 3.8,
                });
            }),
        ),
    ];
    for (field, break_it) in cases {
        let mut fab = plain();
        break_it(&mut fab);
        let said = refusal(&fab);
        assert!(said.contains(field), "{field}: {said:?}");
    }
    let mut two_foils = plain();
    two_foils.layers.remove(1);
    assert!(refusal(&stackup(two_foils.layers)).contains("dielectric between copper"));
}

#[test]
fn taking_one_is_one_step_that_names_every_change_and_keeps_the_planes() -> Fallible<()> {
    let mut engine = four_layers()?;
    let mut marked = engine.project().stackup.clone();
    marked.copper[1].plane = true;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetStackup(marked)),
    )?;
    let before = engine.project().clone();
    let pick = pick(engine.project(), &plain(), &PickOptions::default())?;
    assert_eq!(pick.refused, None);
    assert!(pick.lost.is_empty());
    assert_eq!(engine.project(), &before, "a pick applies nothing");
    for part in [
        "Inner 1 copper 35 µm becomes 15.2 µm",
        "Top to Inner 1: prepreg 2116, 0.2 mm, no Dk entered becomes prepreg 2116, 0.2 mm, Dk 4.2",
        "Inner 1 to Inner 2: core",
        "Board 1.6 mm becomes 1.6004 mm",
    ] {
        assert!(
            pick.changes.iter().any(|c| c.starts_with(part)),
            "{part}: {:?}",
            pick.changes
        );
    }
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditStackup(pick.edit)),
    )?;
    let taken = &engine.project().stackup;
    assert!(plain().is_build_of(taken));
    assert!(taken.copper[1].plane, "the plane mark stays with its layer");
    // Taking what the board has changes nothing.
    let again = super::pick(engine.project(), &plain(), &PickOptions::default())?;
    assert_eq!(again.changes, Vec::<String>::new());
    engine.undo()?;
    assert_eq!(engine.project(), &before);
    Ok(())
}

/// Eight layers to four: Inner 5 carries a track. Without a place for it
/// the pick says the engine's refusal; with one it says where it goes.
#[test]
fn copper_on_a_lost_layer_refuses_until_it_has_a_place() -> Fallible<()> {
    let mut engine = Engine::demo()?;
    let net = engine.project().nets[0].id;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::SetStackup(Stackup::preset(8)?)),
    )?;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::Route {
            request: RouteRequest {
                net,
                start: [20.0, 20.0],
                end: [30.0, 20.0],
                width: 0.3,
                layer: CopperLayer::Inner(5),
            },
        }),
    )?;
    let without = pick(engine.project(), &plain(), &PickOptions::default())?;
    assert!(
        without
            .refused
            .as_ref()
            .is_some_and(|r| r.contains("Inner 5 still carries 1 track")),
        "{:?}",
        without.refused
    );
    assert_eq!(without.lost.len(), 4);
    let options = PickOptions {
        moves: vec![LayerMove {
            from: CopperLayer::Inner(5),
            to: CopperLayer::Inner(2),
        }],
        average_dk: false,
    };
    let with = pick(engine.project(), &plain(), &options)?;
    assert_eq!(with.refused, None);
    assert!(
        with.changes
            .contains(&"Inner 5 goes; its copper moves to Inner 2".to_owned()),
        "{:?}",
        with.changes
    );
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::EditStackup(with.edit)),
    )?;
    assert_eq!(engine.project().tracks[0].layer, CopperLayer::Inner(2));
    // A layer that stays keeps its copper.
    let kept = PickOptions {
        moves: vec![LayerMove {
            from: CopperLayer::Inner(1),
            to: CopperLayer::Inner(2),
        }],
        average_dk: false,
    };
    assert!(pick(engine.project(), &plain(), &kept).is_err());
    Ok(())
}

/// What the profile and the classes say is said before: a layer count the
/// profile does not cover, another maker, a class that names a lost layer.
#[test]
fn conflicts_are_said_before_it_is_taken() -> Fallible<()> {
    let mut project: Project = four_layers()?.project().clone();
    let profile = project.rules.profile.as_mut().ok_or("no profile")?;
    profile.boards = Some(ProfileBoards {
        copper_layers: vec![4],
        source: String::new(),
    });
    profile.fabricator = Some("other-fab".into());
    let two = {
        let mut two = stackup(vec![copper(0.035), core(1.5, 4.4), copper(0.035)]);
        two.id = "test-2".into();
        two
    };
    let spec = NetClassSpec {
        name: "Lanes".into(),
        layers: Some(vec![ClassLayer::Inner(1)]),
        ..Default::default()
    };
    project
        .rules
        .net_classes
        .push(NetClass::from_spec(NetClassId(90), spec));
    let pick = pick(&project, &two, &PickOptions::default())?;
    for part in [
        "is a stackup of Test fab; the board's fab profile is",
        "is for 4-layer boards; the board has 2 copper layers",
        "Net class Lanes names Inner 1, which the board does not have",
    ] {
        assert!(
            pick.conflicts.iter().any(|c| c.contains(part)),
            "{part}: {:?}",
            pick.conflicts
        );
    }
    // The profile's own maker, and a profile it is not listed for.
    let profile = project.rules.profile.as_mut().ok_or("no profile")?;
    profile.fabricator = Some("test-fab".into());
    let listed = super::pick(&project, &plain(), &PickOptions::default())?;
    assert!(
        listed
            .conflicts
            .iter()
            .any(|c| c.contains("is not listed for the fab profile")),
        "{:?}",
        listed.conflicts
    );
    Ok(())
}

/// An outer weight the board's profile does not order refuses the edit, and
/// the pick says the engine's own words.
#[test]
fn an_outer_weight_the_profile_does_not_order_is_refused_before() -> Fallible<()> {
    let mut project: Project = four_layers()?.project().clone();
    project.rules.profile = Some(crate::rules::fixtures::jlcpcb_example()?);
    let mut heavy = plain();
    heavy.outer_copper_oz = 2.0;
    let pick = pick(&project, &heavy, &PickOptions::default())?;
    assert!(
        pick.refused.as_ref().is_some_and(
            |r| r.contains("outer copper weight is 1 oz (35 µm)") && r.contains("stay 35 µm")
        ),
        "{:?}",
        pick.refused
    );
    // The listed foil is not the finished one: both are said.
    assert!(
        pick.open
            .iter()
            .any(|o| o.contains("0.035 mm") && o.contains("2 oz") && o.contains("70 µm")),
        "{:?}",
        pick.open
    );
    Ok(())
}

#[test]
fn a_list_shows_the_profiles_maker_or_every_maker_when_it_names_none() -> Fallible<()> {
    let mut project: Project = four_layers()?.project().clone();
    let mut other = plain();
    other.id = "other-4".into();
    other.fabricator = "other-fab".into();
    let mut two = stackup(vec![copper(0.035), core(1.5, 4.4), copper(0.035)]);
    two.id = "test-2".into();
    let library = [plain(), other, two];
    let ids = |project: &Project, filter: &StackupFilter| -> Vec<String> {
        list(project, &library, filter)
            .stackups
            .into_iter()
            .map(|s| s.id)
            .collect()
    };
    let board = StackupFilter::default();
    let profile = project.rules.profile.as_mut().ok_or("no profile")?;
    profile.fabricator = None;
    assert_eq!(ids(&project, &board), ["other-4", "test-4"]);
    assert_eq!(list(&project, &library, &board).left_out, 1);
    let profile = project.rules.profile.as_mut().ok_or("no profile")?;
    profile.fabricator = Some("test-fab".into());
    assert_eq!(ids(&project, &board), ["test-4"]);
    let all = StackupFilter {
        all: true,
        ..StackupFilter::default()
    };
    assert_eq!(ids(&project, &all), ["other-4", "test-2", "test-4"]);
    let thin = StackupFilter {
        board_mm: Some(0.8),
        ..all
    };
    assert!(ids(&project, &thin).is_empty());
    // The row of the build the board has says so.
    project.stackup = plain().stackup(&project.stackup, false)?;
    let rows = list(&project, &library, &board).stackups;
    assert!(rows.iter().all(|r| r.board), "{rows:?}");
    Ok(())
}

/// The engine has no stackup library: it names the id and changes nothing.
#[test]
fn an_unresolved_id_is_refused() -> Fallible<()> {
    let mut engine = four_layers()?;
    let before = engine.project().clone();
    let refused = engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::UseStackup {
            stackup: "test-4".into(),
            moves: Vec::new(),
            average_dk: false,
        }),
    );
    assert!(
        refused
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("test-4")),
        "{refused:?}"
    );
    assert_eq!(engine.project(), &before);
    Ok(())
}
