//! Native tests: the golden hashes, hand-derived lands and keep-out, the
//! part-file values (slots, parking, the ground array's turn) and refusals.
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

fn values(g: &Generation) -> (&str, &v1::Params) {
    match &g.model {
        Some(v1::Model::PartFile { asset, values }) => (asset.as_str(), values),
        None => ("", &EMPTY),
    }
}

static EMPTY: v1::Params = v1::Params::new();

fn int(values: &v1::Params, key: &str) -> i64 {
    match values.get(key) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    }
}

const WROOM: &str = "module-esp32-s31-wroom-3";

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Module>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_generate_in_both_scopes() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 4);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Module::generate(&request(scope, &preset(&p.key)))?;
            assert_eq!(
                g.package.geometry.leads.len(),
                g.package.terminals.len(),
                "{}",
                p.key
            );
        }
    }
    Ok(())
}

#[test]
fn the_wroom3_has_99_terminals_and_nine_pads_numbered_95() -> Result<(), Vec<Problem>> {
    let g = Module::generate(&request(Scope::Component, &preset(WROOM)))?;
    assert_eq!(g.package.terminals.len(), 99);
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 94 + 9 + 4);
    let ground: Vec<&Pad> = pads.iter().filter(|p| p.terminal == "95").collect();
    assert_eq!(ground.len(), 9);
    assert!(ground.iter().all(|p| p.number.as_deref() == Some("95")));
    assert_eq!(ground.first().map(|p| p.key.as_str()), Some("95#1"));
    // The middle pad on the array's centre; turned 45°: the first (row 0,
    // column 0) at (−2.35 √2, 0) from it.
    assert_eq!(ground.get(4).map(|p| p.at), Some([0, -3_000_000]));
    assert_eq!(ground.first().map(|p| p.at), Some([-3_323_402, -3_000_000]));
    assert!(ground.iter().all(|p| p.rot_mdeg == 45_000));
    // Pin 1: the top of the left row; pad 48 the first of the right row.
    assert_eq!(pads.first().map(|p| p.at), Some([-10_300_000, 7_200_000]));
    assert_eq!(pads.get(47).map(|p| p.at), Some([10_300_000, -13_200_000]));
    // The antenna keep-out: the 6 mm end, full width.
    assert_eq!(
        g.footprint.keepouts,
        vec![Keepout {
            name: "antenna".into(),
            outline: vec![
                [-11_000_000, 9_000_000],
                [11_000_000, 9_000_000],
                [11_000_000, 15_000_000],
                [-11_000_000, 15_000_000]
            ],
        }]
    );
    let (asset, v) = values(&g);
    // The module is black, as Espressif makes it.
    assert_eq!(asset, parts::MODULE_DIAGONAL_BLACK);
    // The ground seed is the form's first pad; four rows in their own
    // slots (LGA pads: no castellation values); four singles, the rest
    // parked.
    assert_eq!(
        (int(v, "ground_x"), int(v, "ground_y")),
        (-3_323_402, -3_000_000)
    );
    assert!(!v.contains_key("notch_1_count") && !v.contains_key("notch_radius"));
    assert_eq!(int(v, "row_4_count"), 22);
    assert_eq!(int(v, "row_5_x"), int(v, "park_x"));
    assert_eq!(int(v, "single_4_x"), -10_300_000);
    assert_eq!(int(v, "single_5_x"), int(v, "park_x"));
    // 20.4 mm of span: 8 turns of 1.275 mm.
    assert_eq!(int(v, "antenna_turns"), 8);
    assert_eq!(int(v, "antenna_step"), 1_275_000);
    assert_eq!(int(v, "can_round"), 300_000);
    Ok(())
}

#[test]
fn castellated_rows_land_outward_and_contact_under_their_plating() -> Result<(), Vec<Problem>> {
    let g = Module::generate(&request(Scope::Package, &[]))?;
    let pads = &g.footprint.pads;
    // Pin 1 at (−8.4, 5.5): its 1.5 × 0.9 mm land reaches 0.3 mm beyond
    // the −9 mm edge.
    assert_eq!(
        pads.first().map(|p| (p.at, p.shape.clone())),
        Some((
            [-8_550_000, 5_500_000],
            PadShape::Rect {
                size: [1_500_000, 900_000]
            }
        ))
    );
    // Pin 15, the first of the bottom row, reaches below −12.5 mm.
    assert_eq!(pads.get(14).map(|p| p.at), Some([-5_715_000, -12_050_000]));
    let lead = &g.package.geometry.leads[0];
    assert_eq!(lead.contact, [-8_850_000, 5_500_000, 0]);
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MODULE_CASTELLATED);
    assert_eq!(int(v, "notch_1_count"), 14);
    assert_eq!(
        (int(v, "notch_1_x"), int(v, "notch_1_y")),
        (-9_000_000, 5_500_000)
    );
    assert_eq!(
        (int(v, "notch_2_x"), int(v, "notch_2_y")),
        (-5_715_000, -12_500_000)
    );
    assert_eq!(int(v, "notch_radius"), 300_000);
    // A 3 × 3 array along the axes, its seed at row 0, column 0.
    assert_eq!(
        (int(v, "ground_x"), int(v, "ground_y")),
        (-1_600_000, -400_000)
    );
    assert_eq!(int(v, "antenna_turns"), 6);
    assert_eq!(int(v, "antenna_units"), 5);
    Ok(())
}

