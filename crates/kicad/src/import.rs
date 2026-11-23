//! A KiCad board's footprints built into a new project through the one
//! command engine: every footprint that differs becomes a real part of the
//! project, the same kind of part as any other. With a host
//! [`crate::models::Provider`], a footprint whose lands identify a
//! generator's package is that generator's part, made by `library.generate`
//! with the footprint's lands as its land pattern; any other becomes a
//! project part of its lands (component, symbol, package, footprint and
//! binding), published. Every footprint is placed on a schematic sheet and
//! on the board where KiCad has it, its copper exactly the source's. Nothing is written to a document
//! directly: each step is a typed [`Operation`] applied by [`Engine`], and
//! any refused step fails the whole import (the engine is dropped).
use crate::board::{self, Board, Footprint};
use crate::population::Why;
use crate::report::{Case, Report};
use crate::translate::{self, DefPad, Mapped};
use sparksmith_core::functions::{Direction, Drive, TerminalFunctions};
use sparksmith_core::geometry::Bounds;
use sparksmith_core::instances::variants::VariantCommand;
use sparksmith_core::layout::LayoutCommand;
use sparksmith_core::layout::fill::FillCommand;
use sparksmith_core::layout::outline::{BoardCommand, BoardEdit};
use sparksmith_core::library::footprint_source::{SketchEdit, SketchRef};
use sparksmith_core::library::symbol_blocks::named_block_symbol;
use sparksmith_core::library::{
    BindingDefinition, ComponentDefinition, Definition, DefinitionMeta, DraftEdit,
    FootprintDefinition, ModelPlacement, PackageDefinition, PackageModel, PackageTerminal, Pad,
    PadDefaults, PadKind, Terminal, TerminalClass, TerminalMap, designator_prefix_valid,
};
use sparksmith_core::model::Population;
use sparksmith_core::model::board::BoardShape;
use sparksmith_core::model::fill::PadFillOverride;
use sparksmith_core::rules::{
    NetClassId, NetClassPattern, NetClassSpec, RuleKey, RuleProfile, RuleValue, RulesCommand,
};
use sparksmith_core::units::{Nm, PointNm};
use sparksmith_core::{
    BindingRef, ComponentRef, DefinitionId, DefinitionKind, FootprintRef, FunctionId, Id,
    InstanceCommand, LibraryCommand, Operation, Outcome, PackageRef, PackageTerminalId, PadId,
    PhysicalChoice, Revision, StackupPresets, SymbolRef, TerminalId, UnitId,
};
use sparksmith_core::{Engine, Evaluators, NewBoard, instances::NewInstance};
use std::collections::BTreeMap;

/// Why an import failed. Messages carry no file content.
#[derive(Debug)]
pub enum ImportError {
    Board(board::BoardError),
    /// The engine refused a step: its command and the core's reason.
    Refused {
        step: &'static str,
        reason: String,
    },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Board(e) => e.fmt(f),
            Self::Refused { step, reason } => write!(f, "kicad.refused: {step}: {reason}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<board::BoardError> for ImportError {
    fn from(e: board::BoardError) -> Self {
        Self::Board(e)
    }
}

/// One imported pad, for difference reports: its class and board box.
#[derive(Clone, Debug, PartialEq)]
pub struct PadRecord {
    pub class: String,
    pub world: Bounds,
    /// Round custom-pad primitives on the board (centre, radius, mm).
    pub arcs: Vec<([f64; 2], f64)>,
}

/// The new project, its report and the import map of pads.
pub struct Imported {
    pub engine: Engine,
    /// Where our board origin lies in KiCad's frame, and KiCad's auxiliary
    /// (drill and place) origin, nm: our output is KiCad's plot about the
    /// auxiliary origin moved by `(aux.x − origin.x, origin.y − aux.y)`.
    pub origin_nm: [i64; 2],
    pub aux_origin_nm: Option<[i64; 2]>,
    pub report: Report,
    pub pads: Vec<PadRecord>,
}

impl Imported {
    /// What moves a KiCad plot about the auxiliary origin onto our output,
    /// nm (`kicad-cli … --use-drill-file-origin`).
    pub fn plot_shift_nm(&self) -> [i64; 2] {
        let aux = self.aux_origin_nm.unwrap_or_default();
        [aux[0] - self.origin_nm[0], self.origin_nm[1] - aux[1]]
    }
}

/// The provenance every imported definition records (L1).
const PROVENANCE: &str = "Imported from a KiCad board's embedded footprint copy. Project-only: \
     it may come from KiCad's CC-BY-SA libraries, so never publish it to a shared library.";

/// The provenance of a part made from a schematic symbol alone (one the
/// board has no footprint of).
const SYMBOL_PROVENANCE: &str = "Imported from a KiCad schematic's embedded symbol copy, with no \
     footprint (the board has none). Project-only: it may come from KiCad's CC-BY-SA libraries, \
     so never publish it to a shared library.";

/// Schematic spacing of the imported instances, mm.
const SHEET_PITCH_MM: f64 = 25.4;

/// What an import builds on: shipped data and host engines the caller
/// loads (the importer reads no files).
#[derive(Clone, Copy)]
pub struct Setup<'a> {
    /// The new project's board.
    pub board: &'a NewBoard,
    /// The stackup family a stackup the file does not state comes from.
    pub presets: &'a StackupPresets,
    /// The fab profile the board's rules start from; the file's own
    /// settings (mask margin, via tenting) are set over it. Without one the
    /// board has no rules, so its outline, tracks and fills are not
    /// imported.
    pub profile: Option<&'a RuleProfile>,
    /// The host's sketch engines: with a board evaluator the outline is
    /// imported as a board sketch, else as the rectangle around it.
    pub evaluators: Evaluators<'a>,
    /// The board's project file (`.kicad_pro`), when there is one: its
    /// rules become the project's own rule set.
    pub project_file: Option<&'a [u8]>,
    /// The project's schematic files, when there are any: drawn as the
    /// schematic; without them each pin gets a label of its board net.
    pub schematic: Option<crate::schematic::SchematicFiles<'a>>,
}

/// Import a KiCad board file into a new project.
pub fn import(bytes: &[u8], setup: &Setup<'_>) -> Result<Imported, ImportError> {
    import_with_models(bytes, setup, None)
}

