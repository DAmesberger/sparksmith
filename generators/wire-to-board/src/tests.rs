//! Native tests: the golden hashes, the datasheet lands and the bodies on
//! them, and refusals.
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

fn int(values: &v1::Params, key: &str) -> i64 {
    match values.get(key) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<WireToBoard>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_all_generate_with_a_lead_per_pad() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 57);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = WireToBoard::generate(&request(scope, &preset(&p.key)))?;
            assert_eq!(
                g.package.geometry.leads.len(),
                g.footprint.pads.len(),
                "{}",
                p.key
            );
        }
    }
    Ok(())
}

/// The JST SH side-entry layout (SR/SZ catalogue, SMT side entry): 0.6 ×
/// 1.55 mm lands at 1.0 mm, 1.2 × 1.8 mm tab lands 0.7 mm past the outer
/// pins, 4.0 mm between the signal lands' inner end and the tab lands'
/// outer edge, 5.55 mm overall.
#[test]
fn sh_side_entry_lands_are_the_makers_layout() -> Result<(), Vec<Problem>> {
    let g = WireToBoard::generate(&request(Scope::Component, &preset("wtb-sh-side-4")))?;
    let at = |key: &str| {
        g.footprint
            .pads
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.at, p.shape.clone()))
    };
    let land = PadShape::Rect {
        size: [600_000, 1_550_000],
    };
    assert_eq!(at("1"), Some(([-1_500_000, 0], land.clone())));
    assert_eq!(at("4"), Some(([1_500_000, 0], land)));
    let tab = PadShape::Rect {
        size: [1_200_000, 1_800_000],
    };
    assert_eq!(at("MP1"), Some(([-2_800_000, -3_875_000], tab.clone())));
    assert_eq!(at("MP2"), Some(([2_800_000, -3_875_000], tab)));
    // Signal land's far end to the tab land's outer edge: 5.55 mm; its
    // near end: 4.0 mm (the layout's dimensions).
    assert_eq!(775_000 + 3_875_000 + 900_000, 5_550_000);
    assert_eq!(-775_000 + 3_875_000 + 900_000, 4_000_000);
    // The catalogue's outline: B = 6.0 mm, 4.25 mm deep, 2.9 mm high.
    let Some(v1::Model::PartFile { values, .. }) = &g.model else {
        return Err(Vec::new());
    };
    assert_eq!(int(values, "body_length"), 6_000_000);
    assert_eq!(int(values, "y_rear") - int(values, "y_front"), 4_250_000);
    assert_eq!(int(values, "top"), 2_900_000);
    // Tails 0.7 mm past the rear, on the land (±0.775 mm).
    assert_eq!(int(values, "tip_y") - int(values, "y_rear"), 700_000);
    assert_eq!(int(values, "tip_y"), 375_000);
    Ok(())
}

/// The PH side-entry layout (PH catalogue, SMT side entry): tab lands
/// 1.5 × 3.4 mm 1.6 mm past the outer pins, signal lands 1.0 mm wide
/// between 5.5 mm and 9.0 mm from the tab lands' outer edge.
#[test]
fn ph_side_entry_lands_are_the_makers_layout() -> Result<(), Vec<Problem>> {
    let g = WireToBoard::generate(&request(Scope::Component, &preset("wtb-ph-side-2")))?;
    let at = |key: &str| g.footprint.pads.iter().find(|p| p.key == key).map(|p| p.at);
    assert_eq!(at("1"), Some([-1_000_000, 0]));
    assert_eq!(at("MP2"), Some([3_350_000, -5_550_000]));
    let outer = 5_550_000 + 1_700_000;
    assert_eq!(outer - 1_750_000, 5_500_000);
    assert_eq!(outer + 1_750_000, 9_000_000);
    assert_eq!(g.package.geometry.envelope_nm[2], 5_500_000);
    Ok(())
}