#[test]
fn the_wroom1_lands_where_its_datasheet_lays_them() -> Result<(), Vec<Problem>> {
    // ESP32-S3-WROOM-1 (datasheet v1.8, Figures 10-1 and 11-1): 41 pads,
    // lands 1.5 × 0.9 mm from 0.5 mm outside the 18 mm outline (x −9.5 to
    // −8.0), pin 1 7.49 mm below the +y edge, pin 15 2.015 mm in from the
    // −x edge; nine 0.9 mm ground pads numbered 41 at 1.4 mm about
    // (−1.5, −2.46); castellations Ø0.55; the can embossed.
    let g = Module::generate(&request(
        Scope::Component,
        &preset("module-esp32-s3-wroom-1"),
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 49);
    assert_eq!(
        pads.first().map(|p| (p.at, p.shape.clone())),
        Some((
            [-8_750_000, 5_260_000],
            PadShape::Rect {
                size: [1_500_000, 900_000]
            }
        ))
    );
    assert_eq!(pads.get(14).map(|p| p.at[0]), Some(-6_985_000));
    assert_eq!(pads.get(39).map(|p| p.at), Some([8_750_000, 5_260_000]));
    let ground: Vec<[i64; 2]> = pads
        .iter()
        .filter(|p| p.number.as_deref() == Some("41"))
        .map(|p| p.at)
        .collect();
    assert_eq!(ground.len(), 9);
    let mid = [
        ground.iter().map(|p| p[0]).sum::<i64>() / 9,
        ground.iter().map(|p| p[1]).sum::<i64>() / 9,
    ];
    assert_eq!(mid, [-1_500_000, -2_460_000]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [18_000_000, 25_500_000, 3_100_000]
    );
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MODULE_CASTELLATED_BLACK);
    assert_eq!(int(v, "notch_radius"), 275_000);
    assert_eq!(
        [
            int(v, "can_emboss_inset"),
            int(v, "can_emboss_width"),
            int(v, "antenna_turns"),
            int(v, "antenna_trace"),
        ],
        [1_000_000, 300_000, 3, 500_000]
    );
    Ok(())
}

#[test]
fn the_wroom1u_is_the_wroom1_pads_without_the_antenna_end() -> Result<(), Vec<Problem>> {
    // ESP32-S3-WROOM-1U (datasheet v1.8, Figures 10-2 and 11-2): the
    // WROOM-1's pads at the same distance from the bottom edge on a
    // 19.2 mm PCB (all 3.15 mm up in the centred frame); no antenna or
    // keep-out; the can notched 4.9 × 4.4 mm at its +x +y corner for the
    // U.FL receptacle 3 mm from the right edge and 2.46 mm from the top.
    let one = Module::generate(&request(Scope::Package, &preset("module-esp32-s3-wroom-1")))?;
    let u = Module::generate(&request(
        Scope::Package,
        &preset("module-esp32-s3-wroom-1u"),
    ))?;
    let shifted: Vec<[i64; 2]> = one
        .footprint
        .pads
        .iter()
        .map(|p| [p.at[0], p.at[1] + 3_150_000])
        .collect();
    let pads: Vec<[i64; 2]> = u.footprint.pads.iter().map(|p| p.at).collect();
    assert_eq!(pads, shifted);
    assert_eq!(
        u.package.geometry.envelope_nm,
        [18_000_000, 19_200_000, 3_200_000]
    );
    assert!(u.footprint.keepouts.is_empty());
    let (_, v) = values(&u);
    assert_eq!(
        [int(v, "connector_x"), int(v, "connector_y")],
        [6_000_000, 7_140_000]
    );
    // The notch's inner corner: the can's +x +y corner (7.825, 9.0) less
    // the notch.
    assert_eq!(
        [int(v, "can_notch_x"), int(v, "can_notch_y")],
        [2_925_000, 4_600_000]
    );
    assert!(int(v, "antenna_shift") < -50 * MM);
    // The WROOM-1 parks both.
    let (_, w) = values(&one);
    assert!(int(w, "connector_x") < -50 * MM && int(w, "can_notch_x") < -50 * MM);
    Ok(())
}

