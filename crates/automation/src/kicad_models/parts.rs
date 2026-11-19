//! A source footprint's part made by the generator of the package its
//! lands are, through the one `library.generate` every generated part goes
//! through: the preset's parameters, with the generator's `land_pattern`
//! set to the source's own lands in the package frame. The board's copper
//! is therefore the source's, pad for pad; the body, symbol, fields and
//! marking are the generator's. A footprint the generator cannot take
//! (a land it has no terminal for, a terminal its pads put on two nets, a
//! land the pattern cannot carry) is refused here with the reason, and the
//! importer makes a project part of its lands instead.
use super::presets::{Candidate, Found};
use sparksmith_core::commands::{Generate, ModuleSource};
use sparksmith_core::library::bundle::{AssetBytes, asset_hash};
use sparksmith_core::library::generated::{
    GeneratorParams, GeneratorRuntime, GeneratorScope, ParamValue,
};
use sparksmith_core::library::graphics::ShapeNm;
use sparksmith_core::library::{
    CornerMask, HoleShape, MaskRule, Pad, PadKind, PadShape, PadSide, PasteRule, Segment,
};
use sparksmith_core::units::{Angle, Nm, PointNm};
use sparksmith_core::{
    BindingRef, ComponentRef, DefinitionKind, Engine, FootprintRef, LibraryCommand, Operation,
    Outcome, SymbolRef,
};
use sparksmith_generators::abi::v1;
use sparksmith_kicad::models::{Generated, PartRequest};
use std::collections::BTreeMap;

/// The quarter turn (0–3) and whole-nanometre offset of `found`'s preset
/// frame in the footprint's frame.
fn frame(found: &Found) -> Result<(i64, [i64; 2]), String> {
    let turn = found.placement.rotation_deg()[2];
    let quarter = (turn / 90.0).round();
    if (turn - 90.0 * quarter).abs() > 1e-9 {
        return Err("the lands fit the preset at other than a quarter turn".into());
    }
    let [x, y, _] = found.placement.translation_nm;
    Ok(((quarter as i64).rem_euclid(4), [x.get(), y.get()]))
}

/// `p` turned by `-quarter` quarter turns.
fn unturn(p: [i64; 2], quarter: i64) -> [i64; 2] {
    match quarter {
        1 => [p[1], -p[0]],
        2 => [-p[0], -p[1]],
        3 => [-p[1], p[0]],
        _ => p,
    }
}

/// A footprint pad in the package frame: moved back by the offset and
/// turned back by the quarter turn.
fn to_package(pad: &Pad, quarter: i64, offset: [i64; 2]) -> Result<Pad, String> {
    let mut out = pad.clone();
    let at = [
        pad.position.x_nm.get() - offset[0],
        pad.position.y_nm.get() - offset[1],
    ];
    let [x, y] = unturn(at, quarter);
    out.position = PointNm::new(
        Nm::new(x).map_err(|e| e.to_string())?,
        Nm::new(y).map_err(|e| e.to_string())?,
    );
    out.rotation_mdeg =
        Angle::normalized(i64::from(pad.rotation_mdeg.millidegrees()) - 90_000 * quarter);
    // Turned as the generation will store it (its symmetry folded), with
    // the pad-frame features (hole offset, slot, paste windows) turned to
    // match, so the expansion moves nothing.
    out.fold_rotation().map_err(|e| e.to_string())?;
    Ok(out)
}

fn nm(v: f64) -> Result<i64, String> {
    Nm::from_mm_f64_rounded(v)
        .map(Nm::get)
        .map_err(|e| e.to_string())
}

fn point(p: PointNm) -> [i64; 2] {
    [p.x_nm.get(), p.y_nm.get()]
}

fn mm_point(p: [f64; 2]) -> Result<[i64; 2], String> {
    Ok([nm(p[0])?, nm(p[1])?])
}

