//! Sparksmith's electronics document and its single typed command engine.
//!
//! This crate is pure and wasm-compatible: no files, threads, windows or
//! transports. `sparksmith-io` reads and writes documents; transports (UI, CLI,
//! MCP) translate requests into [`Operation`]s applied by [`Engine`].
// First: its `declare_entries!` is in textual scope for every module below.
#[macro_use]
pub mod permission;

pub mod catalog;
pub mod commands;
pub mod completeness;
pub mod diagnostics;
pub mod erc;
pub mod fabrication;
pub mod fields;
pub mod firmware;
pub mod format;
pub mod functions;
pub mod geometry;
pub mod help;
pub mod ids;
pub mod instances;
pub mod interface;
pub mod issues;
pub mod layout;
pub mod library;
pub mod materials;
pub mod model;
pub mod nets;
pub mod pins;
pub mod product;
pub mod quantity;
pub mod rules;
pub mod schematic;
pub mod shared;
#[cfg(test)]
mod tests;
pub mod units;
pub mod updates;
pub mod validate;
pub use commands::{BoardPlace, InstanceCommand, LibraryCommand, OffGridPin, Outcome};
use completeness::{CheckTarget, DefinitionReport, InstanceReport};
pub use diagnostics::{Diagnostic, Error, Finding, ObjectRef, Result, Stage};
pub use geometry::point_valid;
pub use ids::*;
use library::part_refs::SampleRefs;
use library::{Definition, DefinitionMeta, Drafts};
pub use model::*;
pub use permission::Class;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
pub use shared::Shared;
use std::collections::VecDeque;
use std::sync::Arc;

/// Every document mutation. UI gestures, CLI requests and MCP tools all
/// produce one of these and apply it through [`Engine::apply`].
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "editor", content = "operation", deny_unknown_fields)]
pub enum Operation {
    Library(LibraryCommand),
    Instance(InstanceCommand),
    Schematic(schematic::SchematicCommand),
    Layout(layout::LayoutCommand),
    /// Design rules and net classes (`rules.*`, `net_class.*`).
    Rules(rules::RulesCommand),
    /// Copper fills and keepouts (`fill.*`, `keepout.*`).
    Fill(layout::fill::FillCommand),
    /// Board shape and edge constraints (`board.*`).
    Board(layout::outline::BoardCommand),
    /// Part dimensions (`layout.dimension_*`).
    Dimension(layout::dimensions::DimensionCommand),
    /// Pre-flight waivers for fabrication output (`fabrication.*`).
    Fabrication(fabrication::FabricationCommand),
    /// The board interface an assembly references (`interface.*`).
    Interface(interface::InterfaceCommand),
    /// The assembly link (`assembly.*`).
    Assembly(interface::assembly_link::AssemblyCommand),
    /// Assembly variants and which parts each fits (`variant.*`).
    Variant(instances::variants::VariantCommand),
    Update(Box<updates::ApplyUpdate>),
}

impl Operation {
    /// This operation's permission class, chosen beside its command enum.
    pub const fn class(&self) -> Class {
        match self {
            Self::Library(command) => command.class(),
            Self::Instance(command) => command.class(),
            Self::Schematic(command) => command.class(),
            Self::Layout(command) => command.class(),
            Self::Rules(command) => command.class(),
            Self::Fill(command) => command.class(),
            Self::Board(command) => command.class(),
            Self::Dimension(command) => command.class(),
            Self::Fabrication(command) => command.class(),
            Self::Interface(command) => command.class(),
            Self::Assembly(command) => command.class(),
            Self::Variant(command) => command.class(),
            Self::Update(_) => Class::Edit,
        }
    }

    /// This operation's wire name; `update` for an applied update.
    pub const fn command(&self) -> &'static str {
        match self {
            Self::Library(command) => command.command(),
            Self::Instance(command) => command.command(),
            Self::Schematic(command) => command.command(),
            Self::Layout(command) => command.command(),
            Self::Rules(command) => command.command(),
            Self::Fill(command) => command.command(),
            Self::Board(command) => command.command(),
            Self::Dimension(command) => command.command(),
            Self::Fabrication(command) => command.command(),
            Self::Interface(command) => command.command(),
            Self::Assembly(command) => command.command(),
            Self::Variant(command) => command.command(),
            Self::Update(_) => "update",
        }
    }

    /// Whether this makes or reshapes copper: work at the widths and
    /// clearances of a fab profile. The engine refuses it on a board without
    /// one
    /// ([`rules::ProjectRules::require`]), as the desktop's tools do;
    /// taking copper away needs none.
    fn needs_profile(&self) -> bool {
        use layout::LayoutCommand as L;
        match self {
            Self::Layout(L::ApplyRoute { edits }) => {
                !(edits.add.is_empty()
                    && edits.replace.is_empty()
                    && edits.vias.is_empty()
                    && edits.restore.is_empty()
                    && edits.move_vias.is_empty())
            }
            Self::Layout(
                L::Route { .. }
                | L::SetTrack { .. }
                | L::AddVia { .. }
                | L::MoveVia { .. }
                | L::RouteLayerChange { .. }
                | L::RoutePair { .. }
                | L::TuneLength { .. }
                | L::RouteStep { .. }
                | L::TrackDrag { .. },
            ) => true,
            Self::Fill(layout::fill::FillCommand::Add { .. }) => true,
            _ => false,
        }
    }

    /// This operation's title for people, from [`Self::entries`].
    pub fn title(&self) -> &'static str {
        permission::find(Self::entries(), self.command()).map_or("Change", |entry| entry.title)
    }

    /// The area of the design this operation changes.
    pub const fn area(&self) -> Area {
        match self {
            Self::Library(_) => Area::Component,
            Self::Instance(InstanceCommand::SetTextPlacement { .. })
            | Self::Layout(_)
            | Self::Rules(_)
            | Self::Fill(_)
            | Self::Board(_)
            | Self::Dimension(_)
            | Self::Fabrication(_)
            | Self::Interface(_)
            | Self::Assembly(_) => Area::Pcb,
            Self::Schematic(
                schematic::SchematicCommand::SetProperties { .. }
                | schematic::SchematicCommand::RenameProject { .. },
            )
            | Self::Variant(_) => Area::Project,
            Self::Instance(_) | Self::Schematic(_) | Self::Update(_) => Area::Schematic,
        }
    }

    /// The one component instance this operation concerns, if it names
    /// exactly one.
    pub fn instance(&self) -> Option<Id> {
        use layout::LayoutCommand as L;
        match self {
            Self::Layout(
                L::Place { instance, .. }
                | L::Move { instance, .. }
                | L::SetPlacementLock { instance, .. }
                | L::SetPart { instance, .. }
                | L::RemovePart { instance },
            )
            | Self::Instance(
                InstanceCommand::Remove { instance }
                | InstanceCommand::SetDesignator { instance, .. }
                | InstanceCommand::SetProperties { instance, .. }
                | InstanceCommand::SelectFunctions { instance, .. }
                | InstanceCommand::SetUnitUnused { instance, .. }
                | InstanceCommand::SetNoConnect { instance, .. }
                | InstanceCommand::SetPurpose { instance, .. }
                | InstanceCommand::SetAnchor { instance, .. }
                | InstanceCommand::SetPhysical { instance, .. }
                | InstanceCommand::SetPinConfig { instance, .. }
                | InstanceCommand::SetMarking { instance, .. }
                | InstanceCommand::SetTextPlacement { instance, .. },
            ) => Some(*instance),
            Self::Instance(InstanceCommand::AssignPart { instances, .. })
            | Self::Variant(instances::variants::VariantCommand::SetPopulation {
                instances, ..
            }) => match instances[..] {
                [one] => Some(one),
                _ => None,
            },
            Self::Variant(instances::variants::VariantCommand::SetPart { instance, .. }) => {
                Some(*instance)
            }
            _ => None,
        }
    }

    /// Every operation as declared beside its command enum, for discovery.
    pub fn entries() -> impl Iterator<Item = &'static permission::EntryDecl> {
        // An applied update moves instances, wiring or routed copper: the
        // person reviews its impact first.
        const UPDATE: &[permission::EntryDecl] =
            &[
                permission::EntryDecl::new("update", "library", Class::Edit, "Apply an update")
                    .review(),
            ];
        [
            LibraryCommand::ENTRIES,
            InstanceCommand::ENTRIES,
            schematic::SchematicCommand::ENTRIES,
            layout::LayoutCommand::ENTRIES,
            rules::RulesCommand::ENTRIES,
            layout::fill::FillCommand::ENTRIES,
            layout::outline::BoardCommand::ENTRIES,
            layout::dimensions::DimensionCommand::ENTRIES,
            fabrication::FabricationCommand::ENTRIES,
            interface::InterfaceCommand::ENTRIES,
            interface::assembly_link::AssemblyCommand::ENTRIES,
            instances::variants::VariantCommand::ENTRIES,
            UPDATE,
        ]
        .into_iter()
        .flatten()
    }
}