/// Import with a host that resolves and evaluates 3D models off the UI thread.
pub fn import_with_models(
    bytes: &[u8],
    setup: &Setup<'_>,
    mut models: Option<&mut dyn crate::models::Provider>,
) -> Result<Imported, ImportError> {
    let mut report = Report::default();
    // Check limits before building any part of the imported project, even
    // when no fabrication profile is available to apply the rules to.
    let project_rules = match setup.project_file.map(crate::project::read) {
        Some(Ok(rules)) => Some(rules),
        Some(Err(crate::project::ReadError::Invalid)) => {
            report.add(Case::ProjectUnreadable);
            None
        }
        Some(Err(error)) => {
            return Err(ImportError::Refused {
                step: "project rules",
                reason: error.to_string(),
            });
        }
        None => {
            report.add(Case::NoProject);
            None
        }
    };
    let board = board::read(bytes, &mut report)?;
    report.format_version = board.version;
    report.counts.copper_layers = board.copper.len();
    report.counts.footprints = board.footprints.len();
    report.counts.footprints_back = board.footprints.iter().filter(|f| f.back).count();
    report.add_n(Case::BoardObjects, board.other.values().sum());
    let mut engine = Engine::new(setup.board);
    // The host's context, current on the importing thread, is the
    // imported project's.
    if let Ok(context) = sparksmith_core::library::standards::Context::current() {
        engine.set_context(context);
    }
    let mut session = Session {
        engine,
        evaluators: setup.evaluators,
    };
    session.rules(&board, setup.profile, &mut report)?;
    let stackup = match crate::shape::stackup(&board, setup.presets, &mut report) {
        Some(stackup) => Some(stackup),
        None if board.copper.len() != setup.board.stackup.copper.len() => Some(
            setup
                .presets
                .preset(board.copper.len())
                .map_err(|e| ImportError::Refused {
                    step: "layout.set_stackup",
                    reason: e.to_string(),
                })?,
        ),
        None => None,
    };
    if let Some(stackup) = stackup {
        session.apply(
            "layout.set_stackup",
            Operation::Layout(LayoutCommand::SetStackup(stackup)),
        )?;
    }
    session.outline(&board, &mut report)?;
    // The schematic, read first: each reference's KiCad symbol shapes its
    // part's symbol.
    let schematic = match setup.schematic.map(crate::schematic::read) {
        Some(Ok(schematic)) => Some(schematic),
        Some(Err(_)) => {
            report.add(Case::SchematicUnreadable);
            None
        }
        None => None,
    };
    let by_reference = schematic
        .as_ref()
        .map(crate::sheets::symbols_by_reference)
        .unwrap_or_default();
    // Each part's KiCad value (the schematic's, else the footprint's) and
    // part number: the part's typed fields and manufacturer part.
    let schematic_values = schematic
        .as_ref()
        .map(crate::sheets::values_by_reference)
        .unwrap_or_default();
    let schematic_parts = schematic
        .as_ref()
        .map(crate::sheets::parts_by_reference)
        .unwrap_or_default();
    let part_numbers: Vec<crate::models::PartNumber> = board
        .footprints
        .iter()
        .map(|fp| {
            let mut part = schematic_parts
                .get(&fp.reference.to_ascii_uppercase())
                .cloned()
                .unwrap_or_default();
            if part.mpn.is_none() {
                part.mpn = fp.part.mpn.clone();
            }
            if part.manufacturer.is_none() {
                part.manufacturer = fp.part.manufacturer.clone();
            }
            part
        })
        .collect();
    // Parts not fitted: KiCad's do-not-populate attribute on the footprint
    // or symbol, or a not-fitted marker in the value, which leaves the
    // value. Exclude-from-BOM is not one: it marks test points, mechanical
    // items and logos.
    let schematic_not_fitted = schematic
        .as_ref()
        .map(crate::sheets::not_fitted_by_reference)
        .unwrap_or_default();
    let mut not_fitted: Vec<Option<Why>> = Vec::with_capacity(board.footprints.len());
    let mut values: Vec<Option<String>> = Vec::with_capacity(board.footprints.len());
    for fp in &board.footprints {
        let key = fp.reference.to_ascii_uppercase();
        let value = value_text(schematic_values.get(&key).unwrap_or(&fp.value));
        let (value, marker) = match value.as_deref().map(crate::population::split_marker) {
            Some(Some(rest)) => ((!rest.is_empty()).then_some(rest), Some(Why::MarkerValue)),
            _ => (value, None),
        };
        let attribute = [
            fp.dnp.then_some(Why::DoNotPopulate),
            schematic_not_fitted.get(&key).copied(),
        ]
        .into_iter()
        .flatten()
        .min();
        not_fitted.push(attribute.or(marker));
        values.push(value);
    }
    let has_variants = board.variants
        || schematic.as_ref().is_some_and(crate::sheets::has_variants)
        || setup.project_file.is_some_and(crate::project::has_variants);
    if has_variants {
        report.add(Case::VariantsNotImported);
    }
    // Each footprint's pads in its own definition frame, with its KiCad
    // symbol; copies of one KiCad footprint (by its `library:name`) with
    // equal pads and symbol share a definition. Distinct KiCad footprints
    // never do, however alike their pads: identity is never inferred from
    // geometry.
    // A board without nets says nothing by a footprint's pads having none.
    let board_nets = board
        .footprints
        .iter()
        .any(|fp| fp.pads.iter().any(|p| p.net.is_some()));
    let mut definitions: Vec<SourceDefinition> = Vec::new();
    let mut by_key: BTreeMap<String, usize> = BTreeMap::new();
    let mut uses: Vec<usize> = Vec::with_capacity(board.footprints.len());
    let mut per_footprint: Vec<Vec<PadRecord>> = Vec::with_capacity(board.footprints.len());
    let mut pad_nets: Vec<Vec<Option<u32>>> = Vec::with_capacity(board.footprints.len());
    let mut zone_overrides: Vec<Vec<(PadId, PadFillOverride)>> =
        Vec::with_capacity(board.footprints.len());
    for (n, fp) in board.footprints.iter().enumerate() {
        let mut records = Vec::new();
        let mut nets = Vec::new();
        let mut overrides = Vec::new();
        let mut pads = footprint_pads(
            &board,
            fp,
            &mut report,
            &mut records,
            &mut nets,
            &mut overrides,
        );
        // Mask and paste polygons that are a pad's opening become its rule.
        let openings = translate::take_openings(&board, fp, &mut pads);
        let tie = if fp.net_tie {
            match translate::copper_ties(fp, &pads) {
                Ok(tie) => {
                    let detail = if tie.groups.len() == 1 {
                        "Choose the purpose of this copper tie.".to_owned()
                    } else {
                        format!(
                            "{} independent copper ties. Choose a purpose for each group.",
                            tie.groups.len()
                        )
                    };
                    report.add_detail(Case::ImportedTie, &fp.reference, "", &detail);
                    Some(tie)
                }
                Err(reason) => {
                    report.add_detail(Case::NetTie, &fp.reference, "", reason);
                    None
                }
            }
        } else {
            None
        };
        let drawings = if tie.is_some() {
            fp.tie_copper.len()
        } else {
            0
        };
        per_footprint.push(records);
        pad_nets.push(nets);
        zone_overrides.push(overrides);
        let symbol = by_reference
            .get(&fp.reference.to_ascii_uppercase())
            .cloned();
        let mut key = format!("{:?}\u{0}{tie:?}\u{0}", fp.lib_id);
        key.push_str(
            &serde_json::to_string(&pads).map_err(|e| ImportError::Refused {
                step: "footprint key",
                reason: e.to_string(),
            })?,
        );
        if let Some(symbol) = &symbol {
            key.push_str(&format!("{symbol:?}"));
        }
        key.push_str(&format!("{:?}\0{}", fp.models, prefix_of(&fp.reference)));
        // A part number, and the value of a part without one, tell parts
        // apart (a generic part's value is each placed part's own field).
        let value = values.get(n).cloned().flatten();
        let part = part_numbers.get(n).cloned().unwrap_or_default();
        key.push_str(&format!("\0{:?}\0{:?}", part.mpn, value));
        // Whether it needs a body is the copy's own (an excluded copy
        // without a model has none, an ordinary one does), so copies that
        // differ there never share a definition, in any source order.
        key.push_str(&format!("\0{:?}", crate::models::no_body(fp, board_nets)));
        let next = definitions.len();
        let index = *by_key.entry(key).or_insert(next);
        if index == next {
            definitions.push(SourceDefinition {
                tie: tie.clone(),
                pads,
                symbol,
                value,
                mpn: part.mpn,
            });
        }
        uses.push(index);
        report.add_n(
            Case::FootprintFabGraphics,
            fp.fab_graphics.saturating_sub(openings + drawings),
        );
        report.add_n(Case::FootprintOpenings, openings);
        report.add_n(Case::FootprintDrawings, fp.drawings);
        if models.is_none() {
            report.add_n(Case::Models, fp.models.len());
        }
        if fp.reference.to_ascii_uppercase().starts_with("JP")
            && fp.pads.len() >= 2
            && fp.models.is_empty()
        {
            report.add_detail(
                Case::JumperSuggestion,
                &fp.reference,
                "",
                "Review whether to convert this part to a solder jumper.",
            );
        }
        report.add_n(
            Case::PadZoneOverrides,
            fp.pads.iter().filter(|p| !p.zone.is_empty()).count(),
        );
        if !fp.zone.is_empty() {
            report.add(Case::FootprintZoneOverrides);
        }
    }
    report.counts.definitions = definitions.len();
    report.add_n(Case::ProjectOnly, definitions.len());
    let mut sets: Vec<Set> = Vec::with_capacity(definitions.len());
    let mut names_taken: BTreeMap<String, usize> = BTreeMap::new();
    for (k, definition) in definitions.into_iter().enumerate() {
        let SourceDefinition {
            tie,
            pads,
            symbol,
            value,
            mpn,
        } = definition;
        // A footprint without electrical pads (a fiducial, a mounting
        // hole) becomes a part without terminals.
        let first = board
            .footprints
            .iter()
            .zip(&uses)
            .find(|(_, u)| **u == k)
            .map(|(fp, _)| fp);
        let prefix = first
            .map(|fp| prefix_of(&fp.reference))
            .unwrap_or_else(|| "U".to_owned());
        let source_id = first.map_or("", |fp| fp.lib_id.as_str());
        let references: Vec<&str> = board
            .footprints
            .iter()
            .zip(&uses)
            .filter(|(_, u)| **u == k)
            .map(|(fp, _)| fp.reference.as_str())
            .collect();
        let nets: Vec<Vec<Option<u32>>> = pad_nets
            .iter()
            .zip(&uses)
            .filter(|(_, u)| **u == k)
            .map(|(n, _)| n.clone())
            .collect();
        if let (Some(lib), Some(fp)) = (&symbol, first) {
            for (number, kind) in kept_pin_types(lib) {
                report.add_detail(Case::PinTypeKept, &fp.reference, &number, &kind);
            }
        }
        let no_body = if tie.is_some() {
            Some("fabricated copper tie")
        } else {
            first.and_then(|fp| crate::models::no_body(fp, board_nets))
        };
        let outcome = match (models.as_deref_mut(), first) {
            (Some(provider), Some(fp)) if no_body.is_none() => Some(
                provider
                    .part(
                        &mut session.engine,
                        &crate::models::PartRequest {
                            pads: &pads,
                            prefix: &prefix,
                            models: &fp.models,
                            facts: &crate::models::Facts::of(fp),
                            nets: &nets,
                            mpn: mpn.as_deref(),
                        },
                    )
                    .map_err(|reason| ImportError::Refused {
                        step: "library.generate",
                        reason,
                    })?,
            ),
            _ => None,
        };
        let (case, reason, placeholder) = match (&outcome, no_body) {
            (_, Some(why)) => (
                Some(Case::NoBody),
                format!("no body (as in the source): {why}"),
                None,
            ),
            (Some(o), None) => match &o.made {
                crate::models::Made::Generated(g) => {
                    (Some(Case::GeneratedPart), g.reason.clone(), None)
                }
                crate::models::Made::Source {
                    case,
                    reason,
                    placeholder,
                } => (Some(*case), reason.clone(), *placeholder),
            },
            (None, None) => (None, String::new(), None),
        };
        if let Some(case) = case {
            for reference in &references {
                report.add_model(case, reference, &reason, placeholder);
            }
        }
        if let Some(contradiction) = outcome.as_ref().and_then(|o| o.contradiction.as_ref()) {
            for reference in &references {
                report.add_detail(Case::PackageContradiction, reference, "", contradiction);
            }
        }
        if let Some(crate::models::Outcome {
            made: crate::models::Made::Generated(generated),
            ..
        }) = outcome
        {
            sets.push(Set {
                component: generated.component,
                symbol: generated.symbol.id,
                footprint: generated.footprint.id,
                binding: generated.binding.id,
                frame: Some((generated.turn_deg, generated.offset)),
                units: generated.units,
            });
            continue;
        }
        // A project part of the source lands is named as an authored part
        // is: by its part number, else its footprint and value (a name an
        // earlier part took is numbered apart).
        let label = match (&mpn, &value) {
            (Some(mpn), _) => mpn.replace(':', " "),
            (None, Some(value)) => format!("{source_id} {value}"),
            (None, None) => source_id.to_owned(),
        };
        let naming = definition_name(&label, k, &mut names_taken);
        if naming.changed && mpn.is_none() {
            report.add(Case::NameShortened);
        }
        if naming.numbered {
            report.add(Case::NameNumbered);
        }
        let name = naming.name;
        let no_body_model = no_body.map(|why| format!("No body, as in the source: {why}."));
        sets.push(session.component_set(
            ComponentSource {
                name: &name,
                source_id,
                pads,
                prefix: &prefix,
                tie,
                kicad_symbol: symbol.as_ref(),
                no_body: no_body_model.as_deref(),
            },
            &mut report,
        )?);
    }
    // The schematic's sheets, and where each reference's unit goes.
    let plan = match schematic {
        Some(schematic) => {
            let plan = crate::sheets::sheets(&mut session, &schematic, &mut report)?;
            Some((schematic, plan))
        }
        None => None,
    };
    let mut labelled = std::collections::BTreeSet::new();
    let mut unlabelled_places = Vec::new();
    let mut first_units: Vec<Option<crate::sheets::UnitPlace>> = Vec::new();
    let mut later_units: Vec<Vec<crate::sheets::UnitPlace>> = Vec::new();
    // Instances on the sheet, then on the board, in batches.
    let mut taken = std::collections::BTreeSet::new();
    let mut place = Vec::with_capacity(board.footprints.len());
    let mut placed: Vec<&Footprint> = Vec::with_capacity(board.footprints.len());
    let mut placed_nets = Vec::with_capacity(board.footprints.len());
    let mut placed_overrides = Vec::with_capacity(board.footprints.len());
    let mut records = Vec::new();
    let mut frames = Vec::with_capacity(board.footprints.len());
    let mut placed_sets = Vec::with_capacity(board.footprints.len());
    for (((fp, (index, recs)), nets), overrides) in board
        .footprints
        .iter()
        .zip(uses.iter().zip(per_footprint))
        .zip(pad_nets)
        .zip(zone_overrides)
    {
        let set = sets.get(*index).ok_or(ImportError::Refused {
            step: "component.place",
            reason: "a footprint without its component".into(),
        })?;
        let n = placed.len();
        placed.push(fp);
        frames.push(set.frame);
        placed_sets.push(*index);
        placed_nets.push(nets);
        placed_overrides.push(overrides);
        records.extend(recs);
        let designator = if sparksmith_core::instances::designator_valid(&fp.reference)
            && taken.insert(fp.reference.to_ascii_uppercase())
        {
            Some(fp.reference.clone())
        } else {
            report.add_detail(Case::Designator, &fp.reference, "", "");
            None
        };
        if fp.board_only {
            // KiCad's "not in schematic": on the board only.
            report.add(Case::BoardOnly);
            place.push(Operation::Instance(InstanceCommand::PlaceBoardOnly {
                component: set.component,
                representation: set.symbol,
                physical: PhysicalChoice {
                    binding: set.binding,
                    footprint: set.footprint,
                },
                designator,
                at: None,
            }));
            continue;
        }
        let column = (n % 16) as f64;
        let row = (n / 16) as f64;
        let units = plan
            .as_ref()
            .and_then(|(_, p)| p.places.get(&fp.reference.to_ascii_uppercase()))
            .filter(|u| !u.is_empty());
        if plan.is_some() && units.is_none() {
            report.add(Case::FootprintWithoutSymbol);
            unlabelled_places.push(n);
        }
        // The first unit with the instance; the others after. A part with
        // fewer units than the source's symbol (a generated part's one
        // symbol) keeps the units it has; its terminals still take every
        // unit's wiring.
        let first = units.and_then(|u| u.first()).copied().map(|mut u| {
            u.unit = u.unit.clamp(1, set.units.max(1));
            u
        });
        let later: Vec<crate::sheets::UnitPlace> = units
            .map(|u| u[1..].to_vec())
            .unwrap_or_default()
            .into_iter()
            .filter(|u| u.unit <= set.units && Some(u.unit) != first.map(|f| f.unit))
            .collect();
        let dropped = units.map_or(0, |u| u.len().saturating_sub(1)) - later.len();
        if dropped > 0 {
            report.add_n(Case::UnitMerged, dropped);
        }
        later_units.push(later);
        first_units.push(first);
        let (sheet, position, unit) = match first {
            Some(u) => (Some(u.sheet), u.at, UnitId(u.unit.max(1))),
            None => (
                None,
                [column * SHEET_PITCH_MM, -row * SHEET_PITCH_MM],
                UnitId(1),
            ),
        };
        place.push(Operation::Instance(InstanceCommand::Place(NewInstance {
            sheet,
            component: set.component,
            designator,
            properties: vec![],
            representation: Some(set.symbol),
            unit: Some(unit),
            position: Some(PointNm::from_mm_f64_rounded(position).map_err(|e| {
                ImportError::Refused {
                    step: "component.place",
                    reason: e.to_string(),
                }
            })?),
            physical: Some(PhysicalChoice {
                binding: set.binding,
                footprint: set.footprint,
            }),
        })));
    }
    let mut instances: Vec<Id> = Vec::with_capacity(place.len());
    let mut first_occurrences: Vec<Option<Id>> = Vec::with_capacity(place.len());
    for outcome in session.batches("component.place", place)? {
        match outcome {
            Outcome::InstancePlaced {
                instance,
                occurrence,
                ..
            } => {
                instances.push(instance);
                first_occurrences.push(Some(occurrence));
            }
            Outcome::PartPlaced { instance, .. } => {
                instances.push(instance);
                first_occurrences.push(None);
            }
            _ => {
                return Err(ImportError::Refused {
                    step: "component.place",
                    reason: "no instance placed".into(),
                });
            }
        }
    }
    // The parts not fitted, not populated in the default, each named.
    let mut unfitted = Vec::new();
    for ((fp, instance), why) in placed.iter().zip(&instances).zip(&not_fitted) {
        if let Some(why) = why {
            let p = session.engine.project();
            let copper = p.instance(*instance).is_some_and(|i| {
                sparksmith_core::instances::ties::form(p, i)
                    == Some(sparksmith_core::library::tie::TieForm::Copper)
            });
            if copper {
                report.add_detail(
                    Case::CopperTieDnp,
                    &fp.reference,
                    "",
                    "Copper bridges stay closed in every assembly variant.",
                );
            } else {
                let case = match why {
                    Why::MarkerValue => Case::NotPopulatedMarker,
                    Why::DoNotPopulate => Case::NotPopulatedAttribute,
                };
                report.add_detail(case, &fp.reference, "", why.text());
                unfitted.push(*instance);
            }
        }
    }
    if !unfitted.is_empty() {
        session.batches(
            "variant.set_population",
            vec![Operation::Variant(VariantCommand::SetPopulation {
                instances: unfitted,
                variant: None,
                population: Some(Population::NotPopulated),
            })],
        )?;
    }
    // BOM and position-file exclusions, each the copy's own.
    for (bom, placement) in [(true, false), (false, true)] {
        let excluded: Vec<_> = placed
            .iter()
            .zip(&instances)
            .filter(|(fp, _)| (bom && fp.exclude_from_bom) || (placement && fp.exclude_from_pos))
            .map(|(_, instance)| *instance)
            .collect();
        if !excluded.is_empty() {
            session.batches(
                "variant.set_exclusions",
                vec![Operation::Variant(VariantCommand::SetExclusions {
                    instances: excluded,
                    bom: bom.then_some(true),
                    placement: placement.then_some(true),
                })],
            )?;
        }
    }
    // Every unit turned as KiCad turns it; the other units placed.
    if plan.is_some() {
        crate::sheets::units(
            &mut session,
            &instances,
            &first_occurrences,
            &first_units,
            &later_units,
            &mut report,
        );
    }
    // Parts on the schematic the board has no footprint of: placed on
    // their sheets with no footprint chosen, so a partly laid out project
    // keeps them (Issues lists each as having no footprint).
    if let Some((_, plan)) = &plan {
        schematic_only(
            &mut session,
            plan,
            &by_reference,
            &schematic_values,
            &schematic_not_fitted,
            &mut names_taken,
            &mut report,
        )?;
    }
    let mut layout = Vec::with_capacity(instances.len());
    for (fp, instance) in placed.iter().zip(&instances) {
        let (position, rotation_deg, layer) = compose(
            translate::placement(&board, fp),
            frames.get(layout.len()).copied().flatten(),
        );
        layout.push(Operation::Layout(LayoutCommand::Place {
            instance: *instance,
            position,
            rotation_deg,
            layer,
        }));
    }
    session.batches("layout.place", layout)?;
    // Each placed part's value as its typed fields, and its part number as
    // its manufacturer part.
    let mut fields = Vec::new();
    let mut parts: BTreeMap<(ComponentRef, String, String), Option<sparksmith_core::PartRef>> =
        BTreeMap::new();
    for ((n, fp), instance) in board.footprints.iter().enumerate().zip(&instances) {
        let Some(set) = placed_sets.get(n).and_then(|k| sets.get(*k)) else {
            continue;
        };
        if let Some(value) = values.get(n).and_then(Option::as_deref) {
            fields.push((*instance, set.component, value.to_owned()));
        }
        let part = part_numbers.get(n).cloned().unwrap_or_default();
        match (part.mpn, part.manufacturer) {
            (Some(mpn), Some(manufacturer)) => {
                let key = (set.component, manufacturer, mpn);
                if !parts.contains_key(&key) {
                    let made = session.manufacturer_part(key.0, set.binding, &key.1, &key.2);
                    if made.is_none() {
                        report.add_detail(Case::PartRefused, &fp.reference, "", "");
                    }
                    parts.insert(key.clone(), made);
                }
                if let Some(Some(part)) = parts.get(&key) {
                    let _ = session.apply(
                        "instance.assign_part",
                        Operation::Instance(InstanceCommand::AssignPart {
                            instances: vec![*instance],
                            part: Some(*part),
                        }),
                    );
                }
            }
            (Some(_), None) => {
                report.add_detail(Case::PartWithoutManufacturer, &fp.reference, "", "")
            }
            _ => {}
        }
    }
    for (instance, component, value) in fields {
        crate::values::set(&mut session, instance, component, &value, &mut report)?;
    }
    // Zone connection overrides: the footprint's for the part, each pad's
    // own over it.
    let mut overrides = Vec::new();
    for (((fp, instance), pads), k) in placed
        .iter()
        .zip(&instances)
        .zip(&placed_overrides)
        .zip(&placed_sets)
    {
        let own_pads = sets
            .get(*k)
            .and_then(|set| session.footprint_pads(*instance, set))
            .unwrap_or_default();
        let part = crate::zones::pad_override(&fp.zone, None);
        if !part.is_empty() {
            overrides.push(Operation::Fill(FillCommand::SetPadOverride {
                instance: *instance,
                pad: None,
                settings: part,
            }));
        }
        for (pad, settings) in pads {
            // The source's pad by its place; the part's pad at that place.
            let Some(own) = own_pads.get(pad.0.saturating_sub(1) as usize) else {
                continue;
            };
            overrides.push(Operation::Fill(FillCommand::SetPadOverride {
                instance: *instance,
                pad: Some(own.0),
                settings: *settings,
            }));
        }
    }
    session.batches("fill.set_pad_override", overrides)?;
    if let Some((schematic, plan)) = &plan {
        crate::sheets::wire(&mut session, schematic, plan, &mut report)?;
        crate::netlist::verify(&session, schematic, &mut report);
        for n in unlabelled_places {
            if let Some(id) = instances.get(n) {
                labelled.insert(*id);
            }
        }
    }
    let terminal_nets: Vec<Vec<(TerminalId, u32)>> = placed_nets
        .iter()
        .zip(&instances)
        .zip(&placed_sets)
        .map(|((nets, instance), k)| {
            let own = sets
                .get(*k)
                .and_then(|set| session.footprint_pads(*instance, set))
                .unwrap_or_default();
            nets.iter()
                .enumerate()
                .filter_map(|(i, net)| Some((own.get(i)?.1?, (*net)?)))
                .collect()
        })
        .collect();
    let nets = crate::routing::connect(
        &mut session,
        &board,
        &terminal_nets,
        &instances,
        plan.is_some().then_some(&labelled),
        &mut report,
    )?;
    crate::routing::copper(&mut session, &board, &nets, &mut report)?;
    session.project_rules(project_rules.as_ref(), &board, &nets, &mut report)?;
    crate::zones::fills(&mut session, &board, &nets, &mut report)?;
    report.counts.pads = board.footprints.iter().map(|f| f.pads.len()).sum();
    report.counts.pads_imported = records.len();
    report.add_n(Case::PadsExact, records.len());
    Ok(Imported {
        engine: session.engine,
        origin_nm: board.origin,
        aux_origin_nm: board.aux_origin,
        report,
        pads: records,
    })
}

