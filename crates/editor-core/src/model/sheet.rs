//! Schematic sheets and what is drawn on them besides symbols and wires:
//! net labels and power ports, sheet symbols with their sheet pins (the
//! hierarchy), and sheet furniture (notes, lines, rectangles), plus the
//! project properties a title block shows. Data only: connectivity rules
//! live in `nets`, capture rules in `schematic`.
//!
//! Sheet coordinates are stored in whole nanometres with y up, as for
//! symbols and wires; the page frame is computed in millimetres.
//! A sheet's page has its top-left corner at [`PAGE_TOP_LEFT`], so content
//! drawn near the origin (as single-sheet designs were) lies inside the
//! frame: the page spans x ∈ [−30, width − 30], y ∈ [30 − height, 30].
use super::Project;
use crate::geometry::Point;
use crate::ids::{Id, SheetId};
use crate::library::Rotation;
use crate::units::{Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A paper size a sheet is drawn and printed on.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Paper {
    #[default]
    A4,
    A3,
    A2,
    A1,
    A0,
    /// US Letter, 8.5 × 11 in.
    Letter,
    /// US Legal, 8.5 × 14 in.
    Legal,
    /// US Tabloid (ANSI B), 11 × 17 in.
    Tabloid,
}

impl Paper {
    pub const ALL: [Self; 8] = [
        Self::A4,
        Self::A3,
        Self::A2,
        Self::A1,
        Self::A0,
        Self::Letter,
        Self::Legal,
        Self::Tabloid,
    ];

    /// Portrait width and height in millimetres (ISO 216; ANSI sizes
    /// converted at 25.4 mm per inch).
    pub fn portrait_mm(self) -> Point {
        match self {
            Self::A4 => [210.0, 297.0],
            Self::A3 => [297.0, 420.0],
            Self::A2 => [420.0, 594.0],
            Self::A1 => [594.0, 841.0],
            Self::A0 => [841.0, 1189.0],
            Self::Letter => [215.9, 279.4],
            Self::Legal => [215.9, 355.6],
            Self::Tabloid => [279.4, 431.8],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::A3 => "A3",
            Self::A2 => "A2",
            Self::A1 => "A1",
            Self::A0 => "A0",
            Self::Letter => "Letter",
            Self::Legal => "Legal",
            Self::Tabloid => "Tabloid",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PaperOrientation {
    #[default]
    Landscape,
    Portrait,
}

/// Where every page's top-left corner lies on its sheet, mm.
pub const PAGE_TOP_LEFT: Point = [-30.0, 30.0];

/// One schematic sheet: a page with its own name, paper and frame.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sheet {
    pub id: SheetId,
    /// Unique among the project's sheets, compared case-insensitively.
    pub name: String,
    #[serde(default)]
    pub paper: Paper,
    #[serde(default)]
    pub orientation: PaperOrientation,
}

impl Sheet {
    /// The sheet a new project starts with.
    pub fn first() -> Self {
        Self {
            id: SheetId::FIRST,
            name: "Sheet 1".into(),
            paper: Paper::A4,
            orientation: PaperOrientation::Landscape,
        }
    }

    /// The page's top-left corner on the sheet ([`PAGE_TOP_LEFT`]).
    pub fn top_left(&self) -> Point {
        PAGE_TOP_LEFT
    }

    /// Page width and height in millimetres.
    pub fn size_mm(&self) -> Point {
        let [w, h] = self.paper.portrait_mm();
        match self.orientation {
            PaperOrientation::Portrait => [w, h],
            PaperOrientation::Landscape => [h, w],
        }
    }
}

/// The root page and the sheet-symbol IDs followed from it.
#[derive(
    Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(deny_unknown_fields)]
pub struct SheetPath {
    pub root: SheetId,
    pub symbols: Vec<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SheetUse {
    pub path: SheetPath,
    pub definition: SheetId,
    /// Drawing-local component to physical board component.
    pub parts: std::collections::BTreeMap<Id, Id>,
}

/// Only the physical choices of a repeated part are stored per occurrence.
/// The binding names its drawing-local component, which supplies every
/// shared field when the file is read.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PhysicalPart {
    pub id: Id,
    pub designator: String,
    #[serde(
        default,
        skip_serializing_if = "crate::library::marking::InstanceMarking::is_default"
    )]
    pub marking: crate::library::marking::InstanceMarking,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub text_placements: std::collections::BTreeMap<crate::ids::GraphicId, super::TextPlacement>,
    #[serde(
        default,
        skip_serializing_if = "crate::model::Population::is_populated"
    )]
    pub population: crate::model::Population,
}

