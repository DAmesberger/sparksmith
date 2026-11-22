//! The automation request contract. Each request is a document query, a
//! document mutation through [`Engine::apply`], or an external file effect.
//! File effects never enter undo history.
use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::{
    Class, DefinitionId, Engine, LibraryCommand, Operation, Project, Query, QueryResult,
    catalog::EntryKey,
    erc,
    fabrication::ProfileChoice,
    format::EditorFile,
    geometry::Point,
    ids::{DefinitionRef, LibraryId, Revision, SourceLineageId},
    layout,
    library::{
        Drafts,
        graphics::{GraphicTarget, SILK_STROKE_MM, SYMBOL_STROKE_MM},
        import::{OtherRevision, PlaceSource},
    },
    permission::FilePath,
    updates,
};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyParams {
    /// The revision the operation was prepared against.
    pub expected_revision: u64,
    pub operation: Operation,
}

/// Document operations applied in order as one undo step.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyAllParams {
    /// The revision the operations were prepared against.
    pub expected_revision: u64,
    /// One to [`sparksmith_core::MAX_BATCH`] operations; each sees the
    /// document the ones before it made.
    pub operations: Vec<Operation>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// A document change that names nothing but the revision it expects
/// (`project.undo`, `project.redo`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionParams {
    /// The revision of the last reply; refused as stale when the document
    /// has moved since.
    pub expected_revision: u64,
}

/// Start a new project (`project.new`) with its board's fab profile.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewParams {
    /// The revision of the last reply; refused as stale when the document
    /// has moved since.
    pub expected_revision: u64,
    /// The board's fab profile: a library or shipped id, or a whole
    /// profile file's `profile`. Omitted: the Settings default, else the
    /// shipped default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<sparksmith_core::fabrication::ProfileChoice>,
}

/// Open a project file in place of the current document.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenParams {
    /// The revision of the document being replaced; refused as stale when
    /// it has moved since.
    pub expected_revision: u64,
    pub path: FilePath,
}

/// What `project.inspect` returns.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InspectParams {
    /// A JSON pointer (RFC 6901) into the snapshot that `verbose: true`
    /// returns; just what it names is returned. Its top-level keys are
    /// `revision`, `project`, `drafts`, `can_undo`, `can_redo`, `erc` and
    /// `board_checks`. A list is indexed by position from 0, not by id:
    /// `/drafts` lists each draft as `{draft, check}` in order, so
    /// `/drafts/0/draft` is the first draft (its `id` is the draft id),
    /// `/drafts/0/draft/definition/body/pads/0` a footprint draft's first
    /// pad and `/drafts/0/check` its publish findings. Others:
    /// `/project/vias`, `/project/nets`, `/project/library`. A pointer
    /// that names nothing is refused with the keys or indices valid where
    /// it stopped matching.
    #[serde(default)]
    pub at: Option<String>,
}

/// Which schema `project.schema` returns.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchemaParams {
    /// An operation command (`layout.add_via`), a query (`library.search`)
    /// or a command or tool name. Without it, the index of all of them.
    #[serde(default)]
    pub name: Option<String>,
    /// One definition `name`'s reply lists under `more_defs`.
    #[serde(default)]
    pub def: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathParams {
    pub path: FilePath,
}

/// Where `project.export_native` writes and how much of the board.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportNativeParams {
    /// A new `.solidcraft` file.
    pub path: FilePath,
    /// `mechanical` (default: laminate, pads, parts) or `full` (all copper
    /// too); the interface tier goes through `interface.publish`.
    #[serde(default)]
    pub tier: sparksmith_core::model::interface::Tier,
    /// Write even if bodies do not fit the plan.
    #[serde(default)]
    pub write_anyway: bool,
}

/// Where `schematic.export_pdf` writes and which sheets it prints.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SchematicPdfParams {
    pub path: FilePath,
    /// Sheets to print, in page order; every sheet when empty.
    #[serde(default)]
    pub sheets: Vec<sparksmith_core::SheetId>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveParams {
    /// Destination `.sparksmith` file; defaults to the path last opened or saved.
    #[serde(default)]
    pub path: Option<FilePath>,
}

/// Import an SVG file's drawing into a symbol or footprint draft.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportSvgParams {
    /// The revision the import was prepared against.
    pub expected_revision: u64,
    /// A symbol or footprint draft.
    pub draft: DefinitionId,
    /// A symbol unit, or a footprint's silkscreen or assembly layer and side.
    pub target: GraphicTarget,
    /// The `.svg` file (at most 1 MiB).
    pub path: FilePath,
    /// Where the artwork's bounding-box centre goes, in millimetres.
    #[serde(default)]
    pub at: Point,
    /// Multiplies the size the SVG declares (its width and height over the
    /// viewBox; 96 user units per inch without units), 0.001–1000.
    #[serde(default = "unit_scale")]
    pub scale: f64,
    /// Chord tolerance for flattened Béziers and ellipses, 0.001–1 mm.
    #[serde(default = "default_tolerance")]
    pub tolerance_mm: f64,
}

