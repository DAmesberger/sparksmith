//! Native tests: the golden hashes, hand-derived geometry and refusals.
use super::*;
use sparksmith_generator_sdk::golden;

fn request(scope: Scope, overrides: &[(String, ParamValue)]) -> Request {
    let mut params = v1::Params::new();
    for param in description().params() {
        params.insert(param.key.clone(), param.default.clone());
    }
    for (key, value) in overrides {
        params.insert(key.clone(), value.clone());
    }
    Request {
        abi: v1::ABI,
        scope,
        params,
    }
}

fn preset(key: &str) -> Vec<(String, ParamValue)> {
    let description = description();
    let found = description.presets().iter().find(|p| p.key == key);
    found
        .map(|p| p.params.clone())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn kv(key: &str, value: i64) -> (String, ParamValue) {
    (key.into(), ParamValue::Int(value))
}

fn asset(g: &Generation) -> String {
    match &g.model {
        Some(v1::Model::PartFile { asset, .. }) => asset.clone(),
        _ => String::new(),
    }
}

/// Every lead contact lies on its own land.
fn contacts_on_lands(g: &Generation) {
    for lead in &g.package.geometry.leads {
        let pad = g.footprint.pads.iter().find(|p| p.terminal == lead.key);
        let b = pad.map(land::pad_box);
        assert!(
            b.is_some_and(|b| (b.min[0]..=b.max[0]).contains(&lead.contact[0])
                && (b.min[1]..=b.max[1]).contains(&lead.contact[1])),
            "lead {} at {:?} off its land {b:?}",
            lead.key,
            lead.contact
        );
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Tactile>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn every_preset_generates_on_its_lands() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 5);
    for p in description.presets() {
        let g = Tactile::generate(&request(Scope::Component, &preset(&p.key)))?;
        contacts_on_lands(&g);
        assert_eq!(
            g.component.map(|c| c.prefix),
            Some("SW".into()),
            "{}",
            p.key
        );
    }
    Ok(())
}

/// Every soldered terminal of every preset, the ground tab's 5 included,
/// has a lead contact, apart from whether each lies on its land.
#[test]
fn every_soldered_terminal_has_a_lead_contact() -> Result<(), Vec<Problem>> {
    for p in description().presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Tactile::generate(&request(scope, &preset(&p.key)))?;
            assert_eq!(
                golden::terminals_without_contacts(&g),
                Vec::<&str>::new(),
                "{}",
                p.key
            );
        }
    }
    Ok(())
}

#[test]
fn the_evqp2_with_ground_tab() -> Result<(), Vec<Problem>> {
    // Feet 0.7 mm long from the 2.75 mm tips: contacts at ±2.4 mm, ±0.85
    // mm; lands 1.45 × 0.7 mm at ±2.575, ±1.0 mm; the ground land at
    // −2.025 mm, the tab reaching 0.3 mm beyond the 3.5 mm base: its foot
    // from −1.75 to −2.05 mm, contact at its middle, −1.9 mm.
    let g = Tactile::generate(&request(Scope::Component, &preset("tactile-evqp2-ground")))?;
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(
        leads,
        vec![
            [-2_400_000, -850_000, 0],
            [-2_400_000, 850_000, 0],
            [2_400_000, -850_000, 0],
            [2_400_000, 850_000, 0],
            [0, -1_900_000, 0]
        ]
    );
    let at: Vec<(&str, [i64; 2])> = g
        .footprint
        .pads
        .iter()
        .map(|p| (p.key.as_str(), p.at))
        .collect();
    assert_eq!(
        at,
        vec![
            ("1", [-2_575_000, -1_000_000]),
            ("2", [-2_575_000, 1_000_000]),
            ("3", [2_575_000, -1_000_000]),
            ("4", [2_575_000, 1_000_000]),
            ("5", [0, -2_025_000])
        ]
    );
    assert_eq!(
        g.package.geometry.envelope_nm,
        [5_500_000, 4_100_000, 2_100_000]
    );
    assert_eq!(asset(&g), parts::TACTILE_MINI_GROUND);
    // 1 and 3, 2 and 4 joined (across); the ground tab is SH.
    let bound: Vec<(&str, &str)> = g
        .binding
        .iter()
        .map(|b| (b.package_terminal.as_str(), b.terminal.as_str()))
        .collect();
    assert_eq!(
        bound,
        vec![("1", "1"), ("2", "2"), ("3", "1"), ("4", "2"), ("5", "3")]
    );
    Ok(())
}

