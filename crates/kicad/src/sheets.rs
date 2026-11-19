//! A KiCad schematic drawn through the one command engine (slice K5):
//! one sheet per sheet instance, each symbol's unit where KiCad has it,
//! then its wires, junctions, labels, power symbols, sheet symbols with
//! their pins and no-connect flags. KiCad ties a symbol pin to the
//! footprint pad of the same number; the importer resolves that tie once,
//! to the terminal the pad lands, so every wire names its terminal
//! explicitly.
use crate::board::PointNm;
use crate::import::{ImportError, Session};
use crate::report::{Case, Report};
use crate::schematic::{self, KLabelKind, LibSymbol, Schematic, ours};
use sparksmith_core::geometry::Bounds;
use sparksmith_core::library::graphics::{
    self, HAlign, Justify, Pin, PinDirection, Rotation, TextContent, VAlign,
};
use sparksmith_core::library::{SymbolDefinition, Unit};
use sparksmith_core::model::sheet::{Paper, PaperOrientation, PortShape, SheetSide};
use sparksmith_core::model::{Corner, FreeEnd, JunctionRef, LabelKind, LabelRef, SheetPinRef};
use sparksmith_core::schematic::{SchematicCommand, WirePoint, WireTarget};
use sparksmith_core::units::{Nm, PointNm as OurPoint};
use sparksmith_core::{DefinitionId, GraphicId, UnitId};
use sparksmith_core::{Endpoint, Id, InstanceCommand, Operation, Outcome, SheetId, TerminalId};
use std::collections::{BTreeMap, BTreeSet};

/// Where each flattened sheet went and where each reference's unit goes.
#[derive(Default)]
pub(crate) struct Plan {
    pub(crate) sheets: Vec<SheetId>,
    /// Our sheet symbol of each flattened sheet's symbol in its parent.
    symbols: BTreeMap<(usize, usize), Id>,
    /// The flattened sheet each parent's sheet symbol shows.
    children: BTreeMap<(usize, usize), usize>,
    /// Reference (upper case) → where each of its units is drawn.
    pub(crate) places: BTreeMap<String, Vec<UnitPlace>>,
}

/// One unit of a symbol on a sheet: our sheet, position (mm) and turn.
#[derive(Clone, Copy, Debug)]
pub(crate) struct UnitPlace {
    pub(crate) unit: u32,
    pub(crate) sheet: SheetId,
    pub(crate) at: [f64; 2],
    pub(crate) rotation: Rotation,
    pub(crate) mirror: bool,
}

/// KiCad's turn and mirror as ours: a quarter-turn count and a
/// reflection across the unit's x-axis before it (`mirror y` is that
/// reflection turned half round).
fn orientation(symbol: &schematic::KSymbol) -> (Rotation, bool) {
    let quarter = |deg: f64| match (deg / 90.0).round().rem_euclid(4.0) as u8 {
        1 => Rotation::R90,
        2 => Rotation::R180,
        3 => Rotation::R270,
        _ => Rotation::R0,
    };
    if symbol.mirror_y {
        (quarter(symbol.angle + 180.0), true)
    } else {
        (quarter(symbol.angle), symbol.mirror_x)
    }
}

/// The library symbol of each reference (upper case), from the first sheet
/// that draws it; power symbols and `#` references aside.
pub(crate) fn symbols_by_reference(schematic: &Schematic) -> BTreeMap<String, LibSymbol> {
    let mut out = BTreeMap::new();
    for sheet in &schematic.sheets {
        let Some(file) = schematic.files.get(&sheet.file) else {
            continue;
        };
        for symbol in &file.symbols {
            let (reference, _) = schematic.reference(sheet, symbol);
            if reference.starts_with('#') {
                continue;
            }
            if let Some((false, lib)) = schematic.library(file, symbol) {
                out.entry(reference.to_ascii_uppercase())
                    .or_insert_with(|| lib.clone());
            }
        }
    }
    out
}

/// Each part's part number and manufacturer in the schematic, by upper-case
/// reference (power symbols left out), the first unit's that names one.
pub(crate) fn parts_by_reference(
    schematic: &Schematic,
) -> BTreeMap<String, crate::models::PartNumber> {
    let mut out: BTreeMap<String, crate::models::PartNumber> = BTreeMap::new();
    for sheet in &schematic.sheets {
        let Some(file) = schematic.files.get(&sheet.file) else {
            continue;
        };
        for symbol in &file.symbols {
            let (reference, _) = schematic.reference(sheet, symbol);
            if reference.starts_with('#') {
                continue;
            }
            let part = out.entry(reference.to_ascii_uppercase()).or_default();
            if part.mpn.is_none() {
                part.mpn = symbol.part.mpn.clone();
            }
            if part.manufacturer.is_none() {
                part.manufacturer = symbol.part.manufacturer.clone();
            }
        }
    }
    out
}

/// Each part's Value property in the schematic, by upper-case reference
/// (power symbols left out), the first unit's where units differ.
pub(crate) fn values_by_reference(schematic: &Schematic) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for sheet in &schematic.sheets {
        let Some(file) = schematic.files.get(&sheet.file) else {
            continue;
        };
        for symbol in &file.symbols {
            let (reference, _) = schematic.reference(sheet, symbol);
            if !reference.starts_with('#') {
                out.entry(reference.to_ascii_uppercase())
                    .or_insert_with(|| symbol.value.clone());
            }
        }
    }
    out
}

fn mm(p: PointNm) -> [f64; 2] {
    [p[0] as f64 / 1e6, p[1] as f64 / 1e6]
}

