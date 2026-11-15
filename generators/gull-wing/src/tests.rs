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
    let checked = golden::check::<GullWing>(include_str!("../golden.json"))?;
    assert!(checked >= 5);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 48);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = GullWing::generate(&request(scope, &preset(&p.key)))?;
            let pins = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pins, "{}", p.key);
            assert_eq!(g.package.terminals.len(), pins, "{}", p.key);
        }
    }
    Ok(())
}

#[test]
fn soic8_lands_sit_on_the_feet_counter_clockwise() -> Result<(), Vec<Problem>> {
    // Feet centred (6.0 − 0.835) / 2 = 2.5825 mm out; lands 0.41 + 0.2 by
    // 0.835 + 0.6 mm.
    let g = GullWing::generate(&request(Scope::Component, &preset("gull-wing-soic-8-3.9")))?;
    let pads = &g.footprint.pads;
    let at: Vec<Point2> = pads.iter().map(|p| p.at).collect();
    assert_eq!(
        at,
        vec![
            [-1_905_000, -2_582_500],
            [-635_000, -2_582_500],
            [635_000, -2_582_500],
            [1_905_000, -2_582_500],
            [1_905_000, 2_582_500],
            [635_000, 2_582_500],
            [-635_000, 2_582_500],
            [-1_905_000, 2_582_500],
        ]
    );
    assert!(pads.iter().all(|p| p.shape
        == PadShape::Rect {
            size: [610_000, 1_435_000]
        }
        && p.hole.is_none()));
    // The envelope: the body's length, the span across y.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [4_900_000, 6_000_000, 1_625_000]
    );
    // The IC box: pins 1–4 down the left, 5–8 up the right.
    let pins: Vec<(Point2, i64)> = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.pins)
        .map(|p| (p.at, p.rot_mdeg))
        .collect();
    assert_eq!(pins.first(), Some(&([-7_500_000, 2_500_000], RIGHT)));
    assert_eq!(pins.get(3), Some(&([-7_500_000, -5_000_000], RIGHT)));
    assert_eq!(pins.get(4), Some(&([7_500_000, -5_000_000], LEFT)));
    assert_eq!(pins.get(7), Some(&([7_500_000, 2_500_000], LEFT)));
    // The pin-1 dot beyond pad 1's −x edge: half the land, 0.05 mm mask,
    // 0.1 mm clearance and the dot's 0.4 mm diameter.
    assert!(g.footprint.silk.iter().any(|s| s.shape
        == Primitive::Circle {
            center: [-1_905_000 - 305_000 - 550_000, -2_582_500],
            radius_nm: 200_000
        }));
    Ok(())
}

#[test]
fn quad_leads_run_round_four_sides_with_lands_turned_on_the_short_sides() -> Result<(), Vec<Problem>>
{
    let g = GullWing::generate(&request(
        Scope::Package,
        &preset("gull-wing-lqfp-128-14x20-0.5"),
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 128);
    // 38 along −y, then 26 up +x (span across x 22 mm), 38 along +y, 26
    // down −x.
    let at = |k: usize| pads.get(k - 1).map(|p| (p.at, p.shape.clone()));
    let long = PadShape::Rect {
        size: [420_000, 1_200_000],
    };
    let short = PadShape::Rect {
        size: [1_200_000, 420_000],
    };
    assert_eq!(at(1), Some(([-9_250_000, -7_700_000], long.clone())));
    assert_eq!(at(39), Some(([10_700_000, -6_250_000], short.clone())));
    assert_eq!(at(65), Some(([9_250_000, 7_700_000], long)));
    assert_eq!(at(128), Some(([-10_700_000, -6_250_000], short)));
    assert_eq!(
        g.package.geometry.envelope_nm,
        [22_000_000, 16_000_000, 1_500_000]
    );
    let Some(v1::Model::PartFile { asset, values }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(asset, parts::GULL_WING_FOUR_SIDED);
    assert_eq!(values.get("leads_short"), Some(&ParamValue::Int(26)));
    Ok(())
}

#[test]
fn the_land_width_follows_the_project_rule() -> Result<(), Vec<Problem>> {
    let mut wide = preset("gull-wing-tssop-8-4.4");
    wide.push(kv("land_width_extra", 300_000));
    let g = GullWing::generate(&request(Scope::Package, &wide))?;
    assert_eq!(
        g.footprint.pads.first().map(|p| p.shape.clone()),
        Some(PadShape::Rect {
            size: [550_000, 1_200_000]
        })
    );
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = GullWing::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 8);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let fields = |overrides: &[(String, ParamValue)]| -> Vec<(String, Option<String>, String)> {
        GullWing::generate(&request(Scope::Component, overrides))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field, p.message))
            .collect()
    };
    assert_eq!(
        fields(&[kv("lead_width", 1_300_000)]),
        vec![
            (
                "param.inconsistent".into(),
                Some("lead_width".into()),
                "Leads 1.3 mm wide touch at a pitch of 1.27 mm.".into()
            ),
            (
                "param.inconsistent".into(),
                Some("leads".into()),
                "4 leads at 1.27 mm do not fit along the 4.9 mm body length.".into()
            )
        ]
    );
    assert_eq!(
        fields(&[kv("span", 4_000_000)]),
        vec![(
            "param.inconsistent".into(),
            Some("span".into()),
            "A span of 4.0 mm leaves no room outside the body width of 3.9 mm for 0.835 mm feet and the bend of 0.2 mm leads."
                .into()
        )]
    );
    // Short-side leads on two sides, and none on four.
    assert_eq!(
        fields(&[kv("leads_short", 2)])
            .into_iter()
            .map(|(_, f, _)| f)
            .collect::<Vec<_>>(),
        vec![Some("leads_short".into())]
    );
    let mut quad = preset("gull-wing-lqfp-48-7x7-0.5");
    quad.push(kv("leads_short", 0));
    assert!(
        GullWing::generate(&request(Scope::Package, &quad))
            .err()
            .unwrap_or_default()
            .iter()
            .any(|p| p.field.as_deref() == Some("leads_short"))
    );
    assert_eq!(
        GullWing::generate(&request(Scope::Component, &[kv("leads", 129)]))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field))
            .collect::<Vec<_>>(),
        vec![("param.out_of_range".into(), Some("leads".into()))]
    );
}

#[test]
fn every_preset_names_its_part_file_with_every_dimension() -> Result<(), String> {
    for p in description().presets() {
        let g = GullWing::generate(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let Some(v1::Model::PartFile { asset, values }) = &g.model else {
            return Err(format!("{}: no model", p.key));
        };
        let four = p.params.get("sides") == Some(&ParamValue::Text("four".into()));
        let mut names = vec![
            "body_height",
            "body_length",
            "body_width",
            "foot_length",
            "lead_thickness",
            "lead_width",
            "leads",
            "pitch",
            "span",
            "standoff",
        ];
        if four {
            names.extend(["leads_short", "span_x"]);
            names.sort_unstable();
            assert_eq!(asset, parts::GULL_WING_FOUR_SIDED, "{}", p.key);
        } else {
            assert_eq!(asset, parts::GULL_WING_TWO_SIDED, "{}", p.key);
        }
        let given: Vec<&str> = values.keys().map(String::as_str).collect();
        assert_eq!(given, names, "{}", p.key);
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
        ("gull-wing-two-sided", parts::GULL_WING_TWO_SIDED),
        ("gull-wing-four-sided", parts::GULL_WING_FOUR_SIDED),
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