/// Every read-only report. Queries never change the document, its revision
/// or history.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "query", content = "params", deny_unknown_fields)]
pub enum Query {
    /// Exact pins, resolved choices, unit states and completeness.
    #[serde(rename = "component.inspect")]
    InspectInstance { instance: Id },
    /// Publish findings of exactly one draft or published revision.
    #[serde(rename = "component.check")]
    CheckDefinition { target: CheckTarget },
    /// `component.check` of every draft, in draft order, staged once.
    #[serde(rename = "component.check_drafts")]
    CheckDrafts {
        /// Every object each finding names. Without it, over JSON lines
        /// and MCP, a finding names its first few objects and counts the
        /// rest in `objects_more`; the report is otherwise the same.
        #[serde(default)]
        detail: bool,
    },
    /// List or search library components and Parts, ranked, at their latest
    /// revision. `src:project kind:component` lists the project's own
    /// components. The query takes free text, `pkg:`/`has:`/`missing:`
    /// filters and unit-aware field predicates such as `res>=10k`, `tol<=1%` or
    /// `C:1n..100n`. Field keys resolve against the project's field
    /// definitions.
    #[serde(rename = "library.search")]
    Search {
        query: String,
        /// At most 500; 50 when omitted.
        #[serde(default)]
        limit: Option<usize>,
        /// List lineages that exist only as drafts instead of counting them
        /// as hidden.
        #[serde(default)]
        include_drafts: bool,
    },
    /// The fields, package labels and completeness badges that the entries
    /// matching a query carry, with value counts.
    #[serde(rename = "library.facets")]
    Facets {
        query: String,
        #[serde(default)]
        include_drafts: bool,
    },
    /// One published component revision (the latest when `revision` is
    /// omitted) with its revisions, symbols, implementations and badges.
    #[serde(rename = "library.inspect")]
    InspectLineage {
        lineage: DefinitionId,
        #[serde(default)]
        revision: Option<Revision>,
    },
    /// The project BOM, with resolved instances grouped by Part and unresolved
    /// lines marked. Unresolved instances group by component revision,
    /// footprint and identical requirements.
    #[serde(rename = "project.bom")]
    Bom {},
    /// Groups of instances without a resolved Part, each with the
    /// `library.search` query that finds Parts for it. A group shares
    /// requirements and footprint; an instance with an unresolved Part is
    /// included.
    #[serde(rename = "library.resolve_groups")]
    ResolveGroups {},
    /// The distinct nets of every deliberate tie, active state, contacts and current margin.
    #[serde(rename = "tie.list")]
    Ties {},
    /// The assembly variants, the active one, and every bought part's
    /// population and fitted Part in the default and in each variant.
    #[serde(rename = "variant.list")]
    Variants {},
    /// The Parts of the project library that can be fitted on a part's
    /// footprint in a variant: the same land pattern and pad grouping.
    #[serde(rename = "variant.substitutes")]
    Substitutes { instance: Id },
    /// What `library.edit_footprint` would produce for these edits, without
    /// changing the document. Pads, graphics and findings, including blocking
    /// ones. Needs the host's footprint evaluator.
    #[serde(rename = "library.footprint_preview")]
    FootprintPreview {
        draft: DefinitionId,
        edits: Vec<library::footprint_source::FootprintEdit>,
    },
    /// Project lineages copied from a library source that now holds a newer
    /// revision, or whose recorded revision changed on disk. Needs the
    /// host's source catalogs (`CatalogContext::sources`).
    #[serde(rename = "library.updates_available")]
    UpdatesAvailable {},
    /// What `library.promote` would write to the library `library` and
    /// record in the project: the export bundle of `root` and its closure.
    /// New lineages take ids from `fresh`, in order (io draws them).
    #[serde(rename = "library.promote_preview")]
    PromotePreview {
        root: DefinitionRef,
        library: LibraryId,
        /// The library's display name, carried in the bundle.
        name: String,
        fresh: Vec<SourceLineageId>,
    },
    /// Every issue of the project and its drafts in `scope`, most severe first,
    /// with its severity, group and title. Each row has the one-line title core
    /// gives the finding (docs/sidebar-ia.md); uncapped, with same-cause
    /// findings merged into one row and every finding kept.
    #[serde(rename = "project.issues")]
    Issues { scope: issues::IssueScope },
    /// The built-in standard fields as a bundle `library.import` takes
    /// (`library.import_standard_fields` does both in one step).
    #[serde(rename = "library.standard_fields")]
    StandardFields {},
    /// D1: 3D library models (and generated families) for a query, ranked
    /// by fit when `fits` names the package draft the chooser serves.
    #[serde(rename = "library.models")]
    Models {
        query: String,
        #[serde(default)]
        fits: Option<DefinitionId>,
        #[serde(default)]
        limit: Option<usize>,
    },
    /// C3: the plan `library.pinout_apply` would apply: changes, findings
    /// and review items.
    #[serde(rename = "library.pinout_preview")]
    PinoutPreview {
        target: library::pinout::PinoutTarget,
        document: Box<library::pinout::PinoutDocument>,
        /// List every change and each finding's objects; by default the
        /// plan gives change counts, findings, review items and rule
        /// checks.
        #[serde(default)]
        detail: bool,
    },
    /// C2: ranked complete assignments that enable peripherals on one
    /// instance, or the rules that block them. Never applies anything.
    #[serde(rename = "functions.suggest")]
    SuggestPins {
        instance: Id,
        request: functions::suggest::SuggestRequest,
    },
    /// Per terminal of one instance: pads, GPIO, active function, peripheral,
    /// role, mux code, pin config and net. Every other endpoint on the net is
    /// listed too (C2).
    #[serde(rename = "project.pin_report")]
    PinReport { instance: Id },
    /// The named parts' pins, one compact row each: terminal id, pads,
    /// name and net. What wiring with `schematic.connect` needs.
    #[serde(rename = "schematic.pins")]
    PartPins { designators: Vec<String> },
    /// Compare exact named net memberships with the schematic and rehearse the
    /// corrective operations. Use instance and terminal IDs from schematic.pins.
    #[serde(rename = "schematic.check_intent")]
    CheckIntent { nets: schematic::intent::IntentNets },
    /// C4: the firmware pin-configuration IR of one instance.
    #[serde(rename = "project.pin_config")]
    PinConfig { instance: Id },
    /// D4: one instance's resolved marking layout (none when unmarked).
    #[serde(rename = "instance.marking")]
    Marking { instance: Id },
    /// Every effective design rule with its profile value and source,
    /// and the net classes.
    #[serde(rename = "rules.effective")]
    RulesEffective {},
    /// The fab profiles by id, with their options, choices and defaults.
    /// `rules.set_fab_profile` takes the id. The board's own comes first
    /// (with its choices); hosts with a profile library (desktop, CLI, MCP)
    /// add the user's profiles and the shipped ones. With `id`, that one
    /// profile in full, with its rules and sources.
    #[serde(rename = "rules.fab_profiles")]
    RuleProfiles {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
    /// What `rules.set_fab_profile` with these arguments would change: rules
    /// and the board issues that would appear or go.
    #[serde(rename = "rules.preview_fab_profile")]
    PreviewProfile {
        profile: rules::ProfileChoice,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        options: std::collections::BTreeMap<String, String>,
        #[serde(default)]
        keep_overrides: bool,
    },
    /// What `rules.set_fab_options` with these choices would change: rules
    /// and the board issues that would appear or go.
    #[serde(rename = "rules.preview_fab_options")]
    PreviewOptions {
        options: std::collections::BTreeMap<String, String>,
    },
    /// Every net class with its resolved width, via and clearance, and
    /// every net with its class and how it got it (explicit or pattern).
    #[serde(rename = "net_class.list")]
    NetClasses {},
    /// The nets whose class `net_class.set_patterns` with `patterns` would
    /// change. Never applies anything.
    #[serde(rename = "net_class.preview_patterns")]
    PreviewNetClassPatterns {
        patterns: Vec<rules::NetClassPattern>,
    },
    /// The solved board outline, cutouts, holes and named edges.
    #[serde(rename = "board.shape")]
    BoardShape {},
    /// Per fill: whether its computed copper is stale.
    #[serde(rename = "fill.status")]
    FillStatus {},
    /// Every schematic sheet in page order with its place in the hierarchy
    /// (parent sheet symbol, depth) and what it holds.
    #[serde(rename = "schematic.sheets")]
    Sheets {},
    /// Where a net appears across sheets: its labels, sheet pins and placed
    /// terminals, in page order. Each with sheet and position, for jumping
    /// between a label's other occurrences.
    #[serde(rename = "schematic.net_places")]
    NetPlaces { net: Id },
    /// What `schematic.paste` needs to recreate these sheet objects. The
    /// symbols' instances, labels, drawings, the wires named and the wires with
    /// both ends on named objects. A wire end on anything else reads as a free
    /// end. Changes nothing, so a transport or the app can keep the result as
    /// its copy buffer.
    #[serde(rename = "schematic.copy")]
    SheetCopy {
        /// The sheet they are on; the first sheet when omitted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sheet: Option<SheetId>,
        objects: Vec<schematic::ClipObject>,
        /// Also carry the library revisions the parts reach as bundles, so
        /// the clip pastes into another project, which imports them with
        /// fresh ids. Without it, such a paste is refused unless that
        /// project holds the same revisions under the same ids.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        portable: bool,
    },
    /// Everything one sheet draws, resolved to shapes and text in sheet
    /// millimetres. Page frame and title block, labels, power ports, sheet
    /// symbols, furniture, wires, junction dots and units.
    #[serde(rename = "schematic.scene")]
    SheetScene {
        sheet: SheetId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<model::sheet::SheetPath>,
    },
    /// Z0 (and, with a gap, the edge-coupled Zdiff) of a trace on a
    /// stackup layer or an explicit structure, with the model's accuracy.
    #[serde(rename = "impedance.calculate")]
    ImpedanceCalculate {
        request: layout::impedance::ImpedanceRequest,
    },
    /// Every layer's line structure and the width and pair gap each impedance
    /// target needs on each layer. Per net class with an impedance target, and
    /// why a target cannot be met on a layer.
    #[serde(rename = "impedance.table")]
    ImpedanceTable {},
    /// Differential pairs proposed from net names (`USB_D+`/`USB_D-`,
    /// `_P`/`_N`, `DP`/`DM`); `diff_pair.add` makes one.
    #[serde(rename = "diff_pair.suggest")]
    SuggestPairs {},
    /// Every routed, paired or grouped net's length with its target and
    /// deviation, and every pair's skew. Length counts tracks plus via barrels;
    /// pairs also report uncoupled length, and every length group its window.
    #[serde(rename = "layout.length_report")]
    LengthReport {},
    /// The placed parts' connected pads: designator, pad number, net and
    /// board position (mm), where `layout.route` starts and ends. Narrowed
    /// to `designators` and to `net` when given.
    #[serde(rename = "layout.pads")]
    Pads {
        #[serde(default)]
        designators: Vec<String>,
        #[serde(default)]
        net: Option<Id>,
    },
    /// What `layout.route_step` would do, without applying it. The edit set
    /// (the step's copper and every shoved track and via), where the head ends
    /// and why it stopped short.
    #[serde(rename = "layout.route_preview")]
    RoutePreview {
        request: layout::router::RouteStepRequest,
    },
    /// What `layout.track_drag` would do, without applying it. The plan has
    /// the moved segment, its neighbours and every shoved track and via, the
    /// moved segment's midpoint (`end`) and why it stopped short.
    #[serde(rename = "layout.track_drag_preview")]
    TrackDragPreview {
        request: layout::router::drag::TrackDragRequest,
    },
    /// What `layout.tune_length` would build, without applying it.
    #[serde(rename = "layout.tune_preview")]
    TunePreview {
        request: layout::meander::TuneRequest,
    },
    /// The production check before fabrication export: the design and
    /// production findings under a fab profile. `profile` is a profile id such
    /// as `std-6-6` or a whole profile, and the project's rules when omitted;
    /// each finding has its waiver key and saved waiver.
    #[serde(rename = "fabrication.preflight")]
    Preflight {
        #[serde(default)]
        profile: Option<fabrication::ProfileChoice>,
    },
    /// What the board needs of a fab profile and which installed profiles
    /// allow it. Under `profile` (an id, a whole profile, or the board's
    /// own when omitted) each profile-limited finding's measurement is a
    /// need: the rule, the measured value, the profile's limit and its
    /// source. The board is then checked, as an export checks it, under
    /// every installed profile (hosts with a profile library add the
    /// user's and the shipped ones to `candidates`), and those that clear
    /// every need are suggested: the board's own fabricator's first, then
    /// other fabricators', then profiles that name none. Switch with
    /// `rules.set_fab_profile` and the suggestion's id. No suggestion
    /// means no installed profile allows the board: the needs say what
    /// one must.
    #[serde(rename = "fabrication.profile_suggestions")]
    ProfileSuggestions {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile: Option<fabrication::ProfileChoice>,
        /// Profiles to check besides the host's library.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        candidates: Vec<rules::RuleProfile>,
        /// Fabricator records naming the candidates' fabricators, besides
        /// the host's.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fabricators: Vec<rules::Fabricator>,
    },
    /// The board interface's exported datums, rules and retired slots.
    /// Each datum has its id, slot, token, display name, role, kind,
    /// source, geometry (board millimetres), status and whether it changed
    /// since the last publish; the list has the last published revision.
    #[serde(rename = "interface.list")]
    InterfaceList {},
    /// The assembly link's state from the document: none, a
    /// broken reference (publishing is blocked) or linked, with
    /// the findings. `assembly.preview` reads the file.
    #[serde(rename = "assembly.status")]
    AssemblyStatus {},
}

// A new command chooses its permission class here.
declare_entries!(Query in project {
    InspectInstance "component.inspect" Read "Inspect an instance" [in schematic, link, returns(instance)],
    CheckDefinition "component.check" Read "Check a definition" [in library, link, returns(definition)],
    CheckDrafts "component.check_drafts" Read "Check all drafts" [in library, link, returns(drafts)],
    Search "library.search" Read "List or search library components" [in library, link, returns(search)],
    Facets "library.facets" Read "List search filters" [in library, link, returns(facets)],
    InspectLineage "library.inspect" Read "Inspect a lineage" [in library, link, returns(lineage)],
    Bom "project.bom" Read "Bill of materials" [link, returns(bom)],
    ResolveGroups "library.resolve_groups" Read "Group instances for purchasing" [in library, link, returns(resolve_groups)],
    Ties "tie.list" Read "List deliberate net ties" [long,link,since(8, 0),returns(ties)],
    Variants "variant.list" Read "List assembly variants" [link, since(7, 13), returns(variants)],
    Substitutes "variant.substitutes" Read "Parts that fit a footprint" [link, since(7, 13), returns(substitutes)],
    FootprintPreview "library.footprint_preview" Read "Preview a footprint" [in library, long, link, returns(footprint_preview)],
    UpdatesAvailable "library.updates_available" Read "List library updates" [in library, link, returns(updates_available)],
    PromotePreview "library.promote_preview" Read "Preview a promotion" [in library, link, returns(promote)],
    Issues "project.issues" Read "List issues" [in checks, long, link, returns(issues)],
    StandardFields "library.standard_fields" Read "List standard fields" [in library, link, returns(standard_fields)],
    Models "library.models" Read "Find 3D models" [in library, link, returns(models)],
    PinoutPreview "library.pinout_preview" Read "Preview a pinout import" [in library, long, link, returns(pinout_plan)],
    SuggestPins "functions.suggest" Read "Suggest pin assignments" [in schematic, link, returns(suggest_pins)],
    PinReport "project.pin_report" Read "Pin report" [in schematic, link, returns(pin_report)],
    CheckIntent "schematic.check_intent" Read "Check intended net connections" [in schematic, long, link, since(1, 10), returns(net_intent)],
    PartPins "schematic.pins" Read "List parts' pins" [in schematic, link, since(1, 7), returns(part_pins)],
    PinConfig "project.pin_config" Read "Firmware pin configuration" [in fab, link, returns(pin_config)],
    Marking "instance.marking" Read "Resolve an instance's marking" [in schematic, returns(marking)],
    RulesEffective "rules.effective" Read "Effective rules" [in rules, link, returns(rules)],
    RuleProfiles "rules.fab_profiles" Read "List fab profiles" [in rules, link, returns(rule_profiles, rule_profile)],
    PreviewProfile "rules.preview_fab_profile" Read "Preview a fab profile" [in rules, link, returns(profile_impact)],
    PreviewOptions "rules.preview_fab_options" Read "Preview fab profile options" [in rules, link, returns(profile_impact)],
    NetClasses "net_class.list" Read "List net classes" [in rules, link, returns(net_classes)],
    PreviewNetClassPatterns "net_class.preview_patterns" Read "Preview net class patterns" [in rules, link, returns(net_class_changes)],
    BoardShape "board.shape" Read "Board shape" [in layout, link, returns(board_shape)],
    FillStatus "fill.status" Read "Fill status" [in layout, link, returns(fill_status)],
    Sheets "schematic.sheets" Read "List sheets" [in schematic, link, returns(sheets)],
    NetPlaces "schematic.net_places" Read "Find a net on the sheets" [in schematic, link, returns(net_places)],
    SheetCopy "schematic.copy" Read "Copy schematic objects" [in schematic, link, returns(sheet_clip)],
    SheetScene "schematic.scene" Read "Sheet scene" [in schematic, link, returns(sheet_scene)],
    ImpedanceCalculate "impedance.calculate" Read "Calculate impedance" [in rules, returns(impedance)],
    ImpedanceTable "impedance.table" Read "Impedance table" [in rules, link, returns(impedance_table)],
    SuggestPairs "diff_pair.suggest" Read "Suggest differential pairs" [in layout, link, since(2, 9), returns(pair_suggestions)],
    LengthReport "layout.length_report" Read "Length report" [in layout, link, returns(length_report)],
    Pads "layout.pads" Read "List the pads to route between" [in layout, link, since(2, 1), returns(pads)],
    RoutePreview "layout.route_preview" Read "Preview a route" [in layout, link, returns(route_preview)],
    TrackDragPreview "layout.track_drag_preview" Read "Preview a track drag" [in layout, link, returns(route_preview)],
    TunePreview "layout.tune_preview" Read "Preview length tuning" [in layout, link, returns(tune_preview)],
    Preflight "fabrication.preflight" Read "Production check" [in checks, long, link, returns(preflight)],
    ProfileSuggestions "fabrication.profile_suggestions" Read "Suggest fab profiles" [in checks, long, link, since(1, 6), returns(profile_suggestions)],
    InterfaceList "interface.list" Read "List the board interface" [in model3d, returns(interface_list)],
    AssemblyStatus "assembly.status" Read "Check the assembly link" [in model3d, returns(assembly_status)],
});

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "report", content = "result", rename_all = "snake_case")]
pub enum QueryResult {
    Instance(Box<InstanceReport>),
    Definition(DefinitionReport),
    Drafts(Vec<DefinitionReport>),
    Search(catalog::SearchReport),
    Facets(catalog::FacetReport),
    Lineage(Box<catalog::ItemReport>),
    Bom(completeness::BomReport),
    ResolveGroups(completeness::ResolveReport),
    Ties(Vec<layout::connectivity::TieReport>),
    Variants(instances::variants::VariantsReport),
    Substitutes(Vec<instances::variants::Substitute>),
    FootprintPreview(Box<library::footprint_edit::FootprintPreview>),
    UpdatesAvailable(Vec<catalog::UpdateAvailable>),
    Promote(Box<library::import::PromotePlan>),
    /// `library.standard_fields`: a bundle for `library.import`.
    StandardFields(Box<library::Bundle>),
    Issues(issues::IssueReport),
    Models(catalog::models::ModelReport),
    PinoutPlan(Box<library::pinout::PinoutPlan>),
    SuggestPins(functions::suggest::SuggestReport),
    PinReport(Box<pins::PinReport>),
    PartPins(Vec<pins::PartPins>),
    NetIntent(Box<schematic::intent::Report>),
    PinConfig(Box<firmware::PinConfigReport>),
    Marking(Option<library::marking::MarkingLayout>),
    Rules(Box<rules::RulesReport>),
    RuleProfiles(Vec<rules::ProfileBrief>),
    RuleProfile(Box<rules::RuleProfile>),
    ProfileImpact(Box<rules::ProfileImpact>),
    NetClasses(Box<rules::netclass::NetClassReport>),
    NetClassChanges(Vec<rules::netclass::ClassChange>),
    BoardShape(Box<model::SolvedBoard>),
    FillStatus(Vec<layout::fill::FillStatus>),
    Sheets(Vec<schematic::SheetRow>),
    NetPlaces(Box<schematic::NetPlaces>),
    SheetClip(Box<schematic::SheetClip>),
    SheetScene(Box<schematic::scene::SheetScene>),
    Impedance(Box<layout::impedance::ImpedanceResult>),
    ImpedanceTable(Box<layout::impedance::ImpedanceTable>),
    PairSuggestions(Vec<rules::signal::PairSuggestion>),
    LengthReport(Box<layout::length::LengthReport>),
    Pads(Vec<layout::PadRow>),
    TunePreview(Box<layout::meander::TunePreview>),
    RoutePreview(Box<layout::router::RoutePlan>),
    Preflight(Box<fabrication::PreflightReport>),
    ProfileSuggestions(Box<fabrication::ProfileSuggestions>),
    InterfaceList(Box<interface::InterfaceList>),
    AssemblyStatus(Box<interface::assembly_link::AssemblyStatus>),
}