fn unit_scale() -> f64 {
    1.0
}
fn default_tolerance() -> f64 {
    sparksmith_solidcraft::svg::DEFAULT_TOLERANCE_MM
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceAddParams {
    /// A library folder holding `sparksmith-library.json`.
    pub path: FilePath,
    /// Whether `library.publish_to_folder` may write to it.
    #[serde(default)]
    pub writable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceCreateParams {
    /// A new folder, or an empty existing one.
    pub path: FilePath,
    /// The library's display name, stored in its manifest.
    pub name: String,
    /// The library id; drawn at random when omitted. A recorded session
    /// passes the one it recorded, so its replay reproduces the folder.
    #[serde(default)]
    pub library: Option<LibraryId>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceEnableParams {
    pub path: FilePath,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FavouriteParams {
    pub key: EntryKey,
    pub favourite: bool,
}

/// One revision of a folder lineage (`source_lineage` of a search entry).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BundleParams {
    pub library: LibraryId,
    pub lineage: SourceLineageId,
    /// Defaults to the newest revision the folder holds.
    #[serde(default)]
    pub revision: Option<Revision>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportFromParams {
    pub expected_revision: u64,
    pub library: LibraryId,
    pub lineage: SourceLineageId,
    #[serde(default)]
    pub revision: Option<Revision>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceFromParams {
    pub expected_revision: u64,
    pub library: LibraryId,
    /// A component lineage of the folder.
    pub lineage: SourceLineageId,
    #[serde(default)]
    pub revision: Option<Revision>,
    #[serde(with = "sparksmith_core::units::point_mm")]
    #[schemars(with = "[sparksmith_core::units::Length; 2]")]
    pub position: sparksmith_core::units::PointNm,
    #[serde(default)]
    pub designator: Option<String>,
    #[serde(default)]
    pub other_revision: OtherRevision,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PublishParams {
    pub expected_revision: u64,
    /// The published project revision to promote, with its closure.
    pub root: DefinitionRef,
    /// A writable library source.
    pub library: LibraryId,
    /// Lineage ids for lineages new to the library, consumed in closure
    /// order (fewer than it needs refuses, unused ones are ignored); drawn
    /// at random when omitted. A recorded session passes the ones it
    /// recorded, so its replay reproduces the folder and the origins.
    #[serde(default)]
    pub fresh: Option<Vec<SourceLineageId>>,
}

/// Import one exact revision and its closure from another project file.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportFromProjectParams {
    pub expected_revision: u64,
    /// The `.sparksmith` file to read. It is not changed.
    pub path: FilePath,
    /// A published revision in that file.
    pub root: DefinitionRef,
}

/// Write the production files of the open project.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FabricationParams {
    /// A new or empty folder, or with `zip` a new `.zip` file.
    pub output: FilePath,
    #[serde(default)]
    pub zip: bool,
    /// The fabrication fab profile to check against, by its id in the
    /// user's profile library or a built-in's. The project's own rules
    /// when omitted.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub options: sparksmith_core::fabrication::FabricationOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewParams {
    pub change: updates::Change,
}

/// One request. `command` names it; `params` holds its typed parameters.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "command", content = "params", deny_unknown_fields)]
pub enum Request {
    /// Apply one document operation at an expected revision.
    #[serde(rename = "project.apply")]
    Apply(ApplyParams),
    /// Apply document operations in order at an expected revision as one
    /// undo step; any failure changes nothing.
    #[serde(rename = "project.apply_all")]
    ApplyAll(ApplyAllParams),
    /// Return the document, drafts with their findings and design checks.
    #[serde(rename = "project.inspect")]
    Inspect(InspectParams),
    /// The exact schema of one operation, query or command, or the index.
    #[serde(rename = "project.schema")]
    Schema(SchemaParams),
    /// Answer one read-only core query (`component.inspect`,
    /// `component.check`, `component.check_drafts`, `library.search`,
    /// `library.facets`, `library.inspect`, `project.bom`,
    /// `library.resolve_groups`, `project.issues`), passed unchanged to
    /// `Engine::query`.
    #[serde(rename = "project.query")]
    Query(Query),
    /// Undo the latest step at an expected revision.
    #[serde(rename = "project.undo")]
    Undo(RevisionParams),
    /// Redo the latest undone step at an expected revision.
    #[serde(rename = "project.redo")]
    Redo(RevisionParams),
    /// Start a new, empty project. History is cleared.
    #[serde(rename = "project.new")]
    New(NewParams),
    /// Open a `.sparksmith` file; a failure leaves the session unchanged.
    #[serde(rename = "project.open")]
    Open(OpenParams),
    /// Atomically write the document to a `.sparksmith` file.
    #[serde(rename = "project.save")]
    Save(SaveParams),
    /// Preview a revision-bound update without changing the document.
    #[serde(rename = "project.preview_update")]
    PreviewUpdate(PreviewParams),
    /// Apply a previewed update at its revision.
    #[serde(rename = "project.apply_update")]
    ApplyUpdate(updates::ApplyUpdate),
    /// Preview replacing placed parts' component by another lineage's.
    #[serde(rename = "project.preview_replace")]
    PreviewReplace(replace::PreviewReplaceParams),
    /// Replace placed parts' component by another lineage's, terminals
    /// mapped explicitly.
    #[serde(rename = "project.replace_component")]
    ReplaceComponent(replace::ReplaceComponentParams),
    /// Write the board as a native, parametric `.solidcraft` model, its
    /// bodies checked against the board plan. Refuses existing files.
    #[serde(rename = "project.export_native")]
    ExportNative(ExportNativeParams),
    /// Check the design and write its production files (Gerber X2,
    /// Excellon, job file, pick-and-place, BOM, PDF drawings).
    #[serde(rename = "project.export_fabrication")]
    ExportFabrication(FabricationParams),
    /// List the configured library sources with their state.
    #[serde(rename = "library.sources")]
    Sources(NoParams),
    /// Add a library folder as a source (per-user settings).
    #[serde(rename = "library.source.add")]
    SourceAdd(SourceAddParams),
    /// Create an empty library folder and add it as a writable source.
    #[serde(rename = "library.source.create")]
    SourceCreate(SourceCreateParams),
    /// Remove a source from the settings; the folder is untouched.
    #[serde(rename = "library.source.remove")]
    SourceRemove(PathParams),
    /// Turn a source on or off.
    #[serde(rename = "library.source.enable")]
    SourceEnable(SourceEnableParams),
    /// The online library and who is signed in to it (downloading part
    /// data needs a free account; searching does not).
    #[serde(rename = "library.account")]
    Account(NoParams),
    /// Sign out of the online library: the registry revokes the token
    /// (best effort when offline) and this computer forgets it.
    #[serde(rename = "library.sign_out")]
    SignOut(NoParams),
    /// Re-read every source's manifest and catalog.
    #[serde(rename = "library.rescan")]
    Rescan(NoParams),
    /// Mark or unmark a favourite (per-user settings).
    #[serde(rename = "library.favourite")]
    Favourite(FavouriteParams),
    /// Read one folder revision and its closure as a bundle.
    #[serde(rename = "library.bundle")]
    Bundle(BundleParams),
    /// Read a folder bundle and apply `library.import` with it.
    #[serde(rename = "library.import_from_folder")]
    ImportFrom(ImportFromParams),
    /// Read a folder bundle and apply `library.place_from` with it.
    #[serde(rename = "library.place_from_folder")]
    PlaceFrom(PlaceFromParams),
    /// Promote a project revision to a writable folder: preview, write the
    /// bundle, then record the origins with `library.promote`.
    #[serde(rename = "library.publish_to_folder")]
    Publish(PublishParams),
    /// Read another project file and apply `library.import` with one exact
    /// revision and its closure: one undo step.
    #[serde(rename = "library.import_from_project")]
    ImportFromProject(ImportFromProjectParams),
    /// Read an SVG file and add its drawing to a draft through
    /// `library.edit_graphics`: one atomic undo step.
    #[serde(rename = "library.import_svg")]
    ImportSvg(ImportSvgParams),
    /// A2: inspect a folder item without importing it.
    #[serde(rename = "library.open_preview")]
    OpenPreview(BundleParams),
    /// C3: the typed pinout document schema.
    #[serde(rename = "library.pinout_schema")]
    PinoutSchema(NoParams),
    /// Check a datasheet review report against the drafts.
    #[serde(rename = "library.review_check")]
    ReviewCheck(crate::review::ReviewCheckParams),
    /// C4: the emitted firmware pin-configuration text.
    #[serde(rename = "firmware.export_preview")]
    FirmwarePreview(firmware::FirmwareParams),
    /// C4: write the emitted text to a new file.
    #[serde(rename = "firmware.export")]
    FirmwareExport(firmware::FirmwareParams),
    /// D2: inspect a STEP, GLB or .solidcraft model file.
    #[serde(rename = "library.inspect_model_file")]
    InspectModelFile(models::ModelFileParams),
    /// Set a component draft's marking with SVG artwork as text.
    #[serde(rename = "library.set_marking")]
    SetMarking(marking::SetMarkingParams),
    /// Print the schematic as a vector PDF.
    #[serde(rename = "schematic.export_pdf")]
    SchematicPdf(SchematicPdfParams),
    /// Where a package can be marked: model area, box faces, anchors.
    #[serde(rename = "library.marking_surfaces")]
    MarkingSurfaces(marking::MarkingSurfacesParams),
    /// Lean parts: evaluate and check a Solidcraft part file at values.
    #[serde(rename = "library.check_part_file")]
    CheckPartFile(part_files::CheckPartFileParams),
    /// Lean parts: a package draft uses a part file at values.
    #[serde(rename = "library.use_part_file")]
    UsePartFile(part_files::UsePartFileParams),
    /// Lean parts: new values for a package draft's part file.
    #[serde(rename = "library.set_part_values")]
    SetPartValues(part_files::SetPartValuesParams),
    /// Mechanical parts: the part files a project can place.
    #[serde(rename = "library.part_files")]
    PartFiles(NoParams),
    /// GP1: shipped and embedded generator modules and generated parts.
    #[serde(rename = "library.generators")]
    Generators(generators::GeneratorsParams),
    /// GP1: make a part with a generator module (`library.generate`).
    #[serde(rename = "library.generate_part")]
    GeneratePart(generators::GeneratePartParams),
    /// A placed part's package and pads, and the body a generator would
    /// give it fitted to them; changes nothing.
    #[serde(rename = "library.body_fit")]
    BodyFit(body::BodyFitParams),
    /// Give a placed part a generator's 3D body, its footprint kept
    /// (`library.replace_body`).
    #[serde(rename = "library.make_body")]
    MakeBody(body::MakeBodyParams),
    /// Mechanical parts: place a part file directly on the board.
    #[serde(rename = "layout.place_part_file")]
    PlacePartFile(Box<mechanical_parts::PlacePartFileParams>),
    /// Mechanical parts: new values (or flags) for a placed part.
    #[serde(rename = "layout.set_part_file_values")]
    SetPartFileValues(mechanical_parts::SetPartFileValuesParams),
    /// Board interface: what publishing it would change, per datum.
    #[serde(rename = "interface.publish_preview")]
    InterfacePreview(sparksmith_core::interface::PublishPreviewParams),
    /// Board interface: write the design an assembly links and its
    /// sidecar, then record the publish.
    #[serde(rename = "interface.publish")]
    InterfacePublish(sparksmith_core::interface::PublishParams),
    /// Assembly link: follow a linked design face or parameter.
    #[serde(rename = "assembly.follow")]
    AssemblyFollow(sparksmith_core::interface::assembly_link::FollowParams),
    /// Assembly link: what an update or relink would change.
    #[serde(rename = "assembly.preview")]
    AssemblyPreview(sparksmith_core::interface::assembly_link::PreviewParams),
    /// Assembly link: link, update or relink, as previewed.
    #[serde(rename = "assembly.update")]
    AssemblyUpdate(sparksmith_core::interface::assembly_link::UpdateParams),
    /// The text of pages of a document the host attached.
    #[serde(rename = "assist.document_text")]
    DocumentText(assist::DocumentTextParams),
    /// One page of a document the host attached, rendered as an image.
    #[serde(rename = "assist.document_page")]
    DocumentPage(assist::DocumentPageParams),
    /// Build a working pinout document in chunks behind a handle.
    #[serde(rename = "assist.pinout_edit")]
    PinoutEdit(Box<assist::PinoutEditParams>),
    /// Preview a working pinout document's import (`library.pinout_preview`).
    #[serde(rename = "assist.pinout_preview")]
    WorkingPreview(assist::WorkingPreviewParams),
    /// Import exactly the working pinout document last previewed.
    #[serde(rename = "assist.pinout_apply")]
    WorkingApply(assist::WorkingApplyParams),
    /// The workflows the host offers, ranked by a request's text.
    #[serde(rename = "assist.workflows")]
    Workflows(crate::workflow::entries::WorkflowsParams),
    /// A workflow model step's answer, checked against its output.
    #[serde(rename = "assist.step_result")]
    StepResult(crate::workflow::entries::StepResultParams),
    /// Show a workflow's card in the chat, with the inputs already known.
    #[serde(rename = "assist.offer")]
    Offer(crate::workflow::entries::OfferParams),
    /// Ask the person questions on one card in the chat; their answers are
    /// the reply.
    #[serde(rename = "assist.ask")]
    AskPerson(crate::workflow::ask::AskParams),
    /// What the window shows: workspace, view, sheet and selection.
    #[serde(rename = "view.context")]
    ViewContext(crate::view::ContextParams),
    /// Select and frame objects in the window.
    #[serde(rename = "view.select")]
    ViewSelect(crate::view::SelectParams),
    /// Show a workspace, a sheet, the PCB in 2D or 3D, or a draft.
    #[serde(rename = "view.open")]
    ViewOpen(crate::view::OpenParams),
    /// Turns: open one; its handle goes in the envelope's `turn`.
    #[serde(rename = "project.turn_begin")]
    TurnBegin(turns::TurnBeginParams),
    /// Turns: close one, with what it did.
    #[serde(rename = "project.turn_end")]
    TurnEnd(turns::TurnEndParams),
}

// A new request chooses its permission class here. `project.apply` and
// `project.query` take their operation's or query's; a request that names a
// path needs at least Ask ([`Request::effective_class`]).
sparksmith_core::declare_entries!(Request in project {
    Apply "project.apply" Edit "Apply an operation",
    ApplyAll "project.apply_all" Edit "Apply operations as one step",
    Inspect "project.inspect" Read "Inspect the project" [link],
    Schema "project.schema" Read "Look up a schema" [link],
    Query "project.query" Read "Run a query" [link],
    Undo "project.undo" Edit "Undo",
    Redo "project.redo" Edit "Redo",
    New "project.new" Ask "New project" [in files],
    Open "project.open" Ask "Open a project" [in files],
    Save "project.save" Ask "Save the project" [in files],
    PreviewUpdate "project.preview_update" Read "Preview an update" [in library, link],
    ApplyUpdate "project.apply_update" Edit "Apply an update" [in library, review],
    PreviewReplace "project.preview_replace" Read "Preview replacing a part's component" [in library, link, since(3, 8)],
    ReplaceComponent "project.replace_component" Edit "Replace a part's component" [in library, review, since(3, 8)],
    ExportNative "project.export_native" Ask "Export the 3D model" [in model3d, long, needs(Solidcraft)],
    ExportFabrication "project.export_fabrication" Ask "Export fabrication files" [in fab, long, link],
    Sources "library.sources" Read "List library folders" [in library, link, reach(Settings)],
    SourceAdd "library.source.add" Ask "Add a library folder" [in library, reach(Settings)],
    SourceCreate "library.source.create" Ask "Create a library folder" [in library, reach(Settings)],
    SourceRemove "library.source.remove" Ask "Remove a library folder" [in library, reach(Settings)],
    SourceEnable "library.source.enable" Ask "Turn a library folder on or off" [in library, reach(Settings)],
    Account "library.account" Read "Online library sign-in" [in library, reach(Settings), since(3, 14)],
    SignOut "library.sign_out" Ask "Sign out of the online library" [in library, short, reach(Network), reach(Settings), since(3, 14)],
    Rescan "library.rescan" Read "Rescan library folders" [in library, long, link, reach(Files)],
    Favourite "library.favourite" Never "Mark a favourite" [in library, reach(Settings)],
    Bundle "library.bundle" Read "Read a folder bundle" [in library, link, reach(Files)],
    ImportFrom "library.import_from_folder" Edit "Import from a folder" [in library, reach(Files)],
    PlaceFrom "library.place_from_folder" Edit "Place from a folder" [in library, reach(Files)],
    Publish "library.publish_to_folder" Ask "Publish to a folder" [in library, reach(Files)],
    ImportFromProject "library.import_from_project" Ask "Import from another project" [in library],
    ImportSvg "library.import_svg" Ask "Import SVG artwork" [in library],
    OpenPreview "library.open_preview" Read "Preview a folder entry" [in library, link, reach(Files)],
    PinoutSchema "library.pinout_schema" Read "Pinout document schema" [in library, link],
    ReviewCheck "library.review_check" Read "Check a datasheet review report" [in library],
    FirmwarePreview "firmware.export_preview" Read "Preview a firmware export" [in fab, link],
    FirmwareExport "firmware.export" Ask "Export firmware pin configuration" [in fab],
    InspectModelFile "library.inspect_model_file" Ask "Inspect a model file" [in library, link, needs(Solidcraft)],
    SetMarking "library.set_marking" Edit "Set a component marking" [in library],
    SchematicPdf "schematic.export_pdf" Ask "Export the schematic as PDF" [in fab],
    MarkingSurfaces "library.marking_surfaces" Read "List marking surfaces" [in library, link, needs(Solidcraft)],
    CheckPartFile "library.check_part_file" Read "Check a part file" [in model3d, link, needs(Solidcraft)],
    UsePartFile "library.use_part_file" Edit "Use a part file as the model" [in model3d, link, needs(Solidcraft)],
    SetPartValues "library.set_part_values" Edit "Set part file values" [in model3d, link, needs(Solidcraft)],
    PartFiles "library.part_files" Read "List part files" [in model3d, link],
    Generators "library.generators" Read "List generator modules" [in generators],
    GeneratePart "library.generate_part" Edit "Generate a part" [in generators],
    BodyFit "library.body_fit" Read "Fit a generated body to a part" [in generators, since(1, 5)],
    MakeBody "library.make_body" Edit "Make a part's 3D body" [in generators, since(1, 5)],
    PlacePartFile "layout.place_part_file" Edit "Place a part file on the board" [in layout, link, needs(Solidcraft)],
    SetPartFileValues "layout.set_part_file_values" Edit "Change a placed part file" [in layout, link, needs(Solidcraft)],
    InterfacePreview "interface.publish_preview" Read "Preview publishing the board interface" [in model3d],
    InterfacePublish "interface.publish" Ask "Publish the board interface" [in model3d],
    AssemblyFollow "assembly.follow" Ask "Follow assembly geometry" [in model3d],
    AssemblyPreview "assembly.preview" Read "Preview a linked design update" [in model3d],
    AssemblyUpdate "assembly.update" Ask "Update from the linked design" [in model3d],
    DocumentText "assist.document_text" Read "Read an attached document's text" [in assist, link, effect(World), since(1, 0)],
    DocumentPage "assist.document_page" Read "See a page of an attached document" [in assist, link, effect(World), since(1, 0)],
    PinoutEdit "assist.pinout_edit" Read "Edit a working pinout document" [in assist, effect(World), since(1, 0)],
    WorkingPreview "assist.pinout_preview" Read "Preview a working pinout document" [in assist, long, link, effect(World), since(1, 0)],
    WorkingApply "assist.pinout_apply" Edit "Apply a working pinout document" [in library, review, since(3, 2)],
    Workflows "assist.workflows" Read "List workflows" [in assist, effect(World), since(1, 1)],
    StepResult "assist.step_result" Read "Answer a workflow step" [in assist, effect(World), since(1, 1)],
    Offer "assist.offer" Show "Offer a workflow in the chat" [in assist, reach(Window), needs(Window), since(1, 2)],
    AskPerson "assist.ask" Show "Ask the person in the chat" [in assist, reach(Window), needs(Window), since(1, 3)],
    ViewContext "view.context" Read "What the window shows" [in view, reach(Window), needs(Window), since(1, 0)],
    ViewSelect "view.select" Show "Select in the window" [in view, reach(Window), needs(Window), since(1, 0)],
    ViewOpen "view.open" Show "Open a workspace or view" [in view, reach(Window), needs(Window), since(1, 0)],
    TurnBegin "project.turn_begin" Read "Begin a turn" [since(6, 2)],
    TurnEnd "project.turn_end" Read "End a turn" [since(6, 2)],
});

impl Request {
    /// The class the gate acts on: the one listed, or the operation's or
    /// query's for those two requests, and at least Ask once a path is named.
    pub fn effective_class(&self) -> Class {
        let own = match self {
            Self::Apply(params) => params.operation.class(),
            // At least an edit, so an empty batch is no read.
            Self::ApplyAll(params) => {
                Class::strictest(params.operations.iter().map(Operation::class)).max(Class::Edit)
            }
            Self::Query(query) => query.class(),
            other => other.class(),
        };
        if self.names_no_path() {
            own
        } else {
            own.max(Class::Ask)
        }
    }

    /// The `component.place` requests among this request's operations.
    pub fn placements(&self) -> Vec<&sparksmith_core::instances::NewInstance> {
        let operations: &[Operation] = match self {
            Self::Apply(params) => std::slice::from_ref(&params.operation),
            Self::ApplyAll(params) => &params.operations,
            _ => &[],
        };
        operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Instance(sparksmith_core::InstanceCommand::Place(new)) => Some(new),
                _ => None,
            })
            .collect()
    }

    /// What the gate reports as requested: the operation or query inside
    /// `project.apply` and `project.query`, else the request itself.
    pub fn subject(&self) -> &'static str {
        match self {
            Self::Apply(params) => params.operation.command(),
            // The strictest member speaks for the batch.
            Self::ApplyAll(params) => {
                let class = Class::strictest(params.operations.iter().map(Operation::class));
                params
                    .operations
                    .iter()
                    .find(|operation| operation.class() == class)
                    .map_or("project.apply_all", Operation::command)
            }
            Self::Query(query) => query.command(),
            other => other.command(),
        }
    }

    /// Every file or folder path this request names: each value its
    /// parameters type as a [`FilePath`], found by walking the request with
    /// its schema ([`paths`]). A request that cannot be written as JSON is
    /// refused rather than read as naming none.
    pub fn paths(&self) -> std::result::Result<Vec<String>, serde_json::Error> {
        Ok(paths::in_request(&serde_json::to_value(self)?))
    }

    /// Whether this request certainly names no path. A request whose paths
    /// cannot be read counts as naming one.
    pub fn names_no_path(&self) -> bool {
        self.paths().is_ok_and(|paths| paths.is_empty())
    }

    /// The entries the gate weighs for this request: the operations inside
    /// `project.apply` and `project.apply_all` and the query inside
    /// `project.query`, each in its own module, else the request itself.
    pub fn subjects(&self) -> Vec<crate::gate::Subject<'static>> {
        fn of(name: &'static str, class: Class) -> crate::gate::Subject<'static> {
            crate::gate::Subject {
                name,
                module: module_of(name),
                class,
                reach: metadata::declared_reach(name),
            }
        }
        match self {
            Self::Apply(params) => vec![of(params.operation.command(), params.operation.class())],
            // At least an edit, so an empty batch is no read.
            Self::ApplyAll(params) if params.operations.is_empty() => {
                vec![of(self.command(), Class::Edit)]
            }
            Self::ApplyAll(params) => params
                .operations
                .iter()
                .map(|operation| of(operation.command(), operation.class()))
                .collect(),
            Self::Query(query) => vec![of(query.command(), query.class())],
            other => vec![of(other.command(), other.class())],
        }
    }

    /// Every request ID with its one-line summary, description and parameter
    /// schema, generated from the Rust types the dispatcher deserializes.
    /// The summary says what the command is for in its first words, within
    /// [`metadata::MAX_SUMMARY`] bytes; the description adds the detail.
    pub fn catalog() -> Vec<CommandInfo> {
        fn info<T: JsonSchema>(
            id: &'static str,
            summary: &'static str,
            description: &'static str,
        ) -> CommandInfo {
            let mut params = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);
            let takes_path = paths::marks_path(&params);
            // Callers see the schema as before; the mark is the gate's.
            paths::unmark(&mut params);
            let decl = sparksmith_core::permission::find(Request::ENTRIES, id);
            CommandInfo {
                id,
                // Every listed request has one; the metadata tests check.
                title: decl.map_or(id, |decl| decl.title),
                summary,
                description,
                // Unlisted cannot pass the gate's tests; refused until then.
                class: decl.map_or(Class::Never, |decl| decl.class),
                takes_path,
                params,
                decl,
            }
        }
        vec![
            info::<ApplyParams>(
                "project.apply",
                "Apply one typed document operation at expected_revision; a stale revision or invalid input changes nothing.",
                "Apply one typed document operation (library, instance including instance.assign_part, schematic, layout including layout.set_fabrication, layout.set_stackup, layout.edit_stackup and layout.add_via, or update) at an expected revision and return its outcome. Fails without changes on stale revisions or invalid input.",
            ),
            info::<ApplyAllParams>(
                "project.apply_all",
                "Apply typed document operations in order at expected_revision as one undo step; any failure changes nothing.",
                "Apply one to 256 typed document operations, in order, at an expected revision as one undo step and one revision, and return their outcomes in order. Each is checked against the document the ones before it made; a stale revision or any failing operation (named by its place) changes nothing.",
            ),
            info::<InspectParams>(
                "project.inspect",
                "Read the revision, object counts, drafts and open findings; at reads one part of the document.",
                "Return the revision, object counts, each draft's state and open findings by code. verbose: true returns the whole snapshot (project, drafts with their publish findings, ERC and board-check findings; about 0.5 MB on a small board); at: a JSON pointer into it returns just what it names, with lists indexed by position: /project/vias, /project/nets, /drafts (each {draft, check}), /drafts/0/draft/definition/body/pads/0. A pointer that names nothing is refused with the valid keys or indices where it stopped.",
            ),
            info::<SchemaParams>(
                "project.schema",
                "Look up one operation, query or command's exact schema and how to send it; without a name, the index.",
                "The exact JSON schema of one project.apply operation (layout.add_via), project.query query (library.search) or command, with how to send it; large definitions come separately through def. Without a name, the index of all of them with one line each. Not a document change.",
            ),
            info::<Query>(
                "project.query",
                "Answer one read-only query by name, such as project.issues, library.search or component.inspect.",
                "Answer one read-only query. component.inspect: an instance's component pin, resolved symbol and implementation, unit states, effective fields with origin, active and unresolved terminal functions, occurrence and placement counts, completeness findings. component.check: a revision's publish findings and the stage each blocks (component.check_drafts: all drafts). library.search: latest published lineages of every ready source ranked for a one-line query (free text, pkg:/has:/missing: filters, unit-aware predicates on the project's fields: res>=10k, tol<=1%, C:1n..100n; a parse error names the term). library.facets: the matches' fields, package labels and badges. library.inspect: one lineage revision with revisions, symbols, implementations, badges. project.bom: resolved instances by Part, unresolved marked. library.footprint_preview: what library.edit_footprint would make of a footprint draft. library.resolve_groups: Part-less instances sharing revision, footprint and requirements, each with a library.search query. library.updates_available: lineages whose source holds a newer revision or modified record. library.promote_preview: what library.promote would write. project.issues: issues in scope (all, component, schematic, board or a selection), most severe first: severity (error: blocks a stage or breaks a rule; warning; to_do: unrouted, unplaced, unpurchased; info), group, title, cause, fix, finding, same-cause findings (also); elsewhere counts the rest.",
            ),
            info::<RevisionParams>(
                "project.undo",
                "Undo the last document change at expected_revision.",
                "Undo the last document change at expected_revision. Refused without changes on a stale revision, so a newer change by someone else is never undone.",
            ),
            info::<RevisionParams>(
                "project.redo",
                "Redo the last undone document change at expected_revision.",
                "Redo the last undone document change at expected_revision. Refused without changes on a stale revision.",
            ),
            info::<NewParams>(
                "project.new",
                "Start a new empty project at expected_revision with its board's fab profile, clearing history.",
                "Start a new empty project at expected_revision; clears history. profile is the board's fab profile: a profile library id (project.query rules.fab_profiles lists them) or a whole profile file's profile; omitted, the Settings default, else the shipped default. Refused without changes on a stale revision.",
            ),
            info::<OpenParams>(
                "project.open",
                "Open a .sparksmith project file in place of the document at expected_revision.",
                "Open a .sparksmith project file of the current format version in place of the document at expected_revision; other versions are refused. Failure, a stale revision included, leaves the session unchanged.",
            ),
            info::<SaveParams>(
                "project.save",
                "Save the project atomically to a .sparksmith file, by default its current path.",
                "Atomically save to a .sparksmith file (defaults to the current path). Not undoable.",
            ),
            info::<PreviewParams>(
                "project.preview_update",
                "Preview a component-revision, instance, schematic or board-sync change and its routed-copper impact.",
                "Preview a revision-bound component-revision, instance, schematic or PCB-sync change and its routed-copper impact.",
            ),
            info::<updates::ApplyUpdate>(
                "project.apply_update",
                "Apply a previewed update at its revision, optionally removing the listed conflicting tracks.",
                "Apply a previewed update at its revision, optionally removing the listed conflicting tracks.",
            ),
            info::<replace::PreviewReplaceParams>(
                "project.preview_replace",
                "Preview replacing placed parts' component by a component of another lineage, with an explicit terminal mapping.",
                "Preview replacing the component of the placed parts designators (one component revision) by target, a component revision of another lineage the project holds (import a folder part first with library.import_from_folder), with representation and physical (binding and footprint) when the target offers several. terminals maps every terminal of the current component, by id (a number) or exact name (a string), to a terminal of the target or to null to drop it; units maps placed units when a symbol has several. Returns plan (the project.preview_update plan at this revision: nets, wires, pads, detached tracks and vias kept for rerouting, conflicting copper, findings, blockers such as an incomplete mapping) with copper (per part: identical, the old-only and new-only pads, renumbered pads), both components' terminals, and suggested_terminals by equal names, offered only and never applied by itself. Not a document change.",
            ),
            info::<replace::ReplaceComponentParams>(
                "project.replace_component",
                "Replace placed parts' component by another lineage's at expected_revision, terminals mapped explicitly; one undo step.",
                "Apply what project.preview_replace previewed (replace holds the same parameters, terminals complete) at expected_revision as one undo step: the parts pin the target revision with its symbol and footprint at the same placement; wires, nets, no-connect markers and pair ends follow the terminal mapping; dropped terminals lose their wires; field overrides the target does not select, function selections, pin configuration and the exact Part are cleared. Tracks and vias are never moved or rerouted: copper the new pads no longer reach stays for rerouting, listed in warnings. Refused, changing nothing, with an incomplete or ambiguous mapping, a stale revision, a blocker, conflicting copper without remove_conflicting_tracks, or joined named nets without merge_named_nets. The reply adds copper, warnings and summary.",
            ),
            info::<ExportNativeParams>(
                "project.export_native",
                "Export the board as a native parametric .solidcraft 3D model to a new file.",
                "Write the board plan as a native parametric .solidcraft model to a new file (refused if it exists): tier mechanical (default: laminate, pads, parts) or full (adds every track, via and fill). Each flat body's faces are checked against the plan's outline, holes and heights first; an unfit model is refused (export_unfit, naming each body, nothing written) unless write_anyway. Returns the tier, bodies, check and build/check/save ms. Not a document change.",
            ),
            info::<FabricationParams>(
                "project.export_fabrication",
                "Check the board for production, then write Gerber, drill, pick-and-place, BOM and drawing files.",
                "Run fabrication.preflight under the chosen fab profile, then write the production files to a new or empty folder or one new .zip: Gerber X2 copper, mask, paste, legend and profile layers with file, aperture and object attributes; plated and non-plated Excellon drills with routed slots, drill map and table; a Gerber job file; pick-and-place and BOM CSVs; fabrication and assembly drawings (PDF). Refused, writing nothing, while unwaived errors remain (waive with project.apply fabrication.waive). Returns the preflight report, files with sizes and step timings. Not a document change.",
            ),
            info::<NoParams>(
                "library.sources",
                "List the configured library folders with their state, or why one is disabled.",
                "List the configured library folders in settings order: path, enabled, writable, state (ready with library id, name, entry and revision counts and how the index was obtained; disabled with the reason, e.g. path missing or unsupported format v2; failed with a diagnostic). Not a document change.",
            ),
            info::<SourceAddParams>(
                "library.source.add",
                "Add a library folder as a source in the person's settings.",
                "Add a library folder (sparksmith-library.json manifest) as a source in the per-user settings. A missing folder, a malformed manifest, another format version, an unknown required feature, a repeated path or a repeated library id is refused and nothing changes. library.search and library.facets then include its entries (source library, with source_lineage).",
            ),
            info::<SourceCreateParams>(
                "library.source.create",
                "Create an empty library folder and add it as a writable source.",
                "Create an empty library folder and add it as a writable source. Its library id is random unless library supplies one (a recorded session passes the id it recorded, so a replay reproduces the folder).",
            ),
            info::<PathParams>(
                "library.source.remove",
                "Remove a library source from the settings; the folder stays untouched.",
                "Remove a library source from the settings. The folder and every project copy are untouched.",
            ),
            info::<SourceEnableParams>(
                "library.source.enable",
                "Turn a library source on or off.",
                "Turn a library source on or off; an off source stays listed as disabled (turned off).",
            ),
            info::<NoParams>(
                "library.account",
                "Name the online library and who is signed in to it; downloading parts needs a free account, searching does not.",
                "Name the online library (its host) and whether this computer is signed in to it, with the account's email and name once the registry said them. Read from this computer's sign-in, without the network. Searching works without an account; placing or importing a part from the online library downloads its data, which needs a free account: the person signs in in Settings › Libraries. Without a registry turned on, registry is null.",
            ),
            info::<NoParams>(
                "library.sign_out",
                "Sign out of the online library: the registry revokes the token and this computer forgets it.",
                "Sign out of the online library: the registry revokes this computer's token (best effort: offline, revoked is false and problem says why) and the token is removed from the system keychain. Searching keeps working; downloading parts needs signing in again in Settings. Refused without a registry turned on.",
            ),
            info::<NoParams>(
                "library.rescan",
                "Re-read every library source's manifest and index, then list the sources.",
                "Re-read every source's manifest and catalog index (after a git pull or another writer), then list the sources.",
            ),
            info::<FavouriteParams>(
                "library.favourite",
                "Mark or unmark a library lineage as a favourite, which ranks search results.",
                "Mark or unmark a lineage, keyed by project lineage or by library id and source lineage, as a favourite. Favourites rank library.search results and persist per user, never in a project.",
            ),
            info::<BundleParams>(
                "library.bundle",
                "Read one revision of a folder lineage with its dependency closure and packed assets.",
                "Read one revision of a folder lineage (default the newest) with its complete dependency closure and packed assets, as the bundle library.import and library.place_from carry.",
            ),
            info::<ImportFromParams>(
                "library.import_from_folder",
                "Import a folder lineage's revision and its closure into the project in one undo step.",
                "Read a folder bundle and apply library.import with it at the expected revision: the closure is copied into the project under fresh project lineages, or appended as the next project revision of a lineage already imported from the same source; an equal origin is reused. One undo step; places nothing. Follow with project.preview_update to move instances to the new revision.",
            ),
            info::<ImportFromProjectParams>(
                "library.import_from_project",
                "Copy one published revision and its closure from another .sparksmith file into this project.",
                "Copy one published revision of another .sparksmith file (never changed; root: its kind, id and revision there) and its full closure, packed assets included, into this project as one library.import at expected_revision: one undo step, nothing placed. Each copy records its origin (exact source revision, content hash); every pin keeps naming the exact revision it names there; copies are numbered in import order. Ids derive from the file's content, so replays are reproducible and importing again reuses the copies (no undo step); a newer revision of a copied lineage becomes its next project revision. A draft in that file cannot be imported: publish it there first. An unknown root lists what the file holds. Refused without changes on a stale revision, a dangling reference, a model that may not be redistributed, or a revision that differs from the copy already held (library.source_modified).",
            ),
            info::<PlaceFromParams>(
                "library.place_from_folder",
                "Import a folder component if needed and place it, in one undo step.",
                "Read a folder bundle of a component lineage and apply library.place_from with it: import if needed, then place, as one transaction and one undo step. When the project holds another revision of that lineage, other_revision refuse (default) fails with code library.other_revision and keep_both imports the folder revision beside it.",
            ),
            info::<PublishParams>(
                "library.publish_to_folder",
                "Promote a published project revision and its closure to a writable library folder.",
                "Promote a published project revision and its closure to a writable library source: compute library.promote_preview with fresh lineage ids (random unless fresh supplies them, as a recorded session does so that a replay reproduces the folder and origins; too few refuses before anything is written), write the records and assets to the folder (an existing rNNNN.json with other content refuses the write before anything is written), then apply library.promote, which records the origins as one undo step. Returns the written and unchanged record paths.",
            ),
            info::<ImportSvgParams>(
                "library.import_svg",
                "Add a .svg file's drawing to a symbol unit or footprint layer as editable graphics.",
                "Add a .svg file's drawing (at most 1 MiB) to a symbol unit or a footprint's silkscreen or assembly layer as editable graphics, centred at `at`, scaled from the SVG's millimetre size: circles, arcs and axis-aligned rectangles stay exact, tangent arcs become corner radii, Béziers and ellipses flatten within tolerance_mm, closed fills stay solid, strokes keep their width. Fills with holes become outlines; text, images and use references are skipped (import.notes). One library.edit_graphics undo step at expected_revision; the outcome lists the new graphic IDs.",
            ),
            info::<BundleParams>(
                "library.open_preview",
                "Inspect one revision of a folder lineage without importing it.",
                "Inspect one revision of a folder lineage (default the newest) without importing it: the same report library.inspect gives a project item. Not a document change.",
            ),
            info::<NoParams>(
                "library.pinout_schema",
                "The schema of the pinout document an agent fills from a datasheet, with its pin-rule API.",
                "The JSON schema of the typed pinout document (format 1) an agent fills from a datasheet: device, package variants, peripherals with roles, pins with explicit numbers per variant, GPIO, options, functions with explicit peripheral, role and mux code, source page, uncertain flags, and Rhai pin-rule scripts. Preview with project.query library.pinout_preview, show the review items to the user, then apply with project.apply library.pinout_apply; never publish without the user seeing the unreviewed count.",
            ),
            info::<crate::review::ReviewCheckParams>(
                "library.review_check",
                "Check a datasheet part's review report against the drafts: citations, confidence and open questions.",
                "Check a datasheet part's review report (the datasheet_to_component prompt's last step) against the project's drafts: every value with a listed document, page, location (table, figure or section) and confidence (high: a table cell or dimensioned drawing; medium: derived or ambiguous; low: inferred, measured or not stated); distinct sources; open questions when any value is low confidence or any imported pin is uncertain; a component draft; and a source page on every imported function. Returns accepted, the problems, each draft's blocking and warning codes, unreviewed and uncertain pin counts and the report as Markdown for the person. Accepted means ready for a person's review, never verified or publishable. Not a document change.",
            ),
            info::<firmware::FirmwareParams>(
                "firmware.export_preview",
                "Emit an instance's firmware pin configuration as JSON, C, a Zephyr overlay, Rust or a template.",
                "Emit one instance's firmware pin configuration (project.pin_config) as JSON, a C header, a Zephyr overlay skeleton, Rust consts or a user template. Pin configuration only; a starting point, never a claim to configure clocks or registers.",
            ),
            info::<firmware::FirmwareParams>(
                "firmware.export",
                "Write a firmware pin-configuration export to a new file.",
                "Write firmware.export_preview's text atomically to a new file at path. Not a document change.",
            ),
            info::<marking::SetMarkingParams>(
                "library.set_marking",
                "Set or clear a component draft's display-only 3D marking: text, artwork or polarity marks.",
                "Set or clear a component draft's marking: display-only decals on the 3D model, never in .solidcraft or fabrication output. Items: template text ({mpn}, {designator}, {field:<id>}, {var:lot}), SVG artwork with its licence (packed by SHA-256; no logo you have no right to: a placeholder box marks the user's own), polarity marks, a QR code (code2d: payload template, size_mm, anchor), an electrolytic's µF/V text and stripe. placement.on: model (top marking area), a box face (top, bottom, pos_x, neg_x, pos_y, neg_y) or a library.marking_surfaces anchor (e.g. can_top, pcb_top, sleeve, body); then offset_mm, rotation_deg, scale, mirror, clip and, on a cylinder, wrap {seam_deg, wrap_deg, along_mm, repeat}. finish: laser_etch, laser_etch_metal, ink_white, {ink: {colour}}, {pad_print: {colour}}, engraved. decals: up to four more (items, finish, placement). One undo step at expected_revision; refused whole on any problem.",
            ),
            info::<marking::MarkingSurfacesParams>(
                "library.marking_surfaces",
                "List where a package can be marked: the model area, box faces and named anchors.",
                "List where a package draft or published package can be marked, for library.set_marking's placement.on: the model area, the six box faces and its family's named anchors (each with a description, plane or cylinder, and size; a cylinder's circumference and whether its u runs along the axis). Not a document change.",
            ),
            info::<SchematicPdfParams>(
                "schematic.export_pdf",
                "Print the schematic as a vector PDF, one page per sheet.",
                "Print the schematic as a vector PDF at path (written atomically, replacing a file there): one page per sheet in page order at its paper size, with the page frame and title block; sheets limits it to a selection (all when empty) and a sheet symbol links to its child sheet's page. Returns the page count. Not a document change.",
            ),
            info::<part_files::CheckPartFileParams>(
                "library.check_part_file",
                "Evaluate a SolidCraft part file at given parameter values and report its convention check.",
                concat!(
                    "Evaluate a SolidCraft part file (path to a .solidcraft design, or builtin: a listed name) at literal values for some of its user parameters ({\"can_height\": {\"mm\": 2.465}}; units mm, deg, count, number) and report the convention check (docs/part-templates.md §11): parameters and values, bodies with the ",
                    sparksmith_core::product_name!(),
                    " material each appearance names, marking surfaces (marking-<name> planes sized by marking_<name>_u/_v), package datums (opening-<name> planes sized by <datum>_u/_v, optional <datum>_r, _depth, _clearance, normal pointing out along the plug's way in; axis-<name> axes, optional <datum>_d), pin1, bounds, timing and part_file.* findings (read, structure, value, feature, seating, material, marking, datum block; origin, no_marking, import, slow advise). Lists the material appearance names. Not a document change."
                ),
            ),
            info::<part_files::UsePartFileParams>(
                "library.use_part_file",
                "Make a SolidCraft part file a package draft's 3D model, at literal parameter values.",
                "Make a SolidCraft part file (path with its licence, or a builtin) a package draft's 3D model at literal parameter values: checked, packed as a solidcraft Model (or the same file's reused) and set as the package's model with the evaluated bounds, marking surfaces and declared datums (the package's, marked as the part file's), in one undo step at expected_revision; refused whole on a blocking finding. Footprint, terminals and keep-outs stay the package's. Returns the outcome and the check report.",
            ),
            info::<part_files::SetPartValuesParams>(
                "library.set_part_values",
                "Set new parameter values for the part file a package draft uses.",
                "Replace the literal values of the part file a package draft uses, re-evaluated and checked, in one undo step at expected_revision; refused whole on a blocking finding. Returns the outcome and the check report.",
            ),
            info::<NoParams>(
                "library.part_files",
                "List the SolidCraft part files this project can place or use.",
                "List the SolidCraft part files this project can place or use (docs/mechanical-parts.md): the built-ins (name, title, summary, designator prefix, kind pads, mechanical or package, parameters with units and defaults, licence) and the project's Models of format solidcraft (model, name, file, licence). Not a document change.",
            ),
            info::<generators::GeneratorsParams>(
                "library.generators",
                "List generator modules (shipped and embedded) and the project's generated parts; generator: <name> for one in full.",
                "List the generator modules that ship with the program, filtered by generator (a name), family or text: by default each by name, title, families, scopes, version and preset keys (with family or text, also their names); with generator or detail: true in full (module SHA-256, parameters with types, defaults and the project setting that fills them, rules, presets with their values), the modules this project embeds, and its generated parts, each with its part key (module and parameters hashes), result hash, generator version and state: current (the shipped module of its generator is this one or the same version), update_available (a newer version ships: library.generate_part with the same parameters makes the next revision of the part), shipped_older (the shipped version is older; the part is never downgraded), superseded (a newer version already made the next revision of this part) or not_shipped (the embedded copy is the only one). Not a document change.",
            ),
            info::<generators::GeneratePartParams>(
                "library.generate_part",
                "Make a part (component or package) with a generator module, e.g. a chip resistor 0603; requires scope and generator or module.",
                "Make a part with a generator module, through library.generate in one undo step at expected_revision: generator (a shipped name) or module (an embedded SHA-256), scope component (component, symbol, package, footprint, binding) or package, an optional preset and params (lengths in integer nm, angles in integer millidegrees; the rest from project settings and the module's defaults). The module is embedded once by hash, so the project reopens without the generator folder; a part the project already holds with the same key, or that the same generator version made from the same parameters, is returned unchanged (created: false). A newer version of the generator that made a part with the same parameters makes the next revisions of the same lineages (replaced names the old root), and the reply carries update, the project.preview_update plan that moves the placed instances (apply it with project.apply_update); an older version is refused (generated.older_version). Refusals name the module's own problems (param.*) or the host's (generator.*); nothing changes.",
            ),
            info::<body::BodyFitParams>(
                "library.body_fit",
                "A placed part's package and pads by designator; with a generator, the 3D body it makes fitted to those pads, changing nothing.",
                "Describe the placed part designator: its package (terminals, envelope, current model), the other parts that use it, and its footprint's pads in the package's datum frame (mm). With generator (a shipped name), preset and params (integer nm and millidegrees) the generator's package-scope body is evaluated and its leads fitted onto those pads (pins maps a lead to another pad number; tolerance in mm, 0–0.5): the reply is the plan library.make_body applies, accepted only when every lead lands on its pad (the contact on the land, within the middle half across it, plus tolerance) on every footprint the parts use, with each lead's miss, the pads no lead lands, the placement, the body before and after, blocking findings and change counts. missing instead of a generator refuses with that reason. Not a document change.",
            ),
            info::<body::MakeBodyParams>(
                "library.make_body",
                "Give a placed part a generator's 3D body fitted to its existing footprint; every part that uses the package changes.",
                "Make the body library.body_fit plans and apply it through library.replace_body in one undo step at expected_revision: the package's next revision takes only the generator's model, its placement on the pads, its envelope and lead contacts; footprints, bindings and components are republished pinning it, unchanged otherwise, and every placed part that used the package moves to them. sources (the datasheets with the pages read) is recorded with the package and its model as AI-made and unreviewed. Refused, changing nothing, when a lead misses its pad, when a lineage has a draft or a newer revision, or on the generator's own refusal.",
            ),
            info::<mechanical_parts::PlacePartFileParams>(
                "layout.place_part_file",
                "Place a parametric 3D part such as a nut, standoff, spacer or heatsink on the board.",
                "Place a parametric 3D part (SMD nut or standoff, spacer, heatsink, bracket: path with its licence, builtin or model) on the board at literal parameter values. The file is checked (blocking findings refuse) and evaluated; lands come from lands, lands_from or a built-in's land rules, else none. With pads terminals, each copper land can join a net via the part's symbol on a sheet. bom defaults to true, pick_and_place to having copper lands. The variant (file, values, lands, flags) is reused or made as a component set with a generated footprint and courtyard, and an instance placed with its symbol unit off-sheet, in one undo step at expected_revision. Returns the outcome (PartPlaced: instance, designator, component, created) and the check report.",
            ),
            info::<mechanical_parts::SetPartFileValuesParams>(
                "layout.set_part_file_values",
                "Give a placed mechanical part new parameter values, pads or BOM flags.",
                "Give a placed mechanical part new literal values (they replace the previous values) and optionally new pads, bom or pick_and_place: re-evaluated and checked, a built-in's lands re-derived (other lands kept), and the instance moved to the variant for them (reused or made), keeping its designator, placement and marking, in one undo step at expected_revision. Returns the snapshot, the outcome and the check report.",
            ),
            info::<sparksmith_core::interface::PublishPreviewParams>(
                "interface.publish_preview",
                "Preview publishing the board interface: each exported datum unchanged, moved (by how much), reshaped, added or removed.",
                "What interface.publish would change for a linked design: each exported datum in slot order, unchanged, moved (offset in mm and turn), reshaped, added, removed (a linked design feature using it will fail) or with a changed status, with the user's sentence (\"J1 opening, 2 mm along X\"); the revision publishing would record and its hash. path: the published .solidcraft design, whose sidecar says where each datum was (else a change reads only as changed). tier: interface, mechanical (default, or the last published) or full. Not a document change.",
            ),
            info::<sparksmith_core::interface::PublishParams>(
                "interface.publish",
                "Publish the board interface: write the .solidcraft design an assembly links and its sidecar, atomically.",
                "Write the board interface at path: a .solidcraft design whose slots carry every exported datum as named reference bodies with stable face names (plus laminate, pads and parts for tier mechanical, the default; all copper for full) and its sidecar <name>.interface.json, both atomically. Records the publish (revision, hash, per-datum hashes) at expected_revision; no undo step, like a save. A stale revision, another board's files there or a broken reference writes nothing. The revision grows only when datums changed. For mechanical and full, published.check is project.export_native's plan check.",
            ),
            info::<sparksmith_core::interface::assembly_link::FollowParams>(
                "assembly.follow",
                "Make the board follow a linked design face or length parameter.",
                "body, face, pick: its trace becomes fixed board-sketch geometry to dimension curves to; or board_param, assembly_param. One undo step.",
            ),
            info::<sparksmith_core::interface::assembly_link::PreviewParams>(
                "assembly.preview",
                "Preview updating the board from the linked design: what moves, what is lost.",
                "Read the linked design at path (linked, or a new place) on a copy: references and board edges, holes and parts that would move, and expect_hash. Not a document change.",
            ),
            info::<sparksmith_core::interface::assembly_link::UpdateParams>(
                "assembly.update",
                "Link the linked design, or update or relink the board from it as previewed.",
                "Read the SolidCraft assembly at path whose component links design (interface.publish's path). None linked: link it. Else apply it at expect_hash, relinking if path moved; one undo step.",
            ),
            info::<crate::view::ContextParams>(
                "view.context",
                "What the window shows: workspace, PCB view, sheet and the selection with ids.",
                "What the open window shows now: the workspace (component, schematic or pcb), the PCB's view (layout or 3d), the schematic sheet, and the selection as objects with ids and their designators or net names, with the revision. Needs the window (mcp --live); a host without one answers host.unavailable.",
            ),
            info::<crate::view::SelectParams>(
                "view.select",
                "Select and frame objects in the window by id; never the document, no undo step.",
                "Replace the window's selection with the objects named (ids: parts, tracks, vias, fills, keepouts, wires; objects: any finding's object, such as a net, pad or sheet) and frame them, in the shown workspace when it shows them, else in one that does. No revision, no undo step, no rebuild. Refused while the person is busy in the window (live.busy).",
            ),
            info::<crate::view::OpenParams>(
                "view.open",
                "Show a workspace: the PCB in layout or 3d, a schematic sheet, or a component draft.",
                "Show workspace component, schematic or pcb; view layout or 3d for the PCB, sheet for the schematic, definition (a draft's lineage) for the Component editor. Answers what the window then shows, as view.context. No revision, no undo step, no rebuild. Refused while the person is busy in the window (live.busy).",
            ),
            info::<models::ModelFileParams>(
                "library.inspect_model_file",
                "Read a STEP, GLB or .solidcraft model file and report its bodies, materials and bounds.",
                "Read a STEP AP214/242, GLB or .solidcraft model file and report its bodies (keys, source colours, glTF materials), bounds and whether it has textures, for the import wizard's explicit mapping. Not a document change.",
            ),
            info::<assist::DocumentTextParams>(
                "assist.document_text",
                "Read the text of pages of a document attached to this run, by its title.",
                "The text taken from pages of a document the host attached to this session (an assistant run's datasheets), each marked === page N ===: every page, or pages such as 3-5 or 1, 4. By title, never by path; a session without attached documents refuses it.",
            ),
            info::<assist::DocumentPageParams>(
                "assist.document_page",
                "See one page of a document attached to this run, rendered as an image.",
                "One page of an attached document rendered as a PNG (drawings, pin diagrams and land patterns are not in the text), for a model declared with vision; a run may view a limited number of pages.",
            ),
            info::<assist::PinoutEditParams>(
                "assist.pinout_edit",
                "Build a working pinout document in chunks behind a handle; the project does not change.",
                "Set the working pinout document's header fields (format, device, variants), upsert peripherals and rules by label and pin rows by row key (at most 40 rows a call), remove rows or rules; clear starts over, new starts another document. All or nothing. Answers with the handle, the counts and per variant the pin numbers still missing. The handle lasts as long as the session.",
            ),
            info::<assist::WorkingPreviewParams>(
                "assist.pinout_preview",
                "Preview importing the working pinout document: changes, findings, review items and rule checks.",
                "library.pinout_preview of the working pinout document (a document given replaces it) on target {target: new, name} or {target: draft, id}: change counts, findings, review items and each rule's evaluation, and the base assist.pinout_apply imports by. Changes nothing in the project.",
            ),
            info::<assist::WorkingApplyParams>(
                "assist.pinout_apply",
                "Import exactly the working pinout document last previewed, as one undo step.",
                "library.pinout_apply of the working pinout document as it was last previewed, by that preview's base (leave base out, or give it exactly; a rounded number is refused): one undo step, imported rows unreviewed. Refused when the working document changed since the preview.",
            ),
            info::<crate::workflow::entries::WorkflowsParams>(
                "assist.workflows",
                "List the workflows this host offers: refined processes for a request, with their inputs.",
                "The workflows the host lent this session (shipped and the person's own), ranked by text against each one's title, summary and example requests: id, version, title, summary, intents, inputs and, with unavailable, those that cannot run here and why. Not a document change.",
            ),
            info::<crate::workflow::entries::StepResultParams>(
                "assist.step_result",
                "Answer the workflow step you are working on; the answer is checked against the step's output.",
                "End the open model step of a workflow run with its answer, an object with the fields of the step's output. Answers accepted, or the problems by field (each a pointer into result) with an example; fix them and answer again. A session with no open step refuses it. Not a document change.",
            ),
            info::<crate::workflow::entries::OfferParams>(
                "assist.offer",
                "Offer a workflow in the chat as one card with every input the conversation gives; the person accepts and continues, or just chats.",
                "Show the workflow's card in the window's chat: its title and summary, why it fits (one sentence), the inputs given (each checked against the workflow's input, changed in place by the person; the latest files the person attached fill its files inputs) and an inline field for each required one still missing. Answers the card's id and the missing inputs; the person's answer comes as their next message. Nothing starts until they choose Accept and continue. Not a document change; a host without a window answers host.unavailable.",
            ),
            info::<crate::workflow::ask::AskParams>(
                "assist.ask",
                "Ask the person what you need to know, as one card of questions in the chat; their answers are the reply.",
                "Show one card of questions in the window's chat and wait for the person. Each question has an id, a short title, the question in plain words (text) and a type: single (options, each with value, title and a short plain about; one recommended), multiple (options; recommended values), number (unit, min, max, recommended) or text (recommended). Recommended values are chosen until the person changes them. The reply is the card's id and answers by question id; or said, when the person answered in their own words in the chat; or skipped, when they left it to you: choose sensibly and say what you chose. Not a document change; a host without the window's chat answers host.unavailable.",
            ),
            info::<turns::TurnBeginParams>(
                "project.turn_begin",
                "Open a turn: calls naming its handle in turn undo as one step under label.",
                "Open a turn and return its handle. Every call whose envelope names it in turn joins one undo step that reads label; an edit by anyone else in between splits it. Not a document change.",
            ),
            info::<turns::TurnEndParams>(
                "project.turn_end",
                "Close a turn.",
                "Close the turn turn and report its handle and label. Not a document change.",
            ),
        ]
    }
}

