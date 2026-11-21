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

/// The distance, nm, from the origin to the line through `a` and `b`
/// (an edge of the clear-area polygon), as an f64 for the test only.
fn edge_distance(a: Point, b: Point) -> f64 {
    let (ax, ay, bx, by) = (a[0] as f64, a[1] as f64, b[0] as f64, b[1] as f64);
    ((bx - ax) * ay - (by - ay) * ax).abs() / (bx - ax).hypot(by - ay)
}

#[test]
fn golden_hashes_match_natively() -> Result<(), String> {
    let checked = golden::check::<Fiducial>(include_str!("../golden.json"))?;
    assert!(checked >= 2);
    Ok(())
}

#[test]
fn every_preset_generates() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 4);
    for p in description.presets() {
        let overrides: Vec<(String, ParamValue)> = p.params.clone().into_iter().collect();
        let g = Fiducial::generate(&request(Scope::Component, &overrides))?;
        let both = p.params.get("sides") == Some(&ParamValue::Text("both".into()));
        assert_eq!(
            g.footprint.pads.len(),
            if both { 2 } else { 1 },
            "{}",
            p.key
        );
    }
    Ok(())
}

/// The default (JLCPCB's): a Ø1.0 mm fiducial land in a Ø2.0 mm opening
/// (0.5 mm expansion on every side), no paste, landing nothing; the clear
/// area a 32-gon round the Ø2.0 mm circle, as courtyard and as a keep-out
/// on the part's own side only; no silkscreen, no terminals, no pin 1.
#[test]
fn the_default_is_a_1_mm_dot_in_a_2_mm_clear_area() -> Result<(), Vec<Problem>> {
    let g = Fiducial::generate(&request(Scope::Component, &[]))?;
    assert_eq!(g.footprint.pads.len(), 1);
    let pad = &g.footprint.pads[0];
    assert_eq!(
        pad.shape,
        PadShape::Circle {
            diameter_nm: 1_000_000
        }
    );
    assert_eq!(pad.kind, Some(PadKind::Fiducial));
    assert_eq!(pad.terminal, "");
    assert_eq!(pad.side, Side::Top);
    assert_eq!(pad.mask, Opening::Expansion(500_000));
    assert_eq!(pad.paste, Opening::None);
    assert!(pad.hole.is_none());
    let area = &g.footprint.courtyard;
    assert_eq!(area.len(), 32);
    // Every edge touches the Ø2.0 mm circle from outside, within 2 nm.
    for (i, a) in area.iter().enumerate() {
        let b = area[(i + 1) % area.len()];
        let d = edge_distance(*a, b);
        assert!((1_000_000.0..=1_000_002.0).contains(&d), "edge {i}: {d}");
    }
    assert_eq!(
        g.footprint.keepouts,
        vec![Keepout {
            name: "clear area".into(),
            outline: area.clone(),
            side: Some(Side::Top),
        }]
    );
    assert!(g.footprint.silk.is_empty());
    assert!(g.package.terminals.is_empty());
    assert!(g.package.geometry.leads.is_empty());
    assert_eq!(g.package.geometry.pin1, "");
    assert_eq!(
        g.package.geometry.envelope_nm,
        [1_000_000, 1_000_000, 35_000]
    );
    let component = g
        .component
        .as_ref()
        .map(|c| (c.prefix.as_str(), c.terminals.len()));
    assert_eq!(component, Some(("FID", 0)));
    assert!(g.binding.is_empty());
    let mut values = v1::Params::new();
    values.insert("diameter".into(), ParamValue::Int(1_000_000));
    values.insert("thickness".into(), ParamValue::Int(35_000));
    assert_eq!(
        g.model,
        Some(v1::Model::PartFile {
            asset: parts::FIDUCIAL_DOT.into(),
            values,
        })
    );
    Ok(())
}

#[test]
fn both_sides_adds_a_far_side_dot_and_its_clear_area() -> Result<(), Vec<Problem>> {
    let both = ("sides".to_string(), ParamValue::Text("both".into()));
    let g = Fiducial::generate(&request(Scope::Package, &[both]))?;
    let sides: Vec<Side> = g.footprint.pads.iter().map(|p| p.side).collect();
    assert_eq!(sides, vec![Side::Top, Side::Bottom]);
    assert!(
        g.footprint
            .pads
            .iter()
            .all(|p| p.kind == Some(PadKind::Fiducial) && p.at == [0, 0])
    );
    let kept: Vec<Option<Side>> = g.footprint.keepouts.iter().map(|k| k.side).collect();
    assert_eq!(kept, vec![Some(Side::Top), Some(Side::Bottom)]);
    assert!(g.component.is_none() && g.symbol.is_none());
    Ok(())
}

#[test]
fn a_3r_clear_area_reaches_1_5_mm() -> Result<(), Vec<Problem>> {
    let g = Fiducial::generate(&request(
        Scope::Package,
        &[
            kv("mask_diameter", 3_000_000),
            kv("clear_diameter", 3_000_000),
        ],
    ))?;
    let area = &g.footprint.courtyard;
    let d = edge_distance(area[0], area[1]);
    assert!((1_500_000.0..=1_500_002.0).contains(&d), "{d}");
    assert_eq!(
        g.footprint.pads.first().map(|p| p.mask.clone()),
        Some(Opening::Expansion(1_000_000))
    );
    Ok(())
}

#[test]
fn an_opening_not_wider_than_the_dot_or_a_clear_area_inside_it_is_refused() {
    let fields = |overrides: &[(String, ParamValue)]| -> Vec<String> {
        match Fiducial::generate(&request(Scope::Package, overrides)) {
            Ok(_) => Vec::new(),
            Err(problems) => problems.into_iter().filter_map(|p| p.field).collect(),
        }
    };
    assert_eq!(
        fields(&[kv("mask_diameter", 1_000_000)]),
        vec!["mask_diameter".to_string()]
    );
    assert_eq!(
        fields(&[kv("clear_diameter", 1_500_000)]),
        vec!["clear_diameter".to_string()]
    );
    assert!(fields(&[kv("clear_diameter", 3_000_000)]).is_empty());
}