/// The host engines a command may need. The core never interprets a
/// sketch; these do, on the candidate only.
#[derive(Clone, Copy, Default)]
pub struct Evaluators<'a> {
    pub footprint: Option<&'a dyn library::FootprintEvaluator>,
    pub board: Option<&'a dyn layout::outline::BoardShapeEvaluator>,
}

/// Undo depth. Entries share every section an edit did not write
/// ([`Shared`]), so a step costs the sections it changed, not a copy of the
/// whole project.
pub const MAX_UNDO: usize = 100;
/// Operations [`Engine::apply_all_hosted`] takes in one step.
pub const MAX_BATCH: usize = 256;

/// The area of the design an undo step changed, as the workspaces name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    Component,
    Schematic,
    Pcb,
    /// The project itself: its name and properties.
    Project,
}

/// What one undo step did, for people: its operation's title, the area
/// it changed and, when it concerns exactly one part, that part's
/// designator as it was. A batch of one kind of operation takes that
/// operation's title; a mixed batch is [`Step::MIXED`], in its first
/// operation's area.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Step {
    pub title: &'static str,
    pub area: Area,
    /// The one part the step concerns, by designator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The name of the group the step gathers ([`Engine::begin_group`]):
    /// a turn's label, read instead of the title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The registry phrases a step's part replaces: "Move a placed part" on R1
/// reads "Move R1".
const PART_NOUNS: [&str; 4] = [
    "a placed part",
    "a part on the board",
    "a mechanical part",
    "a component",
];

impl Step {
    /// The title of a batch of different operations.
    pub const MIXED: &'static str = "Several changes";

    /// `operation`'s step on `project`, the document it applies to.
    fn of(operation: &Operation, project: &Project) -> Self {
        Self {
            title: operation.title(),
            area: operation.area(),
            subject: operation
                .instance()
                .and_then(|id| project.instance(id))
                .map(|i| i.designator.clone()),
            label: None,
        }
    }

    /// The step for people: a group's label, else its title, naming its
    /// part when it has one ("Move R1", "Set instance properties (R1)").
    pub fn line(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        let Some(name) = &self.subject else {
            return self.title.to_owned();
        };
        PART_NOUNS
            .iter()
            .find_map(|noun| self.title.strip_suffix(noun))
            .map_or_else(
                || format!("{} ({name})", self.title),
                |verb| format!("{verb}{name}"),
            )
    }
}

/// What a first save gave the document it wrote ([`first_save`]); only
/// that makes one. The engine takes it with [`Engine::adopt_first_save`]
/// once the file holds the document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FirstSave {
    name: Option<String>,
    id: Option<String>,
}

impl FirstSave {
    /// The project had its name and identity already.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.id.is_none()
    }

    /// `project` with what it lacks of this; `None` when it lacks nothing.
    fn given_to(&self, project: &Project) -> Option<Project> {
        let name = self.name.as_ref().filter(|_| project.name == UNTITLED);
        let id = self
            .id
            .as_ref()
            .filter(|_| project.production.project_id.is_none());
        if name.is_none() && id.is_none() {
            return None;
        }
        let mut project = project.clone();
        if let Some(name) = name {
            project.name.clone_from(name);
        }
        if let Some(id) = id {
            project.production.project_id = Some(id.clone());
        }
        Some(project)
    }
}

/// Every save to `file` starts here, on the copy to be written: a project
/// still called [`UNTITLED`] takes the file's name ("usb-led.sparksmith"
/// names it "usb-led"; a name someone chose is kept), then a project
/// without an identity takes one, [`model::production::project_id_for`] of
/// the file's name, the host's `nonce` and the named project.
///
/// `nonce` is the only thing a host decides: the desktop passes random
/// bytes, so every new project is its own whatever it holds; a scripted
/// host (a session) passes none, so the same script saved under the same
/// file name gives the same identity, in any folder.
pub fn first_save(document: &mut Document, file: &std::path::Path, nonce: &str) -> FirstSave {
    FirstSave {
        name: name_by_file(document, file),
        id: identify_by_file(&mut document.project, file, nonce),
    }
}

/// [`first_save`]'s name. A name the project cannot take, or that would
/// make it too large to save, is not given.
fn name_by_file(document: &mut Document, file: &std::path::Path) -> Option<String> {
    let stem = file.file_stem()?.to_str()?.trim();
    if document.project.name != UNTITLED || stem == UNTITLED {
        return None;
    }
    let mut named = document.clone();
    schematic::sheets::rename_project(Arc::make_mut(&mut named.project), stem).ok()?;
    format::room::check_file(&format::EditorFile::from_document(&named)).ok()?;
    *document = named;
    Some(document.project.name.clone())
}

/// [`first_save`]'s identity; `None` when the project had one.
fn identify_by_file(
    project: &mut Arc<Project>,
    file: &std::path::Path,
    nonce: &str,
) -> Option<String> {
    if project.production.project_id.is_some() {
        return None;
    }
    let bytes = serde_json::to_vec(project.as_ref()).ok()?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let id = model::production::project_id_for(&format!("{name}\0{nonce}"), &bytes);
    Arc::make_mut(project).production.project_id = Some(id.clone());
    Some(id)
}

/// The inputs of a check report ([`Engine::report_key`]): equal keys give
/// equal reports.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReportKey {
    revision: u64,
    /// The document's identity: a state can change without a revision (a
    /// first save's name and identity), never without a new allocation.
    project: usize,
    drafts: usize,
    checks: layout::CheckSettings,
}

/// The document, its revision and one shared undo history.
///
/// Every mutation is staged on a candidate and validated before it replaces
/// the document, so a failed operation leaves document, revision and history
/// unchanged.
pub struct Engine {
    document: Document,
    revision: u64,
    /// The documents before each step, with what the step did.
    undo: VecDeque<(Document, Step)>,
    /// The documents after each undone step, with what it did.
    redo: Vec<(Document, Step)>,
    /// The revision the latest undo step was applied at, until an undo,
    /// redo or new document ([`Engine::join_last_step`]).
    last_step: Option<u64>,
    /// Which state of the history this is: a fresh value from
    /// [`next_history`] whenever a step is made, undone, redone or joined,
    /// or the history is cleared. An undo group joins only the step it
    /// made itself, while this has not moved ([`GroupMark`]).
    history: u64,
    /// Which opening this is: a fresh value whenever a whole document comes
    /// in ([`Engine::opening`]).
    opening: u64,
    /// The undo group open now ([`Engine::begin_group`]).
    group: Option<Group>,
    /// `fabrication.waive` refuses a waiver without a reason (a host
    /// setting; off by default).
    waiver_reason_required: bool,
    /// The host's values for limits the project leaves open (Settings);
    /// none by default, and the checks say what is unset.
    check_settings: layout::CheckSettings,
    /// The copper graph of this revision, built on first use and dropped
    /// with every new revision ([`Engine::copper_graph`]).
    copper: std::sync::OnceLock<Arc<layout::CopperGraph>>,
    /// The last issue report made ([`Engine::issue_report`]), from which
    /// the next state's is made. Shared with this engine's forks, so a
    /// report a worker makes on a fork serves the engine it forked.
    checked: Arc<std::sync::Mutex<Option<Arc<issues::incremental::Checked>>>>,
    /// What the last applied operation re-routed of the copper its moved
    /// parts dragged ([`Engine::rerouted`]).
    rerouted: layout::router::reroute::Rerouted,
    /// The host's generator runtime (`library.generate`, GP1): a host
    /// setting like the waiver rule, kept across documents.
    generators: Option<Arc<dyn library::generated::GeneratorRuntime + Send + Sync>>,
    /// The host's standards and materials ([`Engine::set_context`]): a host
    /// setting like the generator runtime, kept across documents. Current
    /// on the calling thread while an operation applies or a query answers.
    context: Option<Arc<library::standards::Context>>,
    /// The measure that keeps every committed document one that saves
    /// (review finding 15): a commit that would not fit is refused.
    room: format::room::Room,
    sheet_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

/// A validated sheet placement from a worker. Only the engine can construct
/// it, and the receiving engine checks the original document and revision.
pub struct PreparedSheet {
    base: Document,
    revision: u64,
    next: Document,
    step: Step,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl Engine {
    /// Give this engine the host's generator runtime, which
    /// `library.generate` runs modules through (the core never interprets
    /// one). It stays across new and opened documents.
    pub fn set_generators(
        &mut self,
        runtime: Arc<dyn library::generated::GeneratorRuntime + Send + Sync>,
    ) {
        self.generators = Some(runtime);
    }

    /// Give this engine the host's context: the standards (and materials)
    /// it draws on. It stays across new and opened documents. Without one,
    /// whatever needs a standard is refused.
    pub fn set_context(&mut self, context: Arc<library::standards::Context>) {
        self.context = Some(context);
    }

    /// Drop the context, as a host that gave none: a test fixture, never in
    /// the product (test engines start with the fixture context).
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn clear_context(&mut self) {
        self.context = None;
    }

    /// The context the host gave ([`Self::set_context`]).
    pub fn context(&self) -> Option<&Arc<library::standards::Context>> {
        self.context.as_ref()
    }

    /// Make this engine's context current on this thread until the scope
    /// ends: what a host does to draw or build from this engine's document
    /// outside an operation or query.
    pub fn enter_context(&self) -> Option<library::standards::Scope> {
        self.context
            .as_ref()
            .map(library::standards::Context::enter)
    }

    /// The generator runtime the host injected. Lending sessions and engine
    /// forks use the same runtime, including its worker resource limits.
    pub fn generator_runtime(
        &self,
    ) -> Option<&Arc<dyn library::generated::GeneratorRuntime + Send + Sync>> {
        self.generators.as_ref()
    }

    /// What `library.replace_body` would do with `request`, changing
    /// nothing (`library.body_fit`).
    pub fn body_plan(
        &self,
        request: &commands::body::ReplaceBody,
    ) -> Result<commands::body::BodyPlan> {
        let _context = self.enter_context();
        let runtime = self
            .generators
            .as_deref()
            .map(|r| r as &dyn library::generated::GeneratorRuntime);
        commands::body::plan(&self.document, request, runtime)
    }

    /// Whether this engine has a generator runtime.
    pub fn has_generators(&self) -> bool {
        self.generators.is_some()
    }

    /// What the last operation applied re-routed of the copper its moved
    /// parts dragged along, and what it left unrouted: empty when it moved
    /// no part, or the carried copper stayed clean.
    pub fn rerouted(&self) -> &layout::router::reroute::Rerouted {
        &self.rerouted
    }

    /// Whether `fabrication.waive` needs a reason in this engine. The app
    /// sets it from its settings; headless hosts keep the default (off).
    pub fn require_waiver_reason(&mut self, required: bool) {
        self.waiver_reason_required = required;
    }

    /// Whether `fabrication.waive` needs a reason in this engine.
    pub fn waiver_reason_required(&self) -> bool {
        self.waiver_reason_required
    }

    /// Set the values the checks take from the host's Settings, such as
    /// the impedance tolerance of a net class that states none. Every
    /// host (app, CLI, MCP) sets them from the same settings file; a host
    /// that has none leaves them unset and the checks report what is
    /// missing. A value out of bounds is refused.
    pub fn set_check_settings(&mut self, settings: layout::CheckSettings) -> Result<()> {
        settings.check()?;
        self.check_settings = settings;
        Ok(())
    }

    /// The check settings the host set ([`Self::set_check_settings`]).
    pub fn check_settings(&self) -> &layout::CheckSettings {
        &self.check_settings
    }

    /// What a check report of this engine is computed from: the document
    /// state and the host's check settings. A view that caches findings
    /// (Issues, the inspector's slot, the production check) keys them on
    /// this, so a settings change (the impedance tolerance) refreshes every
    /// view together, though it takes no revision.
    pub fn report_key(&self) -> ReportKey {
        ReportKey {
            revision: self.revision,
            project: Arc::as_ptr(&self.document.project) as usize,
            drafts: Arc::as_ptr(&self.document.drafts) as usize,
            checks: self.check_settings,
        }
    }

    fn with_document(document: Document) -> Self {
        Self {
            document,
            revision: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            last_step: None,
            history: next_history(),
            opening: next_history(),
            group: None,
            waiver_reason_required: false,
            check_settings: layout::CheckSettings::default(),
            copper: std::sync::OnceLock::new(),
            checked: Arc::default(),
            rerouted: layout::router::reroute::Rerouted::default(),
            generators: None,
            context: Self::fixture_context(),
            room: format::room::Room::default(),
            sheet_cancel: None,
        }
    }

    /// Test builds give every engine the fixture standards, as they give
    /// it fixture stackups; the product's host sets its own.
    #[cfg(any(test, feature = "test-fixtures"))]
    fn fixture_context() -> Option<Arc<library::standards::Context>> {
        library::standards::Context::fixture().ok()
    }

    #[cfg(not(any(test, feature = "test-fixtures")))]
    fn fixture_context() -> Option<Arc<library::standards::Context>> {
        None
    }

    /// An engine without a board (no layers, no size, which every check
    /// refuses): only for a host that lends it other engines' documents
    /// (a session forwarding to an open window) and never edits its own.
    pub fn placeholder() -> Self {
        Self::with_document(Document::empty_for_file())
    }

    /// An engine on a new, empty project on `board` (the shipped new-board
    /// data `sparksmith-io` loads), with empty history.
    pub fn new(board: &NewBoard) -> Self {
        Self::with_document(Document {
            project: Arc::new(Project::new(board)),
            drafts: Arc::default(),
        })
    }

    /// [`Self::new`] on `profile`: its rules, the stackup's outer foils
    /// where it orders them, and nothing to undo, so no undo leaves the
    /// board without a profile.
    pub fn new_on(board: &NewBoard, profile: rules::RuleProfile) -> Result<Self> {
        let mut project = Project::new(board);
        rules::apply(
            &mut project.rules,
            rules::RulesCommand::set_profile(profile, false),
        )?;
        layout::restack::reconcile_rules(&mut project, layout::restack::Foils::Follow)?;
        Ok(Self::with_document(Document {
            project: Arc::new(project),
            drafts: Arc::default(),
        }))
    }

    /// An engine for an already validated file, with empty history. A file
    /// never carries Dk estimates: its stackup takes `dk_estimates` (the
    /// shipped stackup data, [`Stackup::dk_estimates`]).
    pub fn from_file_with(file: format::EditorFile, dk_estimates: &GlassDk) -> Result<Self> {
        let mut engine = Self::with_document(Document::empty_for_file());
        Arc::make_mut(&mut engine.document.project)
            .stackup
            .dk_estimates = Unsaved(dk_estimates.clone());
        engine.replace_document(file)?;
        Ok(engine)
    }

    /// Give the document's stackup the Dk estimates `dk_estimates` when it
    /// has none (a file opened into an engine that had none to keep).
    /// They are never saved and take no part in revisions or undo.
    pub fn supply_dk_estimates(&mut self, dk_estimates: &GlassDk) {
        if self.document.project.stackup.dk_estimates.0.0.is_empty() {
            Arc::make_mut(&mut self.document.project)
                .stackup
                .dk_estimates = Unsaved(dk_estimates.clone());
        }
    }

    /// A test fixture, never in the product: [`Engine::from_file_with`] the
    /// fixture stackup's Dk estimates.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn from_file(file: format::EditorFile) -> Result<Self> {
        Self::from_file_with(file, &StackupPresets::fixture()?.glass_dk)
    }