/// Our symbol for a KiCad library symbol: a unit per KiCad unit, each pin
/// where KiCad has it (so wires drawn where KiCad draws them meet it),
/// tied to the terminal its number names, and a body of its rectangles
/// (else round its pins). A terminal no pin names (an unmapped pad) gets a
/// visible pin on the first unit, marked added by import ([`added_pins`]).
pub(crate) fn kicad_symbol(
    component: DefinitionId,
    lib: &LibSymbol,
    terminals: &[(TerminalId, &str)],
    unpinned: &mut usize,
) -> sparksmith_core::Result<SymbolDefinition> {
    let mut units: BTreeSet<u32> = lib
        .pins
        .iter()
        .map(|p| p.unit)
        .chain(lib.rects.iter().map(|r| r.0))
        .filter(|u| *u > 0)
        .collect();
    if units.is_empty() {
        units.insert(1);
    }
    let first = units.first().copied().unwrap_or(1);
    let mut pinned = BTreeSet::new();
    let mut next = 2u32;
    let mut out = Vec::new();
    for (k, unit) in units.iter().enumerate() {
        let of_unit = |u: u32| u == *unit || (u == 0 && *unit == first);
        let mut pins = Vec::new();
        let mut ends: Vec<[f64; 2]> = Vec::new();
        for pin in lib.pins.iter().filter(|p| of_unit(p.unit)) {
            let Some((t, _)) = terminals.iter().find(|(_, n)| *n == pin.number) else {
                continue;
            };
            if !pinned.insert(*t) {
                continue;
            }
            let direction = match ((pin.angle / 90.0).round().rem_euclid(4.0)) as u8 {
                1 => PinDirection::Up,
                2 => PinDirection::Left,
                3 => PinDirection::Down,
                _ => PinDirection::Right,
            };
            let at = mm(pin.at);
            let length = (pin.length as f64 / 1e6).max(0.1);
            let (s, c) = pin.angle.to_radians().sin_cos();
            ends.push([at[0] + c * length, at[1] + s * length]);
            pins.push(Pin::new(
                *t,
                OurPoint::from_mm_f64_rounded(at)?,
                direction,
                Nm::from_mm_f64_rounded(length)?,
            ));
        }
        let rects: Vec<[f64; 2]> = lib
            .rects
            .iter()
            .filter(|r| of_unit(r.0))
            .flat_map(|r| [mm(r.1), mm(r.2)])
            .collect();
        let mut body = Bounds::of(if rects.is_empty() { ends } else { rects })
            .unwrap_or(Bounds::point([0.0, 0.0]));
        for axis in 0..2 {
            if body.max[axis] - body.min[axis] < 1.27 {
                let c = (body.max[axis] + body.min[axis]) / 2.0;
                body.min[axis] = c - 0.635;
                body.max[axis] = c + 0.635;
            }
        }
        if *unit == first {
            let missing: Vec<TerminalId> = terminals
                .iter()
                .map(|(t, _)| *t)
                .filter(|t| !pinned.contains(t))
                .collect();
            added_pins(&mut pins, &mut body, &missing)?;
            *unpinned += missing.len();
            pinned.extend(missing);
        }
        let gap = 1.0;
        let graphics = vec![
            graphics::body_rect(GraphicId(next), body)?,
            graphics::text(
                GraphicId(next + 1),
                TextContent::Designator,
                [body.center()[0], body.max[1] + gap],
                Justify {
                    h: HAlign::Center,
                    v: VAlign::Bottom,
                },
            )?,
            graphics::text(
                GraphicId(next + 2),
                TextContent::ComponentName,
                [body.center()[0], body.min[1] - gap],
                Justify {
                    h: HAlign::Center,
                    v: VAlign::Top,
                },
            )?,
        ];
        next += 3;
        let label = char::from_u32(u32::from(b'A') + (k as u32 % 26))
            .map(String::from)
            .unwrap_or_default();
        out.push(Unit {
            id: UnitId(*unit),
            label,
            graphics,
            pins,
        });
    }
    Ok(SymbolDefinition {
        component,
        next_local: next,
        pin_text_height_nm: graphics::PIN_TEXT_HEIGHT,
        units: out,
    })
}

/// Length of a pin added by import, and the step between such pins, mm.
const ADDED_PIN_MM: f64 = 2.54;

/// Pins for `missing` terminals on the body's right side, one step apart
/// from the top down, each clear of the unit's other pins, marked added by
/// import; the body grows down when its side is full. No wire meets them:
/// the source schematic leaves these pads unconnected.
fn added_pins(
    pins: &mut Vec<Pin>,
    body: &mut Bounds,
    missing: &[TerminalId],
) -> sparksmith_core::Result<()> {
    let step = ADDED_PIN_MM;
    let x = body.max[0] + ADDED_PIN_MM;
    // The other pins' stubs, mm: connection end and body end.
    let stubs: Vec<([f64; 2], [f64; 2])> = pins
        .iter()
        .map(|p| {
            let a = p.position_mm();
            let v = p.direction.vector();
            let l = p.length_mm();
            (a, [a[0] + v[0] * l, a[1] + v[1] * l])
        })
        .collect();
    let right = body.max[0];
    let clear = |y: f64| {
        let (a, b) = ([x, y], [right, y]);
        let d = sparksmith_core::geometry::segment_distance;
        stubs.iter().all(|(p, q)| {
            d(*p, a, b)
                .min(d(*q, a, b))
                .min(d(a, *p, *q))
                .min(d(b, *p, *q))
                > step / 2.0 - 1e-9
        })
    };
    let mut k = 0usize;
    for t in missing {
        let y = loop {
            k += 1;
            let y = body.max[1] - step * k as f64;
            // Past the body's side every place is clear: the body grows.
            if y < body.min[1] + step / 2.0 || clear(y) {
                break y;
            }
        };
        body.min[1] = body.min[1].min(y - step / 2.0);
        let mut pin = Pin::new(
            *t,
            OurPoint::from_mm_f64_rounded([x, y])?,
            PinDirection::Left,
            Nm::from_mm_f64_rounded(ADDED_PIN_MM)?,
        );
        pin.added_by_import = true;
        pins.push(pin);
    }
    Ok(())
}

fn refused(step: &'static str, e: impl std::fmt::Display) -> ImportError {
    ImportError::Refused {
        step,
        reason: e.to_string(),
    }
}

fn nm_point(p: [f64; 2]) -> Result<OurPoint, ImportError> {
    OurPoint::from_mm_f64_rounded(p).map_err(|e| refused("schematic.point", e))
}

fn paper(name: &str) -> Paper {
    match name {
        "A3" => Paper::A3,
        "A2" => Paper::A2,
        "A1" => Paper::A1,
        "A0" => Paper::A0,
        "USLetter" | "A" => Paper::Letter,
        "USLegal" => Paper::Legal,
        "USLedger" | "B" => Paper::Tabloid,
        _ => Paper::A4,
    }
}

