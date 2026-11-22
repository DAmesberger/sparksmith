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

fn text(key: &str, value: &str) -> (String, ParamValue) {
    (key.into(), ParamValue::Text(value.into()))
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
    let checked = golden::check::<Molded>(include_str!("../golden.json"))?;
    assert!(checked >= 4);
    Ok(())
}

#[test]
fn presets_are_the_starter_rows_and_all_generate() -> Result<(), Vec<Problem>> {
    let description = description();
    assert_eq!(description.presets().len(), 13);
    for p in description.presets() {
        for scope in [Scope::Component, Scope::Package] {
            let g = Molded::generate(&request(scope, &preset(&p.key)))?;
            assert_eq!(g.footprint.pads.len(), 2, "{}", p.key);
            assert_eq!(g.package.geometry.leads.len(), 2, "{}", p.key);
            assert_eq!(scope == Scope::Component, g.component.is_some());
        }
    }
    Ok(())
}

#[test]
fn sma_lands_and_leads_follow_the_terminals() -> Result<(), Vec<Problem>> {
    // SMA: span 5.2, terminal 1.1 × 1.4 mm: contacts at ±2.05 mm, lands
    // 1.7 × 1.6 mm at ±2.15 mm, corners 0.4 mm (a quarter of 1.6).
    let g = Molded::generate(&request(Scope::Component, &preset("molded-sma")))?;
    let pads: Vec<_> = g
        .footprint
        .pads
        .iter()
        .map(|p| (p.key.as_str(), p.at, p.shape.clone()))
        .collect();
    let shape = PadShape::RoundRect {
        size: [1_700_000, 1_600_000],
        radius_nm: 400_000,
    };
    assert_eq!(
        pads,
        vec![
            ("1", [-2_150_000, 0], shape.clone()),
            ("2", [2_150_000, 0], shape)
        ]
    );
    let leads: Vec<[i64; 3]> = g.package.geometry.leads.iter().map(|l| l.contact).collect();
    assert_eq!(leads, vec![[-2_050_000, 0, 0], [2_050_000, 0, 0]]);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [5_200_000, 2_600_000, 2_200_000]
    );
    // The pin-1 dot: 2.15 + 0.85 + 0.05 + 0.1 + 0.4 = 3.55 mm left of
    // the centre.
    let dot = g.footprint.silk.iter().find(|s| s.key == "pin1");
    assert_eq!(
        dot.map(|d| d.shape.clone()),
        Some(Primitive::Circle {
            center: [-3_550_000, 0],
            radius_nm: 200_000
        })
    );
    // A diode: K is terminal 1 (the band end), A terminal 2.
    let names: Vec<String> = g
        .component
        .iter()
        .flat_map(|c| c.terminals.iter().map(|t| t.name.clone()))
        .collect();
    assert_eq!(names, ["K", "A"]);
    // J-leads 0.15 mm thick, out at half the height, bent at the lead
    // thickness; the banded J-lead file.
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MOLDED_J_LEAD_BAND);
    assert_eq!(
        [
            int(&v, "lead_thickness"),
            int(&v, "lead_exit"),
            int(&v, "lead_root"),
            int(&v, "bend_1"),
            int(&v, "bend_2"),
        ],
        [150_000, 1_100_000, 250_000, 150_000, 150_000]
    );
    // Rounds: corners min(0.2, 0.08 × 2.6, 0.064 × 4.3, 0.6 / 2) = 0.2;
    // top edges min(0.15, 0.264, 0.208, (2.2 − 1.175) / 2, 0.2) = 0.15.
    assert_eq!(
        [int(&v, "corner_round"), int(&v, "edge_round")],
        [200_000, 150_000]
    );
    Ok(())
}

#[test]
fn a_tantalum_capacitor_is_plus_at_terminal_one() -> Result<(), Vec<Problem>> {
    let g = Molded::generate(&request(Scope::Component, &preset("molded-tantalum-b")))?;
    let c = g.component.as_ref();
    assert_eq!(c.map(|c| c.prefix.as_str()), Some("C"));
    assert_eq!(
        c.map(|c| c
            .terminals
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()),
        Some(vec!["1", "2"])
    );
    // The curved plate's ends lie on its circle.
    let arc = g
        .symbol
        .iter()
        .flat_map(|s| &s.units)
        .flat_map(|u| &u.graphics)
        .find_map(|g| match g.shape {
            Primitive::Arc { start, end, .. } => Some((start, end)),
            _ => None,
        });
    assert_eq!(arc, Some(([1_214_643, 1_750_000], [1_214_643, -1_750_000])));
    Ok(())
}