    /// The PCB realism showcase: a 4-layer board with an SOIC-8, a QFN-16,
    /// a 2x5 header, 0603/0805 passives and a polarised part, routed on top
    /// and bottom through tented and open vias, built through the public
    /// commands. The parts are shipped data the host's `context` carries
    /// (`library::showcase_parts`).
    ///
    /// Unconnected terminals are marked no-connect. The board's rules are
    /// `profile`. The undo history is cleared.
    /// Its stackups are `presets`' (the shipped stackup data).
    /// It draws on the host's `context`, which it keeps.
    pub fn showcase_with(
        context: Arc<library::standards::Context>,
        presets: &StackupPresets,
        profile: rules::RuleProfile,
    ) -> Result<Self> {
        Self::showcase_in(context, None, presets, profile)
    }

    /// A test fixture, never in the product: [`Engine::showcase_with`] the
    /// test prototype profile, whose values the showcase was drawn for.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn showcase() -> Result<Self> {
        Self::showcase_in(
            library::standards::Context::fixture()?,
            None,
            &StackupPresets::fixture()?,
            rules::fixtures::prototype()?,
        )
    }

    /// The showcase as the shipped file shows it: [`Engine::showcase`]
    /// with a Power net class over GND and VCC and a top GND pour; with a
    /// board evaluator, also a sketched board with rounded corners, a
    /// round cutout, a milled slot and J1 held to the left edge by board
    /// parameters (`layout::showcase::board_edits`). The pours are filled
    /// last, over the final shape. The undo history is cleared.
    pub fn showcase_hosted(
        context: Arc<library::standards::Context>,
        evaluators: Evaluators<'_>,
        presets: &StackupPresets,
        profile: rules::RuleProfile,
    ) -> Result<Self> {
        Self::showcase_in(context, Some(evaluators), presets, profile)
    }

