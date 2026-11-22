//! ERC drive and supply semantics (docs/component-model.md, "ERC"): the
//! drive compatibility of every pair of drivers on a net, from one matrix,
//! and a power net's supply, which only a power output is. A passive pin
//! (a capacitor, a connector contact) next to a power port supplies
//! nothing; a connector that brings power in says so with a power-output
//! terminal.
use super::support::*;
use crate::erc;
use crate::functions::{Direction, Drive, TerminalFunctions};
use crate::ids::FunctionId;
use crate::library::TerminalClass;
use crate::model::{LabelKind, PowerStyle};
use crate::schematic::{SchematicCommand, WireTarget};
use crate::{Engine, Id, ObjectRef, Operation, Result};

fn driver(drive: Drive) -> TerminalClass {
    let mut functions = TerminalFunctions::fixed(FunctionId(0), Direction::Output);
    for f in &mut functions.functions {
        f.drive = drive;
    }
    TerminalClass::Electrical(functions)
}

/// The ERC codes on `net`, sorted.
fn on_net(engine: &Engine, net: Id) -> Vec<String> {
    let mut codes: Vec<String> = erc::checks(engine.project())
        .into_iter()
        .filter(|f| f.objects.contains(&ObjectRef::Net { id: net }))
        .map(|f| f.code)
        .collect();
    codes.sort();
    codes
}

fn all_codes(engine: &Engine) -> Vec<String> {
    let mut codes: Vec<String> = erc::checks(engine.project())
        .into_iter()
        .map(|f| f.code)
        .collect();
    codes.sort();
    codes
}

/// Two one-pin parts of `a` and `b`, wired together; their net.
fn pair(a: TerminalClass, b: TerminalClass) -> Result<(Engine, Id)> {
    let mut engine = Engine::default();
    let fa = component(&mut engine, &probe("A", a))?;
    let fb = component(&mut engine, &probe("B", b))?;
    let (u1, _) = place(&mut engine, &fa, "U1", 1, [0.0, 0.0])?;
    let (u2, _) = place(&mut engine, &fb, "U2", 1, [20.0, 0.0])?;
    connect(&mut engine, ep(u1, 1), ep(u2, 1))?;
    let net = net_of(&engine, ep(u1, 1))?;
    Ok((engine, net))
}

#[test]
fn every_pair_of_drives_is_judged_by_one_matrix() -> Result<()> {
    use Drive::{OpenDrain as Od, OpenSource as Os, PushPull as Pp};
    // A push-pull pair fights both ways; a sink against something that
    // drives high (push-pull or open-source) shorts when both are on;
    // open drains wire-AND and open sources wire-OR.
    let expected: [(Drive, Drive, &[&str]); 9] = [
        (Pp, Pp, &["erc.driver_conflict"]),
        (Pp, Od, &["erc.open_drain_contention"]),
        (Od, Pp, &["erc.open_drain_contention"]),
        (Pp, Os, &["erc.open_source_contention"]),
        (Os, Pp, &["erc.open_source_contention"]),
        (Od, Os, &["erc.open_source_contention"]),
        (Os, Od, &["erc.open_source_contention"]),
        (Od, Od, &[]),
        (Os, Os, &[]),
    ];
    for (a, b, codes) in expected {
        let (engine, net) = pair(driver(a), driver(b))?;
        assert_eq!(on_net(&engine, net), codes, "{a:?} with {b:?}");
    }
    Ok(())
}

fn power_port(engine: &mut Engine, name: &str, at: crate::Endpoint) -> Result<()> {
    apply(
        engine,
        Operation::Schematic(SchematicCommand::PlaceLabel {
            sheet: None,
            name: name.into(),
            kind: LabelKind::Power {
                style: PowerStyle::Bar,
            },
            position: pt([0.0, 0.0]),
            rotation: Default::default(),
            attach: Some(WireTarget::from(at)),
        }),
    )
    .map(|_| ())
}

#[test]
fn only_a_power_output_supplies_a_power_net() -> Result<()> {
    // A power input and a passive pin under a +3V3 port: nothing supplies
    // the rail.
    let (mut engine, net) = pair(power_in(), passive())?;
    let load = by_designator(&engine, "U1")?;
    power_port(&mut engine, "+3V3", ep(load, 1))?;
    assert_eq!(on_net(&engine, net), ["erc.power_port_undriven"]);
    // A regulator's power output does.
    let f = component(&mut engine, &probe("Regulator", power_out()))?;
    let (u3, _) = place(&mut engine, &f, "U3", 1, [0.0, -20.0])?;
    connect(&mut engine, ep(u3, 1), ep(load, 1))?;
    assert!(on_net(&engine, net).is_empty(), "{:?}", all_codes(&engine));

    // Without a port, a power input beside a passive pin is unsupplied too.
    let (engine, _) = pair(power_in(), passive())?;
    assert_eq!(all_codes(&engine), ["erc.undriven_power_input"]);

    // A ground port names the reference node, which nothing supplies: the
    // power inputs' ground pins need no source.
    let (mut engine, net) = pair(power_in(), passive())?;
    let load = by_designator(&engine, "U1")?;
    apply(
        &mut engine,
        Operation::Schematic(SchematicCommand::PlaceLabel {
            sheet: None,
            name: "GND".into(),
            kind: LabelKind::Power {
                style: PowerStyle::Ground,
            },
            position: pt([0.0, 0.0]),
            rotation: Default::default(),
            attach: Some(WireTarget::from(ep(load, 1))),
        }),
    )?;
    assert!(on_net(&engine, net).is_empty(), "{:?}", all_codes(&engine));

    // Decoupling alone, two passive pins under a port, has no power input
    // to supply and no finding.
    let (mut engine, net) = pair(passive(), passive())?;
    let cap = by_designator(&engine, "U1")?;
    power_port(&mut engine, "+3V3", ep(cap, 1))?;
    assert!(on_net(&engine, net).is_empty());
    Ok(())
}
