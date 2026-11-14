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

fn text(key: &str, value: &str) -> (String, ParamValue) {
    (key.into(), ParamValue::Text(value.into()))
}

fn flag(key: &str, value: bool) -> (String, ParamValue) {
    (key.into(), ParamValue::Bool(value))
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<NoLead>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 25);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = NoLead::generate(&request(scope, &preset(&p.key)))?;
            let pads = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pads, "{}", p.key);
            assert_eq!(g.package.terminals.len(), pads, "{}", p.key);
            // Every starter row has an exposed pad, the last terminal.
            assert_eq!(g.package.terminals.last().map(String::as_str), Some("EP"));
        }
    }
    Ok(())
}

#[test]
fn qfn16_lands_keep_the_pad_clearance() -> Result<(), Vec<Problem>> {
    // 0.25 mm terminals at 0.5 mm: lands 0.30 mm wide (0.5 − 0.2 mm of
    // copper gap allows it) and 0.8 mm long, centred 1.3 mm out. The heel
    // is 0.05 mm from the 1.7 mm pad, so 0.15 mm comes off it, keeping
    // the toe: 0.65 mm long, centred 1.375 mm out.
    let g = NoLead::generate(&request(
        Scope::Component,
        &preset("no-lead-qfn-16-3x3-0.5"),
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 17);
    let at = |k: usize| pads.get(k).map(|p| (p.key.as_str(), p.at, p.shape.clone()));
    assert_eq!(
        at(0),
        Some((
            "1",
            [-750_000, -1_375_000],
            PadShape::Rect {
                size: [300_000, 650_000]
            }
        ))
    );
    // Pin 5 starts the +x row, at its −y end, lying along x.
    assert_eq!(
        at(4),
        Some((
            "5",
            [1_375_000, -750_000],
            PadShape::Rect {
                size: [650_000, 300_000]
            }
        ))
    );
    // Pin 9 starts the +y row at its +x end; pin 16 ends the −x row at −y.
    assert_eq!(at(8).map(|p| p.1), Some([750_000, 1_375_000]));
    assert_eq!(at(15).map(|p| p.1), Some([-1_375_000, -750_000]));
    assert_eq!(
        at(16),
        Some((
            "EP",
            [0, 0],
            PadShape::Rect {
                size: [1_700_000, 1_700_000]
            }
        ))
    );
    // The leads' contacts are the terminals' centres, not the trimmed lands'.
    let leads = &g.package.geometry.leads;
    assert_eq!(
        leads.first().map(|l| l.contact),
        Some([-750_000, -1_300_000, 0])
    );
    assert_eq!(leads.get(16).map(|l| l.contact), Some([0, 0, 0]));
    assert_eq!(
        g.package.geometry.envelope_nm,
        [3_000_000, 3_000_000, 900_000]
    );
    // The pin-1 dot beyond pad 1's −x edge, past its 0.05 mm mask, 0.1 mm
    // clearance and the dot's 0.4 mm diameter.
    assert!(g.footprint.silk.iter().any(|s| s.shape
        == Primitive::Circle {
            center: [-750_000 - 150_000 - 550_000, -1_375_000],
            radius_nm: 200_000
        }));
    // The symbol: nine pins down the left from y = 10 mm, eight and EP up
    // the right; EP opposite pin 1.
    let pins: Vec<(&str, Point2, i64)> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.terminal.as_str(), p.at, p.rot_mdeg))
        .collect();
    assert_eq!(pins.first(), Some(&("1", [-7_500_000, 10_000_000], RIGHT)));
    assert_eq!(pins.get(8), Some(&("9", [-7_500_000, -10_000_000], RIGHT)));
    assert_eq!(pins.get(9), Some(&("10", [7_500_000, -7_500_000], LEFT)));
    assert_eq!(pins.get(16), Some(&("EP", [7_500_000, 10_000_000], LEFT)));
    Ok(())
}