/// The project's sheets in page order and the next sheet identity.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Sheets {
    /// Page order: annotation, printing and "sheet n of m" follow it.
    pub list: Vec<Sheet>,
    pub next: u32,
    /// A child definition stays a definition after its last symbol goes.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub definitions: std::collections::BTreeSet<SheetId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uses: crate::shared::Shared<Vec<SheetUse>>,
    /// Drawing-local parts whose original physical occurrence has gone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locals: crate::shared::Shared<Vec<super::ComponentInstance>>,
}

impl Default for Sheets {
    fn default() -> Self {
        Self {
            list: vec![Sheet::first()],
            next: 2,
            definitions: Default::default(),
            uses: Default::default(),
            locals: Default::default(),
        }
    }
}

impl Sheets {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    pub fn get(&self, id: SheetId) -> Option<&Sheet> {
        self.list.iter().find(|s| s.id == id)
    }
    /// The sheet's page number, from 1.
    pub fn number(&self, id: SheetId) -> Option<usize> {
        self.list.iter().position(|s| s.id == id).map(|i| i + 1)
    }
    /// The first sheet in page order.
    pub fn first(&self) -> SheetId {
        self.list.first().map_or(SheetId::FIRST, |s| s.id)
    }
}

/// Project properties a title block shows. Empty fields print empty; an
/// empty title prints the project name.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProjectProperties {
    pub title: String,
    pub revision: String,
    /// Free text, as the design's owner writes dates.
    pub date: String,
    pub author: String,
    pub company: String,
}

impl ProjectProperties {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// The signal direction a global label, hierarchical label or sheet pin
/// shows. Drawing only: electrical rules use terminal functions.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PortShape {
    Input,
    Output,
    Bidirectional,
    #[default]
    Passive,
}

impl PortShape {
    pub const ALL: [Self; 4] = [
        Self::Input,
        Self::Output,
        Self::Bidirectional,
        Self::Passive,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "Input",
            Self::Output => "Output",
            Self::Bidirectional => "Bidirectional",
            Self::Passive => "Passive",
        }
    }
}

/// How a power port is drawn: the ground symbol's bars, or a supply bar or
/// arrow (the shapes `library::symbol_style` draws for its ground and power
/// kinds).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PowerStyle {
    Ground,
    #[default]
    Bar,
    Arrow,
}

impl PowerStyle {
    pub const ALL: [Self; 3] = [Self::Ground, Self::Bar, Self::Arrow];
    pub fn label(self) -> &'static str {
        match self {
            Self::Ground => "Ground",
            Self::Bar => "Bar",
            Self::Arrow => "Arrow",
        }
    }
}

/// What a label's name joins (docs/schematic-capture.md, "Labels"):
/// - local: labels of the same name on the same sheet;
/// - global (an off-sheet connector): global labels and power ports of the
///   same name on every sheet;
/// - hierarchical: labels of the same name on the same sheet, and the
///   sheet pin of that name on the sheet symbol that uses the sheet;
/// - power: like global, drawn as a power or ground symbol.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LabelKind {
    #[default]
    Local,
    Global {
        #[serde(default)]
        shape: PortShape,
    },
    Hierarchical {
        #[serde(default)]
        shape: PortShape,
    },
    Power {
        #[serde(default)]
        style: PowerStyle,
    },
}

impl LabelKind {
    /// Global scope: global labels and power ports.
    pub fn is_global(self) -> bool {
        matches!(self, Self::Global { .. } | Self::Power { .. })
    }
    pub fn noun(self) -> &'static str {
        match self {
            Self::Local => "label",
            Self::Global { .. } => "global label",
            Self::Hierarchical { .. } => "hierarchical label",
            Self::Power { .. } => "power port",
        }
    }
}

