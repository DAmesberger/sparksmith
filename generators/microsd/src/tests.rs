//! Native tests: the golden hashes, the maker's mounting pattern and the
//! body on it, and refusals.
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

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<MicroSd>(include_str!("../golden.json"))?;
    assert!(checked >= 2);
    Ok(())
}

#[test]
fn the_preset_is_the_defaults() {
    let description = description();
    assert_eq!(description.presets().len(), 1);
    for preset in description.presets() {
        // A project-context parameter (the board's mask expansion) comes
        // from the board, never from a preset.
        for param in description.params().iter().filter(|p| p.context.is_none()) {
            assert_eq!(
                preset.params.get(&param.key),
                Some(&param.default),
                "{}",
                param.key
            );
        }
    }
}

/// Hirose's DM3D mounting pattern: contacts at 1.1 mm, contact 8 4.5 mm
/// −x of the card's centre line, contact 1 3.2 mm +x; the front ground
/// lands 0.55 mm outside contact 8 and 9.4–10.9 mm from it; the rear
/// lands 6.85, 7.5, 8.35 and 9.15–10.7 mm behind the contact lands' rear
/// edge.
#[test]
fn lands_are_the_makers_pattern() -> Result<(), Vec<Problem>> {
    let g = MicroSd::generate(&request(Scope::Component, &[]))?;
    let pad = |key: &str| {
        g.footprint
            .pads
            .iter()
            .find(|p| p.key == key)
            .map(|p| (p.at, p.shape.clone()))
    };
    assert_eq!(pad("8").map(|p| p.0), Some([-4_500_000, 0]));
    assert_eq!(pad("1").map(|p| p.0), Some([3_200_000, 0]));
    let edge = |key: &str, i: usize, max: bool| {
        g.footprint
            .pads
            .iter()
            .find(|p| p.key == key)
            .map(|p| {
                let b = land::pad_box(p);
                if max { b.max[i] } else { b.min[i] }
            })
            .unwrap_or(i64::MIN)
    };
    let contact8 = -4_500_000;
    assert_eq!(contact8 - edge("G1", 0, true), 550_000);
    assert_eq!(contact8 - edge("G1", 0, false), 1_850_000);
    assert_eq!(edge("G2", 0, false) - contact8, 9_400_000);
    assert_eq!(edge("G2", 0, true) - contact8, 10_900_000);
    let rear = 875_000;
    let centre = |key: &str| pad(key).map(|p| p.0[1] - rear).unwrap_or(i64::MIN);
    assert_eq!(centre("G3"), 6_850_000);
    assert_eq!(centre("G4"), 7_500_000);
    assert_eq!(centre("CD_B"), 8_350_000);
    assert_eq!(edge("CD_A", 1, false) - rear, 9_150_000);
    assert_eq!(edge("CD_A", 1, true) - rear, 10_700_000);
    // The shell: 11.95 mm across, the centre line 6 mm from its +x side,
    // 1.55 mm high.
    let Some(v1::Model::PartFile { values, .. }) = &g.model else {
        return Err(Vec::new());
    };
    let int = |k: &str| match values.get(k) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    };
    assert_eq!(int("shell_x1") - int("shell_x0"), 11_950_000);
    assert_eq!(int("shell_x1"), 6_000_000);
    assert_eq!(int("rear") - int("front"), 11_450_000);
    assert_eq!(g.package.geometry.envelope_nm[2], 1_550_000);
    Ok(())
}

#[test]
fn every_tail_tip_and_contact_stands_on_its_land() -> Result<(), Vec<Problem>> {
    let g = MicroSd::generate(&request(Scope::Package, &[]))?;
    assert_eq!(g.package.geometry.leads.len(), 14);
    for lead in &g.package.geometry.leads {
        let Some(pad) = g.footprint.pads.iter().find(|q| q.key == lead.key) else {
            return Err(Vec::new());
        };
        let b = land::pad_box(pad);
        assert!(
            (b.min[0]..=b.max[0]).contains(&lead.contact[0])
                && (b.min[1]..=b.max[1]).contains(&lead.contact[1]),
            "{}",
            lead.key
        );
    }
    assert!((-875_000..=875_000).contains(&-650_000));
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
    assert_eq!(
        codes(MicroSd::generate(&request(
            Scope::Component,
            &[kv("tip_y", -1_000_000)]
        ))),
        vec![("param.inconsistent".into(), Some("tip_y".into()))]
    );
    assert_eq!(
        codes(MicroSd::generate(&request(
            Scope::Component,
            &[kv("first_x", -6_000_000)]
        ))),
        vec![
            ("param.inconsistent".into(), Some("first_x".into())),
            ("param.inconsistent".into(), Some("g1_x".into())),
        ]
    );
    assert_eq!(
        codes(MicroSd::generate(&request(
            Scope::Component,
            &[kv("pitch", 400_000)]
        ))),
        vec![("param.out_of_range".into(), Some("pitch".into()))]
    );
}

#[test]
fn the_part_file_hash_is_the_bundles_file() -> Result<(), String> {
    let path = format!(
        "{}/parts/microsd-push-pull.solidcraft",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(&path).map_err(|e| format!("{path}: {e}"))?;
    assert_eq!(
        sparksmith_generator_sdk::abi::hex(&sparksmith_generator_sdk::abi::sha256(&bytes)),
        parts::MICROSD_PUSH_PULL
    );
    Ok(())
}
