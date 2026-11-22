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

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Tabbed>(include_str!("../golden.json"))?;
    assert!(checked >= 3);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    // The ten starter rows and the WS2812B.
    assert_eq!(description.presets().len(), 11);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = sdk::generate::<Tabbed>(&request(scope, &preset(&p.key)))?;
            let pins = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pins, "{}", p.key);
            assert_eq!(g.package.terminals.len(), pins, "{}", p.key);
        }
    }
    Ok(())
}

#[test]
fn sot23_5_leads_run_counter_clockwise_with_their_own_spacing() -> Result<(), Vec<Problem>> {
    // Three −y leads at 0.95 mm, two +y leads two grid positions apart;
    // the feet's centres (2.8 − 0.4) / 2 = 1.2 mm off the centre line,
    // their lands 0.1 mm further out.
    let g = sdk::generate::<Tabbed>(&request(Scope::Component, &preset("tabbed-sot-23-5")))?;
    let at: Vec<Point2> = g.footprint.pads.iter().map(|p| p.at).collect();
    assert_eq!(
        at,
        vec![
            [-950_000, -1_300_000],
            [0, -1_300_000],
            [950_000, -1_300_000],
            [950_000, 1_300_000],
            [-950_000, 1_300_000],
        ]
    );
    let contacts: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(contacts.first(), Some(&[-950_000, -1_200_000, 0]));
    assert!(g.footprint.pads.iter().all(|p| p.shape
        == PadShape::Rect {
            size: [600_000, 1_000_000]
        }));
    assert_eq!(
        g.package.geometry.envelope_nm,
        [2_900_000, 2_800_000, 1_175_000]
    );
    // The symbol: three pins down the left from 1, two up the right.
    let pins: Vec<(String, Point2, i64)> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.terminal.clone(), p.at, p.rot_mdeg))
        .collect();
    assert_eq!(
        pins,
        vec![
            ("1".into(), [-7_500_000, 2_500_000], RIGHT),
            ("2".into(), [-7_500_000, 0], RIGHT),
            ("3".into(), [-7_500_000, -2_500_000], RIGHT),
            ("4".into(), [7_500_000, 0], LEFT),
            ("5".into(), [7_500_000, 2_500_000], LEFT),
        ]
    );
    Ok(())
}

#[test]
fn a_flange_sits_on_its_solderable_area_and_widens_nothing() -> Result<(), Vec<Problem>> {
    // DPAK: the flange's outer edge 6.1 / 2 + 1 = 4.05 mm out; its 5.3 mm
    // contact centred 2.65 mm inside it; its land 0.2 wider, 0.4 longer,
    // not moved out.
    let g = sdk::generate::<Tabbed>(&request(Scope::Package, &preset("tabbed-dpak")))?;
    let tab = g.footprint.pads.get(2).map(|p| (p.at, p.shape.clone()));
    assert_eq!(
        tab,
        Some((
            [0, 1_400_000],
            PadShape::Rect {
                size: [5_500_000, 5_700_000]
            }
        ))
    );
    // The leads two grid positions apart: ±2.285 mm.
    assert_eq!(g.footprint.pads.first().map(|p| p.at[0]), Some(-2_285_000));
    // The envelope is the span (centred), wider than body and flange.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [6_600_000, 11_700_000, 2_300_000]
    );
    let Some(v1::Model::PartFile { asset, .. }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(asset, parts::TABBED_FLANGE);
    Ok(())
}

#[test]
fn a_tab_lead_is_the_last_terminal_its_land_moved_out() -> Result<(), Vec<Problem>> {
    // SOT-223: the tab's foot centre 7 / 2 − 0.9 / 2 = 3.05 mm out, its
    // land 0.1 mm further.
    let g = sdk::generate::<Tabbed>(&request(Scope::Component, &preset("tabbed-sot-223")))?;
    assert_eq!(g.footprint.pads.len(), 4);
    assert_eq!(
        g.footprint.pads.get(3).map(|p| (p.at, p.shape.clone())),
        Some((
            [0, 3_150_000],
            PadShape::Rect {
                size: [3_200_000, 1_500_000]
            }
        ))
    );
    assert_eq!(
        g.package.geometry.leads.get(3).map(|l| l.contact),
        Some([0, 3_050_000, 0])
    );
    Ok(())
}