/// The designator prefix of a reference: its leading capital letters.
/// Place each schematic part whose reference no placed part has (the
/// board has no footprint of it) as a part of its KiCad symbol alone,
/// with no footprint chosen, its units where KiCad draws them and its
/// value set. Parts of one KiCad symbol share a definition. A reference
/// that is no valid designator stays undrawn ([`Case::SymbolNotDrawn`]).
fn schematic_only(
    session: &mut Session<'_>,
    plan: &crate::sheets::Plan,
    symbols: &BTreeMap<String, crate::schematic::LibSymbol>,
    values: &BTreeMap<String, String>,
    not_fitted: &BTreeMap<String, Why>,
    names_taken: &mut BTreeMap<String, usize>,
    report: &mut Report,
) -> Result<(), ImportError> {
    let mut parts: BTreeMap<String, (ComponentRef, DefinitionId, u32)> = BTreeMap::new();
    let mut place = Vec::new();
    let mut pending = Vec::new();
    for (key, units) in &plan.places {
        let (Some((reference, lib_id)), Some(lib), Some(first)) =
            (plan.names.get(key), symbols.get(key), units.first())
        else {
            continue;
        };
        if session
            .engine
            .project()
            .instance_by_designator(reference)
            .is_some()
            || !sparksmith_core::instances::designator_valid(reference)
        {
            continue;
        }
        let part = match parts.get(lib_id) {
            Some(part) => *part,
            None => {
                let naming = definition_name(lib_id, names_taken.len(), names_taken);
                let made =
                    session.symbol_only_set(&naming.name, lib_id, &prefix_of(reference), lib)?;
                for (number, kind) in kept_pin_types(lib) {
                    report.add_detail(Case::PinTypeKept, reference, &number, &kind);
                }
                parts.insert(lib_id.clone(), made);
                made
            }
        };
        let (component, symbol, count) = part;
        let unit = first.unit.clamp(1, count.max(1));
        let later: Vec<crate::sheets::UnitPlace> = units[1..]
            .iter()
            .filter(|u| u.unit <= count && u.unit != unit)
            .copied()
            .collect();
        report.add_n(
            Case::UnitMerged,
            units.len().saturating_sub(1 + later.len()),
        );
        place.push(Operation::Instance(InstanceCommand::Place(NewInstance {
            sheet: Some(first.sheet),
            component,
            designator: Some(reference.clone()),
            properties: vec![],
            representation: Some(symbol),
            unit: Some(UnitId(unit)),
            position: Some(PointNm::from_mm_f64_rounded(first.at).map_err(|e| {
                ImportError::Refused {
                    step: "component.place",
                    reason: e.to_string(),
                }
            })?),
            physical: None,
        })));
        let first = crate::sheets::UnitPlace { unit, ..*first };
        pending.push((reference.clone(), component, key, first, later));
    }
    let mut unfitted = Vec::new();
    let mut instances = Vec::with_capacity(place.len());
    let mut occurrences = Vec::with_capacity(place.len());
    let mut firsts = Vec::with_capacity(place.len());
    let mut laters = Vec::with_capacity(place.len());
    for (outcome, (reference, component, key, first, later)) in session
        .batches("component.place", place)?
        .into_iter()
        .zip(pending)
    {
        let Outcome::InstancePlaced {
            instance,
            occurrence,
            ..
        } = outcome
        else {
            return Err(ImportError::Refused {
                step: "component.place",
                reason: "no instance placed".into(),
            });
        };
        report.add_detail(Case::SymbolWithoutFootprint, &reference, "", "");
        // Population is the part's own, with or without a footprint: the
        // do-not-populate attribute, or a not-fitted marker taken out of
        // the value, as for a part the board has.
        let value = values.get(key).and_then(|v| value_text(v));
        let (value, marker) = match value.as_deref().map(crate::population::split_marker) {
            Some(Some(rest)) => ((!rest.is_empty()).then_some(rest), Some(Why::MarkerValue)),
            _ => (value, None),
        };
        if let Some(value) = value {
            crate::values::set(session, instance, component, &value, report)?;
        }
        if let Some(why) = not_fitted
            .get(&reference.to_ascii_uppercase())
            .copied()
            .or(marker)
        {
            let case = match why {
                Why::MarkerValue => Case::NotPopulatedMarker,
                Why::DoNotPopulate => Case::NotPopulatedAttribute,
            };
            report.add_detail(case, &reference, "", why.text());
            unfitted.push(instance);
        }
        instances.push(instance);
        occurrences.push(Some(occurrence));
        firsts.push(Some(first));
        laters.push(later);
    }
    if !unfitted.is_empty() {
        session.batches(
            "variant.set_population",
            vec![Operation::Variant(VariantCommand::SetPopulation {
                instances: unfitted,
                variant: None,
                population: Some(Population::NotPopulated),
            })],
        )?;
    }
    crate::sheets::units(session, &instances, &occurrences, &firsts, &laters, report);
    Ok(())
}