/// The sheets: the root is our first sheet; each other instance is a new
/// sheet under a sheet symbol on its parent, named as KiCad names it
/// (repeats numbered). Records where every symbol's unit goes.
pub(crate) fn sheets(
    session: &mut Session<'_>,
    schematic: &Schematic,
    report: &mut Report,
) -> Result<Plan, ImportError> {
    let mut plan = Plan::default();
    let mut names = BTreeSet::new();
    for sheet in &schematic.sheets {
        let file = schematic
            .files
            .get(&sheet.file)
            .ok_or_else(|| refused("sheet.add", "a sheet file"))?;
        let id = match sheet.parent {
            None => session.engine.project().sheets.first(),
            Some((parent, k)) => {
                let parent_file = schematic
                    .files
                    .get(&schematic.sheets[parent].file)
                    .ok_or_else(|| refused("sheet_symbol.place", "a sheet file"))?;
                let symbol = &parent_file.sheets[k];
                let base = crate::routing::label_name(&symbol.name);
                let mut name = base.clone();
                let mut n = 2;
                while !names.insert(name.to_lowercase()) {
                    name = format!("{base} {n}");
                    n += 1;
                }
                let top_left = ours(symbol.at);
                let outcome = session.apply(
                    "sheet_symbol.place",
                    Operation::Schematic(SchematicCommand::PlaceSheetSymbol {
                        sheet: plan.sheets.get(parent).copied(),
                        child: None,
                        name: Some(name),
                        position: nm_point(top_left)?,
                        size: nm_point([symbol.size[0] as f64 / 1e6, symbol.size[1] as f64 / 1e6])?,
                    }),
                )?;
                let Outcome::SheetSymbolPlaced {
                    symbol: ours_id,
                    child,
                } = outcome
                else {
                    return Err(refused("sheet_symbol.place", "no sheet symbol"));
                };
                plan.symbols.insert((parent, k), ours_id);
                plan.children.insert((parent, k), plan.sheets.len());
                report.add(Case::SheetSymbol);
                child
            }
        };
        let paper = paper(&file.paper);
        let page = if file.portrait {
            PaperOrientation::Portrait
        } else {
            PaperOrientation::Landscape
        };
        let _ = session.apply(
            "sheet.set_page",
            Operation::Schematic(SchematicCommand::SetPage {
                sheet: id,
                paper,
                orientation: page,
            }),
        );
        plan.sheets.push(id);
        for symbol in &file.symbols {
            let (reference, unit) = schematic.reference(sheet, symbol);
            if reference.starts_with('#') {
                continue;
            }
            let (rotation, mirror) = orientation(symbol);
            let units = plan
                .places
                .entry(reference.to_ascii_uppercase())
                .or_default();
            // One place per unit; a unit drawn twice keeps its first.
            if !units.iter().any(|u| u.unit == unit) {
                units.push(UnitPlace {
                    unit,
                    sheet: id,
                    at: ours(symbol.at),
                    rotation,
                    mirror,
                });
            }
        }
    }
    report.counts.sheets = plan.sheets.len();
    Ok(plan)
}

/// Turn each placed unit as KiCad turns it and place the other units.
pub(crate) fn units(
    session: &mut Session<'_>,
    instances: &[Id],
    first_occurrences: &[Option<Id>],
    first_units: &[Option<UnitPlace>],
    later_units: &[Vec<UnitPlace>],
    report: &mut Report,
) {
    for (k, instance) in instances.iter().enumerate() {
        let mut occurrences: Vec<(Id, UnitPlace)> = Vec::new();
        if let (Some(Some(o)), Some(Some(u))) = (first_occurrences.get(k), first_units.get(k)) {
            occurrences.push((*o, *u));
        }
        for u in later_units.get(k).into_iter().flatten() {
            let Ok(position) = nm_point(u.at) else {
                continue;
            };
            match session.apply(
                "symbol.place_unit",
                Operation::Schematic(SchematicCommand::PlaceUnit {
                    instance: *instance,
                    unit: UnitId(u.unit.max(1)),
                    position,
                    sheet: Some(u.sheet),
                }),
            ) {
                Ok(Outcome::UnitPlaced { occurrence }) => occurrences.push((occurrence, *u)),
                _ => report.add(Case::UnitRefused),
            }
        }
        for (occurrence, u) in occurrences {
            if u.rotation == Rotation::R0 && !u.mirror {
                continue;
            }
            if session
                .apply(
                    "symbol.orient_unit",
                    Operation::Schematic(SchematicCommand::OrientUnit {
                        occurrence,
                        rotation: u.rotation,
                        mirror: u.mirror,
                    }),
                )
                .is_err()
            {
                report.add(Case::UnitRefused);
            }
        }
    }
}

/// Where a KiCad label name joins: its sheet (local and hierarchical
/// labels) or everywhere (global labels and power symbols).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Scope {
    Sheet(usize),
    Global,
}

/// Our kinds, ranked for choosing a joined net's one name: power and
/// global first, then hierarchical, then local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Rank {
    Power,
    Global,
    Hierarchical,
    Local,
}

type Key = (Scope, String);

fn key_of(sheet: usize, rank: Rank, name: &str) -> Key {
    let scope = match rank {
        Rank::Power | Rank::Global => Scope::Global,
        Rank::Hierarchical | Rank::Local => Scope::Sheet(sheet),
    };
    (scope, name.to_owned())
}

fn kind_of(rank: Rank) -> LabelKind {
    match rank {
        Rank::Power => LabelKind::Power {
            style: Default::default(),
        },
        Rank::Global => LabelKind::Global {
            shape: PortShape::default(),
        },
        Rank::Hierarchical => LabelKind::Hierarchical {
            shape: PortShape::default(),
        },
        Rank::Local => LabelKind::Local,
    }
}

/// The names labels take. KiCad compares names exactly and lets one net
/// carry several; ours compare them without case and give a net one. So
/// names that differ only in case (or in what a label name cannot hold)
/// are spelled apart, and the names KiCad's wires join into one net
/// take that net's one name (power and global first, then hierarchical,
/// then local; then the first in order).
#[derive(Default)]
struct Naming {
    rank: BTreeMap<Key, Rank>,
    canonical: BTreeMap<Key, Key>,
    ours: BTreeMap<Key, String>,
}

