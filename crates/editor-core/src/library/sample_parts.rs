//! Original, unverified sample parts: generic two-terminal part sets (a
//! resistor, a capacitor, a two-pin connector) with a box envelope as
//! their 3D model, built from fixed original values, and a resistance
//! field. The synthetic catalog (`catalog::synthetic`) and tests build on
//! them; every value is ordinary definition data a user then edits.
use super::graphics::{self, HAlign, Justify, PIN_TEXT_HEIGHT, TextContent, VAlign};
pub use super::part_refs::SampleRefs;
use super::symbol_style::{self, SymbolConventions, SymbolKind};
use super::{
    BindingDefinition, ComponentDefinition, Definition, DefinitionMeta, DraftEdit,
    FootprintDefinition, ModelPlacement, PackageDefinition, PackageJoints, PackageLead,
    PackageModel, PackageTerminal, Pad, SymbolDefinition, Terminal, TerminalClass, TerminalMap,
    Unit,
};
use crate::diagnostics::Result;
use crate::fields::{FieldDefinition, QuantityKind, ValueType};
use crate::functions::{Direction, TerminalFunctions};
use crate::geometry::{Bounds, Point};
use crate::ids::{
    DefinitionId, FunctionId, GraphicId, PackageRef, PackageTerminalId, PadId, QuantityKindRef,
    TerminalId, UnitId,
};
use crate::quantity::{UnitOfMeasure, UnitSet};
use crate::units::Nm;

const PROVENANCE: &str =
    "Locally authored generic sample; verify against the selected component datasheet.";

fn meta(name: &str) -> DefinitionMeta {
    DefinitionMeta {
        name: name.into(),
        description: String::new(),
        provenance: PROVENANCE.into(),
    }
}

/// `mm` as nanometres, rounded (fixed fixture values, always in range).
fn typical_nm(mm: f64) -> Nm {
    Nm::from_mm_f64_rounded(mm).unwrap_or(Nm::ZERO)
}

/// What a sample is: its designator prefix, the symbol it draws (`None`
/// takes the one its prefix names, else a body rectangle) and, for a chip,
/// its terminations (`PackageJoints::Chip`: a resistor's or a ceramic
/// body).
#[derive(Clone, Copy, Debug)]
pub struct Kind<'a> {
    pub prefix: &'a str,
    pub drawn: Option<SymbolKind>,
    pub chip: Option<PackageJoints>,
}

impl Kind<'static> {
    pub const RESISTOR: Self = Self {
        prefix: "R",
        drawn: Some(SymbolKind::Resistor),
        chip: Some(PackageJoints::Chip { resistor: true }),
    };
    pub const CAPACITOR: Self = Self {
        prefix: "C",
        drawn: Some(SymbolKind::Capacitor),
        chip: Some(PackageJoints::Chip { resistor: false }),
    };
    pub const CONNECTOR: Self = Self {
        prefix: "J",
        drawn: None,
        chip: None,
    };
}

impl Kind<'_> {
    /// The drawn symbol of a two-terminal sample; `None` keeps the plain
    /// body rectangle (a connector).
    pub fn symbol(&self) -> Option<SymbolKind> {
        self.drawn
            .or_else(|| SymbolKind::for_prefix(self.prefix, false))
    }
}

struct Layout {
    /// The body rectangle, in the unit frame.
    body: Bounds,
    /// Pin connection ends.
    pins: [Point; 2],
    envelope: [f64; 3],
    offset: [f64; 3],
    pads: [Point; 2],
    land: Point,
    /// A kind-specific drawing in place of the body rectangle.
    drawn: Option<SymbolKind>,
}