/// A net label or power port: a named connection point on a sheet. Wires
/// end on it like on a junction; its name joins it to other labels by its
/// kind's scope. It never joins anything by drawn position.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Label {
    pub id: Id,
    #[serde(default, skip_serializing_if = "SheetId::is_first")]
    pub sheet: SheetId,
    /// A valid net name (`nets::name_valid`).
    pub name: String,
    pub kind: LabelKind,
    /// The connection point.
    pub position: PointNm,
    /// Quarter turns counter-clockwise of the drawing about its point.
    #[serde(default)]
    pub rotation: Rotation,
    /// A person placed, moved or turned it: it is drawn at `position` and
    /// `rotation`. Otherwise a label whose only wire goes to a pin sits on
    /// that pin's end and faces away from its symbol, placed when drawn
    /// (`schematic::labels::attached_pin`, `schematic::scene::sheet_labels`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

/// Which edge of a sheet symbol a sheet pin sits on.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SheetSide {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
}

/// A connection point on a sheet symbol's edge. It joins the hierarchical
/// labels of the same name on the child sheet.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SheetPin {
    pub id: Id,
    /// A valid net name, unique on its symbol (case-insensitively).
    pub name: String,
    #[serde(default)]
    pub shape: PortShape,
    pub side: SheetSide,
    /// Distance along the side from the symbol's top-left corner (left and
    /// right sides: downward; top and bottom: rightward).
    pub offset_nm: Nm,
}

impl SheetPin {
    pub fn offset_mm(&self) -> f64 {
        self.offset_nm.to_mm_f64()
    }
}

/// A box on a parent sheet that stands for a child sheet: the hierarchy.
/// Each traversal through this symbol is a use of its child definition.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SheetSymbol {
    pub id: Id,
    /// The sheet the box is drawn on.
    #[serde(default, skip_serializing_if = "SheetId::is_first")]
    pub sheet: SheetId,
    pub child: SheetId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The top-left corner.
    pub position: PointNm,
    /// Width (x) and height (y).
    pub size: PointNm,
    pub pins: Vec<SheetPin>,
}

impl SheetSymbol {
    pub fn pin(&self, id: Id) -> Option<&SheetPin> {
        self.pins.iter().find(|p| p.id == id)
    }
    pub fn position_mm(&self) -> Point {
        self.position.to_mm_f64()
    }
    pub fn size_mm(&self) -> Point {
        self.size.to_mm_f64()
    }
    /// Where a pin's connection point sits on the sheet, mm.
    pub fn pin_position(&self, pin: &SheetPin) -> Point {
        // Each term is within ±2e9 nm: the i64 sums cannot overflow, and
        // the conversion of the exact sum is exact to the nearest f64.
        let mm = |nm: i64| nm as f64 / crate::units::NM_PER_MM as f64;
        let (x, y) = (self.position.x_nm.get(), self.position.y_nm.get());
        let (w, h) = (self.size.x_nm.get(), self.size.y_nm.get());
        let offset = pin.offset_nm.get();
        let [px, py] = match pin.side {
            SheetSide::Left => [x, y - offset],
            SheetSide::Right => [x + w, y - offset],
            SheetSide::Top => [x + offset, y],
            SheetSide::Bottom => [x + offset, y - h],
        };
        [mm(px), mm(py)]
    }
}