/// The module the entry `name` is declared in: a request's, an
/// operation's or a query's. None for a name no declaration has, which no
/// grant reaches.
fn module_of(name: &str) -> &'static str {
    Request::ENTRIES
        .iter()
        .chain(Operation::entries())
        .chain(Query::ENTRIES)
        .find(|decl| decl.name == name)
        .map_or("", |decl| decl.module)
}

impl Request {
    /// [`Request::catalog`], generated once.
    pub fn catalog_shared() -> &'static [CommandInfo] {
        static CATALOG: std::sync::OnceLock<Vec<CommandInfo>> = std::sync::OnceLock::new();
        CATALOG.get_or_init(Request::catalog)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    /// A short name for people (the MCP tool `title`).
    pub title: &'static str,
    /// One line: what the command is for, without the detail.
    pub summary: &'static str,
    pub description: &'static str,
    pub class: Class,
    /// Whether it can name a path, which makes it at least Ask: its
    /// parameters hold a [`FilePath`] ([`Request::paths`]).
    pub takes_path: bool,
    pub params: Value,
    /// Its declaration beside [`Request`].
    #[serde(skip)]
    pub decl: Option<&'static sparksmith_core::permission::EntryDecl>,
}

/// A headless editing session: one engine, the current file path and the
/// library sources.
pub struct Session {
    /// One-shot boundary hook owned only by the test's session.
    #[cfg(test)]
    after_interface_read: Option<Box<dyn FnOnce() -> crate::Result<()> + Send>>,
    engine: Engine,
    path: Option<PathBuf>,
    sources: sources::Sources,
    /// Set while [`Session::respond`] runs a request: mutations answer
    /// without building the snapshot.
    compact: bool,
    baseline: Option<reply::Baseline>,
    /// The shipped library, when the host gives it (else located).
    shipped: Option<sparksmith_io::shipped::ShippedLibrary>,
    /// Attached documents and working pinout documents (`assist.*`).
    assist: assist::Assist,
    /// The turns open in this session.
    pub(crate) turns: turns::Turns,
    /// The shipped generators' descriptions, once read (`library.search`'s
    /// generic parts).
    descriptions: std::sync::OnceLock<Vec<generators::v1::Description>>,
    /// The online registry the host installed (`online::OnlineSource`).
    online: Option<std::sync::Arc<dyn online::OnlineSource>>,
}