#[test]
fn corner_lands_give_up_their_heels_to_keep_the_copper_gap() -> Result<(), Vec<Problem>> {
    // 0.28 mm terminals at 0.5 mm on a 3 × 3 mm body, no exposed pad: the
    // end lands of two sides meet at each corner, 0.3 × 0.8 mm each (the
    // width held to 0.5 − 0.2 mm). Pin 1 spans x −0.9…−0.6, y −1.7…−0.9;
    // pin 16 x −1.7…−0.9, y −0.9…−0.6: they touch at (−0.9, −0.9). Pulling
    // both heels back by d opens d on both axes, so d = 0.2 / √2 =
    // 0.141 421 mm: 2 × 70 711 nm comes off each, the toe kept.
    let g = NoLead::generate(&request(
        Scope::Package,
        &[kv("terminal_width", 280_000), flag("exposed_pad", false)],
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 16);
    let size = |k: usize| match pads.get(k).map(|p| &p.shape) {
        Some(PadShape::Rect { size }) => Some(*size),
        _ => None,
    };
    // Corner lands shortened, the middle ones not.
    assert_eq!(size(0), Some([300_000, 658_578]));
    assert_eq!(pads.first().map(|p| p.at), Some([-750_000, -1_370_711]));
    assert_eq!(size(1), Some([300_000, 800_000]));
    assert_eq!(size(15), Some([658_578, 300_000]));
    // Their copper is now at least 0.2 mm apart.
    let b = |k: usize| pads.get(k).map(land::pad_box);
    if let (Some(a), Some(c)) = (b(0), b(15)) {
        let dx = (c.min[0] - a.max[0]).max(a.min[0] - c.max[0]).max(0);
        let dy = (c.min[1] - a.max[1]).max(a.min[1] - c.max[1]).max(0);
        assert!(dx * dx + dy * dy >= 200_000 * 200_000, "{dx} {dy}");
    }
    Ok(())
}

#[test]
fn a_dfn_without_an_exposed_pad_numbers_two_rows() -> Result<(), Vec<Problem>> {
    let g = NoLead::generate(&request(
        Scope::Component,
        &[
            text("sides", "two"),
            kv("terminals", 4),
            kv("terminals_short", 0),
            kv("pitch", 500_000),
            kv("body_length", 2_000_000),
            kv("body_width", 2_000_000),
            kv("body_height", 750_000),
            kv("terminal_length", 350_000),
            kv("terminal_width", 250_000),
            flag("exposed_pad", false),
        ],
    ))?;
    let keys: Vec<&str> = g.footprint.pads.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, vec!["1", "2", "3", "4", "5", "6", "7", "8"]);
    // Pin 5 starts the +y row at its +x end; no land is trimmed.
    let p = g.footprint.pads.get(4).map(|p| (p.at, p.shape.clone()));
    assert_eq!(
        p,
        Some((
            [750_000, 825_000],
            PadShape::Rect {
                size: [300_000, 750_000]
            }
        ))
    );
    let Some(v1::Model::PartFile { asset, values }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(asset, parts::DFN);
    assert!(!values.contains_key("terminals_short") && !values.contains_key("pad_length"));
    assert_eq!(
        g.component.as_ref().map(|c| c.name.as_str()),
        Some("DFN-8 2×2 0.5")
    );
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = NoLead::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 17);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let problems = |overrides: &[(String, ParamValue)]| -> Vec<(String, Option<String>, String)> {
        NoLead::generate(&request(Scope::Component, overrides))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field, p.message))
            .collect()
    };
    assert_eq!(
        problems(&[kv("pad_length", 2_500_000), kv("pad_width", 2_500_000)]),
        vec![(
            "param.inconsistent".into(),
            Some("pad_length".into()),
            "A 2.5 × 2.5 mm exposed pad does not fit inside the terminals (2.2 × 2.2 mm).".into()
        )]
    );
    assert_eq!(
        problems(&[kv("terminal_width", 500_000)]),
        vec![(
            "param.inconsistent".into(),
            Some("terminal_width".into()),
            "Terminals 0.5 mm wide touch at a pitch of 0.5 mm.".into()
        )]
    );
    assert_eq!(
        problems(&[text("sides", "two")]),
        vec![(
            "param.inconsistent".into(),
            Some("terminals_short".into()),
            "A two-sided package has no short-side terminals (0).".into()
        )]
    );
    let codes: Vec<(String, Option<String>)> = problems(&[kv("terminals", 65)])
        .into_iter()
        .map(|(code, field, _)| (code, field))
        .collect();
    assert_eq!(
        codes,
        vec![("param.out_of_range".into(), Some("terminals".into()))]
    );
}

#[test]
fn every_preset_names_its_part_file_with_every_value() -> Result<(), String> {
    for p in description().presets() {
        let g = NoLead::generate(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        let four = p.key.contains("qfn");
        assert_eq!(
            asset,
            if four { parts::QFN_EP } else { parts::DFN_EP },
            "{}",
            p.key
        );
        let mut names: Vec<&str> = vec![
            "body_height",
            "body_length",
            "body_width",
            "edge_round",
            "frame",
            "mark_depth",
            "mark_radius",
            "pad_length",
            "pad_width",
            "pitch",
            "terminal_length",
            "terminal_width",
            "terminals",
        ];
        if four {
            names.push("terminals_short");
        }
        assert_eq!(
            values.keys().map(String::as_str).collect::<Vec<_>>(),
            names,
            "{}",
            p.key
        );
        assert!(
            values
                .values()
                .all(|v| matches!(v, ParamValue::Int(n) if *n > 0)),
            "{}",
            p.key
        );
    }
    Ok(())
}

#[test]
fn the_part_file_hashes_are_the_bundles_files() -> Result<(), String> {
    for (stem, hash) in [
        ("dfn", parts::DFN),
        ("dfn-ep", parts::DFN_EP),
        ("qfn", parts::QFN),
        ("qfn-ep", parts::QFN_EP),
    ] {
        let path = format!("{}/parts/{stem}.solidcraft", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        assert_eq!(
            sparksmith_generator_sdk::abi::hex(&sparksmith_generator_sdk::abi::sha256(&bytes)),
            hash
        );
    }
    Ok(())
}
