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

fn asset(g: &Generation) -> String {
    match &g.model {
        Some(v1::Model::PartFile { asset, .. }) => asset.clone(),
        _ => String::new(),
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Standoff>(include_str!("../golden.json"))?;
    assert!(checked >= 2);
    Ok(())
}

#[test]
fn every_preset_generates() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 5);
    for p in description.presets() {
        let overrides: Vec<(String, ParamValue)> = p.params.clone().into_iter().collect();
        let g = Standoff::generate(&request(Scope::Component, &overrides))?;
        assert_eq!(g.footprint.pads.len(), 2, "{}", p.key);
    }
    Ok(())
}

#[test]
fn the_wa_smsi_land_and_spigot() -> Result<(), Vec<Problem>> {
    // An annular land of Ø7.4 round a Ø4.4 non-plated hole, the spigot
    // Ø4.2 inside it; the contact on the wall between Ø2.5 and Ø6.0.
    let g = Standoff::generate(&request(Scope::Component, &[]))?;
    let pad = &g.footprint.pads[0];
    assert_eq!(
        pad.shape,
        PadShape::Circle {
            diameter_nm: 7_400_000
        }
    );
    assert_eq!(pad.hole, None);
    let hole = &g.footprint.pads[1];
    assert_eq!(
        hole.hole,
        Some(Hole {
            shape: HoleShape::Round {
                diameter_nm: 4_400_000
            },
            plated: false,
            offset: None,
        })
    );
    assert_eq!(g.package.geometry.leads[0].contact, [2_125_000, 0, 0]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [6_000_000, 6_000_000, 10_000_000]
    );
    assert_eq!(asset(&g), parts::STANDOFF_SPIGOT);
    assert_eq!(g.component.map(|c| c.prefix), Some("H".into()));
    let plain = Standoff::generate(&request(Scope::Package, &[kv("spigot_length", 0)]))?;
    assert_eq!(asset(&plain), parts::STANDOFF);
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Standoff::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("bore_diameter", 5_800_000)]),
        Some(("param.inconsistent".into(), Some("bore_diameter".into())))
    );
    assert_eq!(
        refused(&[kv("spigot_diameter", 6_500_000)]),
        Some(("param.inconsistent".into(), Some("spigot_diameter".into())))
    );
    assert_eq!(
        refused(&[kv("hole_diameter", 4_000_000)]),
        Some(("param.inconsistent".into(), Some("hole_diameter".into())))
    );
}
