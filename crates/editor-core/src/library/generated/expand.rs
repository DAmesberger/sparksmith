//! The expansion: a validated generation (ABI 1 wire types, keyed by
//! generation keys) mapped to the project's definitions (keyed by local
//! ids). A pure function of the generation, the five lineage references
//! and the part's key-to-id map ([`KeyIds`]), so a stored part can be
//! expanded again and compared exactly (`docs/generator-plugins.md`,
//! "Expansion").
//!
//! **Stable keys (slice B1).** Every generated object has a generation key
//! (terminal, unit, symbol drawing, package terminal, pad, footprint
//! drawing), and its local id comes from the map: a key the map holds
//! keeps its id, a new key takes the next id after every id the map has
//! ever held. The map never forgets a key, so a key that comes back gets
//! its old id and no id is ever reused for another key. A part's ids
//! therefore do not depend on the order the module emits things in, and
//! regenerating with a module that inserts a pad leaves every other pad's
//! id, and the tracks on it, where they were. For a fresh map the ids
//! follow the generation's order: terminals 1…n with their fixed functions
//! n+1…2n; symbol units, each followed by its graphics; package terminals
//! 1…m; pads 1…p, then the footprint's drawings (silkscreen, assembly,
//! courtyard).
use crate::diagnostics::{Error, Result};
use crate::functions::{Direction, TerminalFunctions};
use crate::ids::ContentHash;
use crate::ids::{
    BindingRef, DefinitionId, FootprintRef, FunctionId, GraphicId, PackageRef, PackageTerminalId,
    PadId, SymbolRef, TerminalId, UnitId,
};
use crate::library::bundle::{AssetBytes, PackedAsset};
use crate::library::graphics::{
    self, FootprintGraphic, Graphic, HAlign, Item, Justify, Pin, PinDirection, Rotation, ShapeNm,
    TextContent, VAlign,
};
use crate::library::package_model::PartValue;
use crate::library::{
    BindingDefinition, ComponentDefinition, CornerMask, Definition, DrawingLayer,
    FootprintDefinition, FootprintKeepout, Hole, HoleShape, KeepoutScope, MaskRule, ModelPlacement,
    Outline, PackageDefinition, PackageModel, PackageTerminal, Pad, PadKind, PadShape, PadSide,
    PasteRule, Segment, SymbolDefinition, Terminal, TerminalClass, TerminalMap, Unit,
};
use crate::units::{Angle, ArcDirection, MILLIDEGREES_PER_TURN, Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sparksmith_generator_abi::v1;
use std::collections::BTreeMap;

/// The lineages and revisions one expansion fills: all five for a complete
/// part, `package` and `footprint` alone for a package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refs {
    pub component: Option<(DefinitionId, SymbolRef, BindingRef)>,
    pub package: PackageRef,
    pub footprint: FootprintRef,
}

/// The courtyard outline's stroke, 0.05 mm, as the footprint templates
/// draw it (`footprint_drawings::COURTYARD_STROKE_MM`).
const COURTYARD_STROKE: Nm = match Nm::try_new(50_000) {
    Some(nm) => nm,
    None => Nm::ZERO,
};

fn refused(message: String) -> Error {
    Error::Invalid(format!("generated.expansion: {message}"))
}

fn nm(value: i64) -> Result<Nm> {
    Nm::new(value)
}

fn point(p: v1::Point) -> Result<PointNm> {
    Ok(PointNm::new(nm(p[0])?, nm(p[1])?))
}

/// Keys a part's map holds, all namespaces together.
pub const MAX_KEYS: usize = 16 * 1024;

/// A generated part's generation keys and the local ids they were given,
/// one map per definition (each definition numbers its own locals). Keys
/// carry their kind: `t:` a terminal, `f:` its function (component); `u:` a
/// unit, `g:UNIT/KEY` a unit's drawing (symbol); `t:` a package terminal
/// (package); `p:` a pad, `g:` a drawing, `courtyard` (footprint).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct KeyIds {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub component: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symbol: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub package: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub footprint: BTreeMap<String, u32>,
}

