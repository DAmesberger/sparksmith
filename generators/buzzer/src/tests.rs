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

fn kv(key: &str, value: i64) -> (String, ParamValue) {
    (key.into(), ParamValue::Int(value))
}

fn model(g: &Generation) -> (String, v1::Params) {
    match &g.model {
        Some(v1::Model::PartFile { asset, values }) => (asset.clone(), values.clone()),
        _ => (String::new(), v1::Params::new()),
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Buzzer>(include_str!("../golden.json"))?;
    assert!(checked >= 2);
    Ok(())
}

#[test]
fn the_cmi_9653s_lands_under_its_feet() -> Result<(), Vec<Problem>> {
    // Lands 3.0 × 2.5 mm centred 3.8 mm out, so their outer edges lie
    // 0.5 mm beyond the 9.6 mm case, as the drawing lays them. The foot
    // runs from 1.2 mm under the case to the 0.25 mm plate: its middle at
    // (9.6 + 0.25 − 1.2) / 2 = 4.325 mm, on the land.
    let g = Buzzer::generate(&request(Scope::Component, &[]))?;
    let at: Vec<[i64; 2]> = g.footprint.pads.iter().map(|p| p.at).collect();
    assert_eq!(at, vec![[-3_800_000, 0], [3_800_000, 0]]);
    let b = land::pad_box(&g.footprint.pads[1]);
    assert_eq!(b.max[0], 5_300_000);
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(leads, vec![[-4_325_000, 0, 0], [4_325_000, 0, 0]]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [10_100_000, 9_600_000, 5_000_000]
    );
    let c = g.component.as_ref();
    assert_eq!(c.map(|c| c.prefix.as_str()), Some("BZ"));
    assert_eq!(
        c.map(|c| c
            .terminals
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["+", "−"])
    );
    let (asset, values) = model(&g);
    assert_eq!(asset, parts::BUZZER_SMD);
    assert_eq!(
        values.get("corner_bevel"),
        Some(&ParamValue::Int(1_200_000))
    );
    let with_label = Buzzer::generate(&request(
        Scope::Package,
        &[("label".into(), ParamValue::Bool(true))],
    ))?;
    assert_eq!(model(&with_label).0, parts::BUZZER_SMD_LABEL);
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Buzzer::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("terminal_width", 9_600_000)]),
        Some(("param.inconsistent".into(), Some("terminal_width".into())))
    );
    assert_eq!(
        refused(&[kv("terminal_rise", 6_000_000)]),
        Some(("param.inconsistent".into(), Some("terminal_rise".into())))
    );
    assert_eq!(
        refused(&[kv("land_x", 1_000_000)]),
        Some(("param.inconsistent".into(), Some("land_x".into())))
    );
}