    /// A test fixture, never in the product: [`Engine::showcase_hosted`]
    /// on the test prototype profile.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn showcase_hosted_fixture(evaluators: Evaluators<'_>) -> Result<Self> {
        Self::showcase_in(
            library::standards::Context::fixture()?,
            Some(evaluators),
            &StackupPresets::fixture()?,
            rules::fixtures::prototype()?,
        )
    }

    fn showcase_in(
        context: Arc<library::standards::Context>,
        hosted: Option<Evaluators<'_>>,
        presets: &StackupPresets,
        profile: rules::RuleProfile,
    ) -> Result<Self> {
        let _scope = context.enter();
        let mut engine = Self::new(&NewBoard {
            size: layout::showcase::BOARD_MM,
            stackup: presets.preset(MIN_COPPER_LAYERS)?,
        });
        engine.set_context(Arc::clone(&context));
        engine.apply(
            engine.revision,
            Operation::Rules(rules::RulesCommand::set_profile(profile, false)),
        )?;
        engine.apply(
            engine.revision,
            Operation::Schematic(schematic::SchematicCommand::RenameProject {
                name: layout::showcase::PROJECT_NAME.into(),
            }),
        )?;
        let mut refs = Vec::new();
        let parts = context.showcase()?.parts.clone();
        for part in &parts {
            let lineages = engine.reserve_lineages()?;
            let edits = library::showcase_parts::remap(&part.edits, lineages)?;
            engine.apply(
                engine.revision,
                Operation::Library(LibraryCommand::Edit { edits }),
            )?;
            let mut drafts = vec![
                lineages.component,
                lineages.symbol.id,
                lineages.package.id,
                lineages.footprint.id,
                lineages.binding.id,
            ];
            if let Some(model) = &part.model {
                let hash = model.definition.asset.hash;
                let bytes = context.bundled_part(hash).ok_or_else(|| {
                    Error::NotFound(format!(
                        "showcase parts: no shipped generator bundle has the part file sha256 {hash} of {}",
                        part.name
                    ))
                })?;
                let outcome = engine.apply(
                    engine.revision,
                    Operation::Library(LibraryCommand::ImportModel(Box::new(
                        library::model3d::ImportModel {
                            name: model.name.clone(),
                            assets: vec![library::PackedAsset {
                                hash,
                                media_type: model.definition.asset.media_type.clone(),
                                data: library::bundle::AssetBytes::new(bytes.to_vec()),
                            }],
                            definition: Box::new(model.definition.clone()),
                            assign: Some(library::model3d::AssignModel {
                                package: lineages.package.id,
                                lead_map: Vec::new(),
                                part: Some(library::model3d::AssignPart {
                                    values: model.values.clone(),
                                    derived: model.derived.clone(),
                                }),
                            }),
                        },
                    ))),
                )?;
                if let Outcome::DraftCreated { id } = outcome {
                    drafts.push(id);
                }
            }
            engine.apply(
                engine.revision,
                Operation::Library(LibraryCommand::Publish {
                    drafts,
                    close: false,
                }),
            )?;
            refs.push(lineages);
        }
        for (k, (designator, part)) in layout::showcase::INSTANCES.iter().enumerate() {
            let lineages = refs
                .get(*part)
                .ok_or_else(|| Error::Invalid("showcase part index".into()))?;
            let column = (k % 3) as f64;
            let row = (k / 3) as f64;
            engine.apply(
                engine.revision,
                Operation::Instance(InstanceCommand::Place(instances::NewInstance {
                    sheet: None,
                    component: ComponentRef::new(lineages.component, Revision::FIRST),
                    designator: Some((*designator).into()),
                    properties: vec![],
                    representation: Some(lineages.symbol.id),
                    unit: Some(UnitId(1)),
                    position: Some(units::PointNm::from_mm_f64_rounded([
                        column * 40.0,
                        -row * 40.0,
                    ])?),
                    physical: Some(PhysicalChoice {
                        binding: lineages.binding.id,
                        footprint: lineages.footprint.id,
                    }),
                })),
            )?;
        }
        for (name, terminals) in layout::showcase::NETS {
            let mut ends = Vec::with_capacity(terminals.len());
            for (designator, terminal) in *terminals {
                ends.push(layout::showcase::endpoint(
                    engine.project(),
                    designator,
                    *terminal,
                )?);
            }
            // Wire each terminal to the next; where no wire can be drawn
            // without running over another pin, a pair of labels of the
            // net's name, each on its pin, joins them instead.
            let mut labelled = std::collections::BTreeSet::new();
            for pair in ends.windows(2) {
                let wired = engine.apply(
                    engine.revision,
                    Operation::Schematic(schematic::SchematicCommand::Connect {
                        from: pair[0].into(),
                        to: pair[1].into(),
                    }),
                );
                match wired {
                    Ok(_) => {}
                    Err(Error::Conflict(_)) => {
                        for end in [pair[0], pair[1]] {
                            if !labelled.insert(end) {
                                continue;
                            }
                            let position = schematic::pin_position(engine.project(), end)
                                .ok_or_else(|| Error::Invalid("showcase pin".into()))?;
                            engine.apply(
                                engine.revision,
                                Operation::Schematic(schematic::SchematicCommand::PlaceLabel {
                                    sheet: None,
                                    name: (*name).into(),
                                    kind: model::LabelKind::Local,
                                    position: units::PointNm::from_mm_f64_rounded(position)?,
                                    rotation: Default::default(),
                                    attach: Some(end.into()),
                                }),
                            )?;
                        }
                    }
                    Err(other) => return Err(other),
                }
            }
            let net = ends
                .first()
                .and_then(|end| engine.project().net_of(*end))
                .map(|net| net.id)
                .ok_or_else(|| Error::Invalid("showcase net".into()))?;
            engine.apply(
                engine.revision,
                Operation::Schematic(schematic::SchematicCommand::RenameNet {
                    net,
                    name: (*name).into(),
                }),
            )?;
        }
        let mut unconnected: std::collections::BTreeMap<Id, Vec<TerminalId>> = Default::default();
        for net in &engine.project().nets {
            if let [only] = net.terminals[..] {
                unconnected
                    .entry(only.instance)
                    .or_default()
                    .push(only.terminal);
            }
        }
        for (instance, terminals) in unconnected {
            engine.apply(
                engine.revision,
                Operation::Instance(InstanceCommand::SetNoConnect {
                    instance,
                    terminals,
                    no_connect: true,
                }),
            )?;
        }
        for command in layout::showcase::commands(engine.project(), presets)? {
            engine.apply(engine.revision, Operation::Layout(command))?;
        }
        for command in layout::showcase::fill_commands(engine.project())? {
            engine.apply(engine.revision, Operation::Fill(command))?;
        }
        let evaluators = hosted.unwrap_or_default();
        if hosted.is_some() {
            engine.showcase_design(evaluators)?;
        }
        // The pours' copper, where this build computes fills; a build
        // without fill computation keeps them unfilled (stale).
        match engine.apply_hosted(
            engine.revision,
            Operation::Fill(layout::fill::FillCommand::Refill {
                target: layout::fill::RefillTarget::All,
            }),
            evaluators,
        ) {
            Ok(_) | Err(Error::Unsupported(_)) => {}
            Err(e) => return Err(e),
        }
        engine.undo.clear();
        engine.redo.clear();
        Ok(engine)
    }

    /// The hosted showcase's rules and shape over the plain showcase; see
    /// [`Engine::showcase_hosted`].
    fn showcase_design(&mut self, evaluators: Evaluators<'_>) -> Result<()> {
        let class = match self.apply(
            self.revision,
            Operation::Rules(rules::RulesCommand::AddNetClass {
                class: layout::showcase::power_class(),
            }),
        )? {
            Outcome::NetClassAdded { class } => class,
            _ => return Err(Error::Invalid("showcase net class".into())),
        };
        let nets: Vec<Id> = self
            .project()
            .nets
            .iter()
            .filter(|n| layout::showcase::POWER_NETS.contains(&n.name.as_str()))
            .map(|n| n.id)
            .collect();
        self.apply(
            self.revision,
            Operation::Rules(rules::RulesCommand::AssignNets {
                nets,
                class: Some(class),
            }),
        )?;
        for command in layout::showcase::top_fill_commands(self.project())? {
            self.apply(self.revision, Operation::Fill(command))?;
        }
        if evaluators.board.is_none() {
            return Ok(());
        }
        self.apply_hosted(
            self.revision,
            Operation::Board(layout::outline::BoardCommand::StartSketch),
            evaluators,
        )?;
        for edits in layout::showcase::board_edits() {
            self.apply_hosted(
                self.revision,
                Operation::Board(layout::outline::BoardCommand::Edit { edits }),
                evaluators,
            )?;
        }
        for dimension in layout::showcase::j1_dimensions(self.project())? {
            self.apply_hosted(
                self.revision,
                Operation::Dimension(layout::dimensions::DimensionCommand::Add { dimension }),
                evaluators,
            )?;
        }
        Ok(())
    }

    /// A test fixture, never in the product: an empty project on the test
    /// prototype profile.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn profiled() -> Result<Self> {
        let mut engine = Self::default();
        std::sync::Arc::make_mut(&mut engine.document.project).rules = rules::fixtures::rules()?;
        Ok(engine)
    }

    /// A test fixture, never in the product: two connected two-terminal
    /// components, placed on the board, with a typed resistance field
    /// shown on the symbol, on the test prototype profile. The sample
    /// library is built through the public commands and its drafts stay
    /// open for editing. Other crates' tests reach it through the
    /// `test-fixtures` feature of their dev-dependency; a launch and File ›
    /// New open an empty project.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn demo() -> Result<Self> {
        let mut engine = Self::default();
        std::sync::Arc::make_mut(&mut engine.document.project).rules = rules::fixtures::rules()?;
        let refs = engine.reserve_lineages()?;
        let kind = engine.reserve(DefinitionKind::QuantityKind)?;
        let field = engine.reserve(DefinitionKind::Field)?;
        let mut edits =
            library::fixtures::two_terminal("Resistor", library::fixtures::Kind::RESISTOR, refs)?;
        for edit in &mut edits {
            if let Definition::Component(component) = &mut edit.definition {
                component.fields.push(fields::FieldSelection {
                    field: FieldRef::new(field, Revision::FIRST),
                    default: None,
                    show_on_symbol: true,
                });
            }
        }
        edits.extend(library::fixtures::resistance_field(
            QuantityKindRef::new(kind, Revision::FIRST),
            field,
        ));
        engine.apply(
            engine.revision,
            Operation::Library(LibraryCommand::Edit { edits }),
        )?;
        engine.apply(
            engine.revision,
            Operation::Library(LibraryCommand::Publish {
                drafts: vec![
                    refs.component,
                    refs.symbol.id,
                    refs.package.id,
                    refs.footprint.id,
                    refs.binding.id,
                    kind,
                    field,
                ],
                close: false,
            }),
        )?;
        let mut placed = Vec::new();
        for position in [[-12.0, 0.0], [12.0, 0.0]] {
            let position = units::PointNm::from_mm_f64_rounded(position)?;
            let outcome = engine.apply(
                engine.revision,
                Operation::Instance(InstanceCommand::Place(instances::NewInstance {
                    sheet: None,
                    component: ComponentRef::new(refs.component, Revision::FIRST),
                    designator: None,
                    properties: vec![fields::PropertyEdit {
                        field,
                        input: fields::PropertyInput::Text("10k".into()),
                    }],
                    representation: Some(refs.symbol.id),
                    unit: Some(UnitId(1)),
                    position: Some(position),
                    physical: Some(PhysicalChoice {
                        binding: refs.binding.id,
                        footprint: refs.footprint.id,
                    }),
                })),
            )?;
            if let Outcome::InstancePlaced { instance, .. } = outcome {
                placed.push(instance);
            }
        }
        let [first, second] = placed[..] else {
            return Err(Error::Invalid("demo placement failed".into()));
        };
        engine.apply(
            engine.revision,
            Operation::Schematic(schematic::SchematicCommand::Connect {
                from: Endpoint {
                    instance: first,
                    terminal: TerminalId(2),
                }
                .into(),
                to: Endpoint {
                    instance: second,
                    terminal: TerminalId(1),
                }
                .into(),
            }),
        )?;
        // On the board, where the tests that build on the demo expect them
        // (Update PCB would stage them beside it).
        for (instance, position) in [(first, [8.0, 8.0]), (second, [16.0, 8.0])] {
            engine.apply(
                engine.revision,
                Operation::Layout(layout::LayoutCommand::Place {
                    instance,
                    position,
                    rotation_deg: 0.0,
                    layer: Layer::Top,
                }),
            )?;
        }
        engine.undo.clear();
        engine.redo.clear();
        Ok(engine)
    }

    /// Reserve one empty draft lineage of `kind`.
    fn reserve(&mut self, kind: DefinitionKind) -> Result<DefinitionId> {
        match self.apply(
            self.revision,
            Operation::Library(LibraryCommand::NewDraft {
                meta: DefinitionMeta::default(),
                definition: Definition::empty(kind),
            }),
        )? {
            Outcome::DraftCreated { id } => Ok(id),
            _ => Err(Error::Invalid("draft reservation failed".into())),
        }
    }

    /// Reserve one empty draft lineage of each kind and return the pending
    /// first revisions they would pin of each other.
    fn reserve_lineages(&mut self) -> Result<SampleRefs> {
        let mut ids = Vec::new();
        for kind in [
            DefinitionKind::Component,
            DefinitionKind::Symbol,
            DefinitionKind::Package,
            DefinitionKind::Footprint,
            DefinitionKind::Binding,
        ] {
            ids.push(self.reserve(kind)?);
        }
        let [component, symbol, package, footprint, binding] = ids[..] else {
            return Err(Error::Invalid("draft reservation failed".into()));
        };
        Ok(SampleRefs {
            component,
            symbol: SymbolRef::new(symbol, Revision::FIRST),
            package: PackageRef::new(package, Revision::FIRST),
            footprint: FootprintRef::new(footprint, Revision::FIRST),
            binding: BindingRef::new(binding, Revision::FIRST),
        })
    }

    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn project(&self) -> &Project {
        &self.document.project
    }
    /// Shared handle; equal pointers mean an unchanged project.
    pub fn project_arc(&self) -> &Arc<Project> {
        &self.document.project
    }
    pub fn drafts(&self) -> &Drafts {
        &self.document.drafts
    }
    pub fn to_file(&self) -> format::EditorFile {
        format::EditorFile::from_document(&self.document)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    /// What [`Engine::undo`] would take back, if anything.
    pub fn undo_step(&self) -> Option<Step> {
        self.undo.back().map(|(_, step)| step.clone())
    }
    /// What [`Engine::redo`] would apply again, if anything.
    pub fn redo_step(&self) -> Option<Step> {
        self.redo.last().map(|(_, step)| step.clone())
    }

    /// Replace the whole document (open, new, recover). History is cleared;
    /// an invalid file leaves everything unchanged.
    pub fn replace_document(&mut self, file: format::EditorFile) -> Result<()> {
        file.validate()?;
        let revision = self.next_revision()?;
        let estimates = self.document.project.stackup.dk_estimates.clone();
        let mut document = file.into_document();
        if document.project.stackup.dk_estimates.0.0.is_empty() {
            Arc::make_mut(&mut document.project).stackup.dk_estimates = estimates;
        }
        // A document that comes in whole (an import, a recovery) is one
        // that saves, as every edit of it must be.
        let mut room = self.room.clone();
        room.forget();
        room.admit(&document)?;
        self.room = room;
        self.document = document;
        self.set_revision(revision);
        self.undo.clear();
        self.redo.clear();
        self.last_step = None;
        self.history = next_history();
        self.opening = next_history();
        Ok(())
    }

    /// Forget the undo and redo steps: what a host built step by step (an
    /// import) is a document to start from, as an opened file is.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.last_step = None;
    }

    /// Which opened document this is, unique in the process: the same
    /// through every edit, undo and redo, another one once another document
    /// is opened, started or recovered ([`Self::replace_document`]). What a
    /// host keeps or computes for a document and may finish later (a draft,
    /// a background job) holds the opening it was started for, so it never
    /// reaches another document.
    pub fn opening(&self) -> u64 {
        self.opening
    }

    /// Apply one operation at an expected revision as one undo step. A
    /// failure leaves document, revision and history unchanged.
    pub fn apply(&mut self, expected_revision: u64, operation: Operation) -> Result<Outcome> {
        self.apply_inner(expected_revision, operation, Evaluators::default())
    }

    /// [`apply`](Self::apply) with the host's footprint evaluator, which
    /// footprint-source edits need (the core never interprets a sketch).
    /// The evaluator runs on the candidate only; undo and redo restore
    /// snapshots and never call it.
    pub fn apply_with(
        &mut self,
        expected_revision: u64,
        operation: Operation,
        evaluator: &dyn library::FootprintEvaluator,
    ) -> Result<Outcome> {
        self.apply_inner(
            expected_revision,
            operation,
            Evaluators {
                footprint: Some(evaluator),
                board: None,
            },
        )
    }

    /// [`apply`](Self::apply) with every host evaluator given.
    pub fn apply_hosted(
        &mut self,
        expected_revision: u64,
        operation: Operation,
        evaluators: Evaluators<'_>,
    ) -> Result<Outcome> {
        self.apply_inner(expected_revision, operation, evaluators)
    }

    /// Apply `operations` in order at an expected revision as one undo step
    /// and one revision, each checked as [`apply`](Self::apply) checks it
    /// against the document the ones before it made. Any failure leaves
    /// document, revision and history unchanged, and names the operation
    /// that failed. At most [`MAX_BATCH`] operations, and at least one.
    pub fn apply_all_hosted(
        &mut self,
        expected_revision: u64,
        operations: Vec<Operation>,
        evaluators: Evaluators<'_>,
    ) -> Result<Vec<Outcome>> {
        self.apply_all_resolved(expected_revision, operations, evaluators, |_, _, o| Ok(o))
    }

    /// [`apply_all_hosted`](Self::apply_all_hosted) with each operation read
    /// by the host first: `resolve` takes its place in the batch (from 0)
    /// and the project the operations before it made, so what it looks up
    /// there (a fab profile id, a sheet's page) is what the same operation
    /// sent alone would find. Its refusal leaves everything unchanged.
    pub fn apply_all_resolved<E: From<Error>>(
        &mut self,
        expected_revision: u64,
        operations: Vec<Operation>,
        evaluators: Evaluators<'_>,
        resolve: impl Fn(usize, &Project, Operation) -> std::result::Result<Operation, E>,
    ) -> std::result::Result<Vec<Outcome>, E> {
        self.check_revision(expected_revision)?;
        self.rerouted = Default::default();
        let count = operations.len();
        if count == 0 {
            return Err(Error::Invalid("a batch needs at least one operation".into()).into());
        }
        if count > MAX_BATCH {
            return Err(Error::Limit(format!(
                "a batch takes at most {MAX_BATCH} operations, not {count}"
            ))
            .into());
        }
        // One kind of operation names the step; a mix is several changes.
        let project = &self.document.project;
        let mut step = operations
            .first()
            .map(|o| Step::of(o, project))
            .unwrap_or(Step {
                title: Step::MIXED,
                area: Area::Schematic,
                subject: None,
                label: None,
            });
        if operations.iter().any(|o| o.title() != step.title) {
            step.title = Step::MIXED;
        }
        if operations
            .iter()
            .any(|o| Step::of(o, project).subject != step.subject)
        {
            step.subject = None;
        }
        // The steps run on a copy without history; only the end commits.
        let mut scratch = Engine {
            document: self.document.clone(),
            revision: self.revision,
            waiver_reason_required: self.waiver_reason_required,
            check_settings: self.check_settings,
            copper: self.copper.clone(),
            checked: self.checked.clone(),
            generators: self.generators.clone(),
            context: self.context.clone(),
            // Only the batch's end is committed here, and measured then.
            room: format::room::Room::unchecked(),
            ..Engine::with_document(self.document.clone())
        };
        let mut outcomes = Vec::with_capacity(count);
        let mut rerouted = layout::router::reroute::Rerouted::default();
        for (index, operation) in operations.into_iter().enumerate() {
            let operation = resolve(index, scratch.project(), operation)?;
            let at = scratch.revision;
            let outcome = scratch
                .apply_inner(at, operation, evaluators)
                .map_err(|error| error.in_batch(index, count))?;
            rerouted.extend(std::mem::take(&mut scratch.rerouted));
            outcomes.push(outcome);
        }
        if scratch.revision != self.revision {
            self.commit(scratch.document, step)?;
        }
        self.rerouted = rerouted;
        Ok(outcomes)
    }

    /// The preview of an assembly update from `read`, applied on a copy of
    /// this engine with the linked design winning every item both changed:
    /// the rows that would change, and, when both files changed since they
    /// last agreed, both sides' changes and the items to choose
    /// (`interface::assembly_link`). Changes nothing here.
    pub fn assembly_preview(
        &self,
        read: interface::assembly_link::AssemblyRead,
        evaluators: Evaluators<'_>,
    ) -> interface::assembly_link::AssemblyPreview {
        use interface::assembly_link as link;
        let project = self.project();
        let mut fork = self.fork();
        let revision = fork.revision();
        let hash = read.file_sha256;
        let conflicts = link::conflicts(project, &read);
        let resolve = link::linked_design_wins(&conflicts);
        match fork.apply_hosted(
            revision,
            Operation::Assembly(link::AssemblyCommand::RecordUpdate {
                read: read.clone(),
                resolve,
            }),
            evaluators,
        ) {
            Ok(_) => link::with_both_sides(
                project,
                &read,
                conflicts,
                link::diff(project, fork.project(), revision, hash),
            ),
            Err(error) => link::refused(project, revision, hash, &error),
        }
    }

    /// A copy of this engine, history included, to try requests on without
    /// touching this one (an MCP `dry_run`). Documents share their sections,
    /// so the copy costs little.
    pub fn fork(&self) -> Engine {
        Engine {
            document: self.document.clone(),
            revision: self.revision,
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            last_step: self.last_step,
            history: self.history,
            opening: self.opening,
            group: None,
            waiver_reason_required: self.waiver_reason_required,
            check_settings: self.check_settings,
            copper: self.copper.clone(),
            checked: self.checked.clone(),
            rerouted: self.rerouted.clone(),
            generators: self.generators.clone(),
            context: self.context.clone(),
            room: self.room.clone(),
            sheet_cancel: self.sheet_cancel.clone(),
        }
    }

    /// On a worker's fork, run the same typed command as any other caller.
    pub fn prepare_sheet(
        mut self,
        command: schematic::SchematicCommand,
        cancel: Arc<std::sync::atomic::AtomicBool>,
        evaluators: Evaluators<'_>,
    ) -> Result<PreparedSheet> {
        if !matches!(
            command,
            schematic::SchematicCommand::PlaceSheetSymbol { .. }
                | schematic::SchematicCommand::SetSheetChild { .. }
        ) {
            return Err(Error::Invalid("Select a child sheet to place.".into()));
        }
        let base = self.document.clone();
        let revision = self.revision;
        let operation = Operation::Schematic(command);
        let step = Step::of(&operation, &base.project);
        self.sheet_cancel = Some(cancel.clone());
        self.apply_hosted(revision, operation, evaluators)?;
        Ok(PreparedSheet {
            base,
            revision,
            next: self.document,
            step,
            cancel,
        })
    }

    pub fn commit_sheet(&mut self, prepared: PreparedSheet) -> Result<()> {
        self.check_revision(prepared.revision)?;
        if prepared.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::Conflict("Sheet expansion was cancelled.".into()));
        }
        if !Arc::ptr_eq(&self.document.project, &prepared.base.project)
            || !Arc::ptr_eq(&self.document.drafts, &prepared.base.drafts)
        {
            return Err(Error::Conflict(
                "The design changed while expanding the sheet.".into(),
            ));
        }
        self.commit(prepared.next, prepared.step)
    }

    fn apply_inner(
        &mut self,
        expected_revision: u64,
        operation: Operation,
        evaluators: Evaluators<'_>,
    ) -> Result<Outcome> {
        let _context = self.enter_context();
        let evaluator = evaluators.footprint;
        self.check_revision(expected_revision)?;
        if operation.needs_profile() {
            self.document.project.rules.require()?;
        }
        self.rerouted = Default::default();
        let step = Step::of(&operation, &self.document.project);
        let mut rerouted = layout::router::reroute::Rerouted::default();
        let mut candidate = self.document.clone();
        let reviewed = matches!(operation, Operation::Update(_));
        let merge_named = matches!(&operation, Operation::Update(u) if u.merge_named_nets);
        // Only the assembly link's own commands move the board-sketch
        // geometry its references own.
        let assembly_command = matches!(operation, Operation::Assembly(_));
        let sheet_move = matches!(&operation, Operation::Schematic(c) if c.is_move());
        let preserve_copper = matches!(
            operation,
            Operation::Board(_)
                | Operation::Dimension(_)
                | Operation::Layout(layout::LayoutCommand::SetBoardSize { .. })
        );
        // Recording a publish is no undo step, like a save: the files it
        // records stay written (docs/architecture/board-interface.md).
        let unrecorded = matches!(
            operation,
            Operation::Interface(interface::InterfaceCommand::RecordPublish { .. })
        );
        // Parts the operation moves by hand: their attached copper is
        // carried with them, all at once.
        // Selected loose copper a group drag slides with its parts; the
        // carry moves it and re-routes it with the rest.
        let loose = match &operation {
            Operation::Layout(layout::LayoutCommand::MoveParts {
                copper: Some(copper),
                ..
            }) => Some(copper.clone()),
            _ => None,
        };
        let moved_parts: Vec<Id> = match &operation {
            Operation::Layout(layout::LayoutCommand::Move { instance, .. }) => vec![*instance],
            Operation::Layout(layout::LayoutCommand::MoveParts { moves, .. }) => {
                moves.iter().map(|m| m.instance).collect()
            }
            Operation::Layout(
                layout::LayoutCommand::Align { instances, .. }
                | layout::LayoutCommand::Distribute { instances, .. },
            ) => instances.clone(),
            _ => Vec::new(),
        };
        let outcome = match operation {
            Operation::Library(commands::LibraryCommand::Generate(request)) => {
                let runtime = self
                    .generators
                    .as_deref()
                    .map(|r| r as &dyn library::generated::GeneratorRuntime);
                commands::generated::generate(&mut candidate, *request, runtime)?
            }
            Operation::Library(commands::LibraryCommand::PackageFromGenerator(request)) => {
                let runtime = self
                    .generators
                    .as_deref()
                    .map(|r| r as &dyn library::generated::GeneratorRuntime);
                commands::generated::package_from_generator(&mut candidate, *request, runtime)?
            }
            Operation::Library(commands::LibraryCommand::ReplaceBody(request)) => {
                let runtime = self
                    .generators
                    .as_deref()
                    .map(|r| r as &dyn library::generated::GeneratorRuntime);
                commands::body::replace_body(&mut candidate, *request, runtime, evaluator)?
            }
            Operation::Library(command) => {
                commands::apply_library(&mut candidate, command, evaluator)?
            }
            Operation::Instance(command) => {
                if command.restates(&candidate.project)? {
                    Outcome::None
                } else {
                    commands::apply_instance(Arc::make_mut(&mut candidate.project), command)?
                }
            }
            // The project's own name again: no change and no undo step.
            Operation::Schematic(schematic::SchematicCommand::RenameProject { name })
                if name.trim() == candidate.project.name =>
            {
                Outcome::None
            }
            Operation::Schematic(command) => {
                match schematic::apply(
                    Arc::make_mut(&mut candidate.project),
                    command,
                    EditPolicy::Direct,
                )? {
                    schematic::Applied::None => Outcome::None,
                    schematic::Applied::Unit(occurrence) => Outcome::UnitPlaced { occurrence },
                    schematic::Applied::Label(label) => Outcome::LabelPlaced { label },
                    schematic::Applied::Junction(junction) => Outcome::JunctionPlaced { junction },
                    schematic::Applied::Sheet(sheet) => Outcome::SheetAdded { sheet },
                    schematic::Applied::SheetSymbol { symbol, child } => {
                        Outcome::SheetSymbolPlaced { symbol, child }
                    }
                    schematic::Applied::SheetPins(pins) => Outcome::SheetPinsAdded { pins },
                    schematic::Applied::Drawing(drawing) => Outcome::DrawingAdded { drawing },
                    schematic::Applied::Annotated(changes) => Outcome::Annotated { changes },
                    schematic::Applied::Pasted(report) => Outcome::Pasted { report },
                }
            }
            // Restating the current fabrication changes nothing, so it
            // takes no revision or undo step.
            Operation::Layout(layout::LayoutCommand::SetFabrication(fabrication))
                if fabrication == candidate.project.fabrication =>
            {
                Outcome::None
            }
            Operation::Layout(layout::LayoutCommand::RouteStep { request }) => {
                let project = Arc::make_mut(&mut candidate.project);
                let plan = layout::route_step(project, &request)?;
                layout::validate(project)?;
                Outcome::RouteStepped {
                    end: plan.end,
                    end_layer: plan.end_layer,
                    reached: plan.reached,
                    detour_factor: plan.detour_factor,
                }
            }
            Operation::Layout(layout::LayoutCommand::TrackDrag { request }) => {
                let project = Arc::make_mut(&mut candidate.project);
                let original = project
                    .tracks
                    .iter()
                    .find(|t| t.id == request.track)
                    .cloned();
                let plan = layout::track_drag(project, &request)?;
                layout::validate(project)?;
                Outcome::TrackDragged {
                    offset: original
                        .as_ref()
                        .map_or(0.0, |t| layout::router::drag::dragged_by(t, &plan)),
                    reached: plan.reached,
                }
            }
            // Align and distribute ask the host's board evaluator which
            // parts their dimensions hold.
            Operation::Layout(layout::LayoutCommand::Align { instances, to }) => {
                let project = Arc::make_mut(&mut candidate.project);
                layout::arrange::align(project, &instances, to, evaluators.board)?;
                layout::validate(project)?;
                Outcome::None
            }
            Operation::Layout(layout::LayoutCommand::Distribute { instances, axis }) => {
                let project = Arc::make_mut(&mut candidate.project);
                layout::arrange::distribute(project, &instances, axis, evaluators.board)?;
                layout::validate(project)?;
                Outcome::None
            }
            // Parts named with the copper stay; the outcome names them.
            Operation::Layout(layout::LayoutCommand::DeleteObjects { objects }) => {
                let project = Arc::make_mut(&mut candidate.project);
                let parts = layout::several::delete(project, &objects)?;
                layout::validate(project)?;
                if parts.is_empty() {
                    Outcome::None
                } else {
                    Outcome::PartsKept { parts }
                }
            }
            Operation::Layout(command) => {
                let part = command.places_part();
                let before = candidate.project.library.components.len();
                let project = Arc::make_mut(&mut candidate.project);
                match layout::apply_staged(project, command)? {
                    Some(instance) if part => {
                        let found = project.instance(instance);
                        Outcome::PartPlaced {
                            instance,
                            designator: found.map(|i| i.designator.clone()).unwrap_or_default(),
                            component: found.map(|i| i.component).unwrap_or_default(),
                            created: project.library.components.len() > before,
                        }
                    }
                    Some(via) => Outcome::ViaAdded { via },
                    None => Outcome::None,
                }
            }
            Operation::Rules(command) => {
                if let rules::RulesCommand::AssignNets { nets, .. } = &command
                    && nets.iter().any(|n| candidate.project.net(*n).is_none())
                {
                    return Err(Error::NotFound(
                        "net_class.net_missing: a chosen net no longer exists".into(),
                    ));
                }
                // A pair or group names nets that exist (checked before
                // the nets follow-up, which would drop it).
                layout::length::check_command_nets(&candidate.project, &command)?;
                // A command that leaves the rules as they are (the same
                // profile again, an override to its current value) is not
                // an undo step.
                let mut next = candidate.project.rules.clone();
                let added = rules::apply_added(&mut next, command)?;
                // The profile's colour option follows the board's colour.
                layout::mask_colour::follow(&mut next, candidate.project.fabrication.mask_colour)?;
                if next != candidate.project.rules {
                    let project = Arc::make_mut(&mut candidate.project);
                    project.rules = next;
                    layout::restack::reconcile_rules(project, layout::restack::Foils::Follow)?;
                    // A sketch board follows `board.outline_fit`.
                    layout::outline::refit(project, evaluators.board)?;
                }
                match added {
                    Some(rules::signal::Added::Class(class)) => Outcome::NetClassAdded { class },
                    Some(rules::signal::Added::Pair(pair)) => Outcome::DiffPairAdded { pair },
                    Some(rules::signal::Added::Group(group)) => Outcome::LengthGroupAdded { group },
                    None => Outcome::None,
                }
            }
            Operation::Fill(command) => {
                let project = Arc::make_mut(&mut candidate.project);
                let adds_keepout = matches!(command, layout::fill::FillCommand::AddKeepout { .. });
                match layout::fill::apply(project, command)? {
                    Some(keepout) if adds_keepout => Outcome::KeepoutAdded { keepout },
                    Some(fill) => Outcome::FillAdded { fill },
                    None => Outcome::None,
                }
            }
            Operation::Board(command) => {
                if let layout::outline::BoardCommand::Edit { edits } = &command {
                    interface::assembly_link::guard_edits(
                        &self.document.project,
                        edits,
                        evaluators.board,
                    )?;
                }
                let project = Arc::make_mut(&mut candidate.project);
                match layout::outline::apply(project, command, evaluators.board)? {
                    layout::outline::BoardApplied::Edited { created } => {
                        Outcome::BoardEdited { created }
                    }
                    layout::outline::BoardApplied::Nothing => Outcome::None,
                }
            }
            Operation::Dimension(command) => {
                let project = Arc::make_mut(&mut candidate.project);
                match layout::dimensions::apply(project, command, evaluators.board)? {
                    layout::dimensions::DimensionApplied::Added(dimension) => {
                        Outcome::DimensionAdded { dimension }
                    }
                    layout::dimensions::DimensionApplied::Nothing => Outcome::None,
                }
            }
            Operation::Fabrication(command) => {
                let mut accepted = Default::default();
                if let fabrication::FabricationCommand::Waive { keys, profile, .. } = &command {
                    // What the findings measure where they were read.
                    let profile = profile
                        .clone()
                        .map(|p| p.resolve(&candidate.project.rules))
                        .transpose()?;
                    let snapshot = profile
                        .as_ref()
                        .map(|p| {
                            fabrication::ProductionSnapshot::resolve(&candidate.project, Some(p))
                        })
                        .transpose()?;
                    let project: &Project = match &snapshot {
                        Some(s) => s.project(),
                        None => &candidate.project,
                    };
                    accepted = fabrication::preflight::accepted(
                        project,
                        &self.document.drafts,
                        &self.check_settings,
                        keys,
                    )?;
                }
                let mut next = candidate.project.production.clone();
                fabrication::preflight::apply(
                    &mut next,
                    command,
                    self.waiver_reason_required,
                    &accepted,
                )?;
                if next != candidate.project.production {
                    Arc::make_mut(&mut candidate.project).production = next;
                }
                Outcome::None
            }
            Operation::Interface(command) => {
                if let Some(table) = interface::apply::apply(&candidate.project, command)? {
                    Arc::make_mut(&mut candidate.project).interface = table.into();
                }
                Outcome::None
            }
            Operation::Assembly(command) => {
                let project = Arc::make_mut(&mut candidate.project);
                interface::assembly_link::apply(project, command, evaluators.board)?;
                Outcome::None
            }
            // Applied to a copy, so a command that restates the variants
            // takes no revision or undo step.
            Operation::Variant(command) => {
                let mut next = (*candidate.project).clone();
                let added = instances::variants::apply(&mut next, command)?;
                if next != *candidate.project {
                    candidate.project = Arc::new(next);
                }
                match added {
                    Some(variant) => Outcome::VariantAdded { variant },
                    None => Outcome::None,
                }
            }
            Operation::Update(update) => {
                candidate.project = Arc::new(updates::candidate(
                    &self.document.project,
                    &self.copper_graph(),
                    &self.check_settings,
                    self.revision,
                    *update,
                    evaluators.board,
                )?);
                Outcome::None
            }
        };
        if !Arc::ptr_eq(&candidate.project, &self.document.project) {
            let schematic_edited = !candidate.project.same_schematic(&self.document.project);
            let project = Arc::make_mut(&mut candidate.project);
            // A reviewed update's candidate was finalized by its preview,
            // the one the review showed; every other operation finalizes
            // here through the same function.
            if !reviewed {
                // Only parts that move and carry their copper read the
                // graph; any other edit never makes it.
                let make = || self.copper_graph();
                let graph = layout::GraphSource::lazy(&make);
                rerouted = updates::finalize::finalize(
                    &self.document.project,
                    project,
                    &updates::finalize::Finalize {
                        cancel: self.sheet_cancel.as_deref(),
                        graph: &graph,
                        evaluator: evaluators.board,
                        moved_parts: &moved_parts,
                        loose: loose.as_ref(),
                        preserve_copper,
                        reviewed,
                        assembly_command,
                        sheet_move,
                    },
                )?
                .rerouted;
            }
            // Every operation, from any surface: two named nets become one
            // only when a reviewed update asks for it.
            if !merge_named && schematic_edited {
                nets::refuse_named_merges(&self.document.project, project)?;
            }
            // Collections stay in the order a file holds them, so a saved
            // and reopened document is the same document.
            format::order::sort(project);
            validate::edited_document(
                &self.document.project,
                &candidate.project,
                &candidate.drafts,
            )?;
        } else if candidate.drafts == self.document.drafts {
            return Ok(outcome);
        } else {
            validate::drafts_only(&candidate.project, &candidate.drafts)?;
        }
        if unrecorded {
            self.store_unrecorded(candidate)?;
        } else {
            self.commit(candidate, step)?;
        }
        self.rerouted = rerouted;
        Ok(outcome)
    }

    /// `rules.preview_fab_profile` / `rules.preview_fab_options`: the rule,
    /// choice and outer copper changes of taking `profile`, and the board
    /// issues that would appear or go.
    fn preview_profile(
        &self,
        profile: rules::RuleProfile,
        keep_overrides: bool,
        carry: rules::ChoiceCarry,
    ) -> Result<QueryResult> {
        let current = &self.document.project;
        let profile = layout::mask_colour::following(&profile, current.fabrication.mask_colour)?;
        let (next, refusal) = rules::preview_candidate(&current.rules, &profile, keep_overrides);
        let mut impact = rules::rule_changes(&current.rules, &next);
        impact.kept_choices = carry.kept;
        impact.reset_choices = carry.reset;
        impact.chosen_choices = carry.chosen;
        if let Some(reason) = refusal {
            impact.refusal = Some(reason);
            return Ok(QueryResult::ProfileImpact(Box::new(impact)));
        }
        let mut candidate = current.as_ref().clone();
        candidate.rules = next;
        match layout::restack::reconcile_rules(&mut candidate, layout::restack::Foils::Follow) {
            Ok(change) => impact.outer_copper_um = change,
            Err(error) => {
                impact.refusal = Some(error.to_string());
                return Ok(QueryResult::ProfileImpact(Box::new(impact)));
            }
        }
        let scope = issues::IssueScope::All;
        let rows = |p: &Project| -> Result<Vec<(String, String)>> {
            issues::report(p, &self.document.drafts, &scope, &self.check_settings)?
                .rows
                .into_iter()
                .map(|r| {
                    let id = serde_json::to_string(&(&r.finding.code, &r.finding.objects))?;
                    Ok((id, r.title))
                })
                .collect()
        };
        rules::diff_issues(&mut impact, rows(current)?, rows(&candidate)?);
        Ok(QueryResult::ProfileImpact(Box::new(impact)))
    }

    /// Answer a read-only query.
    pub fn query(&self, query: Query) -> Result<QueryResult> {
        self.query_with(query, &catalog::CatalogContext::default())
    }

    /// Answer a read-only query with the host's catalog context (favourite
    /// and recently used lineages), which ranks `library.search` results.
    pub fn query_with(
        &self,
        query: Query,
        context: &catalog::CatalogContext,
    ) -> Result<QueryResult> {
        self.query_using(query, context, None)
    }

    /// [`query_with`](Self::query_with) with the host's footprint
    /// evaluator, which `library.footprint_preview` needs.
    pub fn query_using(
        &self,
        query: Query,
        context: &catalog::CatalogContext,
        evaluator: Option<&dyn library::FootprintEvaluator>,
    ) -> Result<QueryResult> {
        let _context = self.enter_context();
        let index = || {
            catalog::Index::with_sources(
                &self.document.project,
                &self.document.drafts,
                &context.sources,
            )
        };
        match query {
            Query::Search {
                query,
                limit,
                include_drafts,
            } => {
                let index = index()?;
                let parsed = index.parse(&query)?;
                Ok(QueryResult::Search(index.search(
                    &parsed,
                    context,
                    catalog::SearchOptions {
                        offset: 0,
                        limit: limit.unwrap_or(catalog::DEFAULT_LIMIT),
                        include_drafts,
                    },
                )))
            }
            Query::Facets {
                query,
                include_drafts,
            } => {
                let index = index()?;
                let parsed = index.parse(&query)?;
                Ok(QueryResult::Facets(index.facets(&parsed, include_drafts)))
            }
            Query::InspectLineage { lineage, revision } => {
                Ok(QueryResult::Lineage(Box::new(catalog::inspect(
                    &self.document.project,
                    &self.document.drafts,
                    lineage,
                    revision,
                )?)))
            }
            Query::Bom {} => Ok(QueryResult::Bom(completeness::bom(&self.document.project)?)),
            Query::ResolveGroups {} => Ok(QueryResult::ResolveGroups(
                completeness::resolve_groups(&self.document.project)?,
            )),
            Query::Ties {} => Ok(QueryResult::Ties(layout::connectivity::report(
                self.project(),
                &self.copper_graph(),
            ))),
            Query::Variants {} => Ok(QueryResult::Variants(instances::variants::report(
                &self.document.project,
            )?)),
            Query::Substitutes { instance } => Ok(QueryResult::Substitutes(
                instances::variants::substitutes(&self.document.project, instance)?,
            )),
            Query::InspectInstance { instance } => Ok(QueryResult::Instance(Box::new(
                completeness::instance(&self.document.project, instance)?,
            ))),
            Query::CheckDefinition { target } => Ok(QueryResult::Definition(
                completeness::definition(&self.document.project, &self.document.drafts, target)?,
            )),
            Query::CheckDrafts { .. } => Ok(QueryResult::Drafts(completeness::drafts(
                &self.document.project,
                &self.document.drafts,
            )?)),
            Query::FootprintPreview { draft, edits } => {
                Ok(QueryResult::FootprintPreview(Box::new(
                    commands::preview_footprint(&self.document, draft, &edits, evaluator)?,
                )))
            }
            Query::UpdatesAvailable {} => Ok(QueryResult::UpdatesAvailable(
                catalog::updates_available(&self.document.project.library, &context.sources)?,
            )),
            Query::StandardFields {} => Ok(QueryResult::StandardFields(Box::new(
                library::standard_fields::bundle()?,
            ))),
            Query::PromotePreview {
                root,
                library,
                name,
                fresh,
            } => Ok(QueryResult::Promote(Box::new(library::import::export(
                &self.document.project.library,
                root,
                library,
                &name,
                &fresh,
            )?))),
            Query::RulesEffective {} => Ok(QueryResult::Rules(Box::new(rules::report(
                &self.document.project.rules,
            )))),
            Query::RuleProfiles { id: None } => Ok(QueryResult::RuleProfiles(
                self.document
                    .project
                    .rules
                    .profile
                    .iter()
                    .map(|p| rules::ProfileBrief::of(p, true))
                    .collect(),
            )),
            Query::RuleProfiles { id: Some(id) } => Ok(QueryResult::RuleProfile(Box::new(
                rules::ProfileChoice::Id(id).resolve(&self.document.project.rules)?,
            ))),
            Query::PreviewOptions { options } => {
                let profile = rules::with_options(&self.document.project.rules, &options)?;
                self.preview_profile(profile, true, rules::ChoiceCarry::default())
            }
            Query::PreviewProfile {
                profile,
                options,
                keep_overrides,
            } => {
                let current = &self.document.project.rules;
                let (profile, carry) =
                    rules::carry_choices(current, profile.resolve(current)?, &options)?;
                self.preview_profile(profile, keep_overrides, carry)
            }
            Query::NetClasses {} => {
                let project = &self.document.project;
                Ok(QueryResult::NetClasses(Box::new(rules::netclass::report(
                    &project.rules,
                    project.nets.iter().map(|n| (n.id, n.name.as_str())),
                ))))
            }
            Query::PreviewNetClassPatterns { patterns } => {
                let project = &self.document.project;
                Ok(QueryResult::NetClassChanges(
                    rules::netclass::preview_patterns(
                        &project.rules,
                        patterns,
                        project.nets.iter().map(|n| (n.id, n.name.as_str())),
                    )?,
                ))
            }
            Query::BoardShape {} => Ok(QueryResult::BoardShape(Box::new(
                self.document.project.board().into_owned(),
            ))),
            Query::Preflight { profile } => {
                let profile = profile
                    .map(fabrication::ProfileChoice::into_profile)
                    .transpose()?;
                Ok(QueryResult::Preflight(Box::new(
                    fabrication::preflight::report(
                        &self.document.project,
                        &self.document.drafts,
                        profile.as_ref(),
                        &self.check_settings,
                    )?,
                )))
            }
            Query::ProfileSuggestions {
                profile,
                candidates,
                fabricators,
            } => {
                let profile = profile
                    .map(fabrication::ProfileChoice::into_profile)
                    .transpose()?;
                Ok(QueryResult::ProfileSuggestions(Box::new(
                    fabrication::suggest::suggest(
                        &self.document.project,
                        &self.document.drafts,
                        &self.check_settings,
                        profile.as_ref(),
                        &candidates,
                        &fabricators,
                    )?,
                )))
            }
            Query::InterfaceList {} => Ok(QueryResult::InterfaceList(Box::new(
                interface::list::list(&self.document.project)?,
            ))),
            Query::AssemblyStatus {} => Ok(QueryResult::AssemblyStatus(Box::new(
                interface::assembly_link::status(
                    &self.document.project,
                    interface::assembly_link::FileCheck::NotChecked,
                ),
            ))),
            Query::FillStatus {} => Ok(QueryResult::FillStatus(layout::fill::status(
                &self.document.project,
            ))),
            Query::Sheets {} => Ok(QueryResult::Sheets(schematic::sheet_rows(
                &self.document.project,
            ))),
            Query::SheetCopy {
                sheet,
                objects,
                portable,
            } => Ok(QueryResult::SheetClip(Box::new(
                schematic::clipboard::copy(
                    &self.document.project,
                    sheet.unwrap_or_else(|| self.document.project.sheets.first()),
                    &objects,
                    portable,
                )?,
            ))),
            Query::NetPlaces { net } => Ok(QueryResult::NetPlaces(Box::new(
                schematic::net_places(&self.document.project, net)?,
            ))),
            Query::SheetScene { sheet, path } => {
                if self.document.project.sheet(sheet).is_none() {
                    return Err(Error::NotFound("Unknown sheet.".into()));
                }
                if let Some(path) = &path
                    && !(path.symbols.is_empty()
                        && path.root == sheet
                        && self.project().is_root_sheet(sheet))
                    && self
                        .project()
                        .sheet_use(path)
                        .is_none_or(|u| u.definition != sheet)
                {
                    return Err(Error::NotFound("Unknown sheet occurrence.".into()));
                }
                Ok(QueryResult::SheetScene(Box::new(
                    schematic::scene::sheet_scene_in(&self.document.project, sheet, path.as_ref()),
                )))
            }
            Query::ImpedanceCalculate { request } => Ok(QueryResult::Impedance(Box::new(
                layout::impedance::calculate(&self.document.project, &request)?,
            ))),
            Query::ImpedanceTable {} => Ok(QueryResult::ImpedanceTable(Box::new(
                layout::impedance::table(&self.document.project),
            ))),
            Query::SuggestPairs {} => {
                let project = &self.document.project;
                Ok(QueryResult::PairSuggestions(
                    library::standards::get()?.pairs.suggest(
                        &project.rules,
                        project.nets.iter().map(|n| (n.id, n.name.as_str())),
                    ),
                ))
            }
            Query::Pads { designators, net } => Ok(QueryResult::Pads(layout::pad_rows(
                &self.document.project,
                &designators,
                net,
            )?)),
            Query::LengthReport {} => Ok(QueryResult::LengthReport(Box::new(
                layout::length::report_with(&self.document.project, &self.copper_graph()),
            ))),
            Query::RoutePreview { request } => {
                request.check()?;
                let project = &self.document.project;
                project.rules.require()?;
                let index = layout::router::RouteIndex::with_graph(project, self.copper_graph());
                Ok(QueryResult::RoutePreview(Box::new(layout::router::plan(
                    project, &index, &request,
                ))))
            }
            Query::TrackDragPreview { request } => {
                request.check()?;
                let project = &self.document.project;
                project.rules.require()?;
                let index = layout::router::RouteIndex::with_graph(project, self.copper_graph());
                Ok(QueryResult::RoutePreview(Box::new(
                    layout::router::drag::drag_plan(project, &index, &request),
                )))
            }
            Query::TunePreview { request } => {
                let project = &self.document.project;
                project.rules.require()?;
                let obstacles = layout::obstacles::Obstacles::of(project);
                Ok(QueryResult::TunePreview(Box::new(
                    layout::meander::plan(project, &obstacles, &self.copper_graph(), &request)?
                        .preview(),
                )))
            }
            Query::Issues { scope } => {
                let all = self.issue_report()?;
                Ok(QueryResult::Issues(match scope {
                    issues::IssueScope::All => all.as_ref().clone(),
                    scope => all.scoped(&scope, &self.document.project),
                }))
            }
            Query::Models { query, fits, limit } => {
                Ok(QueryResult::Models(catalog::models::search(
                    &self.document.project,
                    &self.document.drafts,
                    context,
                    &query,
                    fits,
                    limit
                        .unwrap_or(catalog::DEFAULT_LIMIT)
                        .min(catalog::models::MAX_MODEL_HITS),
                )?))
            }
            Query::PinoutPreview {
                target,
                document,
                detail,
            } => Ok(QueryResult::PinoutPlan(Box::new(
                commands::pinout::preview(&self.document, &target, &document, detail)?,
            ))),
            Query::SuggestPins { instance, request } => Ok(QueryResult::SuggestPins(
                pins::suggest(&self.document.project, instance, &request)?,
            )),
            Query::PinReport { instance } => Ok(QueryResult::PinReport(Box::new(
                pins::pin_report(&self.document.project, instance)?,
            ))),
            Query::CheckIntent { nets } => {
                let report =
                    schematic::intent::check(self.project(), &nets, MAX_BATCH, &mut self.fork())?;
                Ok(QueryResult::NetIntent(Box::new(report)))
            }
            Query::PartPins { designators } => Ok(QueryResult::PartPins(pins::part_pins(
                &self.document.project,
                &designators,
            )?)),
            Query::PinConfig { instance } => Ok(QueryResult::PinConfig(Box::new(
                firmware::pin_config(&self.document.project, instance)?,
            ))),
            Query::Marking { instance } => Ok(QueryResult::Marking(commands::marking::layout(
                &self.document.project,
                instance,
            )?)),
        }
    }

    pub fn preview_update(&self, change: updates::Change) -> updates::UpdatePlan {
        self.preview_update_hosted(change, Evaluators::default())
    }

    /// [`Self::preview_update`] with the host's evaluators, which trace the
    /// 3D model faces part dimensions name in the new package revisions and
    /// solve the dimensioned parts, as applying the update does.
    pub fn preview_update_hosted(
        &self,
        change: updates::Change,
        evaluators: Evaluators<'_>,
    ) -> updates::UpdatePlan {
        updates::preview(
            &self.document.project,
            &self.copper_graph(),
            &self.check_settings,
            self.revision,
            change,
            evaluators.board,
        )
        .0
    }

    pub fn apply_update(
        &mut self,
        expected_revision: u64,
        change: updates::Change,
        remove_conflicting_tracks: bool,
    ) -> Result<()> {
        self.apply(
            expected_revision,
            Operation::Update(Box::new(updates::ApplyUpdate {
                expected_revision,
                change,
                remove_conflicting_tracks,
                merge_named_nets: false,
            })),
        )
        .map(|_| ())
    }

    /// Store fill copper a host computed off the UI thread
    /// (`layout::fill::compute_stale`) without an undo step: the copper is
    /// derivable, so it belongs to the revision it was computed for, not
    /// to the history. Refused when `expected_revision` is not current;
    /// results whose key no longer matches their fill are skipped.
    /// Returns how many fills were stored.
    pub fn store_fills(
        &mut self,
        expected_revision: u64,
        results: Vec<(Id, model::fill::FilledCopper)>,
    ) -> Result<usize> {
        self.check_revision(expected_revision)?;
        // Only the stored fills' keys (and those they depend on).
        let project = &self.document.project;
        let ids: Vec<Id> = results
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| project.fill(*id).is_some())
            .collect();
        let keys = layout::fill::inputs_keys_of(project, ids)?;
        let fresh: Vec<_> = results
            .into_iter()
            .filter(|(id, c)| keys.get(id) == Some(&c.key))
            .collect();
        if fresh.is_empty() {
            return Ok(0);
        }
        let mut project = (*self.document.project).clone();
        for (id, copper) in &fresh {
            if let Some(f) = project.fills.iter_mut().find(|f| f.id == *id) {
                f.computed = Some(copper.clone());
            }
        }
        layout::fill::validate(&project)?;
        let revision = self.next_revision()?;
        self.document.project = Arc::new(project);
        self.set_revision(revision);
        Ok(fresh.len())
    }

    /// Compute and store the copper of every fill whose copper is missing
    /// or stale, here and now, without an undo step ([`Self::store_fills`]).
    /// For a host without a UI thread to keep free (the headless session
    /// after opening a file, which holds no fill copper); the desktop app
    /// does the same on its fill worker. Returns how many fills were
    /// stored.
    pub fn refill_stale(&mut self) -> Result<usize> {
        let mut cache = layout::fill::FillCache::default();
        let copper = layout::fill::compute_stale(&self.document.project, &mut cache)?;
        if copper.is_empty() {
            return Ok(0);
        }
        self.store_fills(self.revision, copper)
    }

    /// Refused as stale unless `expected_revision` is the current one.
    /// Every document change, undo and redo included, is checked here, so a
    /// caller never changes a document that moved since it last looked.
    pub fn check_revision(&self, expected_revision: u64) -> Result<()> {
        if expected_revision != self.revision {
            return Err(Error::Stale {
                expected: expected_revision,
                actual: self.revision,
            });
        }
        Ok(())
    }

    /// The person's own undo in the window that shows this document: at
    /// the current revision, which is what they see. Transports and agents
    /// use [`Engine::undo_at`].
    pub fn undo(&mut self) -> Result<()> {
        self.undo_at(self.revision)
    }

    /// The person's own redo; see [`Engine::undo`].
    pub fn redo(&mut self) -> Result<()> {
        self.redo_at(self.revision)
    }

    /// Restore the document before the latest step, at an expected
    /// revision: refused, with nothing changed, when the document moved
    /// since, so someone else's newer step is never undone.
    pub fn undo_at(&mut self, expected_revision: u64) -> Result<()> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let (mut previous, step) = self
            .undo
            .back()
            .cloned()
            .ok_or_else(|| Error::History("nothing to undo".into()))?;
        Self::carry_interface(&self.document, &mut previous)?;
        self.undo.pop_back();
        self.redo
            .push((std::mem::replace(&mut self.document, previous), step));
        self.set_revision(revision);
        self.last_step = None;
        self.history = next_history();
        Ok(())
    }

    /// Reapply the latest undone step at an expected revision, as for
    /// [`Engine::undo_at`].
    pub fn redo_at(&mut self, expected_revision: u64) -> Result<()> {
        self.check_revision(expected_revision)?;
        let revision = self.next_revision()?;
        let (mut next, step) = self
            .redo
            .last()
            .cloned()
            .ok_or_else(|| Error::History("nothing to redo".into()))?;
        Self::carry_interface(&self.document, &mut next)?;
        self.redo.pop();
        self.undo
            .push_back((std::mem::replace(&mut self.document, next), step));
        self.set_revision(revision);
        self.last_step = None;
        self.history = next_history();
        Ok(())
    }

    /// An undo or redo moves from `current` to `restored`: the interface's
    /// slot and datum counters keep their highest values, so a slot or id
    /// handed out once is never handed out again
    /// (`interface::auto::carry`).
    fn carry_interface(current: &Document, restored: &mut Document) -> Result<()> {
        if let Some(table) = interface::auto::carry(&current.project.interface, &restored.project)?
        {
            Arc::make_mut(&mut restored.project).interface = table.into();
        }
        Ok(())
    }

    /// Measure documents against `room` from now on: a test's smaller
    /// limit.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn set_room(&mut self, room: format::room::Room) {
        self.room = room;
    }

    /// Take what a save gave the copy it wrote ([`first_save`]), once the
    /// file holds it; a failed or conflicted write gives nothing. A save is
    /// not an edit: no revision and no undo step, and the history stays.
    /// Name and identity are the project's, not a state's, so every state
    /// of the history that lacks them takes them too and no undo or redo
    /// takes them back; a state with a name someone chose keeps it. A state
    /// that takes one is a new allocation, which tells a host that keeps
    /// something by state ([`ReportKey`]).
    pub fn adopt_first_save(&mut self, given: &FirstSave) {
        // Each project replaced, and what replaced it: states that share
        // one go on sharing one.
        let mut taken: Vec<(Arc<Project>, Arc<Project>)> = Vec::new();
        let states = std::iter::once(&mut self.document)
            .chain(self.undo.iter_mut().map(|(document, _)| document))
            .chain(self.redo.iter_mut().map(|(document, _)| document));
        for document in states {
            let known = taken
                .iter()
                .rev()
                .find(|(was, _)| Arc::ptr_eq(was, &document.project))
                .map(|(_, now)| Arc::clone(now));
            if let Some(now) = known {
                document.project = now;
            } else if let Some(project) = given.given_to(&document.project) {
                let now = Arc::new(project);
                let was = std::mem::replace(&mut document.project, Arc::clone(&now));
                taken.push((was, now));
            }
        }
    }

    /// Fold the latest undo step into the one before it, so one undo takes
    /// both back: a step that finishes the one before (placing the part a
    /// search or template has just created). `at` is the revision the
    /// latest step was applied at, which the earlier step made. Refused,
    /// with nothing changed, after an undo or redo, when another step came
    /// between, or with fewer than two steps.
    pub fn join_last_step(&mut self, at: u64) -> Result<()> {
        if self.last_step != Some(at) || self.undo.len() < 2 {
            return Err(Error::History(
                "the latest step does not follow the one before".into(),
            ));
        }
        // The document between the two steps goes; the one before both
        // stays the undo target, named by the latest step.
        if let Some((_, step)) = self.undo.pop_back()
            && let Some(earlier) = self.undo.back_mut()
        {
            earlier.1 = step;
        }
        self.last_step = None;
        self.history = next_history();
        Ok(())
    }

    /// Open an undo group named `label`: every step made until
    /// [`Self::end_group`] joins one undo step, which keeps the document
    /// from before the group as its undo target and reads `label`. Refused
    /// while a group is open, and with an empty label.
    pub fn begin_group(&mut self, label: impl Into<String>) -> Result<()> {
        self.open_group(label.into(), None)
    }

    /// Open the group a former [`Self::end_group`] closed at `mark` again,
    /// for the next call of the same turn: its steps join that group's
    /// step while it is still the latest one, with nothing undone, redone
    /// or made in between. Otherwise (a person's edit came between) they
    /// make a new step under the same label, so undo order stays what
    /// happened.
    pub fn continue_group(&mut self, label: impl Into<String>, mark: GroupMark) -> Result<()> {
        self.open_group(label.into(), Some(mark))
    }

    fn open_group(&mut self, label: String, mark: Option<GroupMark>) -> Result<()> {
        if self.group.is_some() {
            return Err(Error::History("an undo group is open already".into()));
        }
        let label = label.trim();
        if label.is_empty() {
            return Err(Error::Invalid("an undo group needs a label".into()));
        }
        self.group = Some(Group {
            label: label.to_owned(),
            mark,
        });
        Ok(())
    }

    /// Close the open group: where its latest step stands, for
    /// [`Self::continue_group`]; `None` when no group was open or it has
    /// made no step yet.
    pub fn end_group(&mut self) -> Option<GroupMark> {
        self.group.take().and_then(|group| group.mark)
    }

    /// The report of every issue of this state ([`issues::IssueScope::All`]),
    /// what `project.issues` answers and every view projects its scope
    /// from ([`issues::IssueReport::scoped`]). Made once per state and
    /// check settings, from the last report made on this engine or its
    /// forks: a state that changed only tracks and vias checks only what
    /// reads them ([`issues::incremental`]).
    pub fn issue_report(&self) -> Result<Arc<issues::IssueReport>> {
        self.issue_report_of(&self.document)
    }

    /// [`Self::issue_report`] of `document`, another state under this
    /// engine's check settings (a host comparing the state before a
    /// request with the one after), made from and kept as the last report
    /// in the same way.
    pub fn issue_report_of(&self, document: &Document) -> Result<Arc<issues::IssueReport>> {
        let _context = self.enter_context();
        let (project, drafts) = (&document.project, &document.drafts);
        let settings = &self.check_settings;
        let last = self.checked.lock().ok().and_then(|slot| slot.clone());
        if let Some(last) = &last
            && last.describes(project, drafts, settings)
        {
            return Ok(last.report().clone());
        }
        let current = Arc::ptr_eq(project, &self.document.project);
        let graph = || {
            if current {
                self.copper_graph()
            } else {
                Arc::new(layout::CopperGraph::of(project))
            }
        };
        let next = Arc::new(match &last {
            Some(last) => last.next(project, drafts, settings, &graph)?,
            None => issues::incremental::Checked::full(project, drafts, settings, &graph)?,
        });
        if let Ok(mut slot) = self.checked.lock() {
            *slot = Some(next.clone());
        }
        Ok(next.report().clone())
    }

    /// The copper graph of the current revision: built once, on first use,
    /// and shared by the checks, the Issues query and the desktop.
    pub fn copper_graph(&self) -> Arc<layout::CopperGraph> {
        self.copper
            .get_or_init(|| Arc::new(layout::CopperGraph::of(&self.document.project)))
            .clone()
    }

    /// Move to `revision`: every derived result of the old one is dropped.
    fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
        self.copper = std::sync::OnceLock::new();
    }

    fn next_revision(&self) -> Result<u64> {
        self.revision
            .checked_add(1)
            .ok_or_else(|| Error::Limit("revision exhausted".into()))
    }

    /// Store `candidate` as a new revision without an undo step; undo and
    /// redo keep what it recorded (`interface::auto::carry`), and the
    /// steps around it stay as they were.
    fn store_unrecorded(&mut self, candidate: Document) -> Result<()> {
        let revision = self.next_revision()?;
        self.room.admit(&candidate)?;
        self.document = candidate;
        self.last_step = None;
        self.set_revision(revision);
        Ok(())
    }

    fn commit(&mut self, candidate: Document, mut step: Step) -> Result<()> {
        let revision = self.next_revision()?;
        self.room.admit(&candidate)?;
        let history = next_history();
        if let Some(group) = &mut self.group {
            step.label = Some(group.label.clone());
            let joins = group.mark == Some(GroupMark(self.history));
            group.mark = Some(GroupMark(history));
            // The group's own step is the latest: the document before the
            // group stays its undo target.
            if joins && let Some((_, earlier)) = self.undo.back_mut() {
                if earlier.title != step.title {
                    earlier.title = Step::MIXED;
                }
                if earlier.subject != step.subject {
                    earlier.subject = None;
                }
                earlier.label = step.label;
                self.document = candidate;
                self.redo.clear();
                self.last_step = Some(self.revision);
                self.history = history;
                self.set_revision(revision);
                return Ok(());
            }
        }
        if self.undo.len() >= MAX_UNDO {
            self.undo.pop_front();
        }
        self.undo
            .push_back((std::mem::replace(&mut self.document, candidate), step));
        self.redo.clear();
        self.last_step = Some(self.revision);
        self.history = history;
        self.set_revision(revision);
        Ok(())
    }
}