/// A paste window as the ABI carries it.
fn window(shape: &ShapeNm) -> Result<v1::Window, String> {
    Ok(match shape {
        ShapeNm::Rect { a, b, radius_nm } => v1::Window::Rect {
            min: point(*a),
            max: point(*b),
            radius_nm: radius_nm.get(),
        },
        ShapeNm::Circle { center, radius_nm } => v1::Window::Circle {
            center: point(*center),
            radius_nm: radius_nm.get(),
        },
        ShapeNm::Polygon { points, radii_nm } => v1::Window::Polygon {
            points: points.iter().map(|p| point(*p)).collect(),
            radii_nm: radii_nm.iter().map(|r| r.get()).collect(),
        },
        _ => return Err("a paste window that is not an area".into()),
    })
}

/// `pad` (package frame) as the generator ABI's pad of `key`, landing
/// `terminal` (empty: none).
fn abi_pad(pad: &Pad, key: String, terminal: String) -> Result<v1::Pad, String> {
    if !pad.vias.is_empty() {
        return Err(format!("pad {} owns plated holes", pad.number));
    }
    let shape = match &pad.shape {
        PadShape::Rect { size } => v1::PadShape::Rect { size: point(*size) },
        PadShape::Circle { diameter_nm } => v1::PadShape::Circle {
            diameter_nm: diameter_nm.get(),
        },
        PadShape::Oblong { size } => v1::PadShape::Oval { size: point(*size) },
        PadShape::RoundRect {
            size,
            radius_ratio,
            max_radius_nm,
            corners,
        } if *corners == CornerMask::ALL => {
            let [w, h] = size.to_mm_f64();
            let mut radius = radius_ratio.clamp(0.0, 0.5) * w.min(h);
            if let Some(max) = max_radius_nm {
                radius = radius.min(max.to_mm_f64());
            }
            v1::PadShape::RoundRect {
                size: point(*size),
                radius_nm: nm(radius)?,
            }
        }
        PadShape::RoundRect { .. } => {
            return Err(format!("pad {} rounds only some corners", pad.number));
        }
        PadShape::Custom { outline } => v1::PadShape::Outline {
            start: mm_point(outline.start)?,
            segments: outline
                .segments
                .iter()
                .map(|segment| {
                    Ok(match segment {
                        Segment::Line { to } => v1::OutlineSegment {
                            to: mm_point(*to)?,
                            bulge_ppb: None,
                        },
                        Segment::Arc { to, bulge } => v1::OutlineSegment {
                            to: mm_point(*to)?,
                            bulge_ppb: Some((bulge * 1e9).round() as i64),
                        },
                    })
                })
                .collect::<Result<_, String>>()?,
        },
    };
    let hole = pad.hole.map(|hole| v1::Hole {
        shape: match hole.shape {
            HoleShape::Round { diameter_nm } => v1::HoleShape::Round {
                diameter_nm: diameter_nm.get(),
            },
            HoleShape::Slot {
                width_nm,
                length_nm,
                angle_mdeg,
            } => v1::HoleShape::Slot {
                size: [length_nm.get(), width_nm.get()],
                rot_mdeg: Some(i64::from(angle_mdeg.millidegrees())),
            },
        },
        plated: hole.plated,
        offset: (hole.offset != PointNm::ORIGIN).then(|| point(hole.offset)),
    });
    let mask = match pad.mask {
        MaskRule::Inherit => v1::Opening::Default,
        MaskRule::Tented => v1::Opening::None,
        MaskRule::Expansion(by) => v1::Opening::Expansion(by.get()),
    };
    let paste = match &pad.paste {
        PasteRule::Inherit => v1::Opening::Default,
        PasteRule::None => v1::Opening::None,
        PasteRule::Reduction(by) => v1::Opening::Expansion(-by.get()),
        PasteRule::Windows(windows) => v1::Opening::Windows(
            windows
                .iter()
                .map(|w| window(&w.shape))
                .collect::<Result<_, _>>()?,
        ),
    };
    let kind = match pad.kind {
        PadKind::Mechanical => Some(v1::PadKind::Mechanical),
        PadKind::Fiducial => Some(v1::PadKind::Fiducial),
        PadKind::Smd | PadKind::PlatedThrough | PadKind::NonPlatedHole => None,
    };
    Ok(v1::Pad {
        key,
        terminal,
        shape,
        at: point(pad.position),
        rot_mdeg: i64::from(pad.rotation_mdeg.millidegrees()),
        side: match pad.side {
            PadSide::Top => v1::Side::Top,
            PadSide::Bottom => v1::Side::Bottom,
        },
        hole,
        mask,
        paste,
        number: Some(pad.number.clone()),
        kind,
    })
}