/// Sheet furniture: drawn, printed, never electrical.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "item", rename_all = "snake_case", deny_unknown_fields)]
pub enum SheetItem {
    /// A text note; lines split at `\n`.
    Text {
        at: PointNm,
        text: String,
        /// Cap height.
        height_nm: Nm,
        #[serde(default)]
        rotation: Rotation,
    },
    Line {
        from: PointNm,
        to: PointNm,
    },
    Rect {
        min: PointNm,
        max: PointNm,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SheetDrawing {
    pub id: Id,
    #[serde(default, skip_serializing_if = "SheetId::is_first")]
    pub sheet: SheetId,
    pub item: SheetItem,
}

impl SheetPath {
    pub fn root(root: SheetId) -> Self {
        Self {
            root,
            symbols: vec![],
        }
    }
    pub fn child(&self, symbol: Id) -> Self {
        let mut next = self.clone();
        next.symbols.push(symbol);
        next
    }

    pub fn display(&self, project: &Project) -> String {
        if self.symbols.is_empty() {
            return project
                .sheet(self.root)
                .map_or_else(String::new, |s| s.name.clone());
        }
        self.symbols
            .iter()
            .filter_map(|id| {
                let symbol = project.sheet_symbol(*id)?;
                symbol
                    .name
                    .as_deref()
                    .or_else(|| project.sheet(symbol.child).map(|s| s.name.as_str()))
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl Project {
    pub fn is_root_sheet(&self, sheet: SheetId) -> bool {
        !self.sheets.definitions.contains(&sheet) && self.parent_symbol(sheet).is_none()
    }
}

impl Project {
    /// Visible pages, including an unused definition once so the drawing stays accessible.
    /// Roots follow page order and each child follows its parent.
    pub fn sheet_pages(&self) -> Vec<(&Sheet, Option<&SheetPath>)> {
        let numbers: std::collections::BTreeMap<_, _> = self
            .sheets
            .list
            .iter()
            .enumerate()
            .map(|(i, s)| (s.id, i + 1))
            .collect();
        let mut pages = Vec::new();
        for (sheet, path) in self.printed_pages() {
            let root = path.map_or(sheet.id, |p| p.root);
            let mut key = vec![(numbers.get(&root).copied().unwrap_or_default(), 0)];
            if let Some(path) = path {
                for id in &path.symbols {
                    let page = self
                        .sheet_symbol(*id)
                        .and_then(|s| numbers.get(&s.child))
                        .copied()
                        .unwrap_or_default();
                    key.push((page, *id));
                }
            }
            pages.push((key, sheet, path));
        }
        pages.sort_by(|a, b| a.0.cmp(&b.0));
        pages.into_iter().map(|(_, s, p)| (s, p)).collect()
    }

    /// Printable page order follows definition order, then the stable paths within each definition.
    pub fn printed_pages(&self) -> Vec<(&Sheet, Option<&SheetPath>)> {
        let mut uses = std::collections::BTreeMap::<_, Vec<_>>::new();
        for u in &self.sheets.uses {
            uses.entry(u.definition).or_default().push(&u.path);
        }
        let mut pages = Vec::new();
        for sheet in &self.sheets.list {
            if let Some(paths) = uses.get(&sheet.id) {
                pages.extend(paths.iter().map(|p| (sheet, Some(*p))));
            } else {
                pages.push((sheet, None));
            }
        }
        pages
    }

    /// Printed page reached by a box in a particular parent occurrence.
    pub fn child_page(&self, symbol: &SheetSymbol, parent: Option<&SheetPath>) -> Option<usize> {
        let path = parent
            .cloned()
            .unwrap_or_else(|| SheetPath::root(symbol.sheet))
            .child(symbol.id);
        self.printed_pages()
            .iter()
            .position(|(s, p)| s.id == symbol.child && *p == Some(&path))
            .map(|i| i + 1)
    }

    /// Locate a physical part in the shared drawing and the channel that uses it.
    pub fn sheet_binding(&self, physical: Id) -> Option<(&SheetUse, Id)> {
        self.sheets.uses.iter().find_map(|u| {
            u.parts
                .iter()
                .find(|(_, id)| **id == physical)
                .map(|(local, _)| (u, *local))
        })
    }

    pub fn sheet_use(&self, path: &SheetPath) -> Option<&SheetUse> {
        self.sheets
            .uses
            .binary_search_by(|u| u.path.cmp(path))
            .ok()
            .and_then(|i| self.sheets.uses.get(i))
    }

    /// Physical instances affected when a selected component's shared drawing changes.
    pub fn sheet_part_families(
        &self,
        selected: &[Id],
    ) -> std::collections::BTreeMap<Id, std::collections::BTreeSet<Id>> {
        let mut families = std::collections::BTreeMap::<_, std::collections::BTreeSet<_>>::new();
        for id in selected {
            let local = self.sheet_binding(*id).map_or(*id, |(_, local)| local);
            families.entry(local).or_default().insert(*id);
        }
        for u in &self.sheets.uses {
            for (local, physical) in &u.parts {
                if let Some(family) = families.get_mut(local) {
                    family.insert(*physical);
                }
            }
        }
        families
    }

    /// The units drawn for a physical component, through its binding.
    pub fn physical_units(&self, physical: Id) -> impl Iterator<Item = &super::SymbolOccurrence> {
        let local = self
            .sheet_binding(physical)
            .map_or(physical, |(_, local)| local);
        self.occurrences_of(local)
    }

    pub fn drawing_instance(&self, local: Id) -> Option<&super::ComponentInstance> {
        self.instance(local)
            .or_else(|| self.sheets.locals.iter().find(|i| i.id == local))
    }
}