#[test]
fn the_evqp2v02m_on_four_lands_keeps_its_terminals_apart() -> Result<(), Vec<Problem>> {
    // The ground tab is drawn (the ground body, its reach in the envelope)
    // but lands nothing; each switch terminal is its own pin.
    let g = Tactile::generate(&request(
        Scope::Component,
        &preset("tactile-evqp2v02m-four-lands"),
    ))?;
    assert_eq!(asset(&g), parts::TACTILE_MINI_GROUND);
    let keys: Vec<&str> = g.footprint.pads.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["1", "2", "3", "4"]);
    assert_eq!(g.package.terminals, vec!["1", "2", "3", "4"]);
    assert_eq!(g.package.geometry.leads.len(), 4);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [5_500_000, 4_100_000, 2_100_000]
    );
    let bound: Vec<(&str, &str)> = g
        .binding
        .iter()
        .map(|b| (b.package_terminal.as_str(), b.terminal.as_str()))
        .collect();
    assert_eq!(bound, vec![("1", "1"), ("2", "2"), ("3", "3"), ("4", "4")]);
    // Joined across: 1 and 3 on the left contact, 2 and 4 on the right.
    let pins: Vec<(String, [i64; 2])> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.terminal.clone(), p.at))
        .collect();
    assert_eq!(
        pins,
        vec![
            ("1".into(), [-5_000_000, 0]),
            ("3".into(), [-5_000_000, -2_500_000]),
            ("2".into(), [5_000_000, 0]),
            ("4".into(), [5_000_000, -2_500_000])
        ]
    );
    // Soldered and separate, the ground tab is pin 5.
    let mut landed = preset("tactile-evqp2v02m-four-lands");
    landed.push(("ground_land".into(), ParamValue::Bool(true)));
    let g = Tactile::generate(&request(Scope::Component, &landed))?;
    assert_eq!(g.binding.last().map(|b| b.terminal.as_str()), Some("5"));
    Ok(())
}

#[test]
fn square_switches_join_their_pairs_as_drawn() -> Result<(), Vec<Problem>> {
    // TL3305: 1 and 2 joined (the pair at −y).
    let g = Tactile::generate(&request(Scope::Component, &preset("tactile-tl3305a")))?;
    assert_eq!(asset(&g), parts::TACTILE_SQUARE);
    let bound: Vec<&str> = g.binding.iter().map(|b| b.terminal.as_str()).collect();
    assert_eq!(bound, vec!["1", "1", "2", "2"]);
    // Gull-wing feet from 2.25 + 0.3 to 3.75 mm: contacts at ±3.15 mm.
    assert_eq!(
        g.package.geometry.leads[0].contact,
        [-3_150_000, -1_500_000, 0]
    );
    // B3S: joined along x (1 and 3 at −x).
    let g = Tactile::generate(&request(Scope::Component, &preset("tactile-b3s-1000")))?;
    let bound: Vec<&str> = g.binding.iter().map(|b| b.terminal.as_str()).collect();
    assert_eq!(bound, vec!["1", "2", "1", "2"]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [9_000_000, 6_600_000, 4_300_000]
    );
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Tactile::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("height", 1_600_000)]),
        Some(("param.inconsistent".into(), Some("height".into())))
    );
    assert_eq!(
        refused(&[kv("plunger_length", 4_600_000)]),
        Some(("param.inconsistent".into(), Some("plunger_length".into())))
    );
    assert_eq!(
        refused(&[kv("span", 4_900_000)]),
        Some(("param.inconsistent".into(), Some("span".into())))
    );
    let mut square = preset("tactile-tl3305a");
    square.push(("ground".into(), ParamValue::Bool(true)));
    assert_eq!(
        refused(&square),
        Some(("param.inconsistent".into(), Some("ground".into())))
    );
}