fn sample(name: &str, kind: Kind<'_>, refs: SampleRefs, layout: Layout) -> Result<Vec<DraftEdit>> {
    let terminals = [TerminalId(1), TerminalId(2)];
    let package_terminals = [PackageTerminalId(1), PackageTerminalId(2)];
    let package = PackageDefinition {
        next_local: 3,
        terminals: package_terminals
            .iter()
            .map(|id| PackageTerminal {
                id: *id,
                designation: id.to_string(),
            })
            .collect(),
        envelope_nm: layout.envelope.map(typical_nm),
        model: PackageModel::default(),
        // An envelope keeps the layout's lift above the lands. A chip's
        // lead contacts are on the seating plane, so its body lifts only
        // 5 µm, within the seating tolerance (`package_fit`), clear of the
        // terminations the board draws down to the plane.
        placement: ModelPlacement {
            translation_nm: if kind.chip.is_some() {
                [layout.offset[0], layout.offset[1], 0.005].map(typical_nm)
            } else {
                layout.offset.map(typical_nm)
            },
            rotation_mdeg: [crate::units::Angle::ZERO; 3],
        },
        mechanical: None,
        datums: Vec::new(),
        // A chip's terminations stand on the land centres (length / 2 −
        // termination / 2), so its leads fit its pads.
        leads: if kind.chip.is_some() {
            package_terminals
                .iter()
                .zip(layout.pads)
                .map(|(terminal, at)| {
                    Ok(PackageLead {
                        terminal: *terminal,
                        contact: crate::units::PointNm::from_mm_f64_rounded(at)?,
                    })
                })
                .collect::<Result<_>>()?
        } else {
            Vec::new()
        },
        joints: kind.chip,
    };
    Ok(vec![
        DraftEdit {
            id: refs.component,
            meta: meta(name),
            definition: Definition::Component(ComponentDefinition {
                pin_rules: Vec::new(),
                marking: None,
                review: None,
                value_template: None,
                generic: None,
                // Terminals 1–2, then their fixed passive functions 3–4.
                next_local: 5,
                designator_prefix: kind.prefix.into(),
                terminals: terminals
                    .iter()
                    .zip([FunctionId(3), FunctionId(4)])
                    .map(|(id, function)| Terminal {
                        id: *id,
                        name: id.to_string(),
                        class: TerminalClass::Electrical(TerminalFunctions::fixed(
                            function,
                            Direction::Passive,
                        )),
                    })
                    .collect(),
                resources: vec![],
                constraints: vec![],
                representations: vec![refs.symbol],
                implementations: vec![refs.binding],
                fields: vec![],
            }),
        },
        DraftEdit {
            id: refs.symbol.id,
            meta: meta(&format!("{name} symbol")),
            definition: Definition::Symbol(
                layout
                    .drawn
                    .and_then(|kind| {
                        symbol_style::generate(
                            kind,
                            SymbolConventions::default(),
                            refs.component,
                            &terminals,
                        )
                        .ok()
                    })
                    .map_or_else(|| symbol(refs.component, &terminals, &layout), Ok)?,
            ),
        },
        DraftEdit {
            id: refs.package.id,
            meta: meta(&format!("{name} package")),
            definition: Definition::Package(package.clone()),
        },
        DraftEdit {
            id: refs.footprint.id,
            meta: meta(&format!("{name} footprint")),
            definition: Definition::Footprint(footprint(refs.package, &package, &layout)?),
        },
        DraftEdit {
            id: refs.binding.id,
            meta: meta(&format!("{name} binding")),
            definition: Definition::Binding(BindingDefinition {
                component: refs.component,
                package: refs.package,
                mappings: package_terminals
                    .iter()
                    .zip(terminals)
                    .map(|(package_terminal, terminal)| TerminalMap {
                        package_terminal: *package_terminal,
                        terminal,
                    })
                    .collect(),
                footprints: vec![refs.footprint],
                unavailable: vec![],
            }),
        },
    ])
}

/// Two rectangular top-side lands, pads 1–2 landing package terminals 1–2,
/// with the standard drawings (graphics from local ID 3).
fn footprint(
    package_ref: PackageRef,
    package: &PackageDefinition,
    layout: &Layout,
) -> Result<FootprintDefinition> {
    let pads = [PackageTerminalId(1), PackageTerminalId(2)]
        .iter()
        .zip(layout.pads)
        .zip([PadId(1), PadId(2)])
        .map(|((terminal, position), id)| {
            Pad::smd_rect_mm(id, *terminal, id.0.to_string(), position, layout.land)
        })
        .collect::<Result<_>>()?;
    let mut footprint = FootprintDefinition {
        package: package_ref,
        next_local: 3,
        pads,
        ..FootprintDefinition::default()
    };
    redraw(&mut footprint, package);
    Ok(footprint)
}

/// Replace a two-land sample footprint's drawings with the standard ones
/// for its current pads and `package` (graphics from local ID 3). A footprint
/// whose identities are exhausted keeps no drawings.
pub fn redraw(footprint: &mut FootprintDefinition, package: &PackageDefinition) {
    footprint.graphics.clear();
    footprint.polarity = None;
    footprint.next_local = 3;
    if super::footprint_drawings::add_standard_graphics(footprint, package).is_err() {
        footprint.graphics.clear();
    }
}

