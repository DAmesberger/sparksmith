//! Native tests: the golden hashes, hand-derived geometry, the cut-outs
//! of depopulated grids and refusals.
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

fn text(key: &str, value: &str) -> (String, ParamValue) {
    (key.into(), ParamValue::Text(value.into()))
}

fn empty(names: &[&str]) -> (String, ParamValue) {
    (
        "depopulated".into(),
        ParamValue::List(
            names
                .iter()
                .map(|n| ParamValue::Text((*n).into()))
                .collect(),
        ),
    )
}

/// A 10 × 10 ball grid at 0.5 mm on a 5.5 × 5.5 mm body.
fn ten_by_ten() -> Vec<(String, ParamValue)> {
    vec![
        kv("rows", 10),
        kv("columns", 10),
        kv("pitch", 500_000),
        kv("row_pitch", 500_000),
        kv("body_length", 5_500_000),
        kv("body_width", 5_500_000),
        kv("body_height", 600_000),
        kv("contact_diameter", 300_000),
        kv("contact_height", 200_000),
    ]
}

fn values(g: &Generation) -> (&str, &v1::Params) {
    match &g.model {
        Some(v1::Model::PartFile { asset, values }) => (asset.as_str(), values),
        _ => ("", &EMPTY),
    }
}

static EMPTY: v1::Params = v1::Params::new();

fn int(values: &v1::Params, key: &str) -> i64 {
    match values.get(key) {
        Some(ParamValue::Int(v)) => *v,
        _ => i64::MIN,
    }
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Grid>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 14);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Grid::generate(&request(scope, &preset(&p.key)))?;
            let pads = g.footprint.pads.len();
            assert_eq!(g.package.geometry.leads.len(), pads, "{}", p.key);
            assert_eq!(g.package.terminals.len(), pads, "{}", p.key);
            assert_eq!(g.package.terminals.first().map(String::as_str), Some("A1"));
            // Every starter row is a full matrix: its part file has no
            // cut-outs.
            let (asset, _) = values(&g);
            assert!(asset == parts::GRID_BALL || asset == parts::GRID_LAND);
        }
    }
    Ok(())
}

#[test]
fn bga64_lands_are_lettered_from_a1_at_minus_x_plus_y() -> Result<(), Vec<Problem>> {
    // 8 × 8 at 0.8 mm: A1 at (−2.8, 2.8) mm, H8 at (2.8, −2.8); NSMD lands
    // of 0.8 × 0.45 = 0.36 mm.
    let g = Grid::generate(&request(
        Scope::Component,
        &preset("grid-bga-64-0.8-8x8-7x7"),
    ))?;
    let pads = &g.footprint.pads;
    assert_eq!(pads.len(), 64);
    let at = |k: usize| pads.get(k).map(|p| (p.key.as_str(), p.at, p.shape.clone()));
    let land = PadShape::Circle {
        diameter_nm: 360_000,
    };
    assert_eq!(at(0), Some(("A1", [-2_800_000, 2_800_000], land.clone())));
    assert_eq!(at(7), Some(("A8", [2_800_000, 2_800_000], land.clone())));
    assert_eq!(at(8), Some(("B1", [-2_800_000, 2_000_000], land.clone())));
    assert_eq!(at(63), Some(("H8", [2_800_000, -2_800_000], land)));
    assert_eq!(
        g.package.geometry.envelope_nm,
        [7_000_000, 7_000_000, 1_300_000]
    );
    assert_eq!(g.package.geometry.pin1, "A1");
    // The pin-1 dot beyond A1's −x edge, past its 0.05 mm mask, 0.1 mm
    // clearance and the dot's 0.4 mm diameter.
    assert!(g.footprint.silk.iter().any(|s| s.shape
        == Primitive::Circle {
            center: [-2_800_000 - 180_000 - 550_000, 2_800_000],
            radius_nm: 200_000
        }));
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::GRID_BALL);
    assert_eq!(int(v, "contact_height"), 300_000);
    // 6 % of 7 mm is 0.42 mm, held to 0.4 mm.
    assert_eq!(int(v, "mark_radius"), 400_000);
    assert_eq!(int(v, "edge_round"), 50_000);
    Ok(())
}