/// What first differs between two pads' copper, holes and openings, if
/// anything does: the
/// generated pad is the source's in the package frame. Shapes compare by
/// their outline, so a rounded rectangle written by its radius equals one
/// written by its ratio.
/// A rounded rectangle with every corner rounded: its size and corner
/// radius, mm.
fn round_rect(shape: &PadShape) -> Option<(PointNm, f64)> {
    match shape {
        PadShape::RoundRect {
            size,
            radius_ratio,
            max_radius_nm,
            corners,
        } if *corners == CornerMask::ALL => {
            let [w, h] = size.to_mm_f64();
            let radius = radius_ratio.clamp(0.0, 0.5) * w.min(h);
            Some((
                *size,
                max_radius_nm.map_or(radius, |max| radius.min(max.to_mm_f64())),
            ))
        }
        _ => None,
    }
}

fn difference(a: &Pad, b: &Pad) -> Option<&'static str> {
    const NM: f64 = 1e-6;
    let close = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).abs() <= NM && (p[1] - q[1]).abs() <= NM;
    let checks: [(&'static str, bool); 9] = [
        ("kind", a.kind == b.kind),
        ("side", a.side == b.side),
        ("position", a.position == b.position),
        ("hole", a.hole == b.hole),
        ("mask", a.mask == b.mask),
        ("terminal", a.terminal.is_some() == b.terminal.is_some()),
        ("number", a.number == b.number),
        (
            "rotation",
            a.shape.canonical_rotation(a.rotation_mdeg)
                == b.shape.canonical_rotation(b.rotation_mdeg),
        ),
        (
            "paste",
            match (&a.paste, &b.paste) {
                (PasteRule::Windows(x), PasteRule::Windows(y)) => {
                    x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.shape == q.shape)
                }
                (x, y) => x == y,
            },
        ),
    ];
    if let Some((what, _)) = checks.iter().find(|(_, same)| !same) {
        return Some(what);
    }
    // Rounded rectangles of one size and corner radius are one shape,
    // whether the radius is written as a ratio or as a cap.
    if let (Some((sa, ra)), Some((sb, rb))) = (round_rect(&a.shape), round_rect(&b.shape)) {
        return (sa != sb || (ra - rb).abs() > NM).then_some("outline");
    }
    // The outlines as vertex sets: a shape turned by its symmetry starts
    // its ring elsewhere.
    let ring = |p: &Pad| {
        let mut ring = p.outline(1e-4);
        ring.sort_by(|x, y| x[0].total_cmp(&y[0]).then(x[1].total_cmp(&y[1])));
        ring
    };
    let (oa, ob) = (ring(a), ring(b));
    (oa.len() != ob.len() || !oa.iter().zip(&ob).all(|(p, q)| close(*p, *q))).then_some("outline")
}