impl Naming {
    fn of(schematic: &Schematic) -> Self {
        let mut naming = Naming::default();
        let mut parent: BTreeMap<Key, Key> = BTreeMap::new();
        fn root(parent: &mut BTreeMap<Key, Key>, k: &Key) -> Key {
            let mut r = k.clone();
            while let Some(p) = parent.get(&r) {
                if *p == r {
                    break;
                }
                r = p.clone();
            }
            r
        }
        for (index, sheet) in schematic.sheets.iter().enumerate() {
            let Some(file) = schematic.files.get(&sheet.file) else {
                continue;
            };
            let mut labels: Vec<(PointNm, Rank, String)> = file
                .labels
                .iter()
                .map(|l| {
                    let rank = match l.kind {
                        KLabelKind::Local => Rank::Local,
                        KLabelKind::Global => Rank::Global,
                        KLabelKind::Hierarchical => Rank::Hierarchical,
                    };
                    (l.at, rank, l.name.clone())
                })
                .collect();
            for symbol in &file.symbols {
                if let Some((true, pins)) = schematic.lib(file, symbol) {
                    for pin in pins {
                        labels.push((
                            schematic::pin_on_sheet(symbol, pin.at),
                            Rank::Power,
                            symbol.value.clone(),
                        ));
                    }
                }
            }
            // KiCad's wires on this sheet, as connected point sets.
            let points: Vec<PointNm> = labels.iter().map(|l| l.0).collect();
            let mut group: BTreeMap<PointNm, PointNm> = BTreeMap::new();
            fn find(group: &mut BTreeMap<PointNm, PointNm>, p: PointNm) -> PointNm {
                let mut r = p;
                while let Some(q) = group.get(&r).copied() {
                    if q == r {
                        break;
                    }
                    r = q;
                }
                r
            }
            for stroke in schematic::strokes(&file.wires, &points, |_| false) {
                for w in stroke.windows(2) {
                    let (a, b) = (find(&mut group, w[0]), find(&mut group, w[1]));
                    group.entry(a).or_insert(a);
                    if a != b {
                        group.insert(b, a);
                    }
                }
            }
            let mut first_at: BTreeMap<PointNm, Key> = BTreeMap::new();
            for (p, rank, name) in &labels {
                let key = key_of(index, *rank, name);
                naming.rank.entry(key.clone()).or_insert(*rank);
                parent.entry(key.clone()).or_insert_with(|| key.clone());
                let g = find(&mut group, *p);
                match first_at.get(&g).cloned() {
                    None => {
                        first_at.insert(g, key);
                    }
                    Some(other) => {
                        let (a, b) = (root(&mut parent, &other), root(&mut parent, &key));
                        if a != b {
                            parent.insert(b, a);
                        }
                    }
                }
            }
        }
        // Each class's name: the best-ranked, then the first.
        let keys: Vec<Key> = parent.keys().cloned().collect();
        let mut best: BTreeMap<Key, Key> = BTreeMap::new();
        for k in &keys {
            let r = root(&mut parent, k);
            let rank = |k: &Key| naming.rank.get(k).copied().unwrap_or(Rank::Local);
            let better = best
                .get(&r)
                .is_none_or(|b| (rank(k), k.1.as_str()) < (rank(b), b.1.as_str()));
            if better {
                best.insert(r, k.clone());
            }
        }
        // Names apart where ours would join them (case, spelling).
        let mut taken: BTreeMap<Scope, BTreeSet<String>> = BTreeMap::new();
        for k in &keys {
            let r = root(&mut parent, k);
            let c = best.get(&r).cloned().unwrap_or_else(|| k.clone());
            naming.canonical.insert(k.clone(), c.clone());
            if naming.ours.contains_key(&c) {
                continue;
            }
            let base = crate::routing::label_name(&c.1);
            let used = taken.entry(c.0.clone()).or_default();
            let mut name = base.clone();
            let mut n = 2;
            while !used.insert(name.to_ascii_lowercase()) {
                name = format!("{base}~{n}");
                n += 1;
            }
            naming.ours.insert(c, name);
        }
        naming
    }

    /// The kind and name a label of `rank` and KiCad `name` on sheet
    /// `index` takes, and whether that is another label's (an alias).
    fn label(&self, index: usize, rank: Rank, name: &str) -> (LabelKind, String, bool) {
        let key = key_of(index, rank, name);
        let c = self.canonical.get(&key).cloned().unwrap_or(key.clone());
        let r = self.rank.get(&c).copied().unwrap_or(rank);
        let ours = self
            .ours
            .get(&c)
            .cloned()
            .unwrap_or_else(|| crate::routing::label_name(name));
        (kind_of(r), ours, c != key)
    }
}

/// What sits at one KiCad point of a sheet.
#[derive(Default)]
struct Anchors {
    pins: Vec<Endpoint>,
    labels: Vec<Id>,
    sheet_pins: Vec<Id>,
    no_connect: bool,
}

impl Anchors {
    fn target(&self) -> Option<WireTarget> {
        if let Some(p) = self.pins.first() {
            return Some(WireTarget::Terminal(*p));
        }
        if let Some(l) = self.labels.first() {
            return Some(WireTarget::Label(LabelRef { label: *l }));
        }
        self.sheet_pins
            .first()
            .map(|p| WireTarget::SheetPin(SheetPinRef { sheet_pin: *p }))
    }
}

/// The terminal of `instance` a KiCad pin numbered `number` connects: the
/// pad of that number on the part's footprint, through the binding's
/// explicit map (a generated part's terminals carry their own names, and
/// a pad lands its package terminal, so the pin is never matched by
/// name). A pad without a terminal, or no such pad, connects nothing.
fn terminal(project: &sparksmith_core::Project, instance: Id, number: &str) -> Option<TerminalId> {
    let found = project.instance(instance)?;
    let resolved = sparksmith_core::instances::resolve(&project.library, found).ok()?;
    let physical = resolved.physical?;
    let landed = physical
        .footprint
        .definition
        .pads
        .iter()
        .filter(|p| p.number == number)
        .find_map(|p| p.terminal)?;
    physical
        .binding
        .definition
        .mappings
        .iter()
        .find(|m| m.package_terminal == landed)
        .map(|m| m.terminal)
}