fn prefix_of(reference: &str) -> String {
    let letters: String = reference
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if designator_prefix_valid(&letters) {
        letters
    } else {
        "U".to_owned()
    }
}

/// One distinct source footprint: its pads, KiCad symbol, value and part
/// number.
struct SourceDefinition {
    tie: Option<translate::ImportedTies>,
    pads: Vec<Pad>,
    symbol: Option<crate::schematic::LibSymbol>,
    value: Option<String>,
    mpn: Option<String>,
}

/// The board placement of a source footprint composed with a part's
/// package frame in it (`(turn, offset)`, [`crate::models::Generated`]):
/// the placement that puts the part's pads where the source's are. A
/// back-side placement reflects y before it turns, so the frame's turn
/// counts the other way there.
fn compose(
    (position, rotation_deg, layer): (sparksmith_core::Point, f64, sparksmith_core::Layer),
    frame: Option<(f64, PointNm)>,
) -> (sparksmith_core::Point, f64, sparksmith_core::Layer) {
    let Some((turn, offset)) = frame else {
        return (position, rotation_deg, layer);
    };
    let back = layer == sparksmith_core::Layer::Bottom;
    let [ox, oy] = offset.to_mm_f64();
    let oy = if back { -oy } else { oy };
    let (c, s) = sparksmith_core::geometry::cos_sin(rotation_deg);
    let moved = [position[0] + c * ox - s * oy, position[1] + s * ox + c * oy];
    let turned = if back {
        rotation_deg - turn
    } else {
        rotation_deg + turn
    };
    (moved, turned.rem_euclid(360.0), layer)
}