/// An open undo group ([`Engine::begin_group`]).
#[derive(Clone, Debug)]
struct Group {
    label: String,
    /// Where the group's latest step stands; `None` before its first.
    mark: Option<GroupMark>,
}

/// Where an undo group's latest step stands in an engine's history
/// ([`Engine::end_group`]): a later step of the same group joins it only
/// while the history is still there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupMark(u64);

/// A value no history state of any engine in this process had before, so
/// a [`GroupMark`] of one engine never matches another's.
fn next_history() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// JSON schemas of the document types every transport shares.
pub fn schemas() -> serde_json::Value {
    serde_json::json!({
        "operation": schemars::schema_for!(Operation),
        "outcome": schemars::schema_for!(Outcome),
        "query": schemars::schema_for!(Query),
        "query_result": schemars::schema_for!(QueryResult),
        "project": schemars::schema_for!(Project),
        "update_plan": schemars::schema_for!(updates::UpdatePlan),
        "editor_file": schemars::schema_for!(format::EditorFile),
        "finding": schemars::schema_for!(Finding),
        "diagnostic": schemars::schema_for!(Diagnostic),
        "pinout": library::pinout::schema(),
        "pin_config": firmware::schema(),
    })
}

impl schematic::intent::Trial for Engine {
    fn snapshot(&self) -> &Project {
        self.project()
    }

    fn try_edit(&mut self, command: schematic::SchematicCommand) -> Result<()> {
        self.apply(self.revision(), Operation::Schematic(command))?;
        Ok(())
    }
}

/// A test fixture, never in the product: an engine on the fixture project
/// ([`Project::default`]).
#[cfg(any(test, feature = "test-fixtures"))]
impl Default for Engine {
    fn default() -> Self {
        Self::with_document(Document::default())
    }
}
