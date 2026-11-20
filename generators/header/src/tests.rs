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

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Header>(include_str!("../golden.json"))?;
    assert!(checked >= 3);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 54);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Header::generate(&request(scope, &preset(&p.key)))?;
            // Every pad but a bottom-entry socket's mating-pin holes.
            let pins = g
                .footprint
                .pads
                .iter()
                .filter(|p| !p.terminal.is_empty())
                .count();
            assert_eq!(g.package.geometry.leads.len(), pins, "{}", p.key);
            if let Some(c) = &g.component {
                assert_eq!(c.name, p.name, "{}", p.key);
            }
        }
    }
    Ok(())
}

#[test]
fn header_2x5_lands_number_across_the_rows() -> Result<(), Vec<Problem>> {
    // 0.64 mm posts: diagonal 0.905 097 mm; drill 1.005 097 mm; land
    // 0.905 097 + 0.8 = 1.705 097 mm, under 2.54 − 0.25 = 2.29 mm.
    let g = Header::generate(&request(Scope::Component, &preset("header-2x5-2.54")))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 10);
    assert_eq!(
        pads.first().map(|p| (p.at, p.shape.clone())),
        Some((
            [-5_080_000, -1_270_000],
            PadShape::Rect {
                size: [1_705_097, 1_705_097]
            }
        ))
    );
    // Pin 2 beside pin 1 in the next row; pin 3 one pitch along.
    assert_eq!(pads.get(1).map(|p| p.at), Some([-5_080_000, 1_270_000]));
    assert_eq!(pads.get(2).map(|p| p.at), Some([-2_540_000, -1_270_000]));
    assert_eq!(
        pads.get(9).map(|p| (p.at, p.shape.clone())),
        Some((
            [5_080_000, 1_270_000],
            PadShape::Circle {
                diameter_nm: 1_705_097
            }
        ))
    );
    assert!(pads.iter().all(|p| p.hole
        == Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: 1_005_097
            },
            plated: true,
            offset: None,
        })));
    assert_eq!(
        g.package.geometry.envelope_nm,
        [12_700_000, 5_080_000, 8_540_000]
    );
    // The symbol: ten pins down the left; pins 1–9 on 2.5 mm stubs, pin 10
    // on a 5 mm stub (two digits need 2 × 0.9 + 0.8 = 2.6 mm).
    let pins: Vec<(Point2, i64)> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.at, p.length_nm))
        .collect();
    assert_eq!(pins.first(), Some(&([-5_000_000, 0], 2_500_000)));
    assert_eq!(pins.get(9), Some(&([-7_500_000, -22_500_000], 5_000_000)));
    // More than two pins: the pin-1 dot beyond pad 1's −x edge, past its
    // 0.05 mm mask, 0.1 mm clearance and the dot's 0.4 mm diameter.
    assert!(g.footprint.silk.iter().any(|s| s.shape
        == Primitive::Circle {
            center: [-5_080_000 - 852_548 - 550_000, -1_270_000],
            radius_nm: 200_000
        }));
    Ok(())
}

#[test]
fn tight_pitch_lands_keep_the_gap_and_a_ring() -> Result<(), Vec<Problem>> {
    // 1.27 mm, 0.4 mm posts: diagonal 0.565 685 mm, drill 0.665 685 mm;
    // 1.365 685 mm wanted, 1.27 − 0.25 = 1.02 mm allowed, above the
    // 0.865 685 mm least.
    let g = Header::generate(&request(Scope::Package, &preset("header-2x5-1.27")))?;
    assert_eq!(
        g.footprint.pads.get(1).map(|p| p.shape.clone()),
        Some(PadShape::Circle {
            diameter_nm: 1_020_000
        })
    );
    // A single row of two: no pin-1 dot.
    let g = Header::generate(&request(Scope::Package, &preset("header-1x2-2.54")))?;
    assert!(
        !g.footprint
            .silk
            .iter()
            .any(|s| matches!(s.shape, Primitive::Circle { .. }))
    );
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = Header::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 10);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let refused = Header::generate(&request(
        Scope::Component,
        &[kv("pitch", 1_270_000), kv("pin_side", 1_500_000)],
    ));
    let problems = refused.err().unwrap_or_default();
    let found: Vec<(&str, Option<&str>, &str)> = problems
        .iter()
        .map(|p| (p.code.as_str(), p.field.as_deref(), p.message.as_str()))
        .collect();
    assert_eq!(
        found,
        vec![(
            "param.inconsistent",
            Some("pitch"),
            "Pins of 1.5 mm side do not fit a pitch of 1.27 mm."
        )]
    );
    let codes = |r: Result<Generation, Vec<Problem>>| -> Vec<(String, Option<String>)> {
        r.err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field))
            .collect()
    };
    assert_eq!(
        codes(Header::generate(&request(
            Scope::Component,
            &[kv("rows", 5)]
        ))),
        vec![("param.out_of_range".into(), Some("rows".into()))]
    );
}