/// Draw every sheet's wiring.
pub(crate) fn wire(
    session: &mut Session<'_>,
    schematic: &Schematic,
    plan: &Plan,
    report: &mut Report,
) -> Result<(), ImportError> {
    let naming = Naming::of(schematic);
    for (index, sheet) in schematic.sheets.iter().enumerate() {
        let Some(file) = schematic.files.get(&sheet.file) else {
            continue;
        };
        let Some(&sid) = plan.sheets.get(index) else {
            continue;
        };
        let mut at: BTreeMap<PointNm, Anchors> = BTreeMap::new();
        // Pins of parts with their own symbols, at KiCad's pin point.
        let mut moved: Vec<(PointNm, Endpoint)> = Vec::new();
        // Those of them a power port went on.
        let mut on_pin: BTreeSet<Endpoint> = BTreeSet::new();
        // Pins of placed parts; power symbols become power ports.
        let mut power: Vec<(PointNm, String)> = Vec::new();
        // Hidden power inputs: joined to the global net of their name.
        let mut implicit: Vec<(Endpoint, String)> = Vec::new();
        for symbol in &file.symbols {
            let Some((is_power, pins)) = schematic.lib(file, symbol) else {
                report.add(Case::SymbolWithoutLibrary);
                continue;
            };
            let (reference, unit) = schematic.reference(sheet, symbol);
            if is_power {
                for pin in pins {
                    power.push((
                        schematic::pin_on_sheet(symbol, pin.at),
                        symbol.value.clone(),
                    ));
                }
                continue;
            }
            let project = session.engine.project();
            let instance = project
                .instance_by_designator(&reference)
                .filter(|i| !i.board_only && project.occurrences_of(i.id).next().is_some())
                .map(|i| i.id);
            let Some(instance) = instance else {
                report.add(Case::SymbolWithoutFootprint);
                continue;
            };
            // A generated part draws its generator's symbol, not KiCad's.
            let own_symbol = project.instance(instance).is_some_and(|i| {
                project
                    .library
                    .generated
                    .iter()
                    .any(|g| g.root.id == i.component.id)
            });
            for pin in pins.iter().filter(|p| p.unit == 0 || p.unit == unit) {
                if pin.no_connect {
                    report.add(Case::NoConnectPin);
                    continue;
                }
                if pin.hidden_power {
                    match terminal(project, instance, &pin.number) {
                        Some(t) => implicit.push((
                            Endpoint {
                                instance,
                                terminal: t,
                            },
                            pin.name.clone(),
                        )),
                        None => report.add(Case::PinWithoutPad),
                    }
                    continue;
                }
                match terminal(project, instance, &pin.number) {
                    // A pin of a symbol drawn as KiCad draws it anchors
                    // the wiring where KiCad has it; one of a part's own
                    // symbol (a generated part's) lies elsewhere, so the
                    // wiring is drawn as KiCad has it and the pin joins it
                    // afterwards by a wire of its own.
                    Some(t) if own_symbol => moved.push((
                        schematic::pin_on_sheet(symbol, pin.at),
                        Endpoint {
                            instance,
                            terminal: t,
                        },
                    )),
                    Some(t) => at
                        .entry(schematic::pin_on_sheet(symbol, pin.at))
                        .or_default()
                        .pins
                        .push(Endpoint {
                            instance,
                            terminal: t,
                        }),
                    None => report.add(Case::PinWithoutPad),
                }
            }
        }
        for p in &file.no_connects {
            at.entry(*p).or_default().no_connect = true;
        }
        clear_own_symbols(session, sid, file, &at, &moved, &power)?;
        // Sheet symbols' pins on this sheet.
        for (k, symbol) in file.sheets.iter().enumerate() {
            let Some(&ours_id) = plan.symbols.get(&(index, k)) else {
                continue;
            };
            for (name, p) in &symbol.pins {
                let (x, y, w, h) = (symbol.at[0], symbol.at[1], symbol.size[0], symbol.size[1]);
                let (side, offset) = if p[0] == x {
                    (SheetSide::Left, p[1] - y)
                } else if p[0] == x + w {
                    (SheetSide::Right, p[1] - y)
                } else if p[1] == y {
                    (SheetSide::Top, p[0] - x)
                } else {
                    let _ = h;
                    (SheetSide::Bottom, p[0] - x)
                };
                let added = session.apply(
                    "sheet_pin.add",
                    Operation::Schematic(SchematicCommand::AddSheetPin {
                        symbol: ours_id,
                        name: naming
                            .label(k_child(plan, index, k), Rank::Hierarchical, name)
                            .1,
                        shape: PortShape::default(),
                        side,
                        offset: Nm::try_new(offset.max(0)).unwrap_or(Nm::ZERO),
                    }),
                );
                match added {
                    Ok(Outcome::SheetPinsAdded { pins }) => {
                        at.entry(*p).or_default().sheet_pins.extend(pins);
                    }
                    _ => report.add(Case::SchematicItemRefused),
                }
            }
        }
        // Labels and power ports, attached to a pin or sheet pin at their
        // point when there is one.
        let labels = file
            .labels
            .iter()
            .map(|l| {
                let rank = match l.kind {
                    KLabelKind::Local => Rank::Local,
                    KLabelKind::Global => Rank::Global,
                    KLabelKind::Hierarchical => Rank::Hierarchical,
                };
                (l.at, l.name.clone(), rank)
            })
            .chain(power.into_iter().map(|(p, name)| (p, name, Rank::Power)))
            .collect::<Vec<_>>();
        for (p, name, rank) in labels {
            let (kind, name, alias) = naming.label(index, rank, &name);
            if alias {
                report.add(Case::LabelAlias);
            }
            let attach = at.get(&p).and_then(|a| {
                a.pins
                    .first()
                    .map(|e| WireTarget::Terminal(*e))
                    .or_else(|| {
                        a.sheet_pins
                            .first()
                            .map(|s| WireTarget::SheetPin(SheetPinRef { sheet_pin: *s }))
                    })
            });
            // A power port on the point of a pin of a part's own symbol goes
            // on that pin, as it would on KiCad's pin: a port is its net's
            // name where it stands, and no wire of KiCad's meets it there.
            let attach = attach.or_else(|| {
                (rank == Rank::Power)
                    .then(|| moved.iter().find(|(q, _)| *q == p))
                    .flatten()
                    .map(|(_, end)| {
                        on_pin.insert(*end);
                        WireTarget::Terminal(*end)
                    })
            });

            let placed = session.apply(
                "label.place",
                Operation::Schematic(SchematicCommand::PlaceLabel {
                    sheet: Some(sid),
                    name,
                    kind,
                    position: nm_point(ours(p))?,
                    rotation: Default::default(),
                    attach,
                }),
            );
            match placed {
                Ok(Outcome::LabelPlaced { label }) => {
                    report.add(Case::SchematicLabel);
                    at.entry(p).or_default().labels.push(label);
                }
                _ => report.add(Case::SchematicItemRefused),
            }
        }
        // Hidden power inputs: a power port of their name on the pin.
        for (end, name) in implicit {
            let (kind, name, _) = naming.label(index, Rank::Power, &name);
            let Some(at) = sparksmith_core::schematic::pin_position(session.engine.project(), end)
            else {
                continue;
            };
            let placed = session.apply(
                "label.place",
                Operation::Schematic(SchematicCommand::PlaceLabel {
                    sheet: Some(sid),
                    name,
                    kind,
                    position: nm_point(at)?,
                    rotation: Default::default(),
                    attach: Some(WireTarget::Terminal(end)),
                }),
            );
            match placed {
                Ok(_) => report.add(Case::HiddenPowerPin),
                Err(_) => report.add(Case::SchematicItemRefused),
            }
        }
        // Pins that meet nothing (no wire, label, other pin or flag).
        for (p, anchors) in &at {
            let touched = anchors.pins.len() > 1
                || !anchors.labels.is_empty()
                || !anchors.sheet_pins.is_empty()
                || anchors.no_connect
                || file
                    .wires
                    .iter()
                    .any(|w| w[0] == *p || w[1] == *p || schematic::inside(*p, w[0], w[1]));
            if !touched {
                report.add_n(Case::PinUnconnected, anchors.pins.len());
            }
            let mid = file.wires.iter().any(|w| schematic::inside(*p, w[0], w[1]))
                && !file.wires.iter().any(|w| w[0] == *p || w[1] == *p)
                && !file.junctions.contains(p);
            if mid {
                report.add_n(
                    Case::PinOnWire,
                    anchors.pins.len().max(anchors.labels.len()),
                );
            }
        }
        // Several pins at one point: wired to the first.
        for anchors in at.values() {
            if let Some((first, rest)) = anchors.pins.split_first() {
                for other in rest {
                    draw(
                        session,
                        sid,
                        WireTarget::Terminal(*other),
                        &[],
                        WireTarget::Terminal(*first),
                        report,
                    );
                }
            }
        }
        // No-connect flags.
        for anchors in at.values().filter(|a| a.no_connect) {
            for pin in &anchors.pins {
                let _ = session.apply(
                    "instance.set_no_connect",
                    Operation::Instance(InstanceCommand::SetNoConnect {
                        instance: pin.instance,
                        terminals: vec![pin.terminal],
                        no_connect: true,
                    }),
                );
            }
        }
        // Wires, as strokes between anchored points.
        let extra: Vec<PointNm> = at
            .keys()
            .copied()
            .chain(file.junctions.iter().copied())
            .collect();
        let keyed: BTreeSet<PointNm> = at
            .iter()
            .filter(|(_, a)| a.target().is_some())
            .map(|(p, _)| *p)
            .collect();
        let mut pending = schematic::strokes(&file.wires, &extra, |p| keyed.contains(&p));
        // Draw strokes whose ends are known; one that ends on another
        // stroke waits until that stroke is drawn.
        while !pending.is_empty() {
            let before = pending.len();
            let mut later = Vec::new();
            for stroke in pending {
                let (Some(first), Some(last)) = (stroke.first(), stroke.last()) else {
                    continue;
                };
                let end = |session: &Session<'_>, p: PointNm| -> Option<WireTarget> {
                    if let Some(t) = at.get(&p).and_then(Anchors::target) {
                        return Some(t);
                    }
                    on_sheet(session, sid, p)
                };
                let (a, b) = (end(session, *first), end(session, *last));
                let dangling =
                    |p: &PointNm| !keyed.contains(p) && !stroke_through(&file.wires, *p, &stroke);
                let a = a.or_else(|| dangling(first).then(|| free(*first)).flatten());
                let b = b.or_else(|| dangling(last).then(|| free(*last)).flatten());
                match (a, b) {
                    (Some(a), Some(b)) => {
                        let corners: Vec<Corner> = stroke[1..stroke.len() - 1]
                            .iter()
                            .filter_map(|p| Corner::sharp_mm(ours(*p)).ok())
                            .collect();
                        draw(session, sid, a, &corners, b, report);
                    }
                    _ => later.push(stroke),
                }
            }
            if later.len() == before {
                // Ends on nothing drawn: free ends.
                for stroke in later {
                    let (Some(a), Some(b)) = (
                        stroke.first().and_then(|p| free(*p)),
                        stroke.last().and_then(|p| free(*p)),
                    ) else {
                        continue;
                    };
                    let corners: Vec<Corner> = stroke[1..stroke.len() - 1]
                        .iter()
                        .filter_map(|p| Corner::sharp_mm(ours(*p)).ok())
                        .collect();
                    draw(session, sid, a, &corners, b, report);
                }
                break;
            }
            pending = later;
        }
        // Each pin of a part's own symbol joins what KiCad's pin met: a
        // pin, label or sheet pin there, else KiCad's wiring at that point
        // (a junction is placed first where the point is inside a wire, so
        // the join does not depend on the wires drawn to such pins, whose
        // paths are ours); another such pin at the same point; a no-connect
        // flag there marks it. A pin that met nothing stays unconnected,
        // as in the source.
        let wired = |p: PointNm| {
            file.wires
                .iter()
                .any(|w| w[0] == p || w[1] == p || schematic::inside(p, w[0], w[1]))
        };
        for (p, _) in &moved {
            if at.get(p).and_then(Anchors::target).is_none()
                && wired(*p)
                && matches!(on_sheet(session, sid, *p), Some(WireTarget::OnWire(_)))
            {
                let _ = session.apply(
                    "junction.place",
                    Operation::Schematic(SchematicCommand::PlaceJunction {
                        sheet: Some(sid),
                        position: nm_point(ours(*p))?,
                    }),
                );
            }
        }
        let mut alone: BTreeMap<PointNm, Endpoint> = BTreeMap::new();
        // What a pin's own wire must not pass: every other such pin and
        // every point of KiCad's drawing (the engine joins what a wire
        // touches).
        let obstacles: Vec<(Option<Endpoint>, [f64; 2])> = moved
            .iter()
            .filter_map(|(_, e)| {
                Some((
                    Some(*e),
                    sparksmith_core::schematic::pin_position(session.engine.project(), *e)?,
                ))
            })
            .chain(
                at.keys()
                    .chain(moved.iter().map(|(p, _)| p))
                    .map(|p| (None, ours(*p))),
            )
            .collect();
        for (p, end) in moved {
            if on_pin.contains(&end) {
                continue;
            }
            if file.no_connects.contains(&p) {
                let _ = session.apply(
                    "instance.set_no_connect",
                    Operation::Instance(InstanceCommand::SetNoConnect {
                        instance: end.instance,
                        terminals: vec![end.terminal],
                        no_connect: true,
                    }),
                );
                continue;
            }
            let target = at
                .get(&p)
                .and_then(Anchors::target)
                .or_else(|| wired(p).then(|| on_sheet(session, sid, p)).flatten())
                .or_else(|| alone.get(&p).map(|first| WireTarget::Terminal(*first)));
            match target {
                // A label joins by its name: a twin of it on the pin (a
                // wire to the label would pull it onto the pin, off the
                // point KiCad's wires meet it at).
                Some(WireTarget::Label(label)) => {
                    twin_label(session, sid, label.label, end, report)
                }
                Some(target) => {
                    let corners = clear_path(session, end, ours(p), &obstacles);
                    draw(
                        session,
                        sid,
                        WireTarget::Terminal(end),
                        &corners,
                        target,
                        report,
                    )
                }
                None => {
                    // Met only by another such pin, if any comes.
                    alone.insert(p, end);
                }
            }
        }
    }
    Ok(())
}

