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

fn int(g: &Generation, key: &str) -> i64 {
    match &g.model {
        Some(v1::Model::PartFile { values, .. }) => match values.get(key) {
            Some(ParamValue::Int(v)) => *v,
            _ => i64::MIN,
        },
        _ => i64::MIN,
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Reed>(include_str!("../golden.json"))?;
    assert!(checked >= 2);
    Ok(())
}

#[test]
fn the_mdsm_4_lands_under_its_feet() -> Result<(), Vec<Problem>> {
    // Lands 2.34 × 1.30 mm with 16.00 mm between them: centred at
    // ±(8 + 1.17) mm. The foot runs from the leg (8.76 − 0.5 mm) to the
    // tip (9.65 mm), its contact in the middle at 8.955 mm, on the land.
    let g = Reed::generate(&request(Scope::Component, &[]))?;
    let at: Vec<[i64; 2]> = g.footprint.pads.iter().map(|p| p.at).collect();
    assert_eq!(at, vec![[-9_170_000, 0], [9_170_000, 0]]);
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(leads, vec![[-8_955_000, 0, 0], [8_955_000, 0, 0]]);
    for (lead, pad) in leads.iter().zip(&g.footprint.pads) {
        let b = land::pad_box(pad);
        assert!(lead[0] > b.min[0] && lead[0] < b.max[0]);
    }
    assert_eq!(
        g.package.geometry.envelope_nm,
        [19_300_000, 2_280_000, 2_840_000]
    );
    assert_eq!(g.component.as_ref().map(|c| c.prefix.as_str()), Some("SW"));
    // Wire min(0.5, 2.28 / 4); wall min(0.3, 2.28 / 7); seals 1.6 mm.
    assert_eq!(
        [
            int(&g, "lead_diameter"),
            int(&g, "wall"),
            int(&g, "seal_length"),
            int(&g, "seal_diameter"),
        ],
        [500_000, 300_000, 1_600_000, 1_000_000]
    );
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Reed::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("bend_span", 15_500_000)]),
        Some(("param.inconsistent".into(), Some("bend_span".into())))
    );
    assert_eq!(
        refused(&[kv("span", 17_000_000)]),
        Some(("param.inconsistent".into(), Some("span".into())))
    );
    assert_eq!(
        refused(&[kv("height", 2_300_000)]),
        Some(("param.inconsistent".into(), Some("height".into())))
    );
}