#[test]
fn a_land_grid_has_its_own_land_and_a_thin_land_in_3d() -> Result<(), Vec<Problem>> {
    let g = Grid::generate(&request(
        Scope::Package,
        &preset("grid-lga-64-0.65-8x8-6x6"),
    ))?;
    assert_eq!(
        g.footprint.pads.first().map(|p| p.shape.clone()),
        Some(PadShape::Circle {
            diameter_nm: 350_000
        })
    );
    assert_eq!(
        g.package.geometry.envelope_nm,
        [6_000_000, 6_000_000, 900_000]
    );
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::GRID_LAND);
    assert_eq!(int(v, "contact_height"), 30_000);
    Ok(())
}

#[test]
fn rows_past_the_alphabet_run_aa_ab() -> Result<(), Vec<Problem>> {
    let g = Grid::generate(&request(
        Scope::Package,
        &preset("grid-bga-484-1-22x22-23x23"),
    ))?;
    let names = &g.package.terminals;
    assert_eq!(names.len(), 484);
    // JEDEC skips I: row 9 is J; row 20 is Y, then AA and AB.
    assert_eq!(names.get(8 * 22).map(String::as_str), Some("J1"));
    assert_eq!(names.get(19 * 22).map(String::as_str), Some("Y1"));
    assert_eq!(names.get(20 * 22).map(String::as_str), Some("AA1"));
    assert_eq!(names.last().map(String::as_str), Some("AB22"));
    Ok(())
}

#[test]
fn empty_positions_keep_the_other_names_and_are_cut_out() -> Result<(), Vec<Problem>> {
    // A centre void of 2 × 2 (E5, E6, F5, F6) and A2 empty.
    let mut over = ten_by_ten();
    over.push(empty(&["e5", "E6", "F5", "F6", "A2"]));
    let g = Grid::generate(&request(Scope::Component, &over))?;
    let names = &g.package.terminals;
    assert_eq!(names.len(), 95);
    assert_eq!(&names[..2], ["A1", "A3"]);
    assert!(!names.iter().any(|n| n == "E5") && names.iter().any(|n| n == "E4"));
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::GRID_BALL_VOIDS);
    // Two cut-outs: A2 first (lead order), then E5–F6; the other fourteen
    // repeat the first. Columns at 0.5 mm from −2.25 mm, rows from
    // +2.25 mm; each cut-out reaches half a pitch past its contacts.
    let cut = |k: usize| {
        [
            int(v, &format!("void_{k}_x")),
            int(v, &format!("void_{k}_y")),
            int(v, &format!("void_{k}_length")),
            int(v, &format!("void_{k}_width")),
        ]
    };
    assert_eq!(cut(1), [-2_000_000, 2_000_000, 500_000, 500_000]);
    assert_eq!(cut(2), [-500_000, -500_000, 1_000_000, 1_000_000]);
    for k in 3..=MAX_VOIDS {
        assert_eq!(cut(k), cut(1), "cut-out {k}");
    }
    assert!(g.notes.iter().any(|n| n.contains("2 rectangular cut-outs")));
    Ok(())
}

#[test]
fn a_ring_round_a_centre_island_takes_four_cut_outs() -> Result<(), Vec<Problem>> {
    // Rows C–H, columns 3–8 empty but the 2 × 2 island E5–F6.
    let mut names = Vec::new();
    for r in ["C", "D", "E", "F", "G", "H"] {
        for c in 3..=8 {
            let island = matches!(r, "E" | "F") && (5..=6).contains(&c);
            if !island {
                names.push(format!("{r}{c}"));
            }
        }
    }
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut over = ten_by_ten();
    over.push(empty(&refs));
    let g = Grid::generate(&request(Scope::Package, &over))?;
    assert_eq!(g.package.terminals.len(), 100 - 32);
    assert!(g.notes.iter().any(|n| n.contains("4 rectangular cut-outs")));
    // A land grid's cut-out (its body is filled again there) stays inside
    // the body: a channel of empty columns 4–6 down the whole grid ends
    // half a pitch past the outer rows.
    let mut channel = Vec::new();
    for r in ["A", "B", "C", "D", "E", "F", "G", "H", "J", "K"] {
        for c in 4..=6 {
            channel.push(format!("{r}{c}"));
        }
    }
    let refs: Vec<&str> = channel.iter().map(String::as_str).collect();
    let mut over = ten_by_ten();
    over.push(kv("contact_height", 0));
    over.push(text("contact", "land"));
    over.push(empty(&refs));
    let g = Grid::generate(&request(Scope::Package, &over))?;
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::GRID_LAND_VOIDS);
    assert_eq!(int(v, "void_1_y"), -2_500_000);
    assert_eq!(int(v, "void_1_width"), 5_000_000);
    assert_eq!(int(v, "void_1_x"), -1_000_000);
    assert_eq!(int(v, "void_1_length"), 1_500_000);
    Ok(())
}