/// A test fixture, never in the product: a session on the core's fixture
/// engine ([`Engine::default`]).
#[cfg(any(test, feature = "test-fixtures"))]
impl Default for Session {
    fn default() -> Self {
        Self::fixture(Engine::default())
    }
}

impl Session {
    /// A session with an explicit, isolated four-worker test runtime. Its
    /// engine and all forks share that pool; production constructors retain
    /// the process-wide runtime unless a host explicitly injects another.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn fixture(mut engine: Engine) -> Self {
        engine.set_generators(generators::fixture_runtime());
        Self::new(engine)
    }

    pub fn new(engine: Engine) -> Self {
        Self::with_sources(engine, sources::Sources::default())
    }
    /// A session with the given library sources, e.g.
    /// [`sources::Sources::from_env`].
    pub fn with_sources(engine: Engine, sources: sources::Sources) -> Self {
        Self {
            #[cfg(test)]
            after_interface_read: None,
            engine,
            path: None,
            sources,
            compact: false,
            baseline: None,
            shipped: None,
            assist: assist::Assist::default(),
            turns: turns::Turns::default(),
            descriptions: std::sync::OnceLock::new(),
            online: None,
        }
    }
    /// A session without a project of its own, for a host that only lends
    /// it engines ([`Session::lend`]) or forwards its calls to an open
    /// window ([`Engine::placeholder`]).
    pub fn lending() -> Self {
        Self::new(Engine::placeholder())
    }
    pub fn sources(&self) -> &sources::Sources {
        &self.sources
    }
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
    /// The engine, for the front door's undo groups.
    pub(crate) fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Run `f` on this session over a host's own engine (the desktop
    /// window's, for live MCP), then hand the engine back, its revision and
    /// undo history moved on by whatever `f` did. `path` is the host's file,
    /// which requests that name none (`project.save`) would use.
    pub fn lend<R>(
        &mut self,
        engine: &mut Engine,
        path: Option<&Path>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        std::mem::swap(&mut self.engine, engine);
        self.path = path.map(Path::to_path_buf);
        let result = f(self);
        std::mem::swap(&mut self.engine, engine);
        result
    }

    /// Execute one typed request as the operator, answering with the whole
    /// document state: a call through the front door
    /// ([`crate::front::Surface::call`]).
    pub fn execute(&mut self, request: Request) -> Result<Value> {
        use crate::front::{CallCx, NoOne, Surface};
        let call = crate::front::call_of(&request).map_err(Error::refused)?;
        let cx = CallCx {
            who: crate::gate::Principal::Operator,
            asker: &mut NoOne,
            verbose: true,
            window: None,
        };
        self.call(cx, call).map_err(Error::refused)
    }

    /// Run one typed request the front door admitted, at `expected`: the
    /// one revision check of every document-changing request, before any
    /// file is read or written, so a stale one does nothing at all.
    pub(crate) fn run(&mut self, request: Request, expected: Option<u64>) -> Result<Value> {
        self.ensure_generators();
        // The host's standards and materials, current while it runs: what
        // a 3D build or export reads. An engine without them refuses
        // whatever needs them.
        let _context = self.engine.enter_context();
        // The new-project defaults, once per process; a failure is kept and
        // returned, naming the file, by the request that needs them.
        let _ = sparksmith_io::new_project::shipped();
        if let Some(expected) = expected {
            self.engine.check_revision(expected)?;
        }
        match request {
            Request::Apply(params) => {
                let operation = self.hosted_operation(params.operation)?;
                let outcome =
                    self.engine
                        .apply_hosted(params.expected_revision, operation, hosted())?;
                let mut reply = self.document_reply();
                reply["outcome"] = serde_json::to_value(outcome)?;
                self.add_rerouted(&mut reply)?;
                return Ok(reply);
            }
            Request::ApplyAll(params) => {
                let operations = params
                    .operations
                    .into_iter()
                    .map(|o| self.hosted_operation(o))
                    .collect::<Result<Vec<_>>>()?;
                let outcomes =
                    self.engine
                        .apply_all_hosted(params.expected_revision, operations, hosted())?;
                let mut reply = self.document_reply();
                reply["outcomes"] = serde_json::to_value(outcomes)?;
                self.add_rerouted(&mut reply)?;
                return Ok(reply);
            }
            Request::Inspect(params) => return self.inspect(params),
            Request::Schema(params) => return discovery::schema_reply(params),
            Request::Query(query) => {
                // A profile id reads as it does for an export: the board's
                // own, else one of the user's library or a shipped one.
                if let Query::RuleProfiles { id } = query {
                    return Ok(serde_json::to_value(match id {
                        Some(id) => QueryResult::RuleProfile(Box::new(
                            self.hosted_profile(ProfileChoice::Id(id))?
                                .resolve(&self.engine.project().rules)?,
                        )),
                        None => QueryResult::RuleProfiles(self.profile_list()?),
                    })?);
                }
                let query = match query {
                    Query::PreviewProfile {
                        profile,
                        options,
                        keep_overrides,
                    } => Query::PreviewProfile {
                        profile: self.hosted_profile(profile)?,
                        options,
                        keep_overrides,
                    },
                    other => other,
                };
                let query = match query {
                    Query::Preflight {
                        profile: Some(ProfileChoice::Id(id)),
                    } => Query::Preflight {
                        profile: Some(ProfileChoice::Profile(Box::new(
                            sparksmith_io::fabrication::resolve_profile(&id)?,
                        ))),
                    },
                    other => other,
                };
                let searched = match &query {
                    Query::Search { query, limit, .. } => Some((query.clone(), *limit)),
                    _ => None,
                };
                let summarised = matches!(query, Query::CheckDrafts { detail: false });
                let mut reply = serde_json::to_value(self.engine.query_using(
                    query,
                    &self.sources.context(),
                    Some(&sparksmith_solidcraft::footprint::SketchEvaluator),
                )?)?;
                if let Some((text, limit)) = searched {
                    self.add_generic(&text, &mut reply);
                    self.add_online(&text, limit, &mut reply);
                }
                if summarised {
                    reply::summarise_finding_objects(&mut reply["result"]);
                }
                return Ok(reply);
            }
            Request::Undo(params) => {
                let step = self.engine.undo_step();
                self.engine.undo_at(params.expected_revision)?;
                return self.stepped(step);
            }
            Request::Redo(params) => {
                let step = self.engine.redo_step();
                self.engine.redo_at(params.expected_revision)?;
                return self.stepped(step);
            }
            Request::New(params) => {
                let profile = match params.profile {
                    Some(ProfileChoice::Profile(profile)) => *profile,
                    Some(ProfileChoice::Id(id)) => {
                        sparksmith_io::rule_profiles::for_new_board_from_env(Some(&id))?
                    }
                    None => sparksmith_io::rule_profiles::for_new_board_from_env(None)?,
                };
                let mut project = Project::new(&sparksmith_io::new_project::shipped()?.board);
                sparksmith_core::rules::apply(
                    &mut project.rules,
                    sparksmith_core::rules::RulesCommand::set_profile(profile, false),
                )?;
                sparksmith_core::layout::mask_colour::follow(
                    &mut project.rules,
                    project.fabrication.mask_colour,
                )?;
                self.engine.replace_document(EditorFile {
                    version: sparksmith_core::format::FILE_VERSION,
                    project,
                    drafts: Drafts::default(),
                })?;
                self.path = None;
            }
            Request::Open(params) => {
                let path = PathBuf::from(params.path);
                let file = sparksmith_io::read_project(&path)?;
                // Stage host data before replacing the document: a missing
                // library must leave history and the previous save path intact.
                let defaults = sparksmith_io::new_project::shipped()?;
                self.engine.replace_document(file)?;
                self.engine.supply_dk_estimates(&defaults.presets.glass_dk);
                // A file holds no fill copper: refill before answering, so
                // checks, connectivity and fabrication see it. A fill that
                // cannot be computed stays unfilled, and the production
                // check reports it.
                let _ = self.engine.refill_stale();
                self.path = Some(path);
            }
            Request::Save(params) => {
                let path = match params.path {
                    Some(path) => PathBuf::from(path),
                    None => self.path.clone().ok_or_else(|| {
                        sparksmith_io::Error::Invalid(
                            "this project has no file yet; give a path".into(),
                        )
                    })?,
                };
                // A project first saved takes its identity, which every
                // manufacturing file then states. No nonce: a script saved
                // under the same file name reproduces it.
                self.engine.identify_by_file(&path, "");
                sparksmith_io::write_project(&path, &self.engine.to_file())?;
                let saved = path.display().to_string();
                self.path = Some(path);
                return Ok(json!({"revision": self.engine.revision(), "saved_path": saved}));
            }
            Request::PreviewUpdate(params) => {
                return Ok(serde_json::to_value(
                    self.engine.preview_update_hosted(params.change, hosted()),
                )?);
            }
            Request::PreviewReplace(params) => return self.preview_replace(params),
            Request::ReplaceComponent(params) => return self.replace_component(params),
            Request::ApplyUpdate(update) => {
                self.engine.apply_hosted(
                    update.expected_revision,
                    Operation::Update(Box::new(update)),
                    hosted(),
                )?;
            }
            Request::ExportNative(params) => {
                let watch = sparksmith_solidcraft::export::Watch::new();
                let report = sparksmith_solidcraft::export::board(
                    self.engine.project(),
                    params.tier,
                    crate::native_files::new_file(&params.path),
                    &watch,
                    if params.write_anyway {
                        sparksmith_solidcraft::export::Unfit::Write
                    } else {
                        sparksmith_solidcraft::export::Unfit::Refuse
                    },
                )?;
                return Ok(export_reply(
                    self.engine.project(),
                    &report,
                    json!({
                        "revision": self.engine.revision(),
                        "exported_path": params.path,
                    }),
                ));
            }
            Request::ExportFabrication(params) => {
                let profile = params
                    .profile
                    .as_deref()
                    .map(sparksmith_io::fabrication::resolve_profile)
                    .transpose()?;
                let report = sparksmith_io::fabrication::export(
                    self.engine.project(),
                    self.engine.drafts(),
                    &sparksmith_io::fabrication::ExportRequest {
                        output: PathBuf::from(&params.output),
                        zip: params.zip,
                        profile,
                        options: params.options,
                    },
                    self.engine.check_settings(),
                    &mut |_| {},
                )?;
                let mut value = serde_json::to_value(report)?;
                value["revision"] = json!(self.engine.revision());
                return Ok(value);
            }
            Request::Sources(_) => return self.list_sources(),
            Request::SourceAdd(params) => {
                let added = self.sources.add(Path::new(&params.path), params.writable)?;
                return Ok(json!({"added": added, "sources": self.sources.list()}));
            }
            Request::SourceCreate(params) => {
                let created = match params.library {
                    Some(library) => {
                        self.sources
                            .create_as(Path::new(&params.path), &params.name, library)?
                    }
                    None => self.sources.create(Path::new(&params.path), &params.name)?,
                };
                return Ok(json!({"created": created, "sources": self.sources.list()}));
            }
            Request::SourceRemove(params) => {
                self.sources.remove(Path::new(&params.path))?;
                return self.list_sources();
            }
            Request::SourceEnable(params) => {
                self.sources
                    .enable(Path::new(&params.path), params.enabled)?;
                return self.list_sources();
            }
            Request::Account(_) => return self.online_account(),
            Request::SignOut(_) => return self.online_sign_out(),
            Request::Rescan(_) => {
                self.sources.rescan();
                return self.list_sources();
            }
            Request::Favourite(params) => {
                self.sources.favourite(params.key, params.favourite)?;
                return Ok(json!({"favourites": self.sources.settings().favourites}));
            }
            Request::Bundle(params) => {
                let bundle = self.source_bundle(params.library, params.lineage, params.revision)?;
                return Ok(serde_json::to_value(bundle)?);
            }
            Request::ImportFrom(params) => {
                let bundle = self.source_bundle(params.library, params.lineage, params.revision)?;
                let operation = Operation::Library(LibraryCommand::Import {
                    bundle: Box::new(bundle),
                });
                return self.apply(params.expected_revision, operation);
            }
            Request::PlaceFrom(params) => {
                let bundle = self.source_bundle(params.library, params.lineage, params.revision)?;
                let operation = Operation::Library(LibraryCommand::PlaceFrom {
                    source: PlaceSource::Bundle {
                        bundle: Box::new(bundle),
                    },
                    position: params.position,
                    designator: params.designator,
                    other_revision: params.other_revision,
                });
                let mut reply = self.apply(params.expected_revision, operation)?;
                // The placement is done; remembering it as recent is a
                // convenience whose failure is reported, never an error.
                if let Err(error) = self.sources.used(EntryKey::Library {
                    library: params.library,
                    lineage: params.lineage,
                }) {
                    reply["warnings"] = json!([format!("recents not saved: {error}")]);
                }
                return Ok(reply);
            }
            Request::ImportFromProject(params) => return self.import_from_project(params),
            Request::Publish(params) => return self.publish(params),
            Request::ImportSvg(params) => return self.import_svg(params),
            Request::OpenPreview(params) => return self.open_preview(params),
            Request::PinoutSchema(_) => return self.pinout_schema(),
            Request::ReviewCheck(params) => {
                return match &params.body {
                    None => Ok(crate::review::check(
                        &self.review_snapshot(params.component, &params.report.part)?,
                        &params.report,
                    )),
                    Some(part) => Ok(crate::review::check_body(
                        &self.body_state(part)?,
                        &params.report,
                    )),
                };
            }
            Request::FirmwarePreview(params) => return self.firmware(params, false),
            Request::FirmwareExport(params) => return self.firmware(params, true),
            Request::InspectModelFile(params) => return self.inspect_model_file(params),
            Request::SetMarking(params) => return self.set_marking(params),
            Request::SchematicPdf(params) => {
                let pages = sparksmith_io::schematic_pdf::export_schematic_pdf(
                    self.engine.project(),
                    &params.sheets,
                    Path::new(&params.path),
                )?;
                return Ok(json!({
                    "revision": self.engine.revision(),
                    "path": params.path,
                    "pages": pages
                }));
            }
            Request::MarkingSurfaces(params) => return self.marking_surfaces(params),
            Request::CheckPartFile(params) => return self.check_part_file(params),
            Request::UsePartFile(params) => return self.use_part_file(params),
            Request::SetPartValues(params) => return self.set_part_values(params),
            Request::PartFiles(_) => return self.part_files(),
            Request::Generators(params) => return self.generators(&params),
            Request::GeneratePart(params) => return self.generate_part(params),
            Request::BodyFit(params) => return self.body_fit(params),
            Request::MakeBody(params) => return self.make_body(params),
            Request::PlacePartFile(params) => return self.place_part_file(*params),
            Request::SetPartFileValues(params) => return self.set_part_file_values(params),
            Request::InterfacePreview(params) => return self.interface_preview(params),
            Request::InterfacePublish(params) => return self.interface_publish(params),
            Request::AssemblyFollow(params) => return self.assembly_follow(params),
            Request::AssemblyPreview(params) => return self.assembly_preview(params),
            Request::AssemblyUpdate(params) => return self.assembly_update(params),
            Request::DocumentText(params) => return self.document_text(params),
            Request::DocumentPage(params) => return self.document_page(params),
            Request::PinoutEdit(params) => return self.pinout_edit(*params),
            Request::WorkingPreview(params) => return self.working_preview(params),
            Request::WorkingApply(params) => return self.working_apply(params),
            Request::Workflows(params) => return self.workflows(&params),
            Request::StepResult(params) => return self.step_result(params),
            // The front door runs these in the window; here is none.
            request @ (Request::ViewContext(_)
            | Request::ViewSelect(_)
            | Request::ViewOpen(_)
            | Request::Offer(_)
            | Request::AskPerson(_)) => {
                return Err(crate::Error::refused(crate::view::unavailable(
                    request.command(),
                )));
            }
            Request::TurnBegin(params) => return self.turns.begin(params).map_err(Error::refused),
            Request::TurnEnd(params) => return self.turns.end(&params).map_err(Error::refused),
        }
        Ok(self.document_reply())
    }

    /// `choice` with a profile id the board's own profile does not have
    /// read from the user's profile library or the shipped profiles: what
    /// `rules.set_fab_profile`, `rules.preview_fab_profile` and
    /// `rules.fab_profiles {id}` take. The board's own id is left for the
    /// engine, which reads it as the board's snapshot.
    fn hosted_profile(&self, choice: ProfileChoice) -> Result<ProfileChoice> {
        let ProfileChoice::Id(id) = choice else {
            return Ok(choice);
        };
        if let Some(board) = &self.engine.project().rules.profile
            && board.id == id
        {
            return Ok(ProfileChoice::Id(id));
        }
        match sparksmith_io::fabrication::resolve_profile(&id) {
            Ok(profile) => Ok(ProfileChoice::Profile(Box::new(profile))),
            Err(error) if error.code() == "not_found" => {
                let known: Vec<String> = self.profile_list()?.into_iter().map(|p| p.id).collect();
                Err(sparksmith_core::Error::NotFound(format!(
                    "rules.fab_profile_unknown: no fab profile {id}; the profiles are {}",
                    known.join(", ")
                ))
                .into())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// `operation` with a fab profile it names by id read by
    /// [`Self::hosted_profile`].
    fn hosted_operation(&self, operation: Operation) -> Result<Operation> {
        use sparksmith_core::rules::RulesCommand;
        Ok(match operation {
            Operation::Rules(RulesCommand::SetProfile {
                profile,
                options,
                keep_overrides,
            }) => Operation::Rules(RulesCommand::SetProfile {
                profile: self.hosted_profile(profile)?,
                options,
                keep_overrides,
            }),
            other => other,
        })
    }

    /// `rules.fab_profiles`: the board's own profile first, then the user's
    /// library and the shipped profiles whose ids it does not have.
    fn profile_list(&self) -> Result<Vec<sparksmith_core::rules::ProfileBrief>> {
        use sparksmith_core::rules::ProfileBrief;
        let board = self.engine.project().rules.profile.as_ref();
        let dirs = sparksmith_io::settings::UserDirs::from_env()?;
        let listing = sparksmith_io::rule_profiles::ProfileLibrary::from_dirs(&dirs)?.list()?;
        Ok(board
            .map(|p| ProfileBrief::of(p, true))
            .into_iter()
            .chain(
                listing
                    .profiles
                    .iter()
                    .chain(&listing.builtins)
                    .filter(|p| board.is_none_or(|b| b.id != p.id))
                    .map(|p| ProfileBrief::of(p, false)),
            )
            .collect())
    }

    /// Apply one operation and return the document state with its outcome.
    fn apply(&mut self, expected_revision: u64, operation: Operation) -> Result<Value> {
        let outcome = self
            .engine
            .apply_hosted(expected_revision, operation, hosted())?;
        let mut reply = self.document_reply();
        reply["outcome"] = serde_json::to_value(outcome)?;
        self.add_rerouted(&mut reply)?;
        Ok(reply)
    }

    /// The document state after an undo or redo, with the `step` it took
    /// back or applied again: its title and area, as the desktop says them.
    fn stepped(&self, step: Option<sparksmith_core::Step>) -> Result<Value> {
        let mut reply = self.document_reply();
        reply["step"] = serde_json::to_value(step)?;
        Ok(reply)
    }

    /// When the operation just applied moved parts whose dragged copper
    /// was re-routed or left unrouted: `rerouted` with the connections'
    /// nets and the one-line `summary` the desktop toasts.
    fn add_rerouted(&self, reply: &mut Value) -> Result<()> {
        let rerouted = self.engine.rerouted();
        if let Some(summary) = rerouted.line() {
            let mut value = serde_json::to_value(rerouted)?;
            value["summary"] = Value::String(summary);
            reply["rerouted"] = value;
        }
        Ok(())
    }

    /// `library.import_from_project`: read the file, take the exact closure
    /// from it and apply one `library.import` with that bundle inline.
    fn import_from_project(&mut self, params: ImportFromProjectParams) -> Result<Value> {
        let path = Path::new(&params.path);
        let file = sparksmith_io::read_project(path)?;
        let library = &file.project.library;
        if library.definition(params.root).is_none() {
            let held = held_revisions(&file);
            let message = match file.drafts.get(params.root.id) {
                Some(draft) => format!(
                    "{} holds ‘{}’ as an unpublished draft. Only a published revision is immutable: open that project, publish it and save, then import it.",
                    params.path, draft.meta.name
                ),
                None => format!(
                    "{} does not hold {} {} of lineage {}. It holds: {held}",
                    params.path,
                    sparksmith_core::library::names::noun(params.root.kind),
                    params.root.revision,
                    params.root.id
                ),
            };
            return Err(sparksmith_core::Error::NotFound(message).into());
        }
        let name = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .filter(|stem| stem.len() <= sparksmith_core::ids::MAX_LABEL)
            .unwrap_or("another project");
        let bundle = sparksmith_core::library::import::project_bundle(library, params.root, name)?;
        let operation = Operation::Library(LibraryCommand::Import {
            bundle: Box::new(bundle),
        });
        self.apply(params.expected_revision, operation)
    }

    fn list_sources(&self) -> Result<Value> {
        Ok(json!({"sources": self.sources.list()}))
    }

    /// `library.publish_to_folder`: the three promote steps. The revision is
    /// checked before the folder is written, and the plan is recomputed by
    /// `library.promote` with the same arguments, so the recorded origins
    /// equal the records written.
    fn publish(&mut self, params: PublishParams) -> Result<Value> {
        let folder = self.sources.writable(params.library)?.clone();
        let members =
            sparksmith_core::library::bundle::closure(&self.engine.project().library, params.root)?;
        let fresh = match params.fresh {
            Some(fresh) => fresh,
            None => (0..members.len())
                .map(|_| sparksmith_io::library_folder::new_lineage_id())
                .collect::<sparksmith_io::Result<Vec<_>>>()?,
        };
        let name = folder.manifest.name.clone();
        let plan = match self.engine.query(Query::PromotePreview {
            root: params.root,
            library: params.library,
            name: name.clone(),
            fresh: fresh.clone(),
        })? {
            QueryResult::Promote(plan) => plan,
            _ => {
                return Err(sparksmith_core::Error::Invalid(
                    "library.promote_preview gave another report".into(),
                )
                .into());
            }
        };
        // Only the ids the plan used travel in the undoable operation.
        let used = plan.used.clone();
        let written = sparksmith_io::library_folder::write_bundle(&folder, &plan.bundle)?;
        let operation = Operation::Library(LibraryCommand::Promote {
            root: params.root,
            library: params.library,
            name,
            fresh: used,
        });
        let mut reply = self.apply(params.expected_revision, operation)?;
        self.sources.rescan();
        reply["written"] = json!({
            "records": written.written,
            "unchanged": written.unchanged,
            "assets": written.assets,
        });
        reply["root"] = serde_json::to_value(plan.bundle.root)?;
        Ok(reply)
    }

    /// Read, convert and add an SVG drawing as one graphic edit.
    fn import_svg(&mut self, params: ImportSvgParams) -> Result<Value> {
        use sparksmith_solidcraft::svg;
        let text = sparksmith_io::artwork::read_svg(Path::new(&params.path))?;
        let options = svg::Options {
            scale: params.scale,
            tolerance_mm: params.tolerance_mm,
            fallback_stroke_mm: match params.target {
                GraphicTarget::Unit(_) => SYMBOL_STROKE_MM,
                GraphicTarget::Footprint { .. } => SILK_STROKE_MM,
            },
        };
        let artwork = svg::convert(&text, &options)?;
        let edits = artwork.edits(params.target, params.at)?;
        let outcome = self.engine.apply_with(
            params.expected_revision,
            Operation::Library(LibraryCommand::EditGraphics {
                draft: params.draft,
                edits,
            }),
            &sparksmith_solidcraft::footprint::SketchEvaluator,
        )?;
        let mut reply = self.document_reply();
        reply["outcome"] = serde_json::to_value(outcome)?;
        reply["import"] = json!({"size": artwork.size, "notes": artwork.notes});
        Ok(reply)
    }

    /// The whole document state, returned to in-process callers and to
    /// `verbose` requests. Each draft
    /// carries the core's `component.check` report, from one staged
    /// `component.check_drafts` query.
    pub fn snapshot(&self) -> Value {
        let project = self.engine.project();
        let items = &self.engine.drafts().items;
        let drafts: Vec<Value> = match self.engine.query(Query::CheckDrafts { detail: true }) {
            Ok(QueryResult::Drafts(reports)) => items
                .iter()
                .zip(reports)
                .map(|(draft, report)| {
                    let check = serde_json::to_value(QueryResult::Definition(report))
                        .unwrap_or(Value::Null);
                    json!({"draft": draft, "check": check})
                })
                .collect(),
            other => {
                let error = match other {
                    Err(error) => json!({"error": error.diagnostic()}),
                    Ok(_) => json!({"error": "unexpected report"}),
                };
                items
                    .iter()
                    .map(|draft| json!({"draft": draft, "check": error}))
                    .collect()
            }
        };
        json!({
            "revision": self.engine.revision(),
            "project": project,
            "drafts": drafts,
            "can_undo": self.engine.can_undo(),
            "can_redo": self.engine.can_redo(),
            // Raw rule output for scripts that read rule codes directly.
            // The findings view people and agents act on is
            // `project.issues` (severity, wording, merging, locating).
            "erc": erc::checks(project),
            "board_checks": layout::checks(project, self.engine.check_settings()),
        })
    }

    /// Parse and run a JSON request as the operator through the front door,
    /// answering with the whole document state in the shared envelope:
    /// `{"ok":true,"result":…}` or `{"ok":false,"error":{code,message,…}}`.
    pub fn dispatch(&mut self, request: Value) -> Value {
        use crate::front::{CallCx, NoOne, Surface};
        let result = crate::front::from_request(request).and_then(|(call, _)| {
            let cx = CallCx {
                who: crate::gate::Principal::Operator,
                asker: &mut NoOne,
                verbose: true,
                window: None,
            };
            self.call(cx, call)
        });
        crate::front::envelope(result)
    }
}

pub fn envelope(result: Result<Value>) -> Value {
    match result {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(error) => json!({"ok": false, "error": error.diagnostic()}),
    }
}

/// Create a library folder at `path` as a writable source of `session` and
/// publish every latest Part and component of its project to it, Parts
/// first (a Part's closure brings its component, which the later component
/// publish then finds unchanged). Every step is a request of the JSON
/// contract. Returns the library id, the records written and the sources.
pub fn publish_library(session: &mut Session, path: &str, name: &str) -> Result<Value> {
    let library = &session.engine().project().library;
    let roots: Vec<DefinitionRef> = library
        .latest_parts()
        .iter()
        .map(|p| DefinitionRef {
            kind: sparksmith_core::DefinitionKind::Part,
            id: p.id,
            revision: p.revision,
        })
        .chain(library.latest_components().iter().map(|c| DefinitionRef {
            kind: sparksmith_core::DefinitionKind::Component,
            id: c.id,
            revision: c.revision,
        }))
        .collect();
    let created = session.execute(Request::SourceCreate(SourceCreateParams {
        path: path.into(),
        name: name.into(),
        library: None,
    }))?;
    let id: LibraryId = serde_json::from_value(created["created"]["library"].clone())?;
    let mut written = Vec::new();
    for root in roots {
        let reply = session.execute(Request::Publish(PublishParams {
            expected_revision: session.engine().revision(),
            root,
            library: id,
            fresh: None,
        }))?;
        if let Some(records) = reply["written"]["records"].as_array() {
            written.extend(records.iter().cloned());
        }
    }
    Ok(json!({
        "library": id,
        "records_written": written,
        "sources": session.sources.list(),
    }))
}

/// What a project file holds that is worth importing, for a refusal: its
/// latest Parts and components by kind, id, revision and name.
fn held_revisions(file: &EditorFile) -> String {
    const SHOWN: usize = 16;
    let library = &file.project.library;
    let parts = library
        .latest_parts()
        .into_iter()
        .map(|p| ("part", p.id, p.revision, p.meta.name.clone()));
    let components = library
        .latest_components()
        .into_iter()
        .map(|c| ("component", c.id, c.revision, c.meta.name.clone()));
    let all: Vec<String> = parts
        .chain(components)
        .map(|(kind, id, revision, name)| format!("{kind} {} {revision} ‘{name}’", id.0))
        .collect();
    if all.is_empty() {
        return "no published Part or component.".into();
    }
    let more = all.len().saturating_sub(SHOWN);
    let shown = all.into_iter().take(SHOWN).collect::<Vec<_>>().join(", ");
    if more > 0 {
        format!("{shown} and {more} more.")
    } else {
        format!("{shown}.")
    }
}

/// Request, document and diagnostic schemas for discovery.
pub fn schemas() -> Value {
    json!({
        "request": schemars::schema_for!(Request),
        "commands": Request::catalog(),
        "document": sparksmith_core::schemas(),
        "solidcraft_revision": sparksmith_solidcraft::SOLIDCRAFT_REVISION,
    })
}

mod assembly_link;
pub mod assist;
pub mod body;
pub mod discovery;
mod firmware;
pub(crate) mod generated_models;
pub mod generators;
mod generic_search;
mod interface;
mod marking;
pub mod mechanical_parts;
pub mod metadata;
mod models;
pub mod online;
pub mod part_files;
pub mod paths;
mod pinout;
mod preview;
mod replace;
mod reply;
pub use reply::difference;
pub mod shipped_parts;
pub mod surface;
pub mod turns;
pub(crate) use reply::take_verbose;
pub mod sources;

#[cfg(test)]
mod assembly_link_tests;
#[cfg(test)]
mod body_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod connector_model_tests;
#[cfg(test)]
mod dimension_tests;
#[cfg(test)]
mod fabrication_tests;
#[cfg(test)]
mod fiducial_tests;
#[cfg(test)]
mod firmware_tests;
#[cfg(test)]
mod folder_tests;
#[cfg(test)]
mod generated_model_tests;
#[cfg(test)]
mod generators_tests;
#[cfg(test)]
mod high_speed_tests;
#[cfg(test)]
mod interface_publish_tests;
#[cfg(test)]
mod interface_tests;
#[cfg(test)]
mod issues_tests;
#[cfg(test)]
mod metadata_tests;
#[cfg(test)]
mod models_tests;
#[cfg(test)]
mod package_tests;
#[cfg(test)]
mod parts_tests;
#[cfg(test)]
mod pinout_tests;
#[cfg(test)]
mod project_import_tests;
#[cfg(test)]
mod replace_tests;
#[cfg(test)]
mod reply_tests;
#[cfg(test)]
mod svg_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod two_way_tests;

/// The shipped showcase ([`sparksmith_core::Engine::showcase_hosted`]) on
/// SolidCraft's sketch engine: its sketched board, cutout, slot and
/// edge-held header included, on `profile` and `presets`' stackups.
pub fn showcase(
    context: std::sync::Arc<sparksmith_core::library::standards::Context>,
    presets: &sparksmith_core::StackupPresets,
    profile: sparksmith_core::rules::RuleProfile,
) -> sparksmith_core::Result<sparksmith_core::Engine> {
    sparksmith_core::Engine::showcase_hosted(context, hosted(), presets, profile)
}

/// The host evaluators every applied operation gets: SolidCraft's sketch
/// engine for footprint sources and for the board shape.
pub fn hosted() -> sparksmith_core::Evaluators<'static> {
    sparksmith_core::Evaluators {
        footprint: Some(&sparksmith_solidcraft::footprint::SketchEvaluator),
        board: Some(&sparksmith_solidcraft::board::SketchBoardEvaluator),
    }
}

/// `reply` with an export's report ([`sparksmith_solidcraft::export::Report`]):
/// the tier, the bodies written, the check against the board plan with a
/// sentence per differing body, and the timings.
pub(crate) fn export_reply(
    project: &Project,
    report: &sparksmith_solidcraft::export::Report,
    mut reply: Value,
) -> Value {
    reply["tier"] = json!(report.tier);
    reply["bodies"] = json!(report.bodies);
    reply["ms"] = json!(report.ms);
    if let Some(check) = &report.check {
        reply["check"] = json!({
            "planned": check.planned,
            "built": check.built,
            "max_deviation_mm": check.max_deviation_mm,
            "max_area_mm2": check.max_area_mm2,
            "differs": report.differs(),
            "difference_count": report.difference_count(),
            "differences": check.differences,
            "lines": report.lines(project),
        });
    }
    reply
}