/// A label of `label`'s name and kind on the pin of `end`, joining it to
/// the label's net by name.
fn twin_label(
    session: &mut Session<'_>,
    sid: SheetId,
    label: Id,
    end: Endpoint,
    report: &mut Report,
) {
    let project = session.engine.project();
    let (Some(source), Some(at)) = (
        project.label(label).cloned(),
        sparksmith_core::schematic::pin_position(project, end),
    ) else {
        return;
    };
    let Ok(position) = nm_point(at) else {
        return;
    };
    let placed = session.apply(
        "label.place",
        Operation::Schematic(SchematicCommand::PlaceLabel {
            sheet: Some(sid),
            name: source.name,
            kind: source.kind,
            position,
            rotation: source.rotation,
            attach: Some(WireTarget::Terminal(end)),
        }),
    );
    match placed {
        Ok(_) => report.add(Case::SchematicLabel),
        Err(_) => report.add(Case::SchematicItemRefused),
    }
}

/// The corners of a wire from the pin of `end` to `to` that passes no
/// obstacle but its own ends: straight, else one bend either way; none
/// when every way touches one.
fn clear_path(
    session: &Session<'_>,
    end: Endpoint,
    to: [f64; 2],
    obstacles: &[(Option<Endpoint>, [f64; 2])],
) -> Vec<Corner> {
    let Some(from) = sparksmith_core::schematic::pin_position(session.engine.project(), end) else {
        return Vec::new();
    };
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]) < TOUCH_MM;
    let clear = |path: &[[f64; 2]]| {
        obstacles.iter().all(|(owner, q)| {
            *owner == Some(end)
                || near(*q, from)
                || near(*q, to)
                || path.windows(2).all(|s| {
                    sparksmith_core::geometry::segment_distance(*q, s[0], s[1]) >= TOUCH_MM
                })
        })
    };
    // Straight; one bend either way; then out of line by whole grid
    // steps, across, and in.
    let mut ways: Vec<Vec<[f64; 2]>> =
        vec![Vec::new(), vec![[to[0], from[1]]], vec![[from[0], to[1]]]];
    for k in 1..=4 {
        let d = f64::from(k) * CLEAR_STEP_MM * 2.0;
        for d in [d, -d] {
            ways.push(vec![[from[0], from[1] + d], [to[0], from[1] + d]]);
            ways.push(vec![[from[0] + d, from[1]], [from[0] + d, to[1]]]);
        }
    }
    for bends in ways {
        let path: Vec<[f64; 2]> = std::iter::once(from)
            .chain(bends.iter().copied())
            .chain([to])
            .collect();
        if clear(&path) {
            return bends
                .into_iter()
                .filter_map(|b| Corner::sharp_mm(b).ok())
                .collect();
        }
    }
    Vec::new()
}

