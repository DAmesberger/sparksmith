use super::*;
use crate::model::Population;

#[test]
fn ordinary_components_keep_their_serialized_identity() -> Result<(), serde_json::Error> {
    let component = ComponentDefinition::default();
    let value = serde_json::to_value(&component)?;
    assert_eq!(component.assembly, Assembly::Bought);
    assert!(value.get("assembly").is_none());
    assert!(value.get("ties").is_none());
    Ok(())
}

#[test]
fn an_end_cannot_be_listed_twice() {
    let tie = TieSpec {
        id: crate::ids::TieId(1),
        form: TieForm::Copper,
        ends: vec![TerminalId(1), TerminalId(1)],
    };
    let component = ComponentDefinition {
        terminals: [1, 2]
            .map(|i| super::super::Terminal {
                id: TerminalId(i),
                name: i.to_string(),
                class: super::super::TerminalClass::Electrical(
                    crate::functions::TerminalFunctions::fixed(
                        crate::ids::FunctionId(i + 2),
                        crate::functions::Direction::Passive,
                    ),
                ),
            })
            .to_vec(),
        ..Default::default()
    };
    assert!(!tie.valid_ends(&component));
    assert!(
        TieSpec {
            ends: vec![TerminalId(1), TerminalId(2)],
            ..tie
        }
        .valid_ends(&component)
    );
}

#[test]
fn an_open_jumper_uses_the_existing_population_switch() {
    let form = TieForm::SolderJumper {
        default: JumperState::Open,
        bridge: JumperBridge::Trace,
    };
    assert_eq!(Population::from(form), Population::NotPopulated);
    assert_eq!(form.assembly(), Assembly::SolderOnly);
    assert_eq!(Population::from(TieForm::Copper), Population::Populated);
}

#[test]
fn an_unplated_hole_cannot_supply_a_tie_end() -> crate::Result<()> {
    use crate::ids::PackageTerminalId;
    use crate::library::{Pad, TerminalMap};
    let package_pin = PackageTerminalId(12);
    let end = TerminalId(2);
    let component = ComponentDefinition {
        ties: vec![TieSpec {
            id: TieId(4),
            ends: vec![TerminalId(1), end],
            form: TieForm::Copper,
        }],
        ..Default::default()
    };
    let binding = BindingDefinition {
        mappings: vec![TerminalMap {
            package_terminal: package_pin,
            terminal: end,
        }],
        ..Default::default()
    };
    let mut footprint = FootprintDefinition::default();
    let mut land = Pad::smd_rect_mm(PadId(3), package_pin, "12", [0., 0.], [1., 1.])?;
    land.kind = PadKind::NonPlatedHole;
    footprint.pads.push(land);
    assert_eq!(
        TiePads::of(&footprint, &binding, &component).terminal(PadId(3)),
        None
    );
    footprint.pads[0].kind = PadKind::Smd;
    assert_eq!(
        TiePads::of(&footprint, &binding, &component).terminal(PadId(3)),
        Some(end)
    );
    Ok(())
}