#[test]
fn a_ground_array_turned_a_quarter_swaps_its_counts_in_the_part_file() -> Result<(), Vec<Problem>> {
    let mut over = vec![
        kv("ground_rows", 2),
        kv("ground_columns", 3),
        kv("ground_rotation", 90_000),
    ];
    let g = Module::generate(&request(Scope::Package, &over))?;
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MODULE_CASTELLATED);
    assert_eq!((int(v, "ground_rows"), int(v, "ground_columns")), (3, 2));
    // The file's pattern runs +x then −y from the seed: the form's last
    // column of its first row, turned a quarter.
    let ground: Vec<Point2> = g
        .footprint
        .pads
        .iter()
        .filter(|p| p.terminal == "39")
        .map(|p| p.at)
        .collect();
    let min_x = ground.iter().map(|p| p[0]).min();
    let max_y = ground.iter().map(|p| p[1]).max();
    assert_eq!(Some(int(v, "ground_x")), min_x);
    assert_eq!(Some(int(v, "ground_y")), max_y);
    over.push(kv("ground_rotation", 30_000));
    let refused = Module::generate(&request(Scope::Package, &over));
    assert!(refused.is_err_and(|p| p.iter().any(|p| p.message.contains("45°"))));
    Ok(())
}

#[test]
fn no_antenna_parks_the_meander_and_drops_the_keep_out() -> Result<(), Vec<Problem>> {
    let g = Module::generate(&request(
        Scope::Package,
        &[kv("antenna_length", 0), kv("can_y", 0)],
    ))?;
    assert!(g.footprint.keepouts.is_empty());
    let (_, v) = values(&g);
    assert_eq!(int(v, "antenna_length"), 0);
    assert_eq!(int(v, "antenna_depth"), 2_500_000);
    assert!(int(v, "antenna_shift") < -50 * MM);
    Ok(())
}

#[test]
fn inconsistent_values_are_named() {
    let three_left = vec![
        (
            "row_sides".to_string(),
            ParamValue::List(texts(&["left", "left", "left"])),
        ),
        ("row_counts".to_string(), ParamValue::List(ints(&[2, 2, 2]))),
        (
            "row_x".to_string(),
            ParamValue::List(ints(&[-8_400_000, -6_000_000, -4_000_000])),
        ),
        ("row_y".to_string(), ParamValue::List(ints(&[0, 0, 0]))),
    ];
    let refused = Module::generate(&request(Scope::Package, &three_left));
    assert!(refused.is_err_and(|p| p.iter().any(|p| p.message.contains("2 a side"))));
    let refused = Module::generate(&request(
        Scope::Package,
        &[("ground_number".into(), ParamValue::Text("3".into()))],
    ));
    assert!(refused.is_err_and(|p| p.iter().any(|p| p.message.contains("unique"))));
    let refused = Module::generate(&request(Scope::Package, &[kv("can_y", 3_000_000)]));
    assert!(refused.is_err_and(|p| p.iter().any(|p| p.message.contains("shield can"))));
}

/// The ESP32-S3-WROOM-1U's shield label is Espressif's laser-marking
/// artwork filled from the preset: every line of it, at the can's size;
/// a module whose preset leaves a line empty drops that line.
#[test]
fn the_wroom1u_label_carries_every_line_of_the_makers_artwork() -> Result<(), Vec<Problem>> {
    let g = Module::generate(&request(
        Scope::Package,
        &preset("module-esp32-s3-wroom-1u"),
    ))?;
    let missing = |what: &str| {
        vec![Problem {
            code: "test".into(),
            field: None,
            message: format!("no {what}"),
        }]
    };
    let marking = g.marking.ok_or_else(|| missing("label"))?;
    let art = marking
        .artwork
        .first()
        .ok_or_else(|| missing("label artwork"))?;
    let svg = art.svg.as_str();
    for line in [
        "data-art=\"espressif-logo\"",
        ">ESP32-S3-WROOM-1U</text>",
        "data-art=\"ce-mark\"",
        "data-art=\"giteki-mark\"",
        ">201-220894</text>",
        ">FCC ID:2AC7Z-ESPS3WROOM1U</text>",
        ">21098-ESPS3WROOMU</text>",
        ">2022DP8926(M)</text>",
        "data-art=\"espressif-company-zh\"",
        ">XX{code}</text>",
        "data-code=\"datamatrix\" data-payload=\"{var:datamatrix}\"",
    ] {
        assert!(svg.contains(line), "{line} missing from {svg}");
    }
    assert!(!svg.contains('%'), "a token left in {svg}");
    // The artwork's crop of the can top at 0.01 mm a unit: 13.47 × 15.04
    // mm, centred on the 15.65 × 17.5 mm can.
    assert_eq!(art.size, [13_470_000, 15_040_000]);
    assert!(marking.metal);
    assert_eq!(
        marking
            .codes
            .get("ESP32-S3-WROOM-1U-N16R8")
            .map(String::as_str),
        Some("N16R8")
    );

    // The S31-WROOM-3 names only its module and maker: no other line.
    let g = Module::generate(&request(
        Scope::Package,
        &preset("module-esp32-s31-wroom-3"),
    ))?;
    let svg = g
        .marking
        .and_then(|m| m.artwork.first().map(|a| a.svg.clone()))
        .unwrap_or_default();
    assert!(svg.contains(">ESP32-S31-WROOM-3</text>"), "{svg}");
    for gone in [
        "FCC ID",
        "IC:",
        "CMIIT",
        "data-code",
        "{code}",
        "201-",
        "ce-mark",
    ] {
        assert!(!svg.contains(gone), "{gone} in {svg}");
    }
    Ok(())
}