/// Largest number of grid steps a part's own symbol is moved to keep its
/// pins off KiCad's drawing.
const MAX_CLEAR_STEPS: i32 = 12;
/// The step a part's own symbol moves by, mm (KiCad's 50 mil grid).
const CLEAR_STEP_MM: f64 = 1.27;
/// A pin within this of a drawn point or wire touches it, mm.
const TOUCH_MM: f64 = 0.01;

/// Move each part's own symbol (a generated part's, drawn where KiCad
/// draws its symbol) until none of its pins touches KiCad's drawing on
/// the sheet (a wire, a pin, a label or flag point, another such symbol's
/// pin): the engine joins what touches, and KiCad's drawing never meant
/// these pins to meet anything but through their own wires.
fn clear_own_symbols(
    session: &mut Session<'_>,
    sid: SheetId,
    file: &schematic::SheetFile,
    at: &BTreeMap<PointNm, Anchors>,
    moved: &[(PointNm, Endpoint)],
    power: &[(PointNm, String)],
) -> Result<(), ImportError> {
    let mut points: Vec<[f64; 2]> = at
        .keys()
        .chain(moved.iter().map(|(p, _)| p))
        .chain(power.iter().map(|(p, _)| p))
        .chain(file.labels.iter().map(|l| &l.at))
        .chain(file.junctions.iter())
        .map(|p| ours(*p))
        .collect();
    let segments: Vec<[[f64; 2]; 2]> = file
        .wires
        .iter()
        .map(|w| [ours(w[0]), ours(w[1])])
        .collect();
    let instances: BTreeSet<Id> = moved.iter().map(|(_, e)| e.instance).collect();
    for instance in instances {
        let project = session.engine.project();
        let Some(occurrence) = project
            .occurrences_of(instance)
            .find(|o| o.sheet == sid)
            .map(|o| (o.id, o.position))
        else {
            continue;
        };
        let terminals: Vec<Endpoint> = project
            .instance(instance)
            .and_then(|i| project.library.component(i.component))
            .map(|c| {
                c.definition
                    .terminals
                    .iter()
                    .map(|t| Endpoint {
                        instance,
                        terminal: t.id,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let pins = |session: &Session<'_>| -> Vec<[f64; 2]> {
            terminals
                .iter()
                .filter_map(|e| {
                    sparksmith_core::schematic::pin_position(session.engine.project(), *e)
                })
                .collect()
        };
        let touches = |pins: &[[f64; 2]]| {
            pins.iter().any(|q| {
                points
                    .iter()
                    .any(|p| (p[0] - q[0]).hypot(p[1] - q[1]) < TOUCH_MM)
                    || segments.iter().any(|[a, b]| {
                        sparksmith_core::geometry::segment_distance(*q, *a, *b) < TOUCH_MM
                    })
            })
        };
        let start = occurrence.1.to_mm_f64();
        let mut placed = pins(session);
        if touches(&placed) {
            // A spiral of grid steps round where KiCad draws the symbol.
            'search: for ring in 1..=MAX_CLEAR_STEPS {
                for dx in -ring..=ring {
                    for dy in -ring..=ring {
                        if dx.abs() != ring && dy.abs() != ring {
                            continue;
                        }
                        let to = [
                            start[0] + f64::from(dx) * CLEAR_STEP_MM,
                            start[1] + f64::from(dy) * CLEAR_STEP_MM,
                        ];
                        let Ok(position) = nm_point(to) else { continue };
                        if session
                            .apply(
                                "symbol.move_unit",
                                Operation::Schematic(SchematicCommand::MoveUnit {
                                    occurrence: occurrence.0,
                                    position,
                                }),
                            )
                            .is_err()
                        {
                            continue;
                        }
                        placed = pins(session);
                        if !touches(&placed) {
                            break 'search;
                        }
                    }
                }
            }
        }
        // Later symbols keep clear of this one's pins.
        points.extend(placed);
    }
    Ok(())
}

/// The flattened index of the sheet the `k`th sheet symbol of sheet
/// `parent` shows.
fn k_child(plan: &Plan, parent: usize, k: usize) -> usize {
    plan.children.get(&(parent, k)).copied().unwrap_or(parent)
}

/// Whether some wire other than `stroke`'s own runs through `p`; such an
/// end waits for that wire.
fn stroke_through(wires: &[[PointNm; 2]], p: PointNm, stroke: &[PointNm]) -> bool {
    wires.iter().any(|w| {
        (w[0] == p || w[1] == p || schematic::inside(p, w[0], w[1]))
            && !(stroke.contains(&w[0]) && stroke.contains(&w[1]))
    })
}

fn free(p: PointNm) -> Option<WireTarget> {
    FreeEnd::new(ours(p)).map(WireTarget::Free)
}

/// A junction at `p` on sheet `sid`, else a point on a wire drawn
/// through it.
fn on_sheet(session: &Session<'_>, sid: SheetId, p: PointNm) -> Option<WireTarget> {
    let project = session.engine.project();
    let here = OurPoint::from_mm_f64_rounded(ours(p)).ok()?;
    if let Some(j) = project
        .junctions
        .iter()
        .find(|j| j.sheet == sid && j.position == here)
    {
        return Some(WireTarget::Junction(JunctionRef { junction: j.id }));
    }
    let at = ours(p);
    let on = |a: [f64; 2], b: [f64; 2]| {
        let d = sparksmith_core::geometry::segment_distance(at, a, b);
        d < 1e-6
    };
    // A wire that ends here first (KiCad's wires end where they meet a
    // pin), then one that passes through.
    let near = |q: [f64; 2]| (q[0] - at[0]).hypot(q[1] - at[1]) < 1e-6;
    let wires: Vec<(Id, Vec<[f64; 2]>)> = project
        .wires
        .iter()
        .filter(|w| w.sheet == sid)
        .filter_map(|w| Some((w.id, sparksmith_core::schematic::wire_path(project, w)?)))
        .collect();
    let ends = wires.iter().find(|(_, path)| {
        path.first().is_some_and(|q| near(*q)) || path.last().is_some_and(|q| near(*q))
    });
    ends.or_else(|| {
        wires
            .iter()
            .find(|(_, path)| path.windows(2).any(|s| on(s[0], s[1])))
    })
    .map(|(wire, _)| {
        WireTarget::OnWire(WirePoint {
            wire: *wire,
            at: here,
        })
    })
}

fn draw(
    session: &mut Session<'_>,
    sheet: SheetId,
    from: WireTarget,
    corners: &[Corner],
    to: WireTarget,
    report: &mut Report,
) {
    let op = |corners: Vec<Corner>| {
        Operation::Schematic(SchematicCommand::DrawWire {
            sheet: Some(sheet),
            from,
            corners,
            to,
        })
    };
    let first = session.apply("wire.draw", op(corners.to_vec()));
    if first.is_ok() {
        report.add(Case::SchematicWire);
    } else if !corners.is_empty() && session.apply("wire.draw", op(Vec::new())).is_ok() {
        report.add(Case::SchematicWireStraightened);
    } else {
        // The kind of refusal only, never its words, which name nets.
        let kind = match &first {
            Err(ImportError::Refused { reason, .. }) if reason.starts_with("Nets ") => {
                "two named nets"
            }
            _ => "other",
        };
        report.add_detail(Case::SchematicWireRefused, "", "", kind);
    }
}