#[test]
fn package_scope_has_no_component_symbol_or_binding() -> Result<(), Vec<Problem>> {
    let g = Grid::generate(&request(Scope::Package, &[]))?;
    assert!(g.component.is_none() && g.symbol.is_none() && g.binding.is_empty());
    assert_eq!(g.footprint.pads.len(), 20);
    Ok(())
}

#[test]
fn refusals_name_the_field_in_plain_words() {
    let found = |over: &[(String, ParamValue)]| -> Vec<(String, Option<String>, String)> {
        Grid::generate(&request(Scope::Component, over))
            .err()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.code, p.field, p.message))
            .collect()
    };
    assert_eq!(
        found(&[kv("contact_diameter", 800_000)]),
        vec![(
            "param.inconsistent".into(),
            Some("contact_diameter".into()),
            "Contacts 0.8 mm across touch at a pitch of 0.8 × 0.8 mm.".into()
        )]
    );
    assert_eq!(
        found(&[empty(&["A1", "A1"])]),
        vec![(
            "param.inconsistent".into(),
            Some("depopulated".into()),
            "The empty position A1 is listed twice.".into()
        )]
    );
    assert_eq!(
        found(&[empty(&["I1"])]),
        vec![(
            "param.inconsistent".into(),
            Some("depopulated".into()),
            "I1 names no position of the 4 × 5 grid.".into()
        )]
    );
    assert_eq!(
        found(&[text("contact", "land")]),
        vec![(
            "param.inconsistent".into(),
            Some("contact_height".into()),
            "A land grid has no ball height (0, is 0.3 mm).".into()
        )]
    );
    // A checkerboard of empty positions needs a cut-out each.
    let mut board = Vec::new();
    for (r, row) in ["A", "B", "C", "D", "E", "F", "G", "H", "J", "K"]
        .iter()
        .enumerate()
    {
        for c in 1..=10 {
            if (r + c) % 2 == 0 {
                board.push(format!("{row}{c}"));
            }
        }
    }
    let refs: Vec<&str> = board.iter().map(String::as_str).collect();
    let mut over = ten_by_ten();
    over.push(empty(&refs));
    let refused = found(&over);
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0].1.as_deref(), Some("depopulated"));
    assert!(refused[0].2.contains("need 50 rectangular cut-outs"));
    // 40 × 26 positions are more than a part file's pattern holds.
    let codes: Vec<Option<String>> = found(&[
        kv("rows", 26),
        kv("columns", 40),
        kv("body_length", 40 * MM),
        kv("body_width", 30 * MM),
    ])
    .into_iter()
    .map(|p| p.1)
    .collect();
    assert!(codes.contains(&Some("rows".into())), "{codes:?}");
}

#[test]
fn every_preset_names_its_part_file_with_every_value() -> Result<(), String> {
    for p in description().presets() {
        let g = Grid::generate(&request(Scope::Package, &preset(&p.key)))
            .map_err(|e| format!("{e:?}"))?;
        let (_, values) = values(&g);
        let names: Vec<&str> = values.keys().map(String::as_str).collect();
        assert_eq!(
            names,
            vec![
                "body_height",
                "body_length",
                "body_width",
                "columns",
                "contact_diameter",
                "contact_height",
                "edge_round",
                "mark_depth",
                "mark_radius",
                "pitch",
                "row_pitch",
                "rows"
            ],
            "{}",
            p.key
        );
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
        ("grid-ball", parts::GRID_BALL),
        ("grid-ball-voids", parts::GRID_BALL_VOIDS),
        ("grid-land", parts::GRID_LAND),
        ("grid-land-voids", parts::GRID_LAND_VOIDS),
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