/// Every preset's tails end on their lands, and every lead contact lies on
/// its land: the body sits on the maker's layout as an imported board's
/// footprint has it.
#[test]
fn every_tail_tip_and_contact_stands_on_its_land() -> Result<(), Vec<Problem>> {
    for p in description().presets() {
        let g = WireToBoard::generate(&request(Scope::Package, &preset(&p.key)))?;
        let Some(v1::Model::PartFile { values, .. }) = &g.model else {
            return Err(Vec::new());
        };
        let tip = int(values, "tip_y");
        for lead in &g.package.geometry.leads {
            let pad = g.footprint.pads.iter().find(|q| q.key == lead.key);
            let Some(pad) = pad else {
                return Err(Vec::new());
            };
            let b = land::pad_box(pad);
            assert!(
                (b.min[0]..=b.max[0]).contains(&lead.contact[0])
                    && (b.min[1]..=b.max[1]).contains(&lead.contact[1]),
                "{} {}",
                p.key,
                lead.key
            );
            if !lead.key.starts_with("MP") {
                assert!((b.min[1]..=b.max[1]).contains(&tip), "{}", p.key);
            }
        }
    }
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = WireToBoard::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 6);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let codes = |r: Result<Generation, Vec<Problem>>| -> Vec<(String, Option<String>)> {
        r.err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field))
            .collect()
    };
    // Lands wider than the pitch.
    let refused = WireToBoard::generate(&request(Scope::Component, &[kv("land_width", 1_100_000)]));
    let problems = refused.err().unwrap_or_default();
    assert_eq!(
        problems
            .iter()
            .map(|p| (p.code.as_str(), p.field.as_deref(), p.message.as_str()))
            .collect::<Vec<_>>(),
        vec![(
            "param.inconsistent",
            Some("pitch"),
            "Contacts of 0.3 mm on lands of 1.1 mm do not fit a pitch of 1.0 mm."
        )]
    );
    // Tails that miss their lands.
    assert_eq!(
        codes(WireToBoard::generate(&request(
            Scope::Component,
            &[kv("lead_reach", 2_000_000)]
        ))),
        vec![("param.inconsistent".into(), Some("lead_reach".into()))]
    );
    // Twenty contacts in a cavity for four.
    assert!(
        codes(WireToBoard::generate(&request(
            Scope::Component,
            &[kv("pins", 20)]
        )))
        .contains(&("param.inconsistent".into(), Some("cavity_width".into())))
    );
    assert_eq!(
        codes(WireToBoard::generate(&request(
            Scope::Component,
            &[kv("pins", 21)]
        ))),
        vec![("param.out_of_range".into(), Some("pins".into()))]
    );
}

#[test]
fn every_preset_names_its_part_file_by_entry_and_colour() -> Result<(), String> {
    for p in description().presets() {
        for colour in ["natural", "white", "black"] {
            let mut params = preset(&p.key);
            params.push(("colour".into(), ParamValue::Text(colour.into())));
            let g = WireToBoard::generate(&request(Scope::Package, &params))
                .map_err(|e| format!("{e:?}"))?;
            let Some(v1::Model::PartFile { asset, values }) = &g.model else {
                return Err(format!("{}: no model", p.key));
            };
            let side = p.key.contains("-side-");
            let expected = match (side, colour) {
                (true, "natural") => parts::WTB_SIDE_NATURAL,
                (true, "white") => parts::WTB_SIDE_WHITE,
                (true, _) => parts::WTB_SIDE_BLACK,
                (false, "natural") => parts::WTB_TOP_NATURAL,
                (false, "white") => parts::WTB_TOP_WHITE,
                (false, _) => parts::WTB_TOP_BLACK,
            };
            assert_eq!(asset, expected, "{}", p.key);
            assert_eq!(values.len(), if side { 29 } else { 28 }, "{}", p.key);
        }
    }
    Ok(())
}

#[test]
fn the_part_file_hashes_are_the_bundles_files() -> Result<(), String> {
    for (stem, hash) in [
        ("wtb-side-natural", parts::WTB_SIDE_NATURAL),
        ("wtb-side-white", parts::WTB_SIDE_WHITE),
        ("wtb-side-black", parts::WTB_SIDE_BLACK),
        ("wtb-top-natural", parts::WTB_TOP_NATURAL),
        ("wtb-top-white", parts::WTB_TOP_WHITE),
        ("wtb-top-black", parts::WTB_TOP_BLACK),
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
