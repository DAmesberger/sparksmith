use super::*;

/// A test length in mm, as stored.
pub(crate) fn nm(mm: f64) -> sparksmith_core::units::Nm {
    sparksmith_core::units::Nm::from_mm_f64_rounded(mm).unwrap_or_default()
}

/// A test point or size in mm, as stored.
pub(crate) fn pt(p: [f64; 2]) -> sparksmith_core::units::PointNm {
    sparksmith_core::units::PointNm::from_mm_f64_rounded(p).unwrap_or_default()
}

#[test]
fn metadata_chunks_round_trip_and_bound_their_size() -> Result<()> {
    let mut native = Native::new("Package model")?;
    let definition = ModelDefinition {
        component: "pkg1r1".into(),
        bodies: (0..32)
            .map(|i| GeneratedBody {
                name: format!("pkg1r1 lead ({i})"),
                role: BodyRole::Lead,
                material: MaterialId::TinMatte,
                terminal: Some(i + 1),
            })
            .collect(),
    };
    record(&mut native, std::slice::from_ref(&definition))?;
    let back = recorded(&native)?;
    assert_eq!(back.len(), definition.bodies.len());
    for body in &definition.bodies {
        assert_eq!(back.get(&body.name), Some(body));
    }
    // Many bodies split into chunks below the limit; too many are refused.
    let big = ModelDefinition {
        component: "pkgbig".into(),
        bodies: (0..4000)
            .map(|i| GeneratedBody {
                name: format!("pkgbig lead ({i})"),
                role: BodyRole::Lead,
                material: MaterialId::TinMatte,
                terminal: Some(i),
            })
            .collect(),
    };
    let split = chunks(std::slice::from_ref(&big))?;
    assert!(split.len() > 1);
    for chunk in &split {
        let stored = serde_json::to_string(&serde_json::json!({ "bodies": chunk }))?;
        assert!(stored.len() <= CHUNK_BYTES);
    }
    let huge = vec![big; 60];
    assert!(chunks(&huge).is_err());
    Ok(())
}