#[test]
fn every_preset_names_its_part_file_with_every_value() -> Result<(), String> {
    for p in description().presets() {
        let g = Header::generate(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        let names: Vec<&str> = values.keys().map(String::as_str).collect();
        let style = p.params.get("style");
        let socket = |extra: &[&'static str]| {
            let mut names = vec![
                "body_height",
                "cavity",
                "cavity_top",
                "corner_round",
                "entry",
                "entry_depth",
                "lead_in",
                "leaf",
                "leaf_gap",
                "leaf_width",
                "pin_side",
                "pins",
                "pitch",
                "row_pitch",
                "rows",
            ];
            names.extend_from_slice(extra);
            names.sort_unstable();
            names
        };
        if style == Some(&ParamValue::Text("socket".into())) {
            assert_eq!(asset, parts::PIN_SOCKET, "{}", p.key);
            assert_eq!(names, socket(&["floor_z", "pin_below"]), "{}", p.key);
        } else if style == Some(&ParamValue::Text("socket_smd_bottom".into())) {
            assert_eq!(asset, parts::SOCKET_SMD_BOTTOM, "{}", p.key);
            assert_eq!(
                names,
                socket(&[
                    "cavity_bottom",
                    "standoff",
                    "tail_bend",
                    "tail_sheet",
                    "tail_span"
                ]),
                "{}",
                p.key
            );
        } else {
            assert_eq!(asset, parts::PIN_HEADER, "{}", p.key);
            assert_eq!(
                names,
                vec![
                    "body_height",
                    "corner_round",
                    "edge_round",
                    "pin_above",
                    "pin_below",
                    "pin_side",
                    "pins",
                    "pitch",
                    "row_pitch",
                    "rows"
                ],
                "{}",
                p.key
            );
        }
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
        ("pin-header", parts::PIN_HEADER),
        ("pin-socket", parts::PIN_SOCKET),
        ("socket-smd-bottom", parts::SOCKET_SMD_BOTTOM),
    ] {
        let path = format!("{}/parts/{stem}.solidcraft", env!("CARGO_MANIFEST_DIR"));
        let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
        assert_eq!(
            sparksmith_generator_sdk::abi::hex(&sparksmith_generator_sdk::abi::sha256(&bytes)),
            hash,
            "{stem}"
        );
    }
    Ok(())
}

#[test]
fn a_socket_strip_has_oval_lands_across_the_row() -> Result<(), Vec<Problem>> {
    // 0.64 mm tails: drill 1.005 097 mm, land 1.705 097 mm wide; 2.8 mm
    // across the row, pin 1 a rectangle.
    let g = Header::generate(&request(Scope::Component, &preset("socket-1x10-2.54")))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 10);
    assert_eq!(
        pads.first().map(|p| (p.at, p.shape.clone())),
        Some((
            [-11_430_000, 0],
            PadShape::Rect {
                size: [1_705_097, 2_800_000]
            }
        ))
    );
    assert_eq!(
        pads.get(9).map(|p| (p.at, p.shape.clone())),
        Some((
            [11_430_000, 0],
            PadShape::Oval {
                size: [1_705_097, 2_800_000]
            }
        ))
    );
    assert_eq!(
        g.component.map(|c| c.name),
        Some("Pin socket 1×10 2.54 mm".into())
    );
    // No posts above: the envelope is the housing.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [25_400_000, 2_540_000, 8_500_000]
    );
    Ok(())
}

#[test]
fn a_bottom_entry_socket_has_lands_outside_and_a_hole_per_position() -> Result<(), Vec<Problem>> {
    let g = Header::generate(&request(
        Scope::Component,
        &preset("socket-smd-bottom-2x3-2.54"),
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 12);
    // Pin 1 at −x, −y outside its row; pin 2 across from it.
    let land = PadShape::Rect {
        size: [900_000, 1_800_000],
    };
    assert_eq!(
        pads.first()
            .map(|p| (p.at, p.shape.clone(), p.hole.clone())),
        Some(([-2_540_000, -3_200_000], land.clone(), None))
    );
    assert_eq!(
        pads.get(1).map(|p| (p.at, p.shape.clone())),
        Some(([-2_540_000, 3_200_000], land))
    );
    // The mating pins' holes: unnumbered, non-plated, landing no terminal.
    let holes: Vec<&Pad> = pads.iter().filter(|p| p.hole.is_some()).collect();
    assert_eq!(holes.len(), 6);
    assert_eq!(holes.first().map(|p| p.at), Some([-2_540_000, -1_270_000]));
    assert!(holes.iter().all(|p| p.terminal.is_empty()
        && p.number.as_deref() == Some("")
        && p.hole
            == Some(Hole {
                shape: HoleShape::Round {
                    diameter_nm: 1_100_000
                },
                plated: false,
                offset: None,
            })));
    // A lead per pin, on its land.
    let leads: Vec<(&str, [i64; 3])> = g
        .package
        .geometry
        .leads
        .iter()
        .map(|l| (l.key.as_str(), l.contact))
        .collect();
    assert_eq!(leads.len(), 6);
    assert_eq!(leads.first(), Some(&("1", [-2_540_000, -3_200_000, 0])));
    // The tails on the board are part of the envelope.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [7_620_000, 7_200_000, 3_700_000]
    );
    Ok(())
}

#[test]
fn a_bottom_entry_socket_refuses_one_row_and_lands_over_the_holes() {
    let codes = |overrides: &[(String, ParamValue)]| -> Vec<(String, Option<String>)> {
        let mut all = preset("socket-smd-bottom-2x3-2.54");
        all.extend_from_slice(overrides);
        Header::generate(&request(Scope::Package, &all))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field))
            .collect()
    };
    assert_eq!(
        codes(&[kv("rows", 1)]),
        vec![("param.inconsistent".into(), Some("rows".into()))]
    );
    assert_eq!(
        codes(&[kv("land_span", 4_000_000)]),
        vec![("param.inconsistent".into(), Some("land_span".into()))]
    );
    assert_eq!(
        codes(&[kv("land_length", 0)]),
        vec![("param.inconsistent".into(), Some("land_length".into()))]
    );
}