impl KeyIds {
    /// Every key, all namespaces.
    pub fn len(&self) -> usize {
        self.component.len() + self.symbol.len() + self.package.len() + self.footprint.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Bounded, and every id in a namespace used once and nonzero.
    pub fn validate(&self) -> Result<()> {
        if self.len() > MAX_KEYS {
            return Err(Error::Limit(format!(
                "generated.record: a generated part maps at most {MAX_KEYS} keys"
            )));
        }
        for map in [
            &self.component,
            &self.symbol,
            &self.package,
            &self.footprint,
        ] {
            let mut ids = std::collections::BTreeSet::new();
            for (key, id) in map {
                if *id == 0 || !ids.insert(*id) || key.is_empty() || key.len() > 2048 {
                    return Err(Error::Invalid(format!(
                        "generated.record: the key map gives \"{}\" an id another key has, or none",
                        key.chars().take(80).collect::<String>()
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Ids of one namespace: the map's, then new ones after every id it holds.
struct Ids {
    map: BTreeMap<String, u32>,
    next: u32,
    /// Keys given in this expansion, to refuse one given twice.
    given: std::collections::BTreeSet<String>,
}

impl Ids {
    fn new(map: &BTreeMap<String, u32>) -> Self {
        let next = map.values().max().map_or(1, |m| m.saturating_add(1));
        Self {
            map: map.clone(),
            next,
            given: Default::default(),
        }
    }

    /// The id of `key`: the map's, or the next free one.
    fn id(&mut self, key: String) -> Result<u32> {
        if !self.given.insert(key.clone()) {
            return Err(refused(format!("the key \"{key}\" is used twice")));
        }
        if let Some(id) = self.map.get(&key) {
            return Ok(*id);
        }
        let id = self.next;
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| refused("the part has more items than local ids".into()))?;
        self.map.insert(key, id);
        Ok(id)
    }

    /// The id `key` was given in this expansion.
    fn of(&self, key: &str, what: &str) -> Result<u32> {
        self.map
            .get(key)
            .copied()
            .filter(|_| self.given.contains(key))
            .ok_or_else(|| refused(format!("{what} \"{key}\" is not declared")))
    }

    /// The definition's next local id: after every id the map holds.
    fn next_local(&self) -> u32 {
        self.next
    }
}

/// The definitions of `generation` at `refs`, its package holding `model`
/// (the generation's part file as a Model revision of the project, at the
/// generation's values), with local ids from `ids` (a new part's map is
/// empty), in order: component, symbol, package, footprint, binding for a
/// complete part; package and footprint for a package. Also returns the
/// map with every new key added.
pub fn definitions(
    generation: &v1::Generation,
    refs: Refs,
    model: PackageModel,
    ids: &KeyIds,
) -> Result<(Vec<(DefinitionId, Definition)>, KeyIds)> {
    let mut package_terminals = Ids::new(&ids.package);
    for key in &generation.package.terminals {
        package_terminals.id(format!("t:{key}"))?;
    }
    let package = package(generation, &package_terminals, model)?;
    let mut pads = Ids::new(&ids.footprint);
    let footprint = footprint(generation, refs.package, &package_terminals, &mut pads)?;
    let mut out = Vec::with_capacity(5);
    let mut used = KeyIds {
        package: package_terminals.map.clone(),
        footprint: pads.map,
        ..KeyIds::default()
    };
    match (refs.component, &generation.component, &generation.symbol) {
        (Some((component_id, symbol_ref, binding_ref)), Some(component), Some(symbol)) => {
            let mut terminals = Ids::new(&ids.component);
            let definition =
                component_definition(component, symbol_ref, binding_ref, &mut terminals)?;
            out.push((component_id, Definition::Component(definition)));
            let mut units = Ids::new(&ids.symbol);
            out.push((
                symbol_ref.id,
                Definition::Symbol(symbol_definition(
                    symbol,
                    component_id,
                    &terminals,
                    &mut units,
                )?),
            ));
            out.push((refs.package.id, Definition::Package(package)));
            out.push((refs.footprint.id, Definition::Footprint(footprint)));
            let mappings = generation
                .binding
                .iter()
                .map(|entry| {
                    Ok(TerminalMap {
                        package_terminal: PackageTerminalId(
                            package_terminals
                                .of(&format!("t:{}", entry.package_terminal), "package terminal")?,
                        ),
                        terminal: TerminalId(
                            terminals.of(&format!("t:{}", entry.terminal), "terminal")?,
                        ),
                    })
                })
                .collect::<Result<_>>()?;
            used.component = terminals.map;
            used.symbol = units.map;
            out.push((
                binding_ref.id,
                Definition::Binding(BindingDefinition {
                    component: component_id,
                    package: refs.package,
                    mappings,
                    footprints: vec![refs.footprint],
                    unavailable: Vec::new(),
                }),
            ));
        }
        (None, None, None) => {
            out.push((refs.package.id, Definition::Package(package)));
            out.push((refs.footprint.id, Definition::Footprint(footprint)));
            used.component = ids.component.clone();
            used.symbol = ids.symbol.clone();
        }
        _ => {
            return Err(refused(
                "a complete part needs a component and a symbol, and a package needs neither"
                    .into(),
            ));
        }
    }
    Ok((out, used))
}

/// Expand a generation's mechanical land pattern without evaluating its part
/// file. Uses the same ids and geometry conversion as full definition expansion;
/// callers may compare it before paying for native model evaluation.
pub fn land_pattern(generation: &v1::Generation) -> Result<Vec<Pad>> {
    let mut terminals = Ids::new(&BTreeMap::new());
    for key in &generation.package.terminals {
        terminals.id(format!("t:{key}"))?;
    }
    footprint_pads(generation, &terminals, &mut Ids::new(&BTreeMap::new()))
}

fn direction(d: v1::Direction) -> Direction {
    match d {
        v1::Direction::Input => Direction::Input,
        v1::Direction::Output => Direction::Output,
        v1::Direction::Bidirectional => Direction::Bidirectional,
        v1::Direction::Passive => Direction::Passive,
        v1::Direction::PowerInput => Direction::PowerInput,
        v1::Direction::PowerOutput => Direction::PowerOutput,
    }
}

fn component_definition(
    component: &v1::Component,
    symbol: SymbolRef,
    binding: BindingRef,
    ids: &mut Ids,
) -> Result<ComponentDefinition> {
    // Every terminal first, then every function, so a fresh map numbers
    // terminals 1…n and functions n+1…2n.
    let terminal_ids = component
        .terminals
        .iter()
        .map(|t| ids.id(format!("t:{}", t.key)))
        .collect::<Result<Vec<_>>>()?;
    let function_ids = component
        .terminals
        .iter()
        .map(|t| ids.id(format!("f:{}", t.key)))
        .collect::<Result<Vec<_>>>()?;
    // A terminal's further functions after every terminal's first, so a
    // part without any keeps the numbering above.
    let alternate_ids = component
        .terminals
        .iter()
        .map(|t| {
            t.alternates
                .iter()
                .map(|d| {
                    let id = ids.id(format!("f:{}:{}", t.key, direction(*d).wire()))?;
                    Ok((FunctionId(id), direction(*d)))
                })
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let terminals = component
        .terminals
        .iter()
        .zip(terminal_ids.into_iter().zip(function_ids))
        .zip(alternate_ids)
        .map(|((t, (id, function)), alternates)| Terminal {
            id: TerminalId(id),
            name: t.name.clone(),
            class: TerminalClass::Electrical(TerminalFunctions::with_alternates(
                FunctionId(function),
                direction(t.direction),
                &alternates,
            )),
        })
        .collect();
    Ok(ComponentDefinition {
        next_local: ids.next_local(),
        designator_prefix: component.prefix.clone(),
        terminals,
        resources: Vec::new(),
        constraints: Vec::new(),
        representations: vec![symbol],
        implementations: vec![binding],
        // Standard field keys stay in the generation (and its result hash);
        // the host selects a generic family's fields afterwards
        // (`generated::family_fields`).
        fields: Vec::new(),
        pin_rules: Vec::new(),
        marking: None,
        review: None,
        value_template: None,
        generic: None,
    })
}

fn symbol_definition(
    symbol: &v1::Symbol,
    component: DefinitionId,
    terminals: &Ids,
    ids: &mut Ids,
) -> Result<SymbolDefinition> {
    let mut units = Vec::with_capacity(symbol.units.len());
    for unit in &symbol.units {
        let id = UnitId(ids.id(format!("u:{}", unit.key))?);
        let mut graphics = Vec::with_capacity(unit.graphics.len());
        for graphic in &unit.graphics {
            graphics.push(Graphic {
                id: GraphicId(ids.id(format!("g:{}/{}", unit.key, graphic.key))?),
                item: item(graphic)?,
            });
        }
        let pins = unit
            .pins
            .iter()
            .map(|pin| {
                let mut out = Pin::new(
                    TerminalId(terminals.of(&format!("t:{}", pin.terminal), "terminal")?),
                    point(pin.at)?,
                    pin_direction(pin.rot_mdeg)?,
                    nm(pin.length_nm)?,
                );
                out.name.visible = pin.show_name;
                out.number.visible = pin.show_number;
                Ok(out)
            })
            .collect::<Result<_>>()?;
        units.push(Unit {
            id,
            label: unit.key.clone(),
            graphics,
            pins,
        });
    }
    Ok(SymbolDefinition {
        component,
        next_local: ids.next_local(),
        pin_text_height_nm: graphics::PIN_TEXT_HEIGHT,
        units,
    })
}

fn pin_direction(rot_mdeg: i64) -> Result<PinDirection> {
    match rot_mdeg.rem_euclid(i64::from(MILLIDEGREES_PER_TURN)) {
        0 => Ok(PinDirection::Right),
        90_000 => Ok(PinDirection::Up),
        180_000 => Ok(PinDirection::Left),
        270_000 => Ok(PinDirection::Down),
        _ => Err(refused(format!(
            "a pin points at {rot_mdeg} millidegrees; symbol pins point along the axes"
        ))),
    }
}

fn fill(f: v1::Fill) -> graphics::Fill {
    match f {
        v1::Fill::None => graphics::Fill::None,
        v1::Fill::Body => graphics::Fill::Body,
        v1::Fill::Solid => graphics::Fill::Solid,
    }
}

fn shape(primitive: &v1::Primitive) -> Result<ShapeNm> {
    Ok(match primitive {
        v1::Primitive::Line { a, b } => ShapeNm::Line {
            a: point(*a)?,
            b: point(*b)?,
        },
        v1::Primitive::Rect { min, max } => ShapeNm::Rect {
            a: point(*min)?,
            b: point(*max)?,
            radius_nm: Nm::ZERO,
        },
        v1::Primitive::Polyline { points } => ShapeNm::Polyline {
            points: points.iter().map(|p| point(*p)).collect::<Result<_>>()?,
            radii_nm: Vec::new(),
        },
        v1::Primitive::Polygon { points } => ShapeNm::Polygon {
            points: points.iter().map(|p| point(*p)).collect::<Result<_>>()?,
            radii_nm: Vec::new(),
        },
        v1::Primitive::Circle { center, radius_nm } => ShapeNm::Circle {
            center: point(*center)?,
            radius_nm: nm(*radius_nm)?,
        },
        v1::Primitive::Arc {
            center,
            start,
            end,
            clockwise,
        } => ShapeNm::Arc {
            start: point(*start)?,
            end: point(*end)?,
            centre: point(*center)?,
            direction: if *clockwise {
                ArcDirection::Clockwise
            } else {
                ArcDirection::CounterClockwise
            },
        },
        v1::Primitive::Text { .. } => {
            return Err(refused("text is not a shape".into()));
        }
    })
}

fn item(graphic: &v1::Graphic) -> Result<Item> {
    if let v1::Primitive::Text {
        at,
        height_nm,
        content,
        h_align,
        v_align,
    } = &graphic.shape
    {
        return Ok(Item::Text(graphics::Text {
            content: match content {
                v1::TextContent::Designator => TextContent::Designator,
                v1::TextContent::ComponentName => TextContent::ComponentName,
                v1::TextContent::Literal(text) => TextContent::Literal(text.clone()),
            },
            at: point(*at)?,
            height_nm: nm(*height_nm)?,
            rotation: Rotation::R0,
            justify: Justify {
                h: match h_align {
                    v1::HAlign::Left => HAlign::Left,
                    v1::HAlign::Center => HAlign::Center,
                    v1::HAlign::Right => HAlign::Right,
                },
                v: match v_align {
                    v1::VAlign::Top => VAlign::Top,
                    v1::VAlign::Middle => VAlign::Middle,
                    v1::VAlign::Bottom => VAlign::Bottom,
                },
            },
            visible: true,
            line_spacing: graphics::TEXT_LINE_SPACING,
        }));
    }
    Ok(Item::Shape {
        shape: shape(&graphic.shape)?,
        width_nm: nm(graphic.stroke_nm)?,
        fill: fill(graphic.fill),
    })
}

/// Whether the generator marks the part complete without a body of its own
/// (`Model::NoBody`: a fiducial, whose land the board draws).
pub fn no_body(generation: &v1::Generation) -> bool {
    matches!(generation.model, Some(v1::Model::NoBody))
}

/// The part file a generation's model names, by its SHA-256, and the
/// values it gives it; refused when the generation has no model or none
/// of its own.
pub fn part_file(generation: &v1::Generation) -> Result<(ContentHash, &v1::Params)> {
    match &generation.model {
        Some(v1::Model::PartFile { asset, values }) => {
            let hash = ContentHash::parse(asset).ok_or_else(|| {
                refused(format!(
                    "the generator names its part file \"{asset}\", which is not a SHA-256"
                ))
            })?;
            Ok((hash, values))
        }
        Some(v1::Model::NoBody) => Err(refused(
            "the generator makes this part without a body, so it names no part file".into(),
        )),
        None => Err(refused(
            "the generator made no 3D model; a part needs one".into(),
        )),
    }
}

/// Whether `stored` are the generation's part-file `values`: the same
/// names, each a length of the same nanometres, an angle of the same
/// millidegrees, or the same count.
pub fn part_values_match(values: &v1::Params, stored: &BTreeMap<String, PartValue>) -> Result<()> {
    let differs = |name: &str| {
        refused(format!(
            "the part file's value \"{name}\" is not the generator's"
        ))
    };
    if values.len() != stored.len() {
        return Err(refused(
            "the part file is given other values than the generator's".into(),
        ));
    }
    for (name, value) in values {
        let (Some(stored), v1::ParamValue::Int(int)) = (stored.get(name), value) else {
            return Err(differs(name));
        };
        let same = match *stored {
            PartValue::Nm(v) | PartValue::Mdeg(v) => v == *int,
            PartValue::Count(n) => i64::from(n) == *int,
            PartValue::Milli(_) => false,
        };
        if !same {
            return Err(differs(name));
        }
    }
    Ok(())
}

fn package(
    generation: &v1::Generation,
    terminals: &Ids,
    model: PackageModel,
) -> Result<PackageDefinition> {
    let geometry = &generation.package.geometry;
    let [length, width, height] = geometry.envelope_nm;
    if no_body(generation) {
        if !matches!(model, PackageModel::NoBody) {
            return Err(refused(
                "the generator makes this package without a body; it takes no 3D model".into(),
            ));
        }
    } else {
        let (_, values) = part_file(generation)?;
        let PackageModel::PartFile(part) = &model else {
            return Err(refused(
                "a generated package's 3D model is its generator's part file".into(),
            ));
        };
        part_values_match(values, &part.values)?;
    }
    let package = PackageDefinition {
        next_local: terminals.next_local(),
        terminals: generation
            .package
            .terminals
            .iter()
            .map(|key| {
                Ok(PackageTerminal {
                    id: PackageTerminalId(terminals.of(&format!("t:{key}"), "package terminal")?),
                    designation: key.clone(),
                })
            })
            .collect::<Result<_>>()?,
        // The envelope is the generation's; the model is the part file
        // the generator names, at its values (slice GP2), resolved to a
        // Model revision of this project by the caller.
        envelope_nm: [nm(length)?, nm(width)?, nm(height)?],
        model,
        placement: ModelPlacement {
            translation_nm: [Nm::ZERO, Nm::ZERO, nm(geometry.seating_nm)?],
            rotation_mdeg: [Angle::ZERO; 3],
        },
        mechanical: None,
        datums: Vec::new(),
        leads: Vec::new(),
        joints: None,
    };
    Ok(package)
}

fn pad_shape(shape: &v1::PadShape) -> Result<PadShape> {
    Ok(match shape {
        v1::PadShape::Rect { size } => PadShape::Rect {
            size: point(*size)?,
        },
        v1::PadShape::RoundRect { size, radius_nm } => PadShape::RoundRect {
            size: point(*size)?,
            // Half the short side, capped at the radius: exactly the radius
            // whenever it fits.
            radius_ratio: 0.5,
            max_radius_nm: Some(nm(*radius_nm)?),
            corners: CornerMask::ALL,
        },
        v1::PadShape::Circle { diameter_nm } => PadShape::Circle {
            diameter_nm: nm(*diameter_nm)?,
        },
        v1::PadShape::Oval { size } => PadShape::Oblong {
            size: point(*size)?,
        },
        v1::PadShape::Polygon { points } => {
            let mm = |p: &v1::Point| point(*p).map(PointNm::to_mm_f64);
            let Some(first) = points.first() else {
                return Err(refused("a polygon pad has no points".into()));
            };
            let start = mm(first)?;
            let mut segments = points
                .iter()
                .skip(1)
                .map(|p| Ok(Segment::Line { to: mm(p)? }))
                .collect::<Result<Vec<_>>>()?;
            segments.push(Segment::Line { to: start });
            PadShape::Custom {
                outline: Outline { start, segments },
            }
        }
        v1::PadShape::Outline { start, segments } => {
            let mm = |p: &v1::Point| point(*p).map(PointNm::to_mm_f64);
            let segments = segments
                .iter()
                .map(|s| {
                    let to = mm(&s.to)?;
                    Ok(match s.bulge_ppb {
                        Some(b) if b != 0 => Segment::Arc {
                            to,
                            bulge: bulge(b)?,
                        },
                        _ => Segment::Line { to },
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            PadShape::Custom {
                outline: Outline {
                    start: mm(start)?,
                    segments,
                },
            }
        }
    })
}

/// Most |bulge| of an outline arc, in parts per billion (the core's
/// [`crate::library::footprint::MAX_BULGE`]).
const MAX_BULGE_PPB: i64 = 1_000_000_000_000;

/// A bulge in parts per billion as the outline's bulge.
fn bulge(ppb: i64) -> Result<f64> {
    if ppb.checked_abs().is_none_or(|b| b > MAX_BULGE_PPB) {
        return Err(refused(format!(
            "an outline arc's bulge of {ppb} parts per billion is beyond the bound"
        )));
    }
    Ok(ppb as f64 / 1e9)
}

/// A paste window of a pad.
fn window(w: &v1::Window) -> Result<ShapeNm> {
    Ok(match w {
        v1::Window::Rect {
            min,
            max,
            radius_nm,
        } => ShapeNm::Rect {
            a: point(*min)?,
            b: point(*max)?,
            radius_nm: Nm::new(*radius_nm)?,
        },
        v1::Window::Circle { center, radius_nm } => ShapeNm::Circle {
            center: point(*center)?,
            radius_nm: nm(*radius_nm)?,
        },
        v1::Window::Polygon { points, radii_nm } => ShapeNm::Polygon {
            points: points.iter().map(|p| point(*p)).collect::<Result<_>>()?,
            radii_nm: radii_nm
                .iter()
                .map(|r| Nm::new(*r))
                .collect::<Result<_>>()?,
        },
    })
}

fn hole(hole: &v1::Hole) -> Result<Hole> {
    let shape = match &hole.shape {
        v1::HoleShape::Round { diameter_nm } => HoleShape::Round {
            diameter_nm: nm(*diameter_nm)?,
        },
        v1::HoleShape::Slot { size, rot_mdeg } => {
            let along_y = size[1] > size[0];
            HoleShape::Slot {
                width_nm: nm(size[0].min(size[1]))?,
                length_nm: nm(size[0].max(size[1]))?,
                angle_mdeg: match rot_mdeg {
                    Some(turn) => Angle::normalized(*turn),
                    None if along_y => Angle::normalized(90_000),
                    None => Angle::ZERO,
                },
            }
        }
    };
    Ok(Hole {
        shape,
        plated: hole.plated,
        offset: match hole.offset {
            Some(at) => point(at)?,
            None => PointNm::ORIGIN,
        },
    })
}

fn footprint_pads(generation: &v1::Generation, terminals: &Ids, ids: &mut Ids) -> Result<Vec<Pad>> {
    let source = &generation.footprint;
    let mut pads = Vec::with_capacity(source.pads.len());
    for pad in &source.pads {
        let shape = pad_shape(&pad.shape)?;
        let rotation = shape.canonical_rotation(Angle::normalized(pad.rot_mdeg));
        let hole = pad.hole.as_ref().map(hole).transpose()?;
        let kind = match (&hole, pad.kind) {
            (None, Some(v1::PadKind::Mechanical)) => PadKind::Mechanical,
            (None, Some(v1::PadKind::Fiducial)) => PadKind::Fiducial,
            (None, None) => PadKind::Smd,
            (Some(h), _) if h.plated => PadKind::PlatedThrough,
            (Some(_), _) => PadKind::NonPlatedHole,
        };
        pads.push(Pad {
            id: PadId(ids.id(format!("p:{}", pad.key))?),
            // A non-plated hole lands no terminal (a mechanical hole beside
            // the land it belongs to); its generation still names one.
            terminal: if kind == PadKind::NonPlatedHole
                || kind == PadKind::Fiducial
                || pad.terminal.is_empty()
            {
                None
            } else {
                Some(PackageTerminalId(
                    terminals.of(&format!("t:{}", pad.terminal), "package terminal")?,
                ))
            },
            kind,
            number: pad.number.clone().unwrap_or_else(|| pad.key.clone()),
            position: point(pad.at)?,
            rotation_mdeg: rotation,
            side: match pad.side {
                v1::Side::Top => PadSide::Top,
                v1::Side::Bottom => PadSide::Bottom,
            },
            shape,
            hole,
            mask: match &pad.mask {
                v1::Opening::Default => MaskRule::Inherit,
                v1::Opening::Expansion(by) => MaskRule::Expansion(nm(*by)?),
                v1::Opening::None => MaskRule::Tented,
                v1::Opening::Windows(_) => {
                    return Err(refused(format!(
                        "pad {} gives its solder mask windows; only paste has windows",
                        pad.key
                    )));
                }
            },
            paste: match &pad.paste {
                v1::Opening::Default => PasteRule::Inherit,
                v1::Opening::Expansion(by) => PasteRule::Reduction(nm(by.saturating_neg())?),
                v1::Opening::None => PasteRule::None,
                v1::Opening::Windows(windows) => PasteRule::Windows(
                    windows
                        .iter()
                        .enumerate()
                        .map(|(k, w)| {
                            Ok(crate::library::PasteWindow {
                                id: crate::library::ApertureId(
                                    u32::try_from(k + 1).unwrap_or(u32::MAX),
                                ),
                                shape: window(w)?,
                            })
                        })
                        .collect::<Result<_>>()?,
                ),
            },
            vias: Vec::new(),
        });
    }
    Ok(pads)
}

fn footprint(
    generation: &v1::Generation,
    package: PackageRef,
    terminals: &Ids,
    ids: &mut Ids,
) -> Result<FootprintDefinition> {
    let source = &generation.footprint;
    let pads = footprint_pads(generation, terminals, ids)?;
    let mut graphics = Vec::new();
    // The graphics a generator keys `pin1…` mark pin 1 (a dot, a bevel).
    let mut markers = Vec::new();
    let mut push = |key: String, layer: DrawingLayer, item: Item| {
        let id = GraphicId(ids.id(key.clone())?);
        if key.starts_with("g:pin1") && markers.len() < graphics::MAX_POLARITY_MARKERS {
            markers.push(id);
        }
        graphics.push(FootprintGraphic {
            id,
            layer,
            side: graphics::Side::Component,
            item,
        });
        Ok::<(), Error>(())
    };
    for graphic in &source.silk {
        push(
            format!("g:{}", graphic.key),
            DrawingLayer::Silkscreen,
            item(graphic)?,
        )?;
    }
    for graphic in &source.assembly {
        push(
            format!("g:{}", graphic.key),
            DrawingLayer::Assembly,
            item(graphic)?,
        )?;
    }
    if !source.courtyard.is_empty() {
        let courtyard = Item::Shape {
            shape: ShapeNm::Polygon {
                points: source
                    .courtyard
                    .iter()
                    .map(|p| point(*p))
                    .collect::<Result<_>>()?,
                radii_nm: Vec::new(),
            },
            width_nm: COURTYARD_STROKE,
            fill: graphics::Fill::None,
        };
        push("courtyard".into(), DrawingLayer::Courtyard, courtyard)?;
    }
    let polarity = polarity(generation, terminals, &pads, markers);
    Ok(FootprintDefinition {
        package,
        next_local: ids.next_local(),
        source: None,
        pads,
        graphics,
        polarity,
        datum: Default::default(),
        defaults: Default::default(),
        keepouts: source.keepouts.iter().map(keepout).collect::<Result<_>>()?,
    })
}

/// The footprint's pin-1 polarity from the generation: the first pad that
/// lands the package's pin 1, with the graphics the generator keyed
/// `pin1…`. Given when the generator marks pin 1 or the footprint has more
/// pads than a two-terminal part (where pin 1 tells a connector's or an
/// IC's orientation); a two-pad part without a marker (a resistor) has no
/// polarity.
fn polarity(
    generation: &v1::Generation,
    terminals: &Ids,
    pads: &[Pad],
    markers: Vec<GraphicId>,
) -> Option<graphics::Polarity> {
    let pin1 = terminals
        .of(
            &format!("t:{}", generation.package.geometry.pin1),
            "package terminal",
        )
        .ok()
        .map(PackageTerminalId)?;
    let pad = pads
        .iter()
        .find(|p| p.terminal == Some(pin1) && p.kind != PadKind::NonPlatedHole)?;
    let landed = pads
        .iter()
        .filter(|p| p.kind != PadKind::NonPlatedHole)
        .count();
    (!markers.is_empty() || landed > 2).then_some(graphics::Polarity {
        pad: pad.id,
        markers,
    })
}

/// A generated copper keep-out: its polygon on every copper layer, or on
/// the outer layer its side names.
fn keepout(k: &v1::Keepout) -> Result<FootprintKeepout> {
    let mm = |p: &v1::Point| point(*p).map(PointNm::to_mm_f64);
    let Some(first) = k.outline.first() else {
        return Err(refused(format!(
            "the keep-out \"{}\" has no points",
            k.name
        )));
    };
    let start = mm(first)?;
    let mut segments = k
        .outline
        .iter()
        .skip(1)
        .map(|p| Ok(Segment::Line { to: mm(p)? }))
        .collect::<Result<Vec<_>>>()?;
    segments.push(Segment::Line { to: start });
    Ok(FootprintKeepout {
        name: k.name.clone(),
        outline: Outline { start, segments },
        scope: match k.side {
            None => KeepoutScope::AllCopper,
            Some(v1::Side::Top) => KeepoutScope::ComponentSide,
            Some(v1::Side::Bottom) => KeepoutScope::OppositeSide,
        },
        forbid_pads: false,
    })
}

/// The generation's default top marking as a component marking: each line
/// one text item, laser-etched or (`ink`, `colour`) printed, stacked top
/// to bottom on the model's default marking surface or the part file's
/// surface it names (wrapped round a cylinder as it says), then its
/// polarity mark, its artwork (each packed as SVG under `licence`, the
/// generator's) and the pin-1 dot when asked; each further marking a decal
/// of its own. Every `field:<standard field>` a placeholder names is
/// rewritten to the project's field of that name (`field` finds it); one
/// already naming a lineage (`field:D12`) stays. `None` without a marking.
/// The text is the generator's data (its parameters); the template itself
/// is checked when the component is published. Returns the spec and the
/// artwork to pack with it.
pub fn marking(
    generation: &v1::Generation,
    licence: &str,
    mut field: impl FnMut(&str) -> Option<DefinitionId>,
) -> Result<Option<(crate::library::marking::MarkingSpec, Vec<PackedAsset>)>> {
    use crate::library::marking::MAX_DECALS;
    let Some(marking) = &generation.marking else {
        return Ok(None);
    };
    let mut assets = Vec::new();
    let Some(mut spec) = one_marking(marking, licence, &mut field, &mut assets)? else {
        return Ok(None);
    };
    if marking.decals.len() > MAX_DECALS {
        return Err(Error::Limit(
            "generated.marking: too many further markings".into(),
        ));
    }
    for decal in &marking.decals {
        if !decal.decals.is_empty() {
            return Err(Error::Invalid(
                "generated.marking: a further marking has further markings".into(),
            ));
        }
        if let Some(d) = one_marking(decal, licence, &mut field, &mut assets)? {
            spec.decals.push(d);
        }
    }
    Ok(Some((spec, assets)))
}

fn mark_place(at: v1::MarkPlace) -> crate::library::marking::Anchor {
    use crate::library::marking::Anchor;
    match at {
        v1::MarkPlace::Centre => Anchor::Centre,
        v1::MarkPlace::Pin1 => Anchor::Pin1Corner,
        v1::MarkPlace::Start => Anchor::Start,
        v1::MarkPlace::End => Anchor::End,
    }
}

/// One marking of a generation without its further markings.
fn one_marking(
    marking: &v1::Marking,
    licence: &str,
    field: &mut impl FnMut(&str) -> Option<DefinitionId>,
    assets: &mut Vec<PackedAsset>,
) -> Result<Option<crate::library::marking::MarkingSpec>> {
    use crate::library::marking::{
        ARTWORK_MEDIA_TYPE, Align, Anchor, Finish, MAX_ARTWORK_BYTES, MAX_MARKING_ITEMS,
        MAX_TEMPLATE, MarkingItem, MarkingSpec, Placement, PolarityMark, SurfaceRef, Template,
        Wrap,
    };
    let mut codes = std::collections::BTreeMap::new();
    for (mpn, code) in &marking.codes {
        let (key, code) = (
            crate::library::marking::code_key(mpn),
            code.trim().to_string(),
        );
        if key.is_empty()
            || code.is_empty()
            || key.len() > crate::library::marking::MAX_MARKING_CODE
            || code.len() > crate::library::marking::MAX_MARKING_CODE
        {
            return Err(Error::Limit(
                "generated.marking: a marking code entry is empty or too long".into(),
            ));
        }
        codes.insert(key, code);
    }
    if codes.len() > crate::library::marking::MAX_MARKING_CODES {
        return Err(Error::Limit(
            "generated.marking: too many marking codes".into(),
        ));
    }
    let items_wanted = marking.lines.len()
        + usize::from(marking.pin1_dot)
        + usize::from(marking.polarity.is_some())
        + marking.artwork.len();
    if items_wanted == 0 {
        return Ok(None);
    }
    if items_wanted > MAX_MARKING_ITEMS {
        return Err(Error::Limit(
            "generated.marking: too many marking lines and marks".into(),
        ));
    }
    let mut items = Vec::with_capacity(items_wanted);
    for line in &marking.lines {
        let out = field_lineages(line, field)?;
        if out.len() > MAX_TEMPLATE {
            return Err(Error::Limit(format!(
                "generated.marking: a marking line is longer than {MAX_TEMPLATE} bytes"
            )));
        }
        items.push(MarkingItem::Text {
            template: Template(out),
            height_nm: None,
            align: Align::Centre,
        });
    }
    if let Some(polarity) = marking.polarity {
        items.push(MarkingItem::Polarity {
            mark: match polarity.mark {
                v1::PolarityShape::Dot => PolarityMark::Dot,
                v1::PolarityShape::Bar => PolarityMark::Bar,
                v1::PolarityShape::Band => PolarityMark::Band,
                v1::PolarityShape::Stripe => PolarityMark::Stripe,
            },
            at: mark_place(polarity.at),
        });
    }
    for art in &marking.artwork {
        // An artwork template names standard fields in its text as a line
        // does; they become the project's lineages too.
        let bytes = field_lineages(&art.svg, field)?.into_bytes();
        if bytes.is_empty() || bytes.len() > MAX_ARTWORK_BYTES {
            return Err(Error::Limit(
                "generated.marking: marking artwork is empty or too large".into(),
            ));
        }
        let data = AssetBytes::new(bytes);
        let hash = data.hash();
        if assets.iter().all(|a: &PackedAsset| a.hash != hash) {
            assets.push(PackedAsset {
                hash,
                media_type: ARTWORK_MEDIA_TYPE.into(),
                data,
            });
        }
        items.push(MarkingItem::Artwork {
            svg: hash,
            size_nm: [Nm::new(art.size[0])?, Nm::new(art.size[1])?],
            anchor: mark_place(art.at),
            licence: Some(licence.chars().take(crate::ids::MAX_LABEL).collect()),
        });
    }
    if marking.pin1_dot {
        items.push(MarkingItem::Polarity {
            mark: PolarityMark::Dot,
            at: Anchor::Pin1Corner,
        });
    }
    let mut placement = Placement::default();
    if let Some(surface) = &marking.surface {
        placement.on = SurfaceRef::Anchor {
            name: surface.replace('-', "_"),
        };
    }
    if let Some([seam, span]) = marking.wrap {
        placement.wrap = Some(Wrap {
            seam_mdeg: crate::units::Angle::normalized(seam),
            wrap_mdeg: i32::try_from(span).map_err(|_| {
                Error::Limit("generated.marking: a wrap spans more than a turn".into())
            })?,
            ..Wrap::default()
        });
    }
    if let Some(problem) = placement.problem() {
        return Err(Error::Invalid(format!("generated.marking: {problem}")));
    }
    Ok(Some(MarkingSpec {
        items,
        finish: match (marking.colour, marking.ink, marking.metal) {
            (Some(colour), _, _) => Finish::Ink { colour },
            (None, true, _) => Finish::InkWhite,
            (None, false, true) => Finish::LaserEtchMetal,
            (None, false, false) => Finish::LaserEtch,
        },
        placement,
        text_height_nm: marking.text_height.map(Nm::new).transpose()?,
        codes,
        ..MarkingSpec::default()
    }))
}

/// `line` with every `field:<standard field>` a placeholder names (an
/// alternative's or a condition's too) turned into the project's field
/// lineage (`field:D12`); one already naming a lineage stays, and text
/// outside placeholders is kept as it is.
fn field_lineages(
    line: &str,
    field: &mut impl FnMut(&str) -> Option<DefinitionId>,
) -> Result<String> {
    const FIELD: &str = "field:";
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut inside = false;
    // Whether the next token of a placeholder starts here.
    let mut token = false;
    while let Some(c) = rest.chars().next() {
        if !inside && (rest.starts_with("{{") || rest.starts_with("}}")) {
            out.push_str(&rest[..2]);
            rest = &rest[2..];
            continue;
        }
        if inside && token && rest.starts_with(FIELD) {
            let after = &rest[FIELD.len()..];
            let end = after
                .find(['|', '}', '?', ';', '<', '>', '=', '^', '$', '*'])
                .unwrap_or(after.len());
            let (name, tail) = after.split_at(end);
            let name = name.trim();
            out.push_str(FIELD);
            let digits = name.strip_prefix('D').unwrap_or(name);
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                out.push_str(name);
            } else {
                let id = field(name).ok_or_else(|| {
                    Error::NotFound(format!(
                        "generated.marking: the marking names the field {name}, which this project does not have"
                    ))
                })?;
                out.push_str(&format!("D{}", id.0));
            }
            rest = tail;
            token = false;
            continue;
        }
        match c {
            '{' => {
                inside = true;
                token = true;
            }
            '}' => inside = false,
            ';' | '?' if inside => token = true,
            // A negated condition's subject follows its `!`.
            ' ' | '!' if inside => {}
            _ => token = false,
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
    Ok(out)
}

#[cfg(test)]
mod marking_tests {
    use super::*;
    use crate::library::marking::{
        Anchor, Finish, MarkingItem, PolarityMark, SurfaceRef, Template,
    };

    fn ids(name: &str) -> Option<DefinitionId> {
        match name {
            "Resistance" => Some(DefinitionId(7)),
            "Tolerance" => Some(DefinitionId(8)),
            _ => None,
        }
    }

    #[test]
    fn every_field_a_placeholder_names_becomes_its_lineage() -> Result<()> {
        let mut field = ids;
        assert_eq!(
            field_lineages(
                "{field:Resistance|code4?field:Tolerance<=1%;field:Resistance|code3}",
                &mut field
            )?,
            "{field:D7|code4?field:D8<=1%;field:D7|code3}"
        );
        assert_eq!(
            field_lineages("{{field:Resistance}} {mpn; field:D3}", &mut field)?,
            "{{field:Resistance}} {mpn; field:D3}"
        );
        assert!(field_lineages("{field:Colour}", &mut field).is_err());
        Ok(())
    }

    #[test]
    fn a_marking_maps_its_ink_surface_wrap_marks_artwork_and_decals() -> Result<()> {
        let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_owned();
        let marking = v1::Marking {
            lines: vec!["{field:Resistance|num:k}k".into()],
            colour: Some([1, 2, 3]),
            surface: Some("sleeve".into()),
            wrap: Some([-40_000, 80_000]),
            polarity: Some(v1::Polarity {
                mark: v1::PolarityShape::Bar,
                at: v1::MarkPlace::Start,
            }),
            artwork: vec![v1::Artwork {
                svg: svg.clone(),
                size: [1_000_000, 2_000_000],
                at: v1::MarkPlace::End,
            }],
            ..v1::Marking::default()
        };
        let mut assets = Vec::new();
        let spec = one_marking(&marking, "MIT", &mut ids, &mut assets)?
            .ok_or_else(|| Error::Invalid("unmarked".into()))?;
        assert_eq!(spec.finish, Finish::Ink { colour: [1, 2, 3] });
        assert_eq!(
            spec.placement.on,
            SurfaceRef::Anchor {
                name: "sleeve".into()
            }
        );
        let wrap = spec
            .placement
            .wrap
            .ok_or_else(|| Error::Invalid("unwrapped".into()))?;
        assert_eq!(
            (wrap.seam_mdeg.millidegrees(), wrap.wrap_mdeg),
            (320_000, 80_000)
        );
        assert_eq!(assets.len(), 1);
        assert_eq!(assets[0].data.as_slice(), svg.as_bytes());
        assert_eq!(
            spec.items,
            vec![
                MarkingItem::Text {
                    template: Template("{field:D7|num:k}k".into()),
                    height_nm: None,
                    align: crate::library::marking::Align::Centre,
                },
                MarkingItem::Polarity {
                    mark: PolarityMark::Bar,
                    at: Anchor::Start,
                },
                MarkingItem::Artwork {
                    svg: assets[0].hash,
                    size_nm: [Nm::new(1_000_000)?, Nm::new(2_000_000)?],
                    anchor: Anchor::End,
                    licence: Some("MIT".into()),
                },
            ]
        );
        // Nothing to mark is no marking.
        assert_eq!(
            one_marking(&v1::Marking::default(), "MIT", &mut ids, &mut assets)?,
            None
        );
        Ok(())
    }
}