/// A footprint's pads in its definition frame, numbered, with their
/// package terminals: one per distinct pad number (pads sharing a number
/// share it), and one each for unnumbered plated pads.
fn footprint_pads(
    board: &Board,
    fp: &Footprint,
    report: &mut Report,
    records: &mut Vec<PadRecord>,
    nets: &mut Vec<Option<u32>>,
    overrides: &mut Vec<(PadId, PadFillOverride)>,
) -> Vec<Pad> {
    let mut pads = Vec::with_capacity(fp.pads.len());
    let mut terminals: BTreeMap<String, PackageTerminalId> = BTreeMap::new();
    let mut count = 0usize;
    let mut paste_only = Vec::new();
    for kpad in &fp.pads {
        let DefPad {
            mut pad,
            class,
            world,
            arcs,
        } = match translate::pad(board, fp, kpad, report) {
            Mapped::Pad(p) => *p,
            Mapped::PasteOnly(p) => {
                paste_only.push((p.pad, fp.reference.as_str(), kpad.number.as_str()));
                continue;
            }
            Mapped::Refused(case) => {
                if !matches!(
                    case,
                    Case::PadUnknownToken
                        | Case::CustomPadPrimitive
                        | Case::CustomPadNotOneOutline
                        | Case::PadShapeUnsupported
                ) {
                    report.add_detail(case, &fp.reference, &kpad.number, "");
                }
                continue;
            }
        };
        pad.id = PadId(u32::try_from(pads.len() + 1).unwrap_or(u32::MAX));
        // A footprint's "thermal for through-hole pads only" depends on
        // the pad, so it lands on each pad.
        let through_hole = pad.kind == PadKind::PlatedThrough;
        let mut own = crate::zones::pad_override(&kpad.zone, Some(through_hole));
        if fp.zone.connect == Some(3) && own.connection.is_none() {
            own.connection = crate::zones::pad_override(&fp.zone, Some(through_hole)).connection;
        }
        // A round pad without its own spoke angle has its spokes at 45°
        // (what KiCad's plots show for the omitted token); every other
        // shape at 0° and 90°, which our zones' reliefs use already.
        if own.spoke_angle.is_none() && kpad.shape == board::PadShapeK::Circle && kpad.net.is_some()
        {
            own.spoke_angle = Some(crate::zones::round_pad_spoke_angle());
        }
        if !own.is_empty() {
            overrides.push((pad.id, own));
        }
        if translate::needs_terminal(&pad) {
            let key = if pad.number.trim().is_empty() {
                // Unnumbered plated pads land their own terminal each.
                format!("\u{0}{}", pads.len())
            } else {
                pad.number.clone()
            };
            let id = *terminals.entry(key).or_insert_with(|| {
                let id = translate::terminal_id(count);
                count += 1;
                id
            });
            pad.terminal = Some(id);
        }
        nets.push(
            kpad.net
                .as_ref()
                .filter(|_| pad.terminal.is_some())
                .map(|(net, _)| *net),
        );
        records.push(PadRecord { class, world, arcs });
        pads.push(pad);
    }
    for (window, reference, number) in paste_only {
        if !translate::add_paste_window(&mut pads, &window) {
            report.add_detail(Case::PadWithoutCopper, reference, number, "");
        } else {
            report.add(Case::PasteWindow);
        }
    }
    if !pads.iter().any(|p| p.terminal.is_some()) {
        report.add(Case::NoTerminals);
    }
    pads
}

/// Bytes of a KiCad value a part keeps (the field's text limit).
const MAX_VALUE_BYTES: usize = sparksmith_core::fields::MAX_TEXT_BYTES as usize;

/// A KiCad value as a part's value text: without control characters,
/// trimmed and cut to [`MAX_VALUE_BYTES`] on a character boundary; `None`
/// when empty or KiCad's "~" (no value).
fn value_text(value: &str) -> Option<String> {
    let mut out = String::new();
    for c in value.trim().chars().filter(|c| !c.is_control()) {
        if out.len() + c.len_utf8() > MAX_VALUE_BYTES {
            break;
        }
        out.push(c);
    }
    let out = out.trim().to_owned();
    (!out.is_empty() && out != "~").then_some(out)
}

/// Parsed electrical data and the optional mechanical model for one definition.
struct ComponentSource<'a> {
    tie: Option<translate::ImportedTies>,
    name: &'a str,
    /// The KiCad footprint id (`Library:Name`), recorded in the provenance.
    source_id: &'a str,
    pads: Vec<Pad>,
    prefix: &'a str,
    kicad_symbol: Option<&'a crate::schematic::LibSymbol>,
    /// Why the part has no body, when it is no part to draw.
    no_body: Option<&'a str>,
}

/// The part of one source footprint: its component and the lineages a
/// placed part chooses.
pub(crate) struct Set {
    component: ComponentRef,
    symbol: DefinitionId,
    footprint: DefinitionId,
    binding: DefinitionId,
    /// A generated part's package frame in the source footprint's frame.
    frame: Option<(f64, PointNm)>,
    /// The symbol's units.
    units: u32,
}

pub(crate) struct Session<'a> {
    pub(crate) engine: Engine,
    pub(crate) evaluators: Evaluators<'a>,
}

/// Board parameters a rectangle sketch starts with (`board.start_sketch`).
const RECTANGLE_PARAMETERS: [&str; 2] = ["width", "height"];

