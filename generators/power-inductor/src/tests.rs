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

fn model(g: &Generation) -> (String, v1::Params) {
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
    let checked = golden::check::<PowerInductor>(include_str!("../golden.json"))?;
    assert!(checked >= 3);
    Ok(())
}

#[test]
fn every_preset_generates_with_the_l_prefix() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 3);
    for p in description.presets() {
        let g = PowerInductor::generate(&request(Scope::Component, &preset(&p.key)))?;
        assert_eq!(g.component.map(|c| c.prefix), Some("L".into()), "{}", p.key);
        // Each contact lies on its land.
        for (lead, pad) in g.package.geometry.leads.iter().zip(&g.footprint.pads) {
            let b = land::pad_box(pad);
            assert!(
                lead.contact[0] > b.min[0] && lead.contact[0] < b.max[0],
                "{}",
                p.key
            );
        }
    }
    Ok(())
}

#[test]
fn the_xel4030_and_srr6028_follow_their_drawings() -> Result<(), Vec<Problem>> {
    // XEL4030: terminals from ±0.785 to ±1.605 mm, contacts at ±1.195 mm;
    // lands 0.98 × 3.4 mm at ±1.185 mm.
    let g = PowerInductor::generate(&request(Scope::Package, &preset("power-inductor-xel4030")))?;
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(leads, vec![[-1_195_000, 0, 0], [1_195_000, 0, 0]]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [4_000_000, 4_000_000, 3_100_000]
    );
    let (asset, v) = model(&g);
    assert_eq!(asset, parts::INDUCTOR_MOLDED);
    assert_eq!(int(&v, "corner_round"), 200_000);
    // SRR6028: bore 6.8 − 0.9, flange 0.3 and winding 0.7 smaller; the
    // flange a sixth of 2.8 mm.
    let g = PowerInductor::generate(&request(Scope::Package, &preset("power-inductor-srr6028")))?;
    let (asset, v) = model(&g);
    assert_eq!(asset, parts::INDUCTOR_SHIELDED);
    assert_eq!(
        [
            int(&v, "bore_diameter"),
            int(&v, "flange_diameter"),
            int(&v, "winding_diameter"),
            int(&v, "flange_thickness"),
            int(&v, "corner_bevel"),
        ],
        [5_900_000, 5_600_000, 5_200_000, 466_667, 1_000_000]
    );
    let g = PowerInductor::generate(&request(
        Scope::Package,
        &preset("power-inductor-6x6-shielded"),
    ))?;
    assert_eq!(model(&g).0, parts::INDUCTOR_SHIELDED_WINDOW);
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        PowerInductor::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("terminal_gap", 3_000_000)]),
        Some(("param.inconsistent".into(), Some("terminal_gap".into())))
    );
    let mut shielded = preset("power-inductor-srr6028");
    shielded.push(kv("terminal_gap", 2_000_000));
    assert_eq!(
        refused(&shielded),
        Some(("param.inconsistent".into(), Some("terminal_gap".into())))
    );
    assert_eq!(
        refused(&[kv("terminal_width", 4_500_000)]),
        Some(("param.inconsistent".into(), Some("terminal_width".into())))
    );
    assert_eq!(
        refused(&[kv("land_x", 400_000)]),
        Some(("param.inconsistent".into(), Some("land_x".into())))
    );
}
