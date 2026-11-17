//! Small STEP assemblies composed with the existing Solidcraft import and
//! transform features. Used by interchange hosts when a package names several
//! files or a source needs nonuniform scale. No private CAD writer or kernel.
use super::imported;
use crate::native::Native;
use crate::{Error, Result};
use serde_json::json;
use sparksmith_core::library::model3d::{ModelFormat, SourceFrame};

pub struct Instance<'a> {
    pub bytes: &'a [u8],
    pub scale: [f64; 3],
    /// Intrinsic X/Y/Z (Rx Ry Rz), in ordinary right-handed degrees.
    pub rotation_deg: [f64; 3],
    pub offset_mm: [f64; 3],
}

pub fn compose(instances: &[Instance<'_>]) -> Result<Vec<u8>> {
    compose_until(instances, None)
}

/// Cooperative deadline: checked around CAD calls, not an interrupt of a parser.
pub fn compose_until(
    instances: &[Instance<'_>],
    deadline: Option<std::time::Instant>,
) -> Result<Vec<u8>> {
    let check = || -> Result<()> {
        if deadline.is_some_and(|d| std::time::Instant::now() >= d) {
            Err(Error::Invalid(
                "STEP assembly reached the import time limit".into(),
            ))
        } else {
            Ok(())
        }
    };
    check()?;
    if instances.is_empty() || instances.len() > 32 {
        return Err(Error::Invalid("STEP assembly needs 1–32 instances".into()));
    }
    let mut native = Native::new("project model")?;
    let mut all = Vec::new();
    let mut sources: std::collections::BTreeMap<_, imported::Source> =
        std::collections::BTreeMap::new();
    for (index, instance) in instances.iter().enumerate() {
        if instance
            .scale
            .iter()
            .any(|v| !v.is_finite() || !(1e-6..=1e6).contains(v))
            || instance.rotation_deg.iter().any(|v| !v.is_finite())
            || instance
                .offset_mm
                .iter()
                .any(|v| !v.is_finite() || v.abs() > 10_000.0)
        {
            return Err(Error::Invalid(
                "STEP assembly has an invalid transform".into(),
            ));
        }
        check()?;
        let key = sparksmith_core::library::bundle::asset_hash(instance.bytes);
        let source = if let Some(source) = sources.get(&key) {
            source.clone()
        } else {
            let source = imported::read_source(instance.bytes, ModelFormat::Step)?;
            sources.insert(key, source.clone());
            source
        };
        check()?;
        if !source.warnings.is_empty() {
            return Err(Error::Invalid(format!(
                "STEP assembly source reported incomplete geometry: {}",
                source.warnings.join("; ")
            )));
        }
        if all.len() + source.keys.len() > 1024 {
            return Err(Error::Invalid("STEP assembly exceeds 1024 bodies".into()));
        }
        let names: Vec<_> = (0..source.keys.len())
            .map(|n| format!("model{index}_body{n}"))
            .collect();
        imported::add_import(
            &mut native,
            None,
            &format!("model{index}"),
            source,
            (&SourceFrame::default(), ModelFormat::Step),
            &names,
        )?;
        check()?;
        if instance.scale != [1.0; 3] {
            native.call("solid.scale", json!({"bodies": names, "factors": instance.scale, "origin": [0,0,0], "name": format!("model{index} scale")}))?;
        }
        // Intrinsic XYZ means the point is turned about Z first, then Y, X.
        for axis in (0..3).rev() {
            check()?;
            if instance.rotation_deg[axis] != 0.0 {
                let mut direction = [0; 3];
                direction[axis] = 1;
                native.call("solid.move", json!({"bodies": names, "axis": direction, "angle": instance.rotation_deg[axis], "origin": [0,0,0], "name": format!("model{index} rotate{axis}")}))?;
            }
        }
        check()?;
        if instance.offset_mm != [0.0; 3] {
            native.call("solid.move", json!({"bodies": names, "translate": instance.offset_mm, "name": format!("model{index} offset")}))?;
        }
        all.extend(names);
        native.forget_history();
    }
    if let Some(error) = native.evaluation_error() {
        return Err(Error::Invalid(error));
    }
    check()?;
    let bytes = native.step_bytes(&all, "project model")?;
    check()?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::inspect;
    use crate::native::{BoxBody, Expr};
    fn source() -> Result<Vec<u8>> {
        let mut native = Native::new("synthetic box")?;
        native.solid_box(BoxBody {
            name: "box",
            corner: [0, 0, 0].map(Expr::from),
            size: [1, 2, 3].map(Expr::from),
        })?;
        native.look(
            &["box"],
            &crate::native::Look {
                name: "synthetic red",
                srgb8: [200, 50, 20],
                metallic: 0.0,
                roughness: 0.5,
                ior: 1.5,
            },
        )?;
        native.step_bytes(&["box".into()], "original synthetic box")
    }
    #[test]
    fn nonuniform_scale_clockwise_turn_and_offset_have_analytic_bounds() -> Result<()> {
        let source = source()?;
        // Box [0,1]x[0,2]x[0,3], scaled2x3x4, clockwiseZ90,
        // translated(10,20,30) -> [10,16]x[18,20]x[30,42].
        let bytes = compose(&[Instance {
            bytes: &source,
            scale: [2.0, 3.0, 4.0],
            rotation_deg: [0.0, 0.0, -90.0],
            offset_mm: [10.0, 20.0, 30.0],
        }])?;
        let seen = inspect::inspect(&bytes, ModelFormat::Step)?;
        for (actual, expected) in seen
            .bounds_mm
            .into_iter()
            .flatten()
            .zip([10.0, 18.0, 30.0, 16.0, 20.0, 42.0])
        {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
        assert_eq!(seen.bodies.len(), 1);
        let original = inspect::inspect(&source, ModelFormat::Step)?;
        assert!(original.bodies[0].colour.is_some());
        assert_eq!(seen.bodies[0].colour, original.bodies[0].colour);
        Ok(())
    }
    #[test]
    fn compound_rotations_use_intrinsic_xyz_order() -> Result<()> {
        let source = source()?;
        let bytes = compose(&[Instance {
            bytes: &source,
            scale: [2.0, 3.0, 4.0],
            rotation_deg: [-90.0, -90.0, -90.0],
            offset_mm: [10.0, 20.0, 30.0],
        }])?;
        let seen = inspect::inspect(&bytes, ModelFormat::Step)?;
        for (actual, expected) in seen
            .bounds_mm
            .into_iter()
            .flatten()
            .zip([-2.0, 20.0, 30.0, 10.0, 26.0, 32.0])
        {
            assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
        }
        Ok(())
    }

    #[test]
    fn incomplete_sources_refuse_before_composition_can_hide_the_warning() -> Result<()> {
        let mut native = Native::new("original partial source")?;
        for name in ["whole", "broken"] {
            native.solid_box(BoxBody {
                name,
                corner: [0, 0, 0].map(Expr::from),
                size: [1, 2, 3].map(Expr::from),
            })?;
        }
        let bytes = native.step_bytes(&["whole".into(), "broken".into()], "partial")?;
        let mut text = String::from_utf8(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        let start = text
            .find("MANIFOLD_SOLID_BREP(")
            .ok_or_else(|| Error::Invalid("synthetic solid missing".into()))?;
        let end = start
            + text[start..]
                .find(';')
                .ok_or_else(|| Error::Invalid("synthetic solid end missing".into()))?;
        text.replace_range(start..end, "MANIFOLD_SOLID_BREP('broken',#999999)");
        let partial = imported::read_source(text.as_bytes(), ModelFormat::Step)?;
        assert!(
            !partial.keys.is_empty(),
            "fixture must retain one valid body"
        );
        assert!(
            !partial.warnings.is_empty(),
            "fixture must produce a warning"
        );
        let error = compose(&[Instance {
            bytes: text.as_bytes(),
            scale: [2.0, 1.0, 1.0],
            rotation_deg: [0.0; 3],
            offset_mm: [0.0; 3],
        }])
        .err()
        .ok_or_else(|| Error::Invalid("partial source was accepted".into()))?;
        assert!(error.to_string().contains("incomplete geometry"), "{error}");
        Ok(())
    }

    #[test]
    fn a_multiple_reference_package_keeps_both_bodies() -> Result<()> {
        let source = source()?;
        let bytes = compose(&[
            Instance {
                bytes: &source,
                scale: [1.0; 3],
                rotation_deg: [0.0; 3],
                offset_mm: [0.0; 3],
            },
            Instance {
                bytes: &source,
                scale: [1.0; 3],
                rotation_deg: [0.0; 3],
                offset_mm: [5.0, 0.0, 0.0],
            },
        ])?;
        let seen = inspect::inspect(&bytes, ModelFormat::Step)?;
        assert_eq!(seen.bodies.len(), 2);
        assert!((seen.bounds_mm[1][0] - 6.0).abs() < 0.001);
        Ok(())
    }
}
