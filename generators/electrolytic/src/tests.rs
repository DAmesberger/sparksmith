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

fn values(g: &Generation) -> (String, v1::Params) {
    match &g.model {
        Some(v1::Model::PartFile { asset, values }) => (asset.clone(), values.clone()),
        _ => (String::new(), v1::Params::new()),
    }
}

fn int(values: &v1::Params, key: &str) -> i64 {
    match values.get(key) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Electrolytic>(include_str!("../golden.json"))?;
    assert!(checked >= 3);
    Ok(())
}

#[test]
fn every_preset_generates() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 9);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Electrolytic::generate(&request(scope, &preset(&p.key)))?;
            assert_eq!(g.footprint.pads.len(), 2, "{}", p.key);
            assert_eq!(scope == Scope::Component, g.component.is_some());
        }
    }
    Ok(())
}

#[test]
fn size_d_follows_the_drawing_and_the_land_table() -> Result<(), Vec<Problem>> {
    // Panasonic D: lands 3.2 × 1.6 mm, 1.8 mm apart, so centred at
    // ±(0.9 + 1.6) mm; terminals from ±0.9 to ±3.9 mm, the contact in the
    // middle of each at ±2.4 mm.
    let g = Electrolytic::generate(&request(
        Scope::Component,
        &preset("electrolytic-panasonic-d"),
    ))?;
    let pads: Vec<_> = g
        .footprint
        .pads
        .iter()
        .map(|p| (p.key.as_str(), p.at, p.shape.clone()))
        .collect();
    let shape = PadShape::Rect {
        size: [3_200_000, 1_600_000],
    };
    assert_eq!(
        pads,
        vec![
            ("1", [-2_500_000, 0], shape.clone()),
            ("2", [2_500_000, 0], shape)
        ]
    );
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(leads, vec![[-2_400_000, 0, 0], [2_400_000, 0, 0]]);
    // Each contact lies on its land.
    for (lead, pad) in leads.iter().zip(&g.footprint.pads) {
        let b = land::pad_box(pad);
        assert!(lead[0] > b.min[0] && lead[0] < b.max[0]);
    }
    assert_eq!(
        g.package.geometry.envelope_nm,
        [7_800_000, 6_600_000, 5_800_000]
    );
    let c = g.component.as_ref();
    assert_eq!(c.map(|c| c.prefix.as_str()), Some("C"));
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::ELECTROLYTIC_SMD_VENT);
    // Seat plate 1 mm; bevel min(0.99, 1.725); terminals 0.3 mm; bead at
    // min(0.8, 4.8 / 4); depth min(0.2, 0.21); rim min(0.35, 0.35).
    assert_eq!(
        [
            int(&v, "base_height"),
            int(&v, "base_bevel"),
            int(&v, "lead_thickness"),
            int(&v, "bead_height"),
            int(&v, "bead_depth"),
            int(&v, "rim_round"),
        ],
        [1_000_000, 990_000, 300_000, 800_000, 200_000, 350_000]
    );
    Ok(())
}

#[test]
fn without_the_vent_the_plain_file_is_named() -> Result<(), Vec<Problem>> {
    let mut plain = preset("electrolytic-6.3x5.4");
    plain.push(("vent".into(), ParamValue::Bool(false)));
    let g = Electrolytic::generate(&request(Scope::Package, &plain))?;
    assert_eq!(values(&g).0, parts::ELECTROLYTIC_SMD);
    assert_eq!(g.package.geometry.envelope_nm[2], 5_500_000);
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Electrolytic::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("can_diameter", 7_000_000)]),
        Some(("param.inconsistent".into(), Some("can_diameter".into())))
    );
    assert_eq!(
        refused(&[kv("terminal_gap", 8_000_000)]),
        Some(("param.inconsistent".into(), Some("terminal_gap".into())))
    );
    assert_eq!(
        refused(&[kv("height", 2_500_000)]),
        Some(("param.inconsistent".into(), Some("height".into())))
    );
}