/// Make the part of `request`'s footprint with `candidate`'s generator:
/// `Ok(Err(why))` when the generator cannot take these lands (nothing
/// changed).
pub(super) fn make(
    engine: &mut Engine,
    candidate: &Candidate,
    found: &Found,
    request: &PartRequest<'_>,
    runtime: &(dyn GeneratorRuntime + Send + Sync),
) -> Result<Result<Generated, String>, String> {
    let refuse = |why: String| Ok(Err(why));
    if candidate.scope != v1::Scope::Component {
        return refuse(format!(
            "the generator of {} makes packages only",
            candidate.name
        ));
    }
    let (quarter, offset) = match frame(found) {
        Ok(frame) => frame,
        Err(why) => return refuse(why),
    };
    // Each source pad's package terminal: its land's preset pad's; a
    // thermal via (no land) shares its land's number's; holes, mechanical
    // pads and fiducials land none.
    let preset_pads = &candidate.generation.footprint.pads;
    let paired: BTreeMap<usize, &str> = found
        .pads
        .iter()
        .filter_map(|(source, preset)| Some((*source, preset_pads.get(*preset)?.terminal.as_str())))
        .collect();
    let by_number: BTreeMap<super::lands::Key, &str> = paired
        .iter()
        .filter_map(|(i, t)| Some((super::lands::Key::of(&request.pads.get(*i)?.number), *t)))
        .collect();
    let mut terminals = Vec::with_capacity(request.pads.len());
    for (i, pad) in request.pads.iter().enumerate() {
        let free = matches!(
            pad.kind,
            PadKind::NonPlatedHole | PadKind::Mechanical | PadKind::Fiducial
        );
        let terminal = if free && pad.terminal.is_none() {
            String::new()
        } else if let Some(t) = paired
            .get(&i)
            .or_else(|| by_number.get(&super::lands::Key::of(&pad.number)))
        {
            (*t).to_owned()
        } else {
            return refuse(format!(
                "its pad {:?} lands no terminal of {}",
                pad.number, candidate.name
            ));
        };
        terminals.push(terminal);
    }
    // A component terminal never joins pads a placed copy keeps on
    // different nets.
    let component_of: BTreeMap<&str, &str> = candidate
        .generation
        .binding
        .iter()
        .map(|b| (b.package_terminal.as_str(), b.terminal.as_str()))
        .collect();
    for nets in request.nets {
        let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
        for (terminal, net) in terminals.iter().zip(nets) {
            let (Some(component), Some(net)) = (component_of.get(terminal.as_str()), net) else {
                continue;
            };
            if seen
                .insert(component, *net)
                .is_some_and(|other| other != *net)
            {
                return refuse(format!(
                    "{} joins pads the board puts on different nets (terminal {component})",
                    candidate.name
                ));
            }
        }
        // Pads of different numbers (two mounting tabs, say) joined into
        // one terminal only where the board has them on one net: else
        // the part would join what the source keeps apart.
        let mut numbers: BTreeMap<&str, (super::lands::Key, Option<u32>)> = BTreeMap::new();
        for ((terminal, net), pad) in terminals.iter().zip(nets).zip(request.pads) {
            let Some(component) = component_of.get(terminal.as_str()) else {
                continue;
            };
            let key = super::lands::Key::of(&pad.number);
            match numbers.get(component) {
                Some((other, other_net))
                    if *other != key && (net.is_none() || *other_net != *net) =>
                {
                    return refuse(format!(
                        "{} joins pads of different numbers the board keeps apart (terminal {component})",
                        candidate.name
                    ));
                }
                Some(_) => {}
                None => {
                    numbers.insert(component, (key, *net));
                }
            }
        }
    }
    // The land pattern: every source pad in the package frame.
    let mut lands = Vec::with_capacity(request.pads.len());
    let mut expected = Vec::with_capacity(request.pads.len());
    for (i, (pad, terminal)) in request.pads.iter().zip(&terminals).enumerate() {
        let local = to_package(pad, quarter, offset)?;
        let abi = match abi_pad(&local, format!("{}", i + 1), terminal.clone()) {
            Ok(abi) => abi,
            Err(why) => return refuse(why),
        };
        let text = sparksmith_generators::abi::codec::encode(&abi).map_err(|e| e.to_string())?;
        let text = String::from_utf8(text).map_err(|e| e.to_string())?;
        if text.chars().count() > v1::limits::TEXT_CHARS {
            return refuse(format!(
                "its pad {:?} is too detailed for a land pattern item",
                pad.number
            ));
        }
        lands.push(v1::ParamValue::Text(text));
        expected.push(local);
    }
    let mut params: GeneratorParams = candidate
        .params
        .iter()
        .map(|(k, v)| (k.clone(), ParamValue::from_wire(v)))
        .collect();
    params.insert(
        "land_pattern".into(),
        ParamValue::from_wire(&v1::ParamValue::List(lands)),
    );
    let module = &candidate.bundle.module;
    let hash = asset_hash(module);
    let embedded =
        sparksmith_core::library::generated::module(&engine.project().library, hash).is_some();
    let mut generate = Generate {
        module: ModuleSource {
            hash,
            data: (!embedded).then(|| AssetBytes::new(module.clone())),
        },
        scope: GeneratorScope::Component,
        params,
        name: None,
        replaces: None,
        model: None,
        bundle: Some(candidate.bundle.hash),
        preset: Some(candidate.preset.clone()),
    };
    let (bytes, wire) =
        sparksmith_core::commands::generator_request(engine.document(), &generate, runtime)
            .map_err(|e| e.to_string())?;
    let made = match runtime.generate(&bytes, &wire) {
        Ok(made) => made,
        Err(failure) => {
            return refuse(format!(
                "the generator refused the land pattern: {}",
                sparksmith_core::library::generated::failure(&failure)
            ));
        }
    };
    generate.model = Some(
        crate::session::generated_models::generated_model(
            engine,
            &made.generation,
            Some(&candidate.bundle),
        )
        .map_err(|e| e.to_string())?,
    );
    // Made on a fork: kept only when the part's lands are the source's.
    let mut staged = engine.fork();
    let outcome = staged
        .apply(
            staged.revision(),
            Operation::Library(LibraryCommand::Generate(Box::new(generate))),
        )
        .map_err(|e| e.to_string())?;
    let Outcome::Generated { report } = outcome else {
        return Err("library.generate made no part".into());
    };
    let of = |kind: DefinitionKind| report.revisions.iter().find(|r| r.kind == kind).copied();
    let (Some(component), Some(symbol), Some(binding), Some(footprint)) = (
        of(DefinitionKind::Component),
        of(DefinitionKind::Symbol),
        of(DefinitionKind::Binding),
        of(DefinitionKind::Footprint),
    ) else {
        return Err("library.generate made an incomplete part".into());
    };
    let library = &staged.project().library;
    let footprint_ref = FootprintRef::new(footprint.id, footprint.revision);
    let symbol_ref = SymbolRef::new(symbol.id, symbol.revision);
    let pads = library
        .footprint(footprint_ref)
        .map(|f| f.definition.pads.clone())
        .unwrap_or_default();
    let differences: Vec<&str> = pads
        .iter()
        .zip(&expected)
        .filter_map(|(a, b)| difference(a, b))
        .collect();
    if pads.len() != expected.len() || !differences.is_empty() {
        return refuse(format!(
            "the generated lands differ from the source's ({} of {} pads; first in {})",
            differences.len(),
            expected.len(),
            differences.first().copied().unwrap_or("count")
        ));
    }
    let units = library
        .symbol(symbol_ref)
        .map_or(1, |s| u32::try_from(s.definition.units.len()).unwrap_or(1));
    *engine = staged;
    let mut reason = format!(
        "Generated by {} with the source's {} lands as its land pattern.",
        candidate.name,
        expected.len()
    );
    if !found.notes.is_empty() {
        reason.push(' ');
        reason.push_str(&found.notes.join(" "));
    }
    Ok(Ok(Generated {
        component: ComponentRef::new(component.id, component.revision),
        symbol: symbol_ref,
        binding: BindingRef::new(binding.id, binding.revision),
        footprint: footprint_ref,
        turn_deg: (90 * quarter) as f64,
        offset: PointNm::new(
            Nm::new(offset[0]).map_err(|e| e.to_string())?,
            Nm::new(offset[1]).map_err(|e| e.to_string())?,
        ),
        units,
        reason,
        generator: candidate.generator.clone(),
    }))
}