#[test]
fn a_given_land_pattern_replaces_the_estimates() -> Result<(), Vec<Problem>> {
    // TI's DBV0005A land pattern (5 × 0.6 × 1.1 mm, rows 2.6 mm apart
    // centre to centre) on the SOT-23-5 preset's leads.
    let mut given = preset("tabbed-sot-23-5");
    given.push(("lands".into(), ParamValue::Text("explicit".into())));
    given.extend([
        kv("land_width", 600_000),
        kv("land_length", 1_100_000),
        kv("land_span", 2_600_000),
    ]);
    let g = sdk::generate::<Tabbed>(&request(Scope::Component, &given))?;
    let lands: Vec<([i64; 2], PadShape)> = g
        .footprint
        .pads
        .iter()
        .map(|p| (p.at, p.shape.clone()))
        .collect();
    let size = PadShape::Rect {
        size: [600_000, 1_100_000],
    };
    assert_eq!(
        lands,
        vec![
            ([-950_000, -1_300_000], size.clone()),
            ([0, -1_300_000], size.clone()),
            ([950_000, -1_300_000], size.clone()),
            ([950_000, 1_300_000], size.clone()),
            ([-950_000, 1_300_000], size),
        ]
    );
    // The leads and the model are the estimate's: only the lands moved.
    let estimated =
        sdk::generate::<Tabbed>(&request(Scope::Component, &preset("tabbed-sot-23-5")))?;
    assert_eq!(g.package, estimated.package);
    assert_eq!(g.model, estimated.model);
    assert!(g.notes.iter().any(|n| n.contains("given land pattern")));

    // Lands that touch across the rows or along a row are refused.
    for (key, value, field) in [
        ("land_span", 1_000_000, "land_span"),
        ("land_width", 1_000_000, "land_width"),
    ] {
        let mut bad = given.clone();
        bad.push(kv(key, value));
        let refused = sdk::generate::<Tabbed>(&request(Scope::Package, &bad)).err();
        assert!(
            refused.is_some_and(|p| p.iter().any(|p| p.field.as_deref() == Some(field))),
            "{key}"
        );
    }
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = sdk::generate::<Tabbed>(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 3);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let found = |r: Result<Generation, Vec<Problem>>| -> Vec<(String, Option<String>, String)> {
        r.err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field, p.message))
            .collect()
    };
    // A tab with +y leads.
    let tab = (String::from("tab"), ParamValue::Text("lead".into()));
    assert_eq!(
        found(sdk::generate::<Tabbed>(&request(
            Scope::Component,
            &[tab, kv("body_length", 6_500_000), kv("span", 7_000_000)]
        ))),
        vec![(
            "param.inconsistent".into(),
            Some("leads_n".into()),
            "A tab takes the whole +y side: the +y side has no other leads (0).".into()
        )]
    );
    // Leads too close.
    let codes = found(sdk::generate::<Tabbed>(&request(
        Scope::Component,
        &[kv("step_s", 1), kv("pitch", 300_000)],
    )));
    assert!(
        codes
            .iter()
            .any(|(_, f, m)| f.as_deref() == Some("step_s") && m.contains("must not touch")),
        "{codes:?}"
    );
    // A row as long as the body leaves no room for the corner rounds.
    let codes = found(sdk::generate::<Tabbed>(&request(
        Scope::Component,
        &[kv("body_length", 2_300_000)],
    )));
    assert_eq!(
        codes.first().map(|(_, f, _)| f.clone()),
        Some(Some("body_length".into())),
        "{codes:?}"
    );
    // Out of range.
    let codes = found(sdk::generate::<Tabbed>(&request(
        Scope::Component,
        &[kv("leads_s", 17)],
    )));
    assert_eq!(
        codes
            .iter()
            .map(|(c, f, _)| (c.as_str(), f.as_deref()))
            .collect::<Vec<_>>(),
        vec![("param.out_of_range", Some("leads_s"))]
    );
}

#[test]
fn every_preset_names_its_part_file_with_every_value() -> Result<(), String> {
    for p in description().presets() {
        let g = sdk::generate::<Tabbed>(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        let names: Vec<&str> = values.keys().map(String::as_str).collect();
        let mut want = vec![
            "body_height",
            "body_length",
            "body_width",
            "corner_round",
            "edge_round",
            "foot_length",
            "lead_thickness",
            "lead_width",
            "pitch",
            "row_s_leads",
            "row_s_step",
            "span",
            "standoff",
        ];
        let tab = p.params.get("tab");
        if p.params.get("construction") == Some(&ParamValue::Text("led_plcc".into())) {
            assert_eq!(asset, parts::TABBED_PLCC_LED, "{}", p.key);
            want.extend(["row_n_leads", "row_n_step"]);
        } else if tab == Some(&ParamValue::Text("none".into())) {
            assert_eq!(asset, parts::TABBED_ROWS, "{}", p.key);
            want.extend(["row_n_leads", "row_n_step"]);
        } else if tab == Some(&ParamValue::Text("lead".into())) {
            assert_eq!(asset, parts::TABBED_TAB_LEAD, "{}", p.key);
            want.extend(["tab_length", "tab_width"]);
        } else {
            assert_eq!(asset, parts::TABBED_FLANGE, "{}", p.key);
            want.extend(["flange_protrusion", "tab_length", "tab_width"]);
        }
        want.sort_unstable();
        assert_eq!(names, want, "{}", p.key);
        assert!(
            values
                .iter()
                .all(|(k, v)| matches!(v, ParamValue::Int(n) if *n > 0 || k == "standoff")),
            "{}",
            p.key
        );
    }
    Ok(())
}

#[test]
fn the_part_file_hashes_are_the_bundles_files() -> Result<(), String> {
    for (stem, hash) in [
        ("tabbed-rows", parts::TABBED_ROWS),
        ("tabbed-tab-lead", parts::TABBED_TAB_LEAD),
        ("tabbed-flange", parts::TABBED_FLANGE),
        ("tabbed-plcc-led", parts::TABBED_PLCC_LED),
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

/// The WS2812B preset carries its datasheet's pin table.
#[test]
fn the_ws2812b_names_and_types_its_pins() -> Result<(), Vec<Problem>> {
    use Direction::{Input, Output, PowerInput};
    let g = sdk::generate::<Tabbed>(&request(
        Scope::Component,
        &preset("tabbed-plcc4-5050-ws2812b"),
    ))?;
    let named: Vec<(&str, &str, Direction)> = g
        .component
        .iter()
        .flat_map(|c| &c.terminals)
        .map(|t| (t.key.as_str(), t.name.as_str(), t.direction))
        .collect();
    assert_eq!(
        named,
        [
            ("1", "VDD", PowerInput),
            ("2", "DOUT", Output),
            ("3", "VSS", PowerInput),
            ("4", "DIN", Input),
        ]
    );
    Ok(())
}