#[test]
fn bent_leads_bend_at_their_thickness_where_the_slope_allows() -> Result<(), Vec<Problem>> {
    // SOD-123 with bent leads and a 0.3 mm foot (the core's typical form):
    // standoff 0.05, exit 0.575, shoulder 1.35 + 0.3 × 0.2 = 1.41 mm.
    let g = Molded::generate(&request(
        Scope::Package,
        &[text("lead", "bent"), kv("terminal_length", 300_000)],
    ))?;
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MOLDED_BENT_BAND);
    assert_eq!(
        [
            int(&v, "body_bottom"),
            int(&v, "lead_exit"),
            int(&v, "lead_shoulder"),
            int(&v, "bend_1"),
            int(&v, "bend_2"),
        ],
        [50_000, 575_000, 1_410_000, 120_000, 120_000]
    );
    // A short foot tightens the second bend: 0.45 × 0.13 / tan(turn / 2),
    // tan(turn / 2) = 0.515 / (0.5765 + 0.259), about 0.0949 mm.
    let g = Molded::generate(&request(
        Scope::Package,
        &[text("lead", "bent"), kv("terminal_length", 130_000)],
    ))?;
    let (_, v) = values(&g);
    assert_eq!(int(&v, "bend_1"), 120_000);
    assert!((94_000..96_000).contains(&int(&v, "bend_2")));
    Ok(())
}

#[test]
fn melf_and_unbanded_parts_name_their_files() -> Result<(), Vec<Problem>> {
    let g = Molded::generate(&request(Scope::Package, &preset("molded-melf")))?;
    let (asset, v) = values(&g);
    assert_eq!(asset, parts::MELF_BAND);
    // MELF 5.0 × 2.5, caps 0.5 long: round min(0.08, 0.075, 0.125, 0.25).
    assert_eq!(int(&v, "cap_round"), 75_000);
    assert_eq!(
        g.package.geometry.envelope_nm,
        [5_000_000, 2_500_000, 2_500_000]
    );
    let mut plain = preset("molded-melf");
    plain.push(("polarised".into(), ParamValue::Bool(false)));
    let (asset, _) = values(&Molded::generate(&request(Scope::Package, &plain))?);
    assert_eq!(asset, parts::MELF);
    let mut plain = preset("molded-sod-523");
    plain.push(("polarised".into(), ParamValue::Bool(false)));
    let (asset, v) = values(&Molded::generate(&request(Scope::Package, &plain))?);
    assert_eq!(asset, parts::MOLDED_FLAT);
    // SOD-523: the 0.2 mm contact is shorter than the 0.2 + 0.2 mm out to
    // under the body, so the lead reaches 0.4 mm in from the tip.
    assert_eq!(int(&v, "lead_length"), 400_000);
    Ok(())
}

#[test]
fn inconsistent_values_are_refused() {
    let refused = |overrides: &[(String, ParamValue)]| {
        Molded::generate(&request(Scope::Package, overrides))
            .err()
            .and_then(|p| p.first().map(|p| (p.code.clone(), p.field.clone())))
    };
    assert_eq!(
        refused(&[kv("terminal_width", 2_000_000)]),
        Some(("param.inconsistent".into(), Some("terminal_width".into())))
    );
    assert_eq!(
        refused(&[kv("terminal_length", 2_000_000)]),
        Some(("param.inconsistent".into(), Some("terminal_length".into())))
    );
    // A MELF body must be round.
    assert_eq!(
        refused(&[text("lead", "melf")]),
        Some(("param.inconsistent".into(), Some("body_height".into())))
    );
}

/// A tantalum capacitor carries its value code by default (the pF code and
/// the voltage letter, Kyocera AVX TAJ), at its preset's text height;
/// `none` leaves it unmarked, lines given replace it, and a diode carries
/// none.
#[test]
fn a_tantalum_capacitor_marks_its_value_code() -> Result<(), Vec<Problem>> {
    let line = "{field:Capacitance|cap3} {field:Rated voltage|letter:tantalum}";
    let g = Molded::generate(&request(Scope::Component, &preset("molded-tantalum-b")))?;
    let m = g.marking.as_ref();
    assert_eq!(m.map(|m| m.lines.clone()), Some(vec![line.to_string()]));
    assert_eq!(m.and_then(|m| m.text_height), Some(700_000));

    let mut none = preset("molded-tantalum-b");
    none.push(text("value_code", "none"));
    let g = Molded::generate(&request(Scope::Component, &none))?;
    assert!(g.marking.is_none());

    let mut given = preset("molded-tantalum-b");
    given.push((
        "marking".into(),
        ParamValue::List(vec![ParamValue::Text("{mpn}".into())]),
    ));
    let g = Molded::generate(&request(Scope::Component, &given))?;
    assert_eq!(
        g.marking.map(|m| m.lines),
        Some(vec![String::from("{mpn}")])
    );

    let g = Molded::generate(&request(Scope::Component, &preset("molded-minimelf")))?;
    assert!(g.marking.is_none());
    // A diode keeps its preset's lines.
    let g = Molded::generate(&request(Scope::Component, &preset("molded-sma")))?;
    assert_eq!(
        g.marking.map(|m| m.lines),
        Some(vec![String::from("{mpn}")])
    );
    Ok(())
}