impl Session<'_> {
    pub(crate) fn apply(
        &mut self,
        step: &'static str,
        op: Operation,
    ) -> Result<Outcome, ImportError> {
        self.engine
            .apply_hosted(self.engine.revision(), op, self.evaluators)
            .map_err(|e| ImportError::Refused {
                step,
                reason: e.to_string(),
            })
    }

    /// The standard part value field's selection, the standard fields
    /// imported first; `None` without the host's standards.
    fn part_value_selection(
        &mut self,
    ) -> Result<Option<sparksmith_core::fields::FieldSelection>, ImportError> {
        let Some(standards) = self.engine.context().map(|c| c.standards().clone()) else {
            return Ok(None);
        };
        if standards
            .fields
            .part_value(&self.engine.project().library)
            .is_none()
        {
            self.apply(
                "library.import_standard_fields",
                Operation::Library(LibraryCommand::ImportStandardFields {}),
            )?;
        }
        Ok(standards
            .fields
            .part_value_selection(&self.engine.project().library))
    }

    /// The fab profile, with the file's board-wide settings over it: the
    /// mask margin vias and inherited pads take, and whether vias are
    /// tented.
    fn rules(
        &mut self,
        board: &Board,
        profile: Option<&RuleProfile>,
        report: &mut Report,
    ) -> Result<(), ImportError> {
        let Some(profile) = profile else {
            report.add(Case::NoRules);
            return Ok(());
        };
        self.apply(
            "rules.set_fab_profile",
            Operation::Rules(RulesCommand::set_profile(profile.clone(), false)),
        )?;
        let mut overrides = vec![(
            RuleKey::MaskExpansion,
            RuleValue::Length(board.setup.mask_margin as f64 / 1e6),
        )];
        if let Some(open) = board.setup.vias_on_mask {
            overrides.push((RuleKey::TentVias, RuleValue::Flag(!open)));
        }
        self.set_overrides(overrides, report);
        Ok(())
    }

    /// The project file's rules as the project's own rule set, "From the
    /// KiCad project", over the fab profile: the default class's values
    /// and the board minimums as overrides (limits looser than the
    /// profile's are not taken), every other class as a net class with its
    /// name patterns and explicit assignments. Set after the tracks and
    /// vias, which go in as the file has them (a check finding, not a
    /// refusal, is the place for any the project's rules flag), and before
    /// the fills, which they shape.
    pub(crate) fn project_rules(
        &mut self,
        rules: Option<&crate::project::ProjectRules>,
        board: &Board,
        nets: &BTreeMap<u32, Id>,
        report: &mut Report,
    ) -> Result<(), ImportError> {
        if self.engine.project().rules.require().is_err() {
            return Ok(());
        }
        let Some(rules) = rules else {
            return Ok(());
        };
        report.add(Case::ProjectRules);
        let profile = self.engine.project().rules.effective();
        self.set_overrides(crate::project::overrides(rules, &profile, report), report);
        // Every other class, by name.
        let mut classes: BTreeMap<String, NetClassId> = BTreeMap::new();
        for class in &rules.classes {
            let spec = NetClassSpec {
                name: class.name.clone(),
                track_width: class.track_width,
                clearance: class.clearance,
                via_drill: class.via_drill,
                via_diameter: class.via_diameter,
                diff_pair_width: class.diff_pair_width,
                diff_pair_gap: class.diff_pair_gap,
                ..NetClassSpec::default()
            };
            match self.apply(
                "net_class.add",
                Operation::Rules(RulesCommand::AddNetClass { class: spec }),
            ) {
                Ok(Outcome::NetClassAdded { class: id }) => {
                    report.add(Case::NetClass);
                    classes.insert(class.name.clone(), id);
                }
                _ => report.add(Case::NetClassRefused),
            }
        }
        // Patterns that name an imported class and are plain wildcards
        // (`*`, `?`, every other character itself, `+` of power nets
        // such as `+5*` included); the default class's need none. Each
        // refused pattern is named with its reason.
        let default = rules.default.as_ref().map(|c| c.name.as_str());
        let mut patterns = Vec::new();
        for (pattern, class) in &rules.patterns {
            if Some(class.as_str()) == default {
                continue;
            }
            let why = if pattern.trim().is_empty()
                || pattern.len() > sparksmith_core::rules::MAX_PROFILE_TEXT
                || pattern.chars().any(char::is_control)
            {
                Some("empty, too long or with control characters")
            } else if pattern.contains(['[', ']', '(', ')', '{', '}', '^', '$', '|', '\\']) {
                Some("a regular expression; only * and ? wildcards are read")
            } else {
                None
            };
            match (why, classes.get(class)) {
                (None, Some(id)) => patterns.push(NetClassPattern {
                    pattern: pattern.clone(),
                    class: *id,
                }),
                (Some(why), _) => report.refused_pattern(pattern, why),
                (None, None) => report.refused_pattern(pattern, "its class was not imported"),
            }
        }
        // KiCad's class of each net (explicit, then the first matching
        // pattern), assigned explicitly where our patterns, on our net
        // names, would not give it.
        let kicad_class = |name: &str| -> Option<&str> {
            rules
                .assignments
                .iter()
                .find(|(net, _)| net == name)
                .map(|(_, c)| c.as_str())
                .or_else(|| {
                    rules
                        .patterns
                        .iter()
                        .find(|(p, _)| sparksmith_core::rules::glob_matches(p, name))
                        .map(|(_, c)| c.as_str())
                })
        };
        let mut assign: BTreeMap<NetClassId, Vec<Id>> = BTreeMap::new();
        for (number, net) in nets {
            let kicad_name = board.items.nets.get(number).map_or("", String::as_str);
            let Some(class) = kicad_class(kicad_name).filter(|c| Some(*c) != default) else {
                continue;
            };
            let Some(id) = classes.get(class) else {
                report.add(Case::NetClassPatternRefused);
                continue;
            };
            let ours = self.engine.project().net_name(*net).to_owned();
            let by_pattern = patterns
                .iter()
                .find(|p| sparksmith_core::rules::glob_matches(&p.pattern, &ours))
                .map(|p| p.class);
            if by_pattern != Some(*id) {
                assign.entry(*id).or_default().push(*net);
            }
        }
        if !patterns.is_empty() {
            self.apply(
                "net_class.set_patterns",
                Operation::Rules(RulesCommand::SetPatterns { patterns }),
            )?;
        }
        for (class, nets) in assign {
            report.add_n(Case::NetClassAssigned, nets.len());
            self.apply(
                "net_class.assign",
                Operation::Rules(RulesCommand::AssignNets {
                    nets,
                    class: Some(class),
                }),
            )?;
        }
        self.apply(
            "rules.set_project_source",
            Operation::Rules(RulesCommand::SetProjectSource {
                source: Some(crate::project::SOURCE.to_owned()),
            }),
        )?;
        Ok(())
    }

    fn set_overrides(&mut self, overrides: Vec<(RuleKey, RuleValue)>, report: &mut Report) {
        for (key, value) in overrides {
            if self
                .apply(
                    "rules.set_override",
                    Operation::Rules(RulesCommand::SetOverride { key, value }),
                )
                .is_err()
            {
                report.add(Case::RuleRefused);
            }
        }
    }

    /// The board outline: a board sketch of the `Edge.Cuts` drawings (with
    /// a board evaluator), else the rectangle from the origin around them.
    fn outline(&mut self, board: &Board, report: &mut Report) -> Result<(), ImportError> {
        let Some(bounds) = crate::shape::outline_bounds(board) else {
            report.add(Case::OutlineMissing);
            return Ok(());
        };
        if self.engine.project().rules.require().is_err() {
            report.add(Case::OutlineNotImported);
            return Ok(());
        }
        let curves = crate::shape::outline_edits(board, report);
        if self.evaluators.board.is_none() {
            // The box's corner is the origin, up to its fitted arcs.
            if bounds.min[0] < -1e-6 || bounds.min[1] < -1e-6 {
                report.add(Case::OutlineNotImported);
                return Ok(());
            }
            self.apply(
                "board.set_rectangle",
                Operation::Board(BoardCommand::SetRectangle { size: bounds.max }),
            )?;
            report.add(Case::OutlineRectangle);
            return Ok(());
        }
        self.apply(
            "board.start_sketch",
            Operation::Board(BoardCommand::StartSketch),
        )?;
        let BoardShape::Sketch(sketch) = &self.engine.project().board_shape else {
            return Err(ImportError::Refused {
                step: "board.start_sketch",
                reason: "no board sketch".into(),
            });
        };
        // The starting rectangle goes: its lines, then its parameters.
        let mut edits = vec![BoardEdit::Sketch {
            edit: SketchEdit::Delete {
                ids: sketch
                    .edges
                    .iter()
                    .map(|e| SketchRef::Curve(e.curve.clone()))
                    .collect(),
            },
        }];
        edits.extend(
            sketch
                .parameters
                .iter()
                .filter(|p| RECTANGLE_PARAMETERS.contains(&p.name.0.as_str()))
                .map(|p| BoardEdit::RemoveParameter {
                    name: p.name.clone(),
                }),
        );
        edits.extend(curves.into_iter().map(|edit| BoardEdit::Sketch { edit }));
        let max = sparksmith_core::layout::outline::MAX_BOARD_EDITS;
        for chunk in edits.chunks(max) {
            self.apply(
                "board.edit",
                Operation::Board(BoardCommand::Edit {
                    edits: chunk.to_vec(),
                }),
            )?;
        }
        match &self.engine.project().board_shape {
            BoardShape::Sketch(sketch) if sketch.fault.is_none() => {
                report.add(Case::OutlineSketch);
                report.counts.cutouts = sketch.solved.cutouts.len();
            }
            _ => report.add(Case::OutlineNotClosed),
        }
        Ok(())
    }

    /// The pads of `instance`'s footprint in order, each with its
    /// component terminal (through the part's binding).
    pub(crate) fn footprint_pads(
        &self,
        instance: Id,
        set: &Set,
    ) -> Option<Vec<(PadId, Option<TerminalId>)>> {
        let project = self.engine.project();
        let found = project.instance(instance)?;
        let resolved = sparksmith_core::instances::resolve(&project.library, found).ok()?;
        let physical = resolved.physical?;
        let _ = set;
        Some(
            physical
                .footprint
                .definition
                .pads
                .iter()
                .map(|pad| {
                    let terminal = pad.terminal.and_then(|t| {
                        physical
                            .binding
                            .definition
                            .mappings
                            .iter()
                            .find(|m| m.package_terminal == t)
                            .map(|m| m.terminal)
                    });
                    (pad.id, terminal)
                })
                .collect(),
        )
    }

    /// A manufacturer part of `component` (through `binding`), published:
    /// its manufacturer and part number as the source names them.
    fn manufacturer_part(
        &mut self,
        component: ComponentRef,
        binding: DefinitionId,
        manufacturer: &str,
        mpn: &str,
    ) -> Option<sparksmith_core::PartRef> {
        // The binding revision the component lists.
        let binding = self
            .engine
            .project()
            .library
            .component(component)?
            .definition
            .implementations
            .iter()
            .find(|b| b.id == binding)
            .copied()?;
        let Ok(Outcome::DraftCreated { id }) = self.apply(
            "library.new_draft",
            Operation::Library(LibraryCommand::NewDraft {
                meta: DefinitionMeta {
                    name: format!("{manufacturer} {mpn}"),
                    description: String::new(),
                    provenance: "Imported from a KiCad project's part number.".into(),
                },
                definition: Definition::Part(Box::new(sparksmith_core::library::PartDefinition {
                    manufacturer: manufacturer.to_owned(),
                    mpn: mpn.to_owned(),
                    component,
                    binding,
                    ..Default::default()
                })),
            }),
        ) else {
            return None;
        };
        self.apply(
            "library.publish",
            Operation::Library(LibraryCommand::Publish {
                drafts: vec![id],
                close: true,
            }),
        )
        .ok()?;
        Some(sparksmith_core::PartRef::new(id, Revision::FIRST))
    }

    /// Apply `ops` in batches of at most [`sparksmith_core::MAX_BATCH`].
    pub(crate) fn batches(
        &mut self,
        step: &'static str,
        ops: Vec<Operation>,
    ) -> Result<Vec<Outcome>, ImportError> {
        let mut out = Vec::with_capacity(ops.len());
        let mut ops = ops.into_iter().peekable();
        while ops.peek().is_some() {
            let batch: Vec<Operation> = ops.by_ref().take(sparksmith_core::MAX_BATCH).collect();
            let outcomes = self
                .engine
                .apply_all_hosted(self.engine.revision(), batch, self.evaluators)
                .map_err(|e| ImportError::Refused {
                    step,
                    reason: e.to_string(),
                })?;
            out.extend(outcomes);
        }
        Ok(out)
    }

    /// A part of a KiCad symbol alone, for a schematic part the board has
    /// no footprint of: a passive terminal per pin number, named by it, and
    /// no implementation (placed with no footprint chosen). Its component,
    /// symbol and the symbol's unit count.
    fn symbol_only_set(
        &mut self,
        name: &str,
        lib_id: &str,
        prefix: &str,
        lib: &crate::schematic::LibSymbol,
    ) -> Result<(ComponentRef, DefinitionId, u32), ImportError> {
        let refused = |reason: String| ImportError::Refused {
            step: "library.edit",
            reason,
        };
        let reserve = [DefinitionKind::Component, DefinitionKind::Symbol]
            .iter()
            .map(|kind| {
                Operation::Library(LibraryCommand::NewDraft {
                    meta: DefinitionMeta::default(),
                    definition: Definition::empty(*kind),
                })
            })
            .collect();
        let mut ids = Vec::with_capacity(2);
        for outcome in self.batches("library.new_draft", reserve)? {
            if let Outcome::DraftCreated { id } = outcome {
                ids.push(id);
            }
        }
        let [component, symbol] = ids[..] else {
            return Err(ImportError::Refused {
                step: "library.new_draft",
                reason: "two drafts expected".into(),
            });
        };
        let mut numbers: Vec<&str> = Vec::new();
        for pin in &lib.pins {
            let number = pin.number.as_str();
            if !number.trim().is_empty() && !numbers.contains(&number) {
                numbers.push(number);
            }
        }
        let named: Vec<(TerminalId, &str)> = (1u32..)
            .map(TerminalId)
            .zip(numbers.iter().copied())
            .collect();
        let mut unpinned = 0;
        let symbol_body = crate::sheets::kicad_symbol(component, lib, &named, &mut unpinned)
            .map_err(|e| refused(e.to_string()))?;
        let units = u32::try_from(symbol_body.units.len()).unwrap_or(1);
        let mut body = component_body(prefix, &named, symbol, None, Some(lib));
        if let Some(value) = self.part_value_selection()? {
            body.fields = vec![value];
        }
        let id: String = lib_id
            .chars()
            .filter(|c| !c.is_control())
            .take(4 * MAX_NAME_CHARS)
            .collect();
        let meta = |name: String| DefinitionMeta {
            name,
            description: String::new(),
            provenance: format!("{SYMBOL_PROVENANCE}\nKiCad symbol: {id}"),
        };
        let edits = vec![
            DraftEdit {
                id: component,
                meta: meta(name.to_owned()),
                definition: Definition::Component(body),
            },
            DraftEdit {
                id: symbol,
                meta: meta(format!("{name} symbol")),
                definition: Definition::Symbol(symbol_body),
            },
        ];
        self.batches(
            "library.publish",
            vec![
                Operation::Library(LibraryCommand::Edit { edits }),
                Operation::Library(LibraryCommand::Publish {
                    drafts: vec![component, symbol],
                    close: true,
                }),
            ],
        )?;
        Ok((ComponentRef::new(component, Revision::FIRST), symbol, units))
    }

    fn component_set(
        &mut self,
        source: ComponentSource<'_>,
        report: &mut Report,
    ) -> Result<Set, ImportError> {
        let ComponentSource {
            tie,
            name,
            source_id,
            pads,
            prefix,
            kicad_symbol,
            no_body,
        } = source;
        let kinds = [
            DefinitionKind::Component,
            DefinitionKind::Symbol,
            DefinitionKind::Package,
            DefinitionKind::Footprint,
            DefinitionKind::Binding,
        ];
        let reserve = kinds
            .iter()
            .map(|kind| {
                Operation::Library(LibraryCommand::NewDraft {
                    meta: DefinitionMeta::default(),
                    definition: Definition::empty(*kind),
                })
            })
            .collect();
        let mut ids = Vec::with_capacity(5);
        for outcome in self.batches("library.new_draft", reserve)? {
            match outcome {
                Outcome::DraftCreated { id } => ids.push(id),
                _ => {
                    return Err(ImportError::Refused {
                        step: "library.new_draft",
                        reason: "no draft created".into(),
                    });
                }
            }
        }
        let [component, symbol, package, footprint, binding] = ids[..] else {
            return Err(ImportError::Refused {
                step: "library.new_draft",
                reason: "five drafts expected".into(),
            });
        };
        let mut unpinned = 0;
        let fiducial = no_body.is_some()
            && !pads.is_empty()
            && pads.iter().all(|p| p.kind == PadKind::Fiducial);
        let mut edits = drafts(
            name,
            pads,
            prefix,
            kicad_symbol,
            &mut unpinned,
            [component, symbol, package, footprint, binding],
        )
        .map_err(|e| ImportError::Refused {
            step: "library.edit",
            reason: e.to_string(),
        })?;
        // The part holds its value in the standard part value field, as
        // a generated part of no family does.
        if let Some(value) = self.part_value_selection()? {
            for edit in &mut edits {
                if let Definition::Component(component) = &mut edit.definition {
                    component.fields = vec![value.clone()];
                }
            }
        }
        if !source_id.trim().is_empty() {
            let id: String = source_id
                .chars()
                .filter(|c| !c.is_control())
                .take(4 * MAX_NAME_CHARS)
                .collect();
            for edit in &mut edits {
                edit.meta
                    .provenance
                    .push_str(&format!("\nKiCad footprint: {id}"));
            }
        }
        if let Some(why) = no_body {
            for edit in &mut edits {
                match &mut edit.definition {
                    Definition::Package(package) => {
                        package.model = PackageModel::NoBody;
                        edit.meta.provenance.push_str(&format!("\n{why}"));
                    }
                    Definition::Component(component) if fiducial => {
                        component.assembly = sparksmith_core::library::tie::Assembly::CopperOnly;
                    }
                    _ => {}
                }
            }
        }
        if let Some(ties) = tie {
            for edit in &mut edits {
                match &mut edit.definition {
                    Definition::Footprint(fp) => {
                        fp.next_local = ties
                            .bridges
                            .iter()
                            .map(|b| b.id.0 + 1)
                            .max()
                            .unwrap_or(fp.next_local);
                        fp.bridges = ties.bridges.clone();
                    }
                    Definition::Component(c) => {
                        c.assembly = sparksmith_core::library::tie::Assembly::CopperOnly;
                        c.ties = ties.groups.clone();
                        if let Some(field) = self.engine.context().and_then(|ctx| {
                            ctx.standards()
                                .fields
                                .local(&self.engine.project().library, "Rated current")
                        }) {
                            c.fields.push(sparksmith_core::fields::FieldSelection {
                                field,
                                default: None,
                                show_on_symbol: false,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }
        report.add_n(Case::TerminalWithoutPin, unpinned);
        self.batches(
            "library.publish",
            vec![
                Operation::Library(LibraryCommand::Edit { edits }),
                Operation::Library(LibraryCommand::Publish {
                    drafts: vec![component, symbol, package, footprint, binding],
                    close: true,
                }),
            ],
        )?;
        let units = match self
            .engine
            .project()
            .library
            .symbol(SymbolRef::new(symbol, Revision::FIRST))
        {
            Some(symbol) => u32::try_from(symbol.definition.units.len()).unwrap_or(1),
            None => 1,
        };
        Ok(Set {
            component: ComponentRef::new(component, Revision::FIRST),
            symbol,
            footprint,
            binding,
            frame: None,
            units,
        })
    }
}

/// Longest definition name taken from a footprint's KiCad name, chars.
const MAX_NAME_CHARS: usize = 120;

/// The name of the `k`th imported definition, and how it was reached.
struct Naming {
    name: String,
    /// The name is not the KiCad id as it is (library nickname dropped,
    /// control characters or extra whitespace removed).
    changed: bool,
    /// An earlier definition took the name: this one is numbered apart.
    numbered: bool,
}

/// The name of the `k`th imported definition: its first footprint's KiCad
/// footprint name without the library nickname (`R_0805` of
/// `Library:R_0805`; the full id stays in the provenance), without control
/// characters, with whitespace runs made one space, trimmed and bounded,
/// else "KiCad footprint {k + 1}". A name an earlier definition took (two
/// libraries' footprints of one name, or one footprint whose copies differ)
/// gets ", 2", ", 3" and so on.
fn definition_name(lib_id: &str, k: usize, taken: &mut BTreeMap<String, usize>) -> Naming {
    let short = match lib_id.split_once(':') {
        Some((nickname, name)) if !nickname.trim().is_empty() && !name.trim().is_empty() => name,
        _ => lib_id,
    };
    let clean = short
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect::<String>();
    let base = if clean.is_empty() {
        format!("KiCad footprint {}", k + 1)
    } else {
        clean
    };
    let changed = !lib_id.trim().is_empty() && base != lib_id;
    let seen = taken.entry(base.clone()).or_insert(0);
    *seen += 1;
    if *seen == 1 {
        Naming {
            name: base,
            changed,
            numbered: false,
        }
    } else {
        Naming {
            name: format!("{base}, {seen}"),
            changed,
            numbered: true,
        }
    }
}

/// The core functions of a KiCad pin's electrical type (KiCad's public
/// file-format documentation names them), when it has an equivalent: the
/// direction and, for an open collector or emitter, the drive. `None` for
/// a type the core has no equivalent of (`tri_state`), and for a pin with
/// no type to keep (`passive`, `unspecified`, `free`, `no_connect`, whose
/// joining the netlist reads), which stay passive.
pub(crate) fn pin_direction(kind: &str) -> Option<(Direction, Option<Drive>)> {
    Some(match kind {
        "input" => (Direction::Input, None),
        "output" => (Direction::Output, None),
        "bidirectional" => (Direction::Bidirectional, None),
        "power_in" => (Direction::PowerInput, None),
        "power_out" => (Direction::PowerOutput, None),
        "open_collector" => (Direction::Output, Some(Drive::OpenDrain)),
        "open_emitter" => (Direction::Output, Some(Drive::OpenSource)),
        _ => return None,
    })
}

/// KiCad pin types with no core equivalent: imported passive and named in
/// the report ([`Case::PinTypeKept`]).
pub(crate) const UNSUPPORTED_PIN_TYPES: &[&str] = &["tri_state"];

/// The electrical type `lib` declares for the pin numbered `number`: its
/// first pin of that number.
fn pin_kind<'a>(lib: Option<&'a crate::schematic::LibSymbol>, number: &str) -> Option<&'a str> {
    lib?.pins
        .iter()
        .find(|p| p.number == number)
        .map(|p| p.electrical.as_str())
}

/// Each pin of `lib` whose type is kept as passive for want of an
/// equivalent: its number and type.
pub(crate) fn kept_pin_types(lib: &crate::schematic::LibSymbol) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for pin in &lib.pins {
        if UNSUPPORTED_PIN_TYPES.contains(&pin.electrical.as_str())
            && !out.iter().any(|(n, _)| *n == pin.number)
        {
            out.push((pin.number.clone(), pin.electrical.clone()));
        }
    }
    out
}

/// An imported component: terminals `named` (each by its pin or pad
/// number), each with the functions its KiCad pin's type has in `lib`
/// (passive without one), drawn by `symbol`, implemented by `binding`
/// (none for a part the board does not have).
fn component_body(
    prefix: &str,
    named: &[(TerminalId, &str)],
    symbol: DefinitionId,
    binding: Option<DefinitionId>,
    lib: Option<&crate::schematic::LibSymbol>,
) -> ComponentDefinition {
    let count = u32::try_from(named.len()).unwrap_or(u32::MAX);
    ComponentDefinition {
        assembly: Default::default(),
        ties: Default::default(),
        pin_rules: Vec::new(),
        marking: None,
        review: None,
        value_template: None,
        generic: None,
        next_local: count.saturating_mul(2).saturating_add(1),
        designator_prefix: prefix.into(),
        terminals: named
            .iter()
            .map(|(id, name)| {
                let (direction, drive) = pin_kind(lib, name)
                    .and_then(pin_direction)
                    .unwrap_or((Direction::Passive, None));
                let mut functions =
                    TerminalFunctions::fixed(FunctionId(id.0.saturating_add(count)), direction);
                if let (Some(drive), Some(first)) = (drive, functions.functions.first_mut()) {
                    first.drive = drive;
                }
                Terminal {
                    id: *id,
                    name: (*name).to_owned(),
                    class: TerminalClass::Electrical(functions),
                }
            })
            .collect(),
        resources: vec![],
        constraints: vec![],
        representations: vec![SymbolRef::new(symbol, Revision::FIRST)],
        implementations: binding
            .map(|b| BindingRef::new(b, Revision::FIRST))
            .into_iter()
            .collect(),
        fields: Vec::new(),
    }
}

/// The five definition bodies of one imported footprint, under `name`:
/// the component and footprint take it as it is, the symbol, package and
/// binding with their kind after it.
fn drafts(
    name: &str,
    pads: Vec<Pad>,
    prefix: &str,
    kicad_symbol: Option<&crate::schematic::LibSymbol>,
    unpinned: &mut usize,
    [component, symbol, package, footprint, binding]: [DefinitionId; 5],
) -> sparksmith_core::Result<Vec<DraftEdit>> {
    // Package terminals in id order, designated by their first pad's
    // number (an unnumbered plated pad is "MP" and its terminal number).
    let mut designations: BTreeMap<PackageTerminalId, String> = BTreeMap::new();
    for pad in &pads {
        if let Some(t) = pad.terminal {
            designations.entry(t).or_insert_with(|| {
                if pad.number.trim().is_empty() {
                    format!("MP{}", t.0)
                } else {
                    pad.number.clone()
                }
            });
        }
    }
    let count = u32::try_from(designations.len()).unwrap_or(u32::MAX);
    let terminals: Vec<TerminalId> = (1..=count).map(TerminalId).collect();
    let named: Vec<(TerminalId, &str)> = terminals
        .iter()
        .zip(designations.values())
        .map(|(t, n)| (*t, n.as_str()))
        .collect();
    let symbol_body = match kicad_symbol {
        Some(lib) => crate::sheets::kicad_symbol(component, lib, &named, unpinned)?,
        None => named_block_symbol(component, &named)?,
    };
    let bounds = Bounds::of(pads.iter().flat_map(|p| {
        let b = p.bounds();
        [b.min, b.max]
    }))
    .unwrap_or(Bounds::point([0.0, 0.0]));
    let size = bounds.size();
    // The placeholder box is centred on its pads, not on the footprint's
    // origin (a header's origin is often its pin 1).
    let centre = bounds.center();
    let placeholder = ModelPlacement::from_mm([centre[0], centre[1], 0.0], [0.0; 3])?;
    let envelope = [
        Nm::from_mm_f64_rounded(size[0].max(0.1))?,
        Nm::from_mm_f64_rounded(size[1].max(0.1))?,
        Nm::from_mm_f64_rounded(1.0)?,
    ];
    let next_local = u32::try_from(pads.len() + 1).unwrap_or(u32::MAX);
    let meta = |what: &str| DefinitionMeta {
        name: match what {
            "component" | "footprint" => name.to_owned(),
            what => format!("{name} {what}"),
        },
        description: String::new(),
        provenance: PROVENANCE.into(),
    };
    Ok(vec![
        DraftEdit {
            id: component,
            meta: meta("component"),
            definition: Definition::Component(component_body(
                prefix,
                &named,
                symbol,
                Some(binding),
                kicad_symbol,
            )),
        },
        DraftEdit {
            id: symbol,
            meta: meta("symbol"),
            definition: Definition::Symbol(symbol_body),
        },
        DraftEdit {
            id: package,
            meta: meta("package"),
            definition: Definition::Package(PackageDefinition {
                next_local: count + 1,
                terminals: designations
                    .iter()
                    .map(|(id, designation)| PackageTerminal {
                        id: *id,
                        designation: designation.clone(),
                    })
                    .collect(),
                envelope_nm: envelope,
                // No body of its own yet: the package's default, as for any
                // part authored without a model, until a generator's body
                // or a recreation from its datasheet replaces it.
                model: PackageModel::default(),
                placement: placeholder,
                mechanical: None,
                datums: Vec::new(),
                leads: Vec::new(),
                joints: None,
            }),
        },
        DraftEdit {
            id: footprint,
            meta: meta("footprint"),
            definition: Definition::Footprint(FootprintDefinition {
                package: PackageRef::new(package, Revision::FIRST),
                next_local,
                pads,
                defaults: PadDefaults::default(),
                ..FootprintDefinition::default()
            }),
        },
        DraftEdit {
            id: binding,
            meta: meta("binding"),
            definition: Definition::Binding(BindingDefinition {
                component,
                package: PackageRef::new(package, Revision::FIRST),
                mappings: designations
                    .keys()
                    .zip(&terminals)
                    .map(|(p, t)| TerminalMap {
                        package_terminal: *p,
                        terminal: *t,
                    })
                    .collect(),
                footprints: vec![FootprintRef::new(footprint, Revision::FIRST)],
                unavailable: vec![],
            }),
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A definition is named by its KiCad footprint name without the
    /// library nickname; a repeated name is numbered, and a footprint
    /// without one is numbered by its place.
    #[test]
    fn definitions_take_their_kicad_footprint_names() {
        let mut taken = BTreeMap::new();
        let mut name = |lib_id: &str, k: usize| {
            let n = definition_name(lib_id, k, &mut taken);
            (n.name, n.changed, n.numbered)
        };
        assert_eq!(name("Lib:R_0805", 0), ("R_0805".into(), true, false));
        assert_eq!(name("Other:R_0805", 1), ("R_0805, 2".into(), true, true));
        assert_eq!(name("R_0805", 2), ("R_0805, 3".into(), false, true));
        assert_eq!(name("  ", 3), ("KiCad footprint 4".into(), false, false));
        assert_eq!(name("Lib:C\n_0603", 4), ("C_0603".into(), true, false));
        assert_eq!(
            name("Lib:  Wide   name ", 5),
            ("Wide name".into(), true, false)
        );
        assert_eq!(name(":Bare", 6), (":Bare".into(), false, false));
        assert_eq!(name("Lib:", 7), ("Lib:".into(), false, false));
        assert_eq!(name(&"x".repeat(500), 8).0.chars().count(), MAX_NAME_CHARS);
    }
}
