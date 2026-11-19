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
    assert!(found.is_some(), "no preset {key}");
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

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Tht>(include_str!("../golden.json"))?;
    assert!(checked >= 6);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 22);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Tht::generate(&request(scope, &preset(&p.key)))?;
            let pins = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pins, "{}", p.key);
            assert!(g.model.is_some(), "{}", p.key);
        }
    }
    Ok(())
}

#[test]
fn dip_8_runs_counter_clockwise_with_a_square_pin_one() -> Result<(), Vec<Problem>> {
    // 0.46 mm strips 0.25 mm thick: section √(0.46² + 0.25²) = 0.523546 mm,
    // drill 0.823546 → 0.85 mm; land 0.85 + 0.6 = 1.45 mm, under
    // 2.54 − 0.25 = 2.29 mm.
    let g = Tht::generate(&request(Scope::Component, &preset("tht-dip-8-300-mil")))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 8);
    let at: Vec<Point2> = pads.iter().map(|p| p.at).collect();
    assert_eq!(
        at,
        vec![
            [-3_810_000, -3_810_000],
            [-1_270_000, -3_810_000],
            [1_270_000, -3_810_000],
            [3_810_000, -3_810_000],
            [3_810_000, 3_810_000],
            [1_270_000, 3_810_000],
            [-1_270_000, 3_810_000],
            [-3_810_000, 3_810_000],
        ]
    );
    assert_eq!(
        pads.first().map(|p| p.shape.clone()),
        Some(PadShape::RoundRect {
            size: [1_450_000, 1_450_000],
            radius_nm: 362_500
        })
    );
    assert!(pads.iter().skip(1).all(|p| p.shape
        == PadShape::Circle {
            diameter_nm: 1_450_000
        }));
    assert!(pads.iter().all(|p| p.hole
        == Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: 850_000
            },
            plated: true,
            offset: None,
        })));
    // Envelope: the body, the rows' outer faces (7.62 + 0.25) and the
    // estimated 0.5 mm standoff under the 3.3 mm body.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [9_270_000, 7_870_000, 3_800_000]
    );
    // The box symbol: pins 1–4 down the left, 5–8 up the right.
    let pins: Vec<(String, Point2, i64)> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.terminal.clone(), p.at, p.rot_mdeg))
        .collect();
    assert_eq!(
        pins.first(),
        Some(&("1".into(), [-7_500_000, 2_500_000], RIGHT))
    );
    assert_eq!(
        pins.get(4),
        Some(&("5".into(), [7_500_000, -5_000_000], LEFT))
    );
    assert_eq!(
        pins.get(7),
        Some(&("8".into(), [7_500_000, 2_500_000], LEFT))
    );
    assert_eq!(g.component.as_ref().map(|c| c.prefix.as_str()), Some("U"));
    Ok(())
}

#[test]
fn axial_diodes_name_the_cathode_and_mark_it() -> Result<(), Vec<Problem>> {
    let g = Tht::generate(&request(Scope::Component, &preset("tht-axial-do-35-7.62")))?;
    let names: Vec<&str> = g
        .component
        .iter()
        .flat_map(|c| &c.terminals)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(names, vec!["K", "A"]);
    assert_eq!(g.component.as_ref().map(|c| c.prefix.as_str()), Some("D"));
    // Round lands at ±3.81 mm; the polarised part's pin-1 dot past pad 1.
    let pads = &g.footprint.pads;
    assert_eq!(pads.first().map(|p| p.at), Some([-3_810_000, 0]));
    assert_eq!(
        pads.first().map(|p| p.shape.clone()),
        Some(PadShape::Circle {
            diameter_nm: 1_400_000
        })
    );
    assert!(g.footprint.silk.iter().any(|s| matches!(
        s.shape,
        Primitive::Circle {
            radius_nm: 200_000,
            ..
        }
    )));
    // A resistor is not polarised: no dot.
    let g = Tht::generate(&request(
        Scope::Component,
        &preset("tht-axial-resistor-0.25-w-10.16"),
    ))?;
    assert!(
        !g.footprint
            .silk
            .iter()
            .any(|s| matches!(s.shape, Primitive::Circle { .. }))
    );
    Ok(())
}

#[test]
fn formed_to_92_lands_follow_the_board_pitch() -> Result<(), Vec<Problem>> {
    // Square 0.45 mm leads: diagonal 0.636396 mm, drill 0.95 mm; land
    // 1.55 mm (2.54 mm pitch, under 2.29 mm).
    let g = Tht::generate(&request(Scope::Package, &preset("tht-to-92-3-2.54-formed")))?;
    let pads = &g.footprint.pads;
    assert_eq!(
        pads.iter().map(|p| p.at).collect::<Vec<_>>(),
        vec![[-2_540_000, 0], [0, 0], [2_540_000, 0]]
    );
    assert_eq!(
        pads.get(1).map(|p| p.shape.clone()),
        Some(PadShape::Circle {
            diameter_nm: 1_550_000
        })
    );
    let Some(v1::Model::PartFile { asset, values }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(asset, parts::THT_TO_FORMED);
    assert_eq!(values.get("body_pitch"), Some(&ParamValue::Int(1_270_000)));
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = Tht::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 8);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let found = |overrides: &[(String, ParamValue)]| -> Vec<(String, Option<String>)> {
        Tht::generate(&request(Scope::Component, overrides))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field))
            .collect()
    };
    assert_eq!(
        found(&[kv("pins", 7)]),
        vec![("param.inconsistent".into(), Some("pins".into()))]
    );
    // An axial part whose leads cannot bend down outside its body.
    let mut axial = preset("tht-axial-do-35-7.62");
    axial.push(kv("lead_pitch", 5_000_000));
    assert_eq!(
        found(&axial),
        vec![("param.inconsistent".into(), Some("lead_pitch".into()))]
    );
    // Formed TO leads are made for three leads only.
    let mut to = preset("tht-to-92-3-2.54-formed");
    to.push(kv("pins", 2));
    to.push(kv("standoff", 1_000_000));
    to.push(text("standoff_basis", "given"));
    assert_eq!(
        found(&to),
        vec![("param.inconsistent".into(), Some("body_pitch".into()))]
    );
    // A diode symbol on a DIP.
    assert_eq!(
        found(&[text("symbol", "diode")]),
        vec![("param.inconsistent".into(), Some("symbol".into()))]
    );
}

#[test]
fn every_preset_names_a_bundled_part_file_with_its_values() -> Result<(), String> {
    let files = [
        parts::THT_AXIAL,
        parts::THT_RADIAL,
        parts::THT_RADIAL_FORMED,
        parts::THT_DIP,
        parts::THT_TO,
        parts::THT_TO_FORMED,
    ];
    for p in description().presets() {
        let g = Tht::generate(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        assert!(files.contains(&asset.as_str()), "{}", p.key);
        for shared in [
            "lead_pitch",
            "lead_diameter",
            "lead_below",
            "standoff",
            "root",
        ] {
            assert!(values.contains_key(shared), "{}: {shared}", p.key);
        }
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
        ("tht-axial", parts::THT_AXIAL),
        ("tht-radial", parts::THT_RADIAL),
        ("tht-radial-formed", parts::THT_RADIAL_FORMED),
        ("tht-dip", parts::THT_DIP),
        ("tht-to", parts::THT_TO),
        ("tht-to-formed", parts::THT_TO_FORMED),
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
