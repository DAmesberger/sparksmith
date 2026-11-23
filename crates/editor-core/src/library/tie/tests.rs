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
