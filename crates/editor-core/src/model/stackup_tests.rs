use super::*;

#[test]
fn every_even_count_up_to_the_bound_has_a_symmetric_preset_that_passes_the_rules() -> Result<()> {
    let counts: Vec<usize> = Stackup::preset_counts().collect();
    assert_eq!(counts.first(), Some(&MIN_COPPER_LAYERS));
    assert_eq!(counts.last(), Some(&MAX_COPPER_LAYERS));
    assert_eq!(counts.len(), 32);
    for count in counts {
        let s = Stackup::preset(count)?;
        s.check()?;
        assert_eq!(s.copper.len(), count);
        assert_eq!(s.layers().len(), count);
        assert_eq!(s.dielectrics.len(), count - 1);
        // Symmetric about the middle.
        let flipped: Vec<_> = s.dielectrics.iter().rev().copied().collect();
        assert_eq!(s.dielectrics, flipped, "{count} layers");
        if count > 2 {
            // Foil build: prepreg outside.
            assert_eq!(
                s.dielectrics.first().map(|d| d.kind),
                Some(DielectricKind::Prepreg)
            );
        }
        // A multilayer preset finishes at 1.6 mm with its foils (within
        // half a µm a core, as the cores are rounded to the µm), up to ten
        // layers (e2e run 13,
        // finding 2: the 4-layer preset was 1.74 mm).
        let finished = s.thickness_mm();
        if count <= 10 {
            assert!((finished - 1.6).abs() < 0.0025, "{count}: {finished}");
        } else {
            assert!(finished > 1.6, "{count}: {finished}");
        }
    }
    // The common presets keep their builds.
    assert_eq!(Stackup::preset(2)?, Stackup::default());
    let four = Stackup::preset(4)?;
    let thick: Vec<f64> = four.dielectrics.iter().map(|d| d.thickness_mm).collect();
    // 1.6 mm less four 35 µm foils and two 0.2 mm prepregs: a 1.06 mm core.
    assert_eq!(thick, vec![0.2, 1.06, 0.2]);
    assert!((four.thickness_mm() - 1.6).abs() < 1e-9);
    Ok(())
}

#[test]
fn the_rules_bound_the_count_require_pairs_and_one_dielectric_between_foils() -> Result<()> {
    for count in [0, 1, 3, 5, 63, 65, 66, 1000] {
        assert!(Stackup::preset(count).is_err(), "{count}");
    }
    let mut odd = Stackup::preset(6)?;
    odd.copper.pop();
    odd.dielectrics.pop();
    let refused = odd.check();
    assert!(
        matches!(&refused, Err(Error::Invalid(m)) if m.contains("even")),
        "{refused:?}"
    );
    let mut big = Stackup::preset(MAX_COPPER_LAYERS)?;
    big.check()?;
    let dielectric = big.dielectrics[0];
    big.copper.extend([CopperFoil::of_um(35.0); 2]);
    big.dielectrics.extend([dielectric; 2]);
    assert!(matches!(big.check(), Err(Error::Invalid(m)) if m.contains("at most 64")));
    let mut gap = Stackup::preset(8)?;
    gap.dielectrics.pop();
    assert!(matches!(gap.check(), Err(Error::Invalid(m)) if m.contains("one dielectric")));
    let mut thin = Stackup::preset(8)?;
    thin.dielectrics[3].thickness_mm = 0.01;
    assert!(thin.check().is_err());
    let mut dk = Stackup::preset(8)?;
    dk.dielectrics[3].dk = Some(f64::NAN);
    assert!(dk.check().is_err());
    let mut heavy = Stackup::preset(8)?;
    heavy.copper[5].thickness_um = 400.0;
    assert!(heavy.check().is_err());
    Ok(())
}

#[test]
fn a_many_layer_stackup_names_and_places_every_layer() -> Result<()> {
    let s = Stackup::preset(MAX_COPPER_LAYERS)?;
    let layers = s.layers();
    assert_eq!(layers[0], CopperLayer::Top);
    assert_eq!(layers[62], CopperLayer::Inner(62));
    assert_eq!(layers[63], CopperLayer::Bottom);
    assert!(s.has(CopperLayer::Inner(62)));
    assert!(!s.has(CopperLayer::Inner(63)));
    // Each layer lies below the one above it.
    let mut above = f64::INFINITY;
    for layer in layers {
        let (low, high) = s
            .copper_span(layer)
            .ok_or_else(|| Error::Invalid(format!("no span for {layer:?}")))?;
        assert!(high <= above + 1e-9 && low < high, "{layer:?}");
        above = low;
    }
    Ok(())
}

#[test]
fn a_stackup_records_the_wall_source_and_refuses_a_missing_physical_model() -> Result<()> {
    let mut stackup = Stackup::preset(4)?;
    let assumed = stackup.plating()?;
    assert!(assumed.assumed);
    assert_eq!(assumed.wall_nm.get(), 25_000);
    assert!(assumed.source.contains("IPC-6012"));
    let measured = BarrelPlating {
        wall_nm: Nm::new(36_000)?,
        source: "Original test coupon section".into(),
        assumed: false,
    };
    stackup.barrel_plating = Some(measured.clone());
    let snapshot = serde_json::to_value(&stackup).map_err(|e| Error::Invalid(e.to_string()))?;
    assert_eq!(snapshot["barrel_plating"]["wall_nm"], 36_000);
    let reopened: Stackup =
        serde_json::from_value(snapshot).map_err(|e| Error::Invalid(e.to_string()))?;
    assert_eq!(reopened.plating()?, &measured);
    stackup.barrel_plating = None;
    assert!(stackup.check().is_err());
    assert_eq!(stackup.keeping_unstated(&reopened).plating()?, &measured);
    let mut unsupported = reopened;
    if let Some(plating) = &mut unsupported.barrel_plating {
        plating.source.clear();
    }
    assert!(unsupported.check().is_err());
    Ok(())
}