/// One unit: a body rectangle, a pin per terminal pointing into it, the
/// designator above the body and the component name below it. Unit 1 and
/// graphics 2–4 use the symbol's local IDs. The samples are passives and
/// connectors whose terminal names are their pin numbers, so pins show
/// numbers only.
fn symbol(
    component: DefinitionId,
    terminals: &[TerminalId],
    layout: &Layout,
) -> Result<SymbolDefinition> {
    let body = layout.body;
    let gap = 0.5;
    Ok(SymbolDefinition {
        component,
        next_local: 5,
        pin_text_height_nm: PIN_TEXT_HEIGHT,
        units: vec![Unit {
            id: UnitId(1),
            label: "A".into(),
            graphics: vec![
                graphics::body_rect(GraphicId(2), body)?,
                graphics::text(
                    GraphicId(3),
                    TextContent::Designator,
                    [body.center()[0], body.max[1] + gap],
                    Justify {
                        h: HAlign::Center,
                        v: VAlign::Bottom,
                    },
                )?,
                graphics::text(
                    GraphicId(4),
                    TextContent::ComponentName,
                    [body.center()[0], body.min[1] - gap],
                    Justify {
                        h: HAlign::Center,
                        v: VAlign::Top,
                    },
                )?,
            ],
            pins: terminals
                .iter()
                .zip(layout.pins)
                .map(|(terminal, position)| {
                    let mut pin = graphics::pin_toward(*terminal, position, body)?;
                    pin.name.visible = false;
                    Ok(pin)
                })
                .collect::<Result<_>>()?,
        }],
    })
}

/// A two-terminal component with an axial symbol (the kind-specific drawing
/// of [`Kind::symbol`], else a body rectangle), a small two-land footprint
/// and a straight terminal mapping.
pub fn two_terminal(name: &str, kind: Kind<'_>, refs: SampleRefs) -> Result<Vec<DraftEdit>> {
    sample(
        name,
        kind,
        refs,
        Layout {
            body: Bounds {
                min: [-2.5, -1.0],
                max: [2.5, 1.0],
            },
            pins: [[-5.0, 0.0], [5.0, 0.0]],
            envelope: [2.0, 1.2, 0.6],
            offset: [0.0, 0.0, 0.05],
            // Land centres on the chip's termination centres (length / 2 −
            // termination / 2 = 0.8 mm), so the leads fit their pads.
            pads: [[-0.8, 0.0], [0.8, 0.0]],
            land: [1.0, 1.4],
            drawn: kind.symbol(),
        },
    )
}

/// The same two terminals with both pins on one side and a 2.54 mm land
/// pitch.
pub fn two_terminal_header(name: &str, kind: Kind<'_>, refs: SampleRefs) -> Result<Vec<DraftEdit>> {
    sample(
        name,
        kind,
        refs,
        Layout {
            body: Bounds {
                min: [-2.5, -3.75],
                max: [2.5, 3.75],
            },
            pins: [[-5.0, 2.5], [-5.0, -2.5]],
            envelope: [5.0, 3.5, 4.0],
            offset: [0.0, 0.0, 0.05],
            pads: [[-1.27, 0.0], [1.27, 0.0]],
            land: [1.5, 2.0],
            drawn: None,
        },
    )
}

/// A resistance quantity kind (ohm, SI prefixes) and a non-negative
/// quantity field of that kind: ordinary library data, not a category.
pub fn resistance_field(kind: QuantityKindRef, field: DefinitionId) -> Vec<DraftEdit> {
    vec![
        DraftEdit {
            id: kind.id,
            meta: meta("Resistance"),
            definition: Definition::QuantityKind(QuantityKind {
                units: UnitSet {
                    dimension: [2, 1, -3, -2, 0, 0, 0],
                    units: vec![UnitOfMeasure {
                        symbol: "Ω".into(),
                        aliases: vec!["ohm".into(), "R".into()],
                        scale: 1.0,
                        si_prefixes: true,
                    }],
                },
                default_unit: "Ω".into(),
            }),
        },
        DraftEdit {
            id: field,
            meta: meta("Resistance"),
            definition: Definition::Field(FieldDefinition {
                next_local: 1,
                value_type: ValueType::Quantity {
                    kind,
                    min: Some(0.0),
                    max: None,
                },
                default: None,
                required_at: None,
            }),
        },
    ]
}
