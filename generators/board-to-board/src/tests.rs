//! Native tests: the golden hashes, the datasheet lands and the tails on
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
    let checked = golden::check::<BoardToBoard>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_all_generate_with_a_lead_per_pad() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 80);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = BoardToBoard::generate(&request(scope, &preset(&p.key)))?;
            // Every tail's land; an alignment pin's hole is unplated and
            // lands no terminal.
            let pads = g.footprint.pads.iter().filter(|p| !p.terminal.is_empty());
            for hole in g.footprint.pads.iter().filter(|p| p.terminal.is_empty()) {
                assert!(hole.hole.as_ref().is_some_and(|h| !h.plated), "{}", p.key);
            }
            assert_eq!(g.package.geometry.leads.len(), pads.count(), "{}", p.key);
        }
    }
    Ok(())
}

/// Samtec's FTSH-DV layout: 0.74 × 2.79 mm lands at 1.27 mm, 6.86 mm over
/// their outer edges (centres ±2.035 mm); odd pins in the −y row.
#[test]
fn ftsh_lands_are_the_makers_layout() -> Result<(), Vec<Problem>> {
    let g = BoardToBoard::generate(&request(Scope::Component, &preset("b2b-ftsh-5-01")))?;
    let at = |key: &str| g.footprint.pads.iter().find(|p| p.key == key).map(|p| p.at);
    assert_eq!(at("1"), Some([-2_540_000, -2_035_000]));
    assert_eq!(at("2"), Some([-2_540_000, 2_035_000]));
    assert_eq!(at("10"), Some([2_540_000, 2_035_000]));
    assert!(g.footprint.pads.iter().all(|p| p.shape
        == PadShape::Rect {
            size: [740_000, 2_790_000]
        }));
    // The insulator 5 × 1.27 = 6.35 mm long, 3.43 mm wide; posts' tops
    // 2.92 + 3.05 = 5.97 mm up; tails 5.8 mm tip to tip.
    assert_eq!(
        g.package.geometry.envelope_nm,
        [6_350_000, 5_800_000, 5_970_000]
    );
    Ok(())
}

/// TSM-1xx-03-x-DV-A: alignment pins (positions − 2) × 2.54 mm apart, on
/// the centre line, 2.54 mm under the insulator's 1.27 mm standoff.
#[test]
fn tsm_alignment_pins_follow_the_drawing() -> Result<(), Vec<Problem>> {
    for (key, apart) in [
        ("b2b-tsm-4-03-a", 5_080_000),
        ("b2b-tsm-8-03-a", 15_240_000),
    ] {
        let g = BoardToBoard::generate(&request(Scope::Component, &preset(key)))?;
        let holes: Vec<[i64; 2]> = g
            .footprint
            .pads
            .iter()
            .filter(|p| p.hole.is_some())
            .map(|p| p.at)
            .collect();
        assert_eq!(holes, vec![[-apart / 2, 0], [apart / 2, 0]], "{key}");
        let Some(v1::Model::PartFile { values, .. }) = &g.model else {
            return Err(Vec::new());
        };
        assert_eq!(int(values, "peg_bottom"), 1_270_000 - 2_540_000);
        assert_eq!(int(values, "post_top"), 1_270_000 + 2_540_000 + 10_670_000);
        assert_eq!(int(values, "tail_tip"), 3_720_000);
    }
    Ok(())
}

#[test]
fn every_tail_tip_and_contact_stands_on_its_land() -> Result<(), Vec<Problem>> {
    for p in description().presets() {
        let g = BoardToBoard::generate(&request(Scope::Package, &preset(&p.key)))?;
        let Some(v1::Model::PartFile { values, .. }) = &g.model else {
            return Err(Vec::new());
        };
        let tip = int(values, "tail_tip");
        for lead in &g.package.geometry.leads {
            let Some(pad) = g.footprint.pads.iter().find(|q| q.key == lead.key) else {
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
            let signed = pad.at[1].signum() * tip;
            assert!((b.min[1]..=b.max[1]).contains(&signed), "{}", p.key);
        }
    }
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
        codes(BoardToBoard::generate(&request(
            Scope::Component,
            &[kv("tail_span", 8_000_000)]
        ))),
        vec![("param.inconsistent".into(), Some("tail_span".into()))]
    );
    assert_eq!(
        codes(BoardToBoard::generate(&request(
            Scope::Component,
            &[kv("post_side", 1_300_000)]
        ))),
        vec![
            ("param.inconsistent".into(), Some("post_side".into())),
            ("param.inconsistent".into(), Some("tail_span".into())),
        ]
    );
    // Alignment pins of 3 mm between 1.27 mm rows.
    let mut params = preset("b2b-ftsh-5-01");
    params.push(("alignment".into(), ParamValue::Text("pins".into())));
    params.push(kv("peg_diameter", 3_000_000));
    assert_eq!(
        codes(BoardToBoard::generate(&request(Scope::Component, &params))),
        vec![("param.inconsistent".into(), Some("peg_diameter".into()))]
    );
    assert_eq!(
        codes(BoardToBoard::generate(&request(
            Scope::Component,
            &[kv("positions", 51)]
        ))),
        vec![("param.out_of_range".into(), Some("positions".into()))]
    );
}

#[test]
fn the_part_file_hashes_are_the_bundles_files() -> Result<(), String> {
    for (stem, hash) in [
        ("b2b-dual-row", parts::B2B_DUAL_ROW),
        ("b2b-dual-row-pegs", parts::B2B_DUAL_ROW_PEGS),
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

/// A length between -1 mm and 0 keeps its minus sign (`nm / MM` is 0 there),
/// as in the refusal that prints a negative peg position.
#[test]
fn mm_keeps_the_sign_of_a_length_below_one_millimetre() {
    assert_eq!(mm(-500_000), "-0.5");
    assert_eq!(mm(-1), "-0.000001");
    assert_eq!(mm(-1_500_000), "-1.5");
    assert_eq!(mm(0), "0.0");
    assert_eq!(mm(2_540_000), "2.54");
    assert_eq!(mm(2_000_000), "2.0");
}
