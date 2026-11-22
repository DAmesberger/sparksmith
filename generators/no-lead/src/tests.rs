//! Native tests: the golden hashes, hand-derived geometry and refusals.
use super::*;
use sparksmith_generator_sdk::{self as sdk, golden};

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
    // The 25 starter rows, the rows after a device's drawing, then the
    // packages whose terminals are given one by one (`ti-`).
    assert_eq!(description.presets().len(), 30);
    for p in description.presets() {
        if p.key.starts_with("no-lead-ti-") {
            continue;
        }
        for scope in [Scope::Component, Scope::Package] {
            let g = sdk::generate::<NoLead>(&request(scope, &preset(&p.key)))?;
            let pads = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pads, "{}", p.key);
            assert_eq!(g.package.terminals.len(), pads, "{}", p.key);
            // Every starter row has an exposed pad, the last terminal; the
            // LGA and the sensor have none.
            let pad = !(p.key.contains("-lga-") || p.key.contains("sensirion"));
            assert_eq!(
                g.package.terminals.last().map(String::as_str) == Some("EP"),
                pad,
                "{}",
                p.key
            );
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
    let g = sdk::generate::<NoLead>(&request(
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
    let g = sdk::generate::<NoLead>(&request(
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
    let g = sdk::generate::<NoLead>(&request(
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
    let g = sdk::generate::<NoLead>(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 17);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let problems = |overrides: &[(String, ParamValue)]| -> Vec<(String, Option<String>, String)> {
        sdk::generate::<NoLead>(&request(Scope::Component, overrides))
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
        if p.key.starts_with("no-lead-ti-") {
            continue;
        }
        let g = sdk::generate::<NoLead>(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        let lga = p.key.contains("-lga-");
        let window = p.key.contains("sensirion");
        let four = p.key.contains("qfn") || lga;
        assert_eq!(
            asset,
            match (lga, window, four) {
                (true, ..) => parts::LGA,
                (_, true, _) => parts::DFN_WINDOW,
                (_, _, true) => parts::QFN_EP,
                _ => parts::DFN_EP,
            },
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
            "pullback",
            "terminal_length",
            "terminal_width",
            "terminals",
        ];
        if lga || window {
            names.retain(|n| !n.starts_with("pad_"));
        }
        if four {
            names.push("terminals_short");
        }
        if window {
            names.push("window_diameter");
        }
        assert_eq!(
            values.keys().map(String::as_str).collect::<Vec<_>>(),
            names,
            "{}",
            p.key
        );
        // Every value positive, but the pull-back of a terminal flush with
        // the edge.
        assert!(
            values.iter().all(|(k, v)| matches!(
                v,
                ParamValue::Int(n) if *n > 0 || (k == "pullback" && *n == 0)
            )),
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
        ("sawn-terminals", parts::SAWN_TERMINALS),
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

/// The pads of `g` landing `terminal`.
fn lands_of<'a>(g: &'a Generation, terminal: &str) -> Vec<&'a Pad> {
    g.footprint
        .pads
        .iter()
        .filter(|p| p.terminal == terminal)
        .collect()
}

#[test]
fn ti_dml0010a_lands_its_comb_and_parks_unused_slots() -> Result<(), Vec<Problem>> {
    let g = sdk::generate::<NoLead>(&request(Scope::Component, &preset("no-lead-ti-dml0010a")))?;
    let terminals: Vec<&str> = g.package.terminals.iter().map(String::as_str).collect();
    assert_eq!(
        terminals,
        ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "EP"]
    );
    // Pin 8 is its numbered land and the unnumbered lug towards pin 9.
    let eight = lands_of(&g, "8");
    assert_eq!(eight.len(), 2);
    assert_eq!(eight[0].key, "8");
    assert_eq!(eight[0].number, None);
    assert_eq!(eight[1].number.as_deref(), Some(""));
    assert_eq!(eight[1].at, [812_500, 300_000]);
    // Each lead's contact is its numbered land's centre.
    for lead in &g.package.geometry.leads {
        let land = g.footprint.pads.iter().find(|p| p.key == lead.key);
        assert_eq!(land.map(|p| p.at), Some([lead.contact[0], lead.contact[1]]));
    }
    let Some(v1::Model::PartFile { asset, values }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(asset, parts::SAWN_TERMINALS);
    let int = |k: &str| match values.get(k) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    };
    // Thirteen rectangles; slot 14 on is parked beyond the −x edge.
    assert_eq!(int("terminal_13_length"), 1_100_000);
    assert_eq!(int("terminal_14_x"), int("park_x"));
    assert_eq!(int("park_x"), -101_000_000);
    // Pin 1 is at the −x, +y corner: the dot is there.
    assert!(int("mark_x") < 0 && int("mark_y") > 0);
    Ok(())
}

/// The ADI LGA-14 marks each orderable ADXL343 its data sheet's branding
/// code, and nothing for another part number.
#[test]
fn adi_lga_14_marks_each_orderable_part_its_code() -> Result<(), Vec<Problem>> {
    let g = sdk::generate::<NoLead>(&request(
        Scope::Component,
        &preset("no-lead-lga-14-3x5-0.8-adi-cc-14-1"),
    ))?;
    let marking = g.marking.ok_or_else(Vec::new)?;
    assert_eq!(marking.lines, ["{code}"]);
    let codes: Vec<(&str, &str)> = marking
        .codes
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        codes,
        [
            ("ADXL343BCCZ", "343B"),
            ("ADXL343BCCZ-RL", "343B"),
            ("ADXL343BCCZ-RL7", "343B")
        ]
    );
    Ok(())
}

#[test]
fn ti_rxb0014a_lands_its_corners_round_pins_and_via() -> Result<(), Vec<Problem>> {
    let g = sdk::generate::<NoLead>(&request(Scope::Package, &preset("no-lead-ti-rxb0014a")))?;
    assert_eq!(g.package.terminals.len(), 15);
    let one = lands_of(&g, "1");
    assert_eq!(one.len(), 2);
    assert_eq!(one[1].number.as_deref(), Some("1"));
    assert!(matches!(
        lands_of(&g, "5")[0].shape,
        PadShape::Circle {
            diameter_nm: 225_000
        }
    ));
    let pad = lands_of(&g, "15");
    assert_eq!(pad.len(), 2);
    assert_eq!(pad[1].number.as_deref(), Some(""));
    assert!(pad[1].hole.is_some());
    let Some(v1::Model::PartFile { values, .. }) = &g.model else {
        return Err(Vec::new());
    };
    // 37 rectangles, the necks lifted 0.1 mm; slot 38 on parked.
    assert_eq!(
        values.get("terminal_2_lift"),
        Some(&ParamValue::Int(100_000))
    );
    assert_eq!(values.get("terminal_1_lift"), Some(&ParamValue::Int(0)));
    assert_ne!(values.get("terminal_37_x"), values.get("park_x"));
    assert_eq!(values.get("terminal_38_x"), values.get("park_x"));
    Ok(())
}

#[test]
fn given_terminals_refuse_what_cannot_land() {
    let list = |k: &str, v: ParamValue| (k.to_string(), ParamValue::List(vec![v]));
    let rect = |n: &str, x: i64| -> Vec<(String, ParamValue)> {
        vec![
            list("terminal_numbers", ParamValue::Text(n.into())),
            list("terminal_x", ParamValue::Int(x)),
            list("terminal_y", ParamValue::Int(0)),
            list("terminal_size_x", ParamValue::Int(500_000)),
            list("terminal_size_y", ParamValue::Int(500_000)),
        ]
    };
    let refused = |o: Vec<(String, ParamValue)>| -> Vec<String> {
        sdk::generate::<NoLead>(&request(Scope::Package, &o))
            .err()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| p.field)
            .collect()
    };
    // Beyond the 3 mm body.
    assert_eq!(refused(rect("1", 1_400_000)), ["terminal_x"]);
    // A land naming no terminal (and so terminal 1 has no land).
    let mut stray = rect("1", 0);
    stray.extend([
        list("given_land_numbers", ParamValue::Text("2".into())),
        list("given_land_x", ParamValue::Int(0)),
        list("given_land_y", ParamValue::Int(0)),
        list("given_land_size_x", ParamValue::Int(500_000)),
        list("given_land_size_y", ParamValue::Int(500_000)),
    ]);
    assert_eq!(refused(stray), ["given_land_numbers", "given_land_numbers"]);
    // Estimated lands: the rectangle grown 0.025 mm a side.
    let g = sdk::generate::<NoLead>(&request(Scope::Package, &rect("1", 0)));
    assert_eq!(
        g.map(|g| g.footprint.pads[0].shape.clone()).ok(),
        Some(PadShape::Rect {
            size: [550_000, 550_000]
        })
    );
}

/// Each terminal's name and default direction, by key.
fn named(g: &Generation) -> Vec<(String, String, Direction)> {
    g.component
        .iter()
        .flat_map(|c| &c.terminals)
        .map(|t| (t.key.clone(), t.name.clone(), t.direction))
        .collect()
}

/// The part presets carry their datasheets' pin tables; a package preset
/// keeps its pins numbered.
#[test]
fn part_presets_name_and_type_their_pins() -> Result<(), Vec<Problem>> {
    use Direction::{Input, Output, Passive, PowerInput, PowerOutput};
    let row = |k: &str, n: &str, d| (k.to_string(), n.to_string(), d);
    let g = sdk::generate::<NoLead>(&request(Scope::Component, &preset("no-lead-ti-dml0010a")))?;
    assert_eq!(
        named(&g),
        [
            row("1", "CT", Passive),
            row("2", "NC", Passive),
            row("3", "VIN", PowerInput),
            row("4", "VBIAS", PowerInput),
            row("5", "ON", Input),
            row("6", "GND", PowerInput),
            row("7", "PG", Output),
            row("8", "VOUT", PowerOutput),
            row("9", "VOUT", PowerOutput),
            row("10", "VOUT", PowerOutput),
            row("EP", "VIN", PowerInput),
        ]
    );
    let g = sdk::generate::<NoLead>(&request(Scope::Component, &preset("no-lead-ti-rxb0014a")))?;
    let rows = named(&g);
    assert_eq!(rows.len(), 15);
    assert_eq!(rows.first(), Some(&row("1", "EN/UVLO", Input)));
    assert_eq!(rows.get(9), Some(&row("10", "VCC", PowerOutput)));
    assert_eq!(rows.last(), Some(&row("15", "GND/DAP", PowerInput)));
    let g = sdk::generate::<NoLead>(&request(
        Scope::Component,
        &preset("no-lead-dfn-4-1.5x1.5-0.8-sensirion-sht4x"),
    ))?;
    assert_eq!(
        named(&g),
        [
            row("1", "SDA", Input),
            row("2", "SCL", Input),
            row("3", "VDD", PowerInput),
            row("4", "VSS", PowerInput),
        ]
    );
    let sda = g.component.iter().flat_map(|c| &c.terminals).next();
    assert_eq!(sda.map(|t| t.alternates.as_slice()), Some(&[Output][..]));
    let g = sdk::generate::<NoLead>(&request(
        Scope::Component,
        &preset("no-lead-qfn-16-3x3-0.5"),
    ))?;
    assert!(named(&g).iter().all(|(k, n, d)| k == n && *d == Passive));
    Ok(())
}
