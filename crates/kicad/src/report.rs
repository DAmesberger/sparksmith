//! The import report: what mapped exactly, what was approximated, what is
//! only reported and what was refused, as counted rows under the case
//! codes of the importer design (§3), plus private details.
//!
//! Privacy: the rows and [`Report::summary`] hold fixed text and counts
//! only. Names from the file (designators, pad numbers, library names)
//! appear only in [`Detail`]s, which a host writes to a local file and
//! never prints. The rules not taken ([`LooserRule`]) carry the file's
//! values: [`Report::summary_named`] lists them, for the person who
//! imported their own project (the CLI and the app), never for a shared
//! log.
use serde::Serialize;
use std::collections::BTreeMap;

/// How faithfully a case maps.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// Imported as it is.
    Exact,
    /// Imported close, within a stated bound.
    Approximate,
    /// Imported (or dropped) as it is and listed.
    Report,
    /// Not imported.
    Refused,
}

/// One case of the design's table.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Case {
    /// Pads whose copper, mask, paste and drill map one to one.
    PadsExact,
    /// F2: a token this importer does not know.
    UnknownToken,
    /// L1: every imported footprint is project-only.
    ProjectOnly,
    /// G2: footprint drawings on copper, mask or paste (slice K7).
    FootprintFabGraphics,
    /// G2: footprint mask and paste polygons that grow one pad's land
    /// evenly, imported as that pad's mask or paste rule.
    FootprintOpenings,
    /// G3: a custom pad whose union is not one outline without holes.
    CustomPadNotOneOutline,
    /// A custom-pad primitive this importer cannot draw (arcs, curves).
    CustomPadPrimitive,
    /// A pad with a token under it this importer does not know.
    PadUnknownToken,
    /// A pad shape this importer does not draw yet (trapezoid).
    PadShapeUnsupported,
    /// Custom pads whose round primitives became chords.
    CustomPadChords,
    /// Mask or paste openings of custom pads grown or shrunk with round
    /// joins that KiCad draws as chords.
    CustomPadOffset,
    /// A through-hole pad with a mask opening on one side only.
    OneSidedThroughMask,
    /// Paste with a relative margin on a pad that is not square, of a
    /// shape no window draws (custom): shrunk by the short side.
    PasteRatio,
    /// Paste with a relative margin on a pad that is not square, imported
    /// as a paste window of the opening KiCad draws.
    PasteRatioWindow,
    /// G4: footprint text and drawings (silkscreen, courtyard, fab).
    FootprintDrawings,
    /// A footprint that marked a tie whose groups or drawings could not be imported.
    NetTie,
    /// One footprint with all its independent copper tie groups imported.
    ImportedTie,
    JumperSuggestion,
    /// G6: per-pad zone connection and clearance overrides.
    PadZoneOverrides,
    /// G6: footprint-wide zone connection and clearance overrides.
    FootprintZoneOverrides,
    /// G8: a pad with no copper (paste or mask only).
    PadWithoutCopper,
    /// G8: a paste-only pad, imported as a paste window of the pad under it.
    PasteWindow,
    /// G8: a paste-only pad that, with the pad under it, needs an opening no
    /// paste window holds (one with a hole in it).
    PasteWindowShape,
    /// A footprint with no electrical pad (a fiducial, a mounting hole):
    /// a part without terminals.
    NoTerminals,
    /// A footprint KiCad marks "not in schematic": a board-only part.
    BoardOnly,
    /// A board-only part's pad on a net: no pin on a sheet joins it.
    BoardOnlyNet,
    /// G8: a surface pad with copper on both outer sides.
    PadOnBothSides,
    /// A designator that was not valid or repeated, renumbered.
    Designator,
    /// Definition names without their KiCad library nickname, or cleaned
    /// of control characters and extra whitespace.
    NameShortened,
    /// Definition names an earlier definition took, numbered apart.
    NameNumbered,
    /// G9/G10: board-level drawings, texts and dimensions (not imported).
    BoardObjects,
    /// G12: 3D model references.
    Models,
    /// Footprint instances made as generated parts: a shipped generator's
    /// part, its land pattern the source's lands.
    GeneratedPart,
    /// Footprint instances made as project parts of the source lands: no
    /// generator fits (the reason in details); their body comes from a
    /// recreation from datasheets.
    SourcePart,
    /// Footprint instances whose part number names another package than
    /// their lands are (the lands kept as the source has them).
    PackageContradiction,
    /// Source symbol units a part's own symbol has no unit for (a
    /// generated part's one symbol): their wiring joins its terminals.
    UnitMerged,
    /// Placed parts whose value set their fields (typed, or the part value
    /// text field).
    ValueField,
    /// Placed parts whose value has words no field took.
    ValueWordUnread,
    /// Placed parts with a value whose part selects no value field (the
    /// value is not carried).
    ValueWithoutField,
    /// Part numbers without a manufacturer: no manufacturer part made.
    PartWithoutManufacturer,
    /// Manufacturer parts the library refused.
    PartRefused,
    /// Footprint instances that are no part to draw (a jumper, a contact
    /// pad, a fiducial, a logo): no body, as in the source.
    NoBody,
    /// A pad whose size, place or margins are beyond the coordinate bound.
    PadOutOfRange,
    /// No fab profile was given: no rules, so no outline, tracks or fills.
    NoRules,
    /// A board-wide setting the fab profile's bounds refused.
    RuleRefused,
    /// The physical stackup: copper, dielectric and mask thicknesses and
    /// Dk taken over (glass styles from the stackup family).
    StackupImported,
    /// A stated stackup outside the stackup bounds: the family's preset.
    StackupRefused,
    /// A dielectric of several sublayers, summed into one.
    StackupSublayers,
    /// Solder mask thicknesses that differ by side: the top's kept for
    /// both (the stackup holds one), the bottom's named in the details.
    StackupMaskSides,
    /// The outline, imported as a board sketch.
    OutlineSketch,
    /// The outline as the rectangle from the origin around it (no board
    /// sketch engine).
    OutlineRectangle,
    /// `Edge.Cuts` drawings that do not close into one board: the board
    /// keeps its rectangle.
    OutlineNotClosed,
    /// No outline: the board keeps its new-board rectangle.
    OutlineMissing,
    /// An outline that cannot be imported (no rules, or reaching below the
    /// origin without a sketch engine).
    OutlineNotImported,
    /// Outline drawings that are curves or degenerate arcs.
    OutlineDrawing,
    /// `Edge.Cuts` drawings inside footprints.
    FootprintOutline,
    /// The project file's clearances and least track width, set over the
    /// fab profile.
    ProjectRules,
    /// No project file: the fab profile's clearances stand.
    NoProject,
    /// A project file that is not readable JSON: the profile's stand.
    ProjectUnreadable,
    /// A net class imported into the project's own rule set.
    NetClass,
    /// A net class the rules refused (a value out of bounds).
    NetClassRefused,
    /// A class pattern that is not a plain wildcard or names no imported
    /// class, or a net whose class was not imported.
    NetClassPatternRefused,
    /// Nets assigned to their class explicitly.
    NetClassAssigned,
    /// A KiCad limit looser than the fab profile's: the profile's kept.
    RuleLooserThanFab,
    /// Net names that are not label names (spaces, over 64 bytes, equal
    /// to another but for case), spelled anew.
    NetRenamed,
    /// Pads of one terminal on two nets: the first net kept.
    TerminalOnTwoNets,
    /// Board nets whose terminals the schematic keeps apart: the
    /// schematic's nets kept, each a finding.
    BoardJoinsNets,
    /// Schematic nets whose terminals the board has on several nets (or
    /// on none): the schematic's net kept, each a finding.
    BoardSplitsNet,
    /// Disagreements past the project's bound, not recorded.
    NetDisagreementOverLimit,
    /// Source schematic nets the imported schematic has apart: an import
    /// fault, each named in the details.
    SchematicNetSplit,
    /// Imported schematic nets that join source schematic nets: an import
    /// fault, each named in the details.
    SchematicNetJoined,
    /// No schematic, or one that could not be read: pins labelled.
    SchematicUnreadable,
    /// Sheet symbols drawn (one per sheet instance).
    SheetSymbol,
    /// Symbols whose library symbol is missing from the file.
    SymbolWithoutLibrary,
    /// Parts on the schematic with no placed footprint of their reference:
    /// placed on their sheet with no footprint chosen (an Issues finding),
    /// each named.
    SymbolWithoutFootprint,
    /// Symbols not drawn: no part of their reference could be placed (an
    /// invalid or repeated designator).
    SymbolNotDrawn,
    /// Symbol pins with no pad of their number.
    PinWithoutPad,
    /// Footprints with no symbol in the schematic: their pins labelled.
    FootprintWithoutSymbol,
    /// Labels and power ports placed.
    SchematicLabel,
    /// Hidden power input pins, given a power port of their name.
    HiddenPowerPin,
    /// Pins of type "not connected", left unwired.
    NoConnectPin,
    /// Pins or labels on a wire's middle, with no wire end or junction
    /// there: joined to it.
    PinOnWire,
    /// Labels named as the net KiCad's wires join them into (KiCad lets
    /// one net carry several names; ours carry one).
    LabelAlias,
    /// Terminals no pin of their KiCad symbol names (unmapped pads): a
    /// visible pin added by import, each a finding.
    TerminalWithoutPin,
    /// Units the engine refused to place or turn.
    UnitRefused,
    /// Symbol pins that meet no wire, label, other pin or no-connect flag.
    PinUnconnected,
    /// Labels, power ports and sheet pins the engine refused.
    SchematicItemRefused,
    /// Wires drawn.
    SchematicWire,
    /// Wires drawn straight, their corners refused.
    SchematicWireStraightened,
    /// Wires the engine refused.
    SchematicWireRefused,
    /// Arc tracks, imported as chords.
    ArcTrack,
    /// Tracks on no net with pads (or on no copper layer).
    TrackWithoutNet,
    /// Track pieces shorter than the engine's least track length, removed
    /// with their neighbours' ends joined.
    TinyTrackCleaned,
    /// Tracks the board cannot hold (off the board, or a size no track has).
    /// None is left out for breaking a rule.
    TrackRefused,
    /// Vias on no net with pads.
    ViaWithoutNet,
    /// Vias the board cannot hold (off the board, without a ring, blind or
    /// buried).
    ViaRefused,
    /// Vias whose unused inner annular rings KiCad removes.
    ViaUnusedLayers,
    /// Zones imported as fills (one per copper layer).
    Zone,
    /// Rule areas imported with every restriction intact.
    Keepout,
    /// Source rule areas that prohibit footprint placement, which a
    /// Sparksmith keepout cannot enforce.
    FootprintRestrictionOmitted,
    /// Zones not imported (arcs in the outline, no net with pads, no rules).
    ZoneRefused,
    /// Zones of several outline polygons, each a fill.
    ZonePolygons,
    /// Hatched zones, filled solid.
    ZoneHatched,
    /// Zones with corner smoothing, filled without it.
    ZoneSmoothing,
    /// Parts KiCad marks do-not-populate (on the footprint or the symbol):
    /// not populated in the default, body kept.
    NotPopulatedAttribute,
    /// A not-fitted marker on a fabricated copper tie.
    CopperTieDnp,
    /// Parts whose value is or carries a not-fitted marker ("DNP"): not
    /// populated in the default, the marker taken out of the value.
    NotPopulatedMarker,
    /// Variant data in the source files, not imported (KiCad's public
    /// file-format documentation does not describe it).
    VariantsNotImported,
    /// Pins whose KiCad electrical type has no core equivalent
    /// (`tri_state`), imported passive; each named with its type. Every
    /// other declared type keeps its direction and drive.
    PinTypeKept,
}

impl Case {
    /// The design's code, fidelity and a fixed description.
    pub fn describe(self) -> (&'static str, Fidelity, &'static str) {
        use Fidelity::*;
        match self {
            Self::PadsExact => ("-", Exact, "pads mapped one to one"),
            Self::UnknownToken => ("F2", Report, "unknown tokens outside pads, skipped"),
            Self::ProjectOnly => (
                "L1",
                Report,
                "footprints imported project-only (never publishable to a shared library)",
            ),
            Self::FootprintFabGraphics => (
                "G2",
                Refused,
                "footprint drawings on copper, mask or paste (slice K7)",
            ),
            Self::FootprintOpenings => (
                "G2",
                Approximate,
                "footprint mask and paste polygons imported as their pad's opening (an even growth of its land; corners rounded)",
            ),
            Self::CustomPadNotOneOutline => (
                "G3",
                Refused,
                "custom pads whose union is not one outline without holes",
            ),
            Self::CustomPadPrimitive => (
                "G3",
                Refused,
                "custom pads with a primitive this importer cannot draw",
            ),
            Self::PadUnknownToken => ("F2", Refused, "pads with an unknown token"),
            Self::PadShapeUnsupported => ("-", Refused, "trapezoid pads, not drawn yet"),
            Self::CustomPadChords => (
                "G3",
                Approximate,
                "custom pads whose round primitives became chords within 0.25 µm",
            ),
            Self::CustomPadOffset => (
                "-",
                Approximate,
                "custom-pad mask or paste openings offset with round joins (KiCad's are chords)",
            ),
            Self::OneSidedThroughMask => (
                "-",
                Approximate,
                "through-hole pads with a mask opening on one side, opened on both",
            ),
            Self::PasteRatio => (
                "-",
                Approximate,
                "relative paste margins on custom pads that are not square, by the short side",
            ),
            Self::PasteRatioWindow => (
                "-",
                Exact,
                "relative paste margins on pads that are not square, imported as paste windows",
            ),
            Self::FootprintDrawings => (
                "G4",
                Report,
                "footprint text and drawings, not imported in this slice",
            ),
            Self::ImportedTie => ("G5", Exact, "footprints imported with copper tie groups"),
            Self::JumperSuggestion => (
                "G5",
                Report,
                "possible solder jumpers to review for conversion",
            ),
            Self::NetTie => (
                "G5",
                Report,
                "net ties not imported because their declared groups or copper drawings need review",
            ),
            Self::PadZoneOverrides => (
                "G6",
                Exact,
                "pads with zone connection, thermal or clearance overrides, imported as pad fill overrides",
            ),
            Self::FootprintZoneOverrides => (
                "G6",
                Exact,
                "footprints with zone connection, thermal or clearance overrides, imported as part fill overrides",
            ),
            Self::PadWithoutCopper => (
                "G8",
                Refused,
                "pads without copper over no other pad's copper",
            ),
            Self::PasteWindowShape => (
                "G8",
                Refused,
                "paste-only pads whose opening no paste window holds (it has a hole)",
            ),
            Self::PasteWindow => (
                "G8",
                Exact,
                "paste-only pads imported as paste windows of the pad under them",
            ),
            Self::NoTerminals => (
                "-",
                Exact,
                "footprints without electrical pads, imported as parts without terminals",
            ),
            Self::BoardOnly => (
                "-",
                Exact,
                "footprints not in the schematic, imported as board-only parts",
            ),
            Self::BoardOnlyNet => (
                "N1",
                Refused,
                "board-only parts' pads on a net (no schematic pin joins them)",
            ),
            Self::PadOnBothSides => ("G8", Refused, "surface pads on both outer sides"),
            Self::Designator => ("-", Report, "designators renumbered"),
            Self::NameShortened => (
                "-",
                Report,
                "definition names without their KiCad library nickname or extra whitespace (the full id kept in the provenance)",
            ),
            Self::NameNumbered => (
                "-",
                Report,
                "definition names shared with another footprint, numbered apart (\", 2\")",
            ),
            Self::BoardObjects => (
                "G9",
                Report,
                "board-level drawings, texts and dimensions, not imported (slice K7)",
            ),
            Self::Models => (
                "G12",
                Report,
                "3D model references (host resolution unavailable)",
            ),
            Self::GeneratedPart => (
                "P1",
                Exact,
                "footprint instances made as generated parts (a shipped generator's part, its land pattern the source's lands)",
            ),
            Self::PackageContradiction => (
                "P3",
                Report,
                "footprint instances whose part number names another package than their lands (lands kept as in the source)",
            ),
            Self::UnitMerged => (
                "P4",
                Report,
                "source symbol units drawn by their part's one symbol (wiring kept on its terminals)",
            ),
            Self::ValueField => ("P5", Exact, "placed parts whose value set their fields"),
            Self::ValueWordUnread => ("P5", Report, "placed parts with value words no field took"),
            Self::ValueWithoutField => (
                "P5",
                Report,
                "placed parts with a value their part has no value field for (not carried)",
            ),
            Self::PartWithoutManufacturer => (
                "P6",
                Report,
                "part numbers without a manufacturer (no manufacturer part made)",
            ),
            Self::PartRefused => ("P6", Refused, "manufacturer parts the library refused"),
            Self::NoBody => (
                "G12B",
                Exact,
                "footprint instances with no body (as in the source: no model, and not an assembled part)",
            ),
            Self::SourcePart => (
                "P2",
                Report,
                "footprint instances made as project parts of the source lands: no generator fits (reason below and in details)",
            ),
            Self::PadOutOfRange => (
                "-",
                Refused,
                "pads with a size, place or margin beyond the coordinate bound",
            ),
            Self::NoRules => (
                "-",
                Refused,
                "no fab profile given: outline, tracks and fills not imported",
            ),
            Self::RuleRefused => (
                "-",
                Report,
                "board-wide settings (mask margin, via tenting) the fab profile refused",
            ),
            Self::StackupImported => (
                "S1",
                Exact,
                "stackup: copper, dielectric and mask thicknesses and Dk (glass from the family)",
            ),
            Self::StackupRefused => (
                "S1",
                Refused,
                "stackup outside the stackup bounds: the family's preset of its copper count",
            ),
            Self::StackupSublayers => (
                "S1",
                Approximate,
                "dielectrics of several sublayers, summed into one (first Dk kept)",
            ),
            Self::StackupMaskSides => (
                "S1",
                Approximate,
                "solder mask thicknesses that differ by side: the top's kept for both sides, the bottom's dropped",
            ),
            Self::OutlineSketch => ("B1", Exact, "board outline imported as a board sketch"),
            Self::OutlineRectangle => (
                "B1",
                Approximate,
                "board outline as the rectangle around it (no board sketch engine)",
            ),
            Self::OutlineNotClosed => (
                "B1",
                Refused,
                "outline drawings that do not close into one board (rectangle kept)",
            ),
            Self::OutlineMissing => ("B1", Report, "no board outline (new-board rectangle kept)"),
            Self::OutlineNotImported => (
                "B1",
                Refused,
                "board outline not imported (no rules, or below the origin without a sketch engine)",
            ),
            Self::OutlineDrawing => (
                "B1",
                Refused,
                "outline curves (Béziers) and degenerate arcs, not imported",
            ),
            Self::FootprintOutline => (
                "B1",
                Refused,
                "outline drawings inside footprints, not imported",
            ),
            Self::ProjectRules => (
                "D1",
                Exact,
                "project rules (default class, board minimums) set over the fab profile as the project's own rule set",
            ),
            Self::NoProject => (
                "D1",
                Report,
                "no project file: the fab profile's clearances stand",
            ),
            Self::ProjectUnreadable => (
                "D1",
                Refused,
                "project file not readable: the fab profile's clearances stand",
            ),
            Self::NetClass => ("D1", Exact, "net classes imported"),
            Self::NetClassRefused => ("D1", Refused, "net classes the rules refused"),
            Self::NetClassPatternRefused => (
                "D1",
                Refused,
                "net class patterns that are not plain wildcards or name no imported class",
            ),
            Self::NetClassAssigned => (
                "D1",
                Exact,
                "nets assigned to their class explicitly (no pattern gives it on our names)",
            ),
            Self::RuleLooserThanFab => (
                "D1",
                Report,
                "KiCad rules looser than the fab profile, not taken (the fab limit kept)",
            ),
            Self::NetRenamed => (
                "N1",
                Report,
                "net names spelled anew as label names (spaces, length, case)",
            ),
            Self::TerminalOnTwoNets => (
                "N1",
                Refused,
                "pads of one terminal on two nets (first net kept)",
            ),
            Self::BoardJoinsNets => (
                "N2",
                Report,
                "board nets the schematic keeps apart (schematic kept; each a finding)",
            ),
            Self::BoardSplitsNet => (
                "N2",
                Report,
                "schematic nets the board has apart (schematic kept; each a finding)",
            ),
            Self::NetDisagreementOverLimit => (
                "N2",
                Refused,
                "net disagreements past the project's bound, not recorded as findings",
            ),
            Self::SchematicNetSplit => (
                "N3",
                Refused,
                "source schematic nets the imported schematic splits (import fault; pins in the details)",
            ),
            Self::SchematicNetJoined => (
                "N3",
                Refused,
                "imported schematic nets joining source schematic nets (import fault; pins in the details)",
            ),
            Self::SchematicUnreadable => (
                "K5",
                Refused,
                "schematic not readable: pins labelled with the board's nets",
            ),
            Self::SheetSymbol => ("K5", Exact, "sheet instances drawn as sheet symbols"),
            Self::SymbolWithoutLibrary => (
                "K5",
                Refused,
                "symbols whose library symbol is missing from the file",
            ),
            Self::SymbolWithoutFootprint => (
                "K5",
                Report,
                "parts on the schematic with no footprint on the board, placed without one (each a finding)",
            ),
            Self::SymbolNotDrawn => (
                "K5",
                Refused,
                "symbols not drawn: no part of their reference could be placed",
            ),
            Self::PinWithoutPad => ("K5", Report, "symbol pins without a pad of their number"),
            Self::FootprintWithoutSymbol => (
                "K5",
                Approximate,
                "footprints without a symbol: their pins labelled with the board's nets",
            ),
            Self::SchematicLabel => ("K5", Exact, "labels and power ports placed"),
            Self::NoConnectPin => (
                "K5",
                Exact,
                "pins of type \"not connected\", left unwired (they join nothing)",
            ),
            Self::HiddenPowerPin => (
                "K5",
                Exact,
                "hidden power input pins, given a power port of their name",
            ),
            Self::PinOnWire => (
                "K5",
                Report,
                "pins or labels on a wire's middle without a junction (joined to it)",
            ),
            Self::LabelAlias => (
                "K5",
                Approximate,
                "labels renamed to the one name of the net KiCad's wires join them into",
            ),
            Self::TerminalWithoutPin => (
                "K5",
                Report,
                "unmapped pads: no pin of their number on the KiCad symbol (a pin added by import)",
            ),
            Self::UnitRefused => (
                "K5",
                Refused,
                "symbol units the engine refused to place or turn",
            ),
            Self::PinUnconnected => (
                "K5",
                Report,
                "symbol pins that meet no wire, label, other pin or no-connect flag",
            ),
            Self::SchematicItemRefused => (
                "K5",
                Refused,
                "labels, power ports and sheet pins the engine refused",
            ),
            Self::SchematicWire => ("K5", Exact, "wires drawn"),
            Self::SchematicWireStraightened => (
                "K5",
                Approximate,
                "wires drawn straight (their corners refused)",
            ),
            Self::SchematicWireRefused => ("K5", Refused, "wires the engine refused"),
            Self::ArcTrack => (
                "R1",
                Approximate,
                "arc tracks imported as chords within 0.25 µm",
            ),
            Self::TrackWithoutNet => ("R1", Refused, "tracks on no net with pads, not imported"),
            Self::TinyTrackCleaned => (
                "R1",
                Approximate,
                "track pieces shorter than 0.01 mm removed, their neighbours' ends joined",
            ),
            Self::TrackRefused => ("R1", Refused, "tracks the board cannot hold, not imported"),
            Self::ViaWithoutNet => ("R1", Refused, "vias on no net with pads, not imported"),
            Self::ViaRefused => ("R1", Refused, "vias the board cannot hold, not imported"),
            Self::ViaUnusedLayers => (
                "R1",
                Approximate,
                "vias without unused inner rings in KiCad, imported with rings on every layer",
            ),
            Self::Zone => (
                "Z1",
                Exact,
                "zone layers imported as fills, filled by our engine",
            ),
            Self::Keepout => ("Z1", Exact, "rule areas imported as keepouts"),
            Self::FootprintRestrictionOmitted => (
                "Z1",
                Approximate,
                "footprint placement restrictions omitted from rule areas",
            ),
            Self::ZoneRefused => (
                "Z1",
                Refused,
                "zones not imported (arcs in the outline, no net with pads, no rules)",
            ),
            Self::ZonePolygons => ("Z1", Approximate, "zones of several outlines, each a fill"),
            Self::ZoneHatched => ("Z1", Approximate, "hatched zones, filled solid"),
            Self::ZoneSmoothing => (
                "Z1",
                Approximate,
                "zones with corner smoothing, filled without",
            ),
            Self::CopperTieDnp => (
                "P7",
                Report,
                "DNP markings on fabricated copper ties, which stay closed",
            ),
            Self::NotPopulatedAttribute => (
                "P7",
                Exact,
                "parts KiCad marks do-not-populate, not populated (body kept for variants)",
            ),
            Self::NotPopulatedMarker => (
                "P7",
                Exact,
                "parts whose value says not fitted (\"DNP\"), not populated and the marker taken out of the value",
            ),
            Self::VariantsNotImported => (
                "P7",
                Report,
                "variant data in the source files, not imported",
            ),
            Self::PinTypeKept => (
                "-",
                Approximate,
                "pins of a type with no equivalent (tri-state), imported passive",
            ),
        }
    }
}

/// Why a footprint instance keeps its labelled placeholder body
/// ([`Case::SourcePart`]) and needs a recreation, counted per instance in the
/// summary.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Placeholder {
    /// Every model reference is hidden.
    Hidden,
    /// A vendor model file was found in the project and recorded as a
    /// reference source (never used as the body); no preset matched.
    VendorReferenceFound,
    /// A vendor model reference names no readable file in the project; no
    /// preset matched.
    VendorReferenceMissing,
    /// No numbered lands to match a preset against (a fiducial, a logo).
    NoLands,
    /// A two-pad preset fits the lands, but no fitting preset's reference
    /// prefix begins the designator's (an inductor on a chip resistor's lands).
    PrefixMismatch,
    /// Only KiCad library references (never resolved); no preset matched.
    KicadLibraryReferenceNoPreset,
    /// No model reference at all; no preset matched.
    NoReferenceNoPreset,
    /// The lands are a preset's package, but its generator cannot take
    /// them as its land pattern (the reason in details).
    GeneratorRefused,
}

impl Placeholder {
    /// A fixed description.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Hidden => "all model references hidden",
            Self::VendorReferenceFound => "needs recreation (vendor reference found)",
            Self::VendorReferenceMissing => "needs recreation (vendor reference not found)",
            Self::PrefixMismatch => {
                "needs recreation (a preset fits, but its reference prefix differs)"
            }
            Self::NoLands => "needs recreation (no numbered lands to match a preset)",
            Self::KicadLibraryReferenceNoPreset => {
                "needs recreation (KiCad library reference, no matching preset)"
            }
            Self::NoReferenceNoPreset => "needs recreation (no reference, no matching preset)",
            Self::GeneratorRefused => {
                "needs recreation (the package's generator cannot take these lands)"
            }
        }
    }
}

/// One counted row.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Row {
    pub case: Case,
    pub code: &'static str,
    pub fidelity: Fidelity,
    pub title: &'static str,
    pub count: usize,
}

/// A private detail of a row: which footprint (by designator) and pad, or
/// which token path. Written only to the local report file.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Detail {
    pub case: Case,
    pub footprint: String,
    pub pad: String,
    pub path: String,
    /// For [`Case::SourcePart`]: why the placeholder stays.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Placeholder>,
}

impl Detail {
    /// The detail as one line naming what it is about: the footprint (or
    /// part) and what the import says of it.
    fn named(&self) -> String {
        match (self.footprint.is_empty(), self.path.is_empty()) {
            (false, false) => format!("{}: {}", self.footprint, self.path),
            (false, true) => self.footprint.clone(),
            (true, _) => self.path.clone(),
        }
    }
}

/// A KiCad limit looser than the fab profile's, not taken
/// ([`Case::RuleLooserThanFab`]): which rule, where KiCad sets it, and
/// both values.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LooserRule {
    /// The rule as issue texts name it ("min drill").
    pub rule: String,
    /// The KiCad project setting it came from (`min_through_hole_diameter`,
    /// "default net class clearance").
    pub kicad_setting: &'static str,
    pub kicad_mm: f64,
    pub profile_mm: f64,
}

/// A net class pattern not taken ([`Case::NetClassPatternRefused`]): the
/// pattern as the project file has it, and why.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RefusedPattern {
    pub pattern: String,
    pub reason: &'static str,
}

/// A track piece or via the board cannot hold
/// ([`Case::TrackRefused`], [`Case::ViaRefused`]): where it ran, in our
/// board frame, and why, in the core's own sentence.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RefusedCopper {
    /// [`Case::TrackRefused`] or [`Case::ViaRefused`].
    pub case: Case,
    /// "Track on VIN (Top) from (4, 16.8) to (10, 16.8) mm".
    pub what: String,
    /// "track copper must lie inside the board outline".
    pub check: String,
}

impl RefusedCopper {
    /// A refused track piece of `net` on `layer` from `start` to `end` (mm).
    pub fn track(net: &str, layer: &str, start: [f64; 2], end: [f64; 2], check: &str) -> Self {
        Self {
            case: Case::TrackRefused,
            what: format!(
                "Track on {net} ({layer}) from {} to {} mm",
                point_text(start),
                point_text(end)
            ),
            check: check.to_owned(),
        }
    }

    /// A refused via of `net` at `at` (mm).
    pub fn via(net: &str, at: [f64; 2], check: &str) -> Self {
        Self {
            case: Case::ViaRefused,
            what: format!("Via on {net} at {} mm", point_text(at)),
            check: check.to_owned(),
        }
    }

    /// "Track on VIN (Top) from (4, 16.8) to (10, 16.8) mm: track copper …".
    pub fn line(&self) -> String {
        format!("{}: {}", self.what, self.check)
    }
}

fn point_text(p: [f64; 2]) -> String {
    format!("({}, {})", mm_text(p[0]), mm_text(p[1]))
}

impl RefusedPattern {
    /// "+5*: its class was not imported".
    pub fn line(&self) -> String {
        format!("{}: {}", self.pattern, self.reason)
    }
}

impl LooserRule {
    /// "min drill: KiCad 0.2 mm, fab profile 0.3 mm (min_through_hole_diameter)".
    pub fn line(&self) -> String {
        format!(
            "{}: KiCad {} mm, fab profile {} mm ({})",
            self.rule,
            mm_text(self.kicad_mm),
            mm_text(self.profile_mm),
            self.kicad_setting
        )
    }
}

/// A length in millimetres to the micrometre, without trailing zeros.
fn mm_text(mm: f64) -> String {
    let text = format!("{mm:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// Overall counts.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Counts {
    pub copper_layers: usize,
    pub footprints: usize,
    pub footprints_back: usize,
    pub definitions: usize,
    pub pads: usize,
    pub pads_imported: usize,
    /// Cutouts of the imported board outline.
    pub cutouts: usize,
    /// Nets with pads, tracks and vias imported, fills made.
    pub nets: usize,
    pub tracks: usize,
    pub vias: usize,
    pub fills: usize,
    /// Schematic sheets drawn.
    pub sheets: usize,
    /// Board nets whose pads the schematic joins exactly, net for net;
    /// split over several schematic nets (or unjoined); joined with
    /// another board net's pads.
    pub schematic_nets_equal: usize,
    pub schematic_nets_split: usize,
    pub schematic_nets_merged: usize,
    /// Source schematic nets with a pin on a placed part, and those the
    /// imported schematic has exactly.
    pub source_nets: usize,
    pub source_nets_equal: usize,
}

/// The cases whose occurrences [`Report::summary_named`] lists by name:
/// what the import left for the person to resolve.
pub const NAMED_CASES: [Case; 5] = [
    Case::NetTie,
    Case::ImportedTie,
    Case::JumperSuggestion,
    Case::StackupMaskSides,
    Case::SymbolWithoutFootprint,
];

/// Most private details kept (the rows still count every case).
pub const MAX_DETAILS: usize = 100_000;

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct Report {
    pub format_version: u32,
    pub counts: Counts,
    rows: BTreeMap<Case, usize>,
    details: Vec<Detail>,
    looser: Vec<LooserRule>,
    patterns: Vec<RefusedPattern>,
    refused: Vec<RefusedCopper>,
    placeholders: BTreeMap<Placeholder, usize>,
}

impl Report {
    /// Count one occurrence of `case`.
    pub fn add(&mut self, case: Case) {
        *self.rows.entry(case).or_default() += 1;
    }

    /// Count `n` occurrences of `case`.
    pub fn add_n(&mut self, case: Case, n: usize) {
        if n > 0 {
            *self.rows.entry(case).or_default() += n;
        }
    }

    /// Count one occurrence with its private detail.
    pub fn add_detail(&mut self, case: Case, footprint: &str, pad: &str, path: &str) {
        self.add(case);
        if self.details.len() < MAX_DETAILS {
            self.details.push(Detail {
                case,
                footprint: footprint.to_owned(),
                pad: pad.to_owned(),
                path: path.to_owned(),
                placeholder: None,
            });
        }
    }

    /// One footprint instance's 3D model outcome with its reason; a
    /// placeholder ([`Case::SourcePart`]) is also counted under its category.
    pub fn add_model(
        &mut self,
        case: Case,
        footprint: &str,
        reason: &str,
        placeholder: Option<Placeholder>,
    ) {
        let kept = self.details.len();
        self.add_detail(case, footprint, "", reason);
        if let Some(placeholder) = placeholder {
            *self.placeholders.entry(placeholder).or_default() += 1;
            if let Some(detail) = self.details.get_mut(kept) {
                detail.placeholder = Some(placeholder);
            }
        }
    }

    /// Placeholder counts by category.
    pub fn placeholders(&self) -> &BTreeMap<Placeholder, usize> {
        &self.placeholders
    }

    /// A limit not taken, counted and named.
    pub fn looser_rule(&mut self, rule: LooserRule) {
        self.add(Case::RuleLooserThanFab);
        if self.looser.len() < MAX_DETAILS {
            self.looser.push(rule);
        }
    }

    /// A net class pattern not taken, counted and named with its reason.
    pub fn refused_pattern(&mut self, pattern: &str, reason: &'static str) {
        self.add(Case::NetClassPatternRefused);
        if self.patterns.len() < MAX_DETAILS {
            self.patterns.push(RefusedPattern {
                pattern: pattern.to_owned(),
                reason,
            });
        }
    }

    /// A track piece or via the engine refused, counted under its case and
    /// named with the check that refused it.
    pub fn refused_copper(&mut self, refused: RefusedCopper) {
        self.add_detail(refused.case, "", "", &refused.check);
        if self.refused.len() < MAX_DETAILS {
            self.refused.push(refused);
        }
    }

    /// The refused track pieces and vias of `case`, each with its check, in
    /// the order they were tried.
    pub fn refused_copper_of(&self, case: Case) -> impl Iterator<Item = &RefusedCopper> {
        self.refused.iter().filter(move |r| r.case == case)
    }

    /// The net class patterns not taken, in the project file's order.
    pub fn refused_patterns(&self) -> &[RefusedPattern] {
        &self.patterns
    }

    /// The limits not taken, in the order the project file's rules were read.
    pub fn looser_rules(&self) -> &[LooserRule] {
        &self.looser
    }

    /// An unknown token outside pads, by path.
    pub fn unknown(&mut self, path: &str) {
        self.add_detail(Case::UnknownToken, "", "", path);
    }

    pub fn count(&self, case: Case) -> usize {
        self.rows.get(&case).copied().unwrap_or(0)
    }

    /// The counted rows, in case order.
    pub fn rows(&self) -> Vec<Row> {
        self.rows
            .iter()
            .map(|(case, count)| {
                let (code, fidelity, title) = case.describe();
                Row {
                    case: *case,
                    code,
                    fidelity,
                    title,
                    count: *count,
                }
            })
            .collect()
    }

    pub fn details(&self) -> &[Detail] {
        &self.details
    }

    /// For a case of [`NAMED_CASES`], each occurrence as one line naming
    /// what it is about (the file's names and values): for the person who
    /// imported their own project, never for a shared log.
    pub fn named(&self, case: Case) -> impl Iterator<Item = String> + '_ {
        self.details
            .iter()
            .filter(move |d| d.case == case && NAMED_CASES.contains(&case))
            .map(Detail::named)
    }

    /// The report for people: fixed text and counts only, never a name or
    /// value from the file.
    pub fn summary(&self) -> String {
        let c = &self.counts;
        let mut out = format!(
            "KiCad board format {}: {} copper layers, {} footprints ({} on the back), {} footprint definitions, {} of {} pads imported, {} nets, {} track pieces, {} vias, {} fills\n",
            self.format_version,
            c.copper_layers,
            c.footprints,
            c.footprints_back,
            c.definitions,
            c.pads_imported,
            c.pads,
            c.nets,
            c.tracks,
            c.vias,
            c.fills
        );
        if c.sheets > 0 {
            out.push_str(&format!(
                "  schematic: {} sheets; board nets equal {}, split {}, joined with another {}\n",
                c.sheets, c.schematic_nets_equal, c.schematic_nets_split, c.schematic_nets_merged
            ));
            out.push_str(&format!(
                "  imported schematic nets equal to the source schematic's: {} of {}\n",
                c.source_nets_equal, c.source_nets
            ));
        }
        let looser = self.count(Case::RuleLooserThanFab);
        if looser > 0 {
            let (rules, were) = if looser == 1 {
                ("rule", "was")
            } else {
                ("rules", "were")
            };
            out.push_str(&format!(
                "  {looser} KiCad {rules} looser than the fab profile {were} not taken\n"
            ));
        }
        let mut rows = self.rows();
        rows.sort_by_key(|r| (r.fidelity, r.case));
        for row in rows {
            let fidelity = match row.fidelity {
                Fidelity::Exact => "exact",
                Fidelity::Approximate => "approximate",
                Fidelity::Report => "report",
                Fidelity::Refused => "refused",
            };
            out.push_str(&format!(
                "  {fidelity:<11} {:>3}  {:>6}  {}\n",
                row.code, row.count, row.title
            ));
            if row.case == Case::SourcePart {
                for (placeholder, count) in &self.placeholders {
                    out.push_str(&format!(
                        "  {:<11} {:>3}  {count:>6}    {}\n",
                        "",
                        "",
                        placeholder.describe()
                    ));
                }
            }
        }
        out
    }

    /// [`Self::summary`] with each rule not taken named under its count,
    /// with the KiCad and fab profile values, and each net class pattern
    /// not taken with its reason: for the person importing their own
    /// project.
    pub fn summary_named(&self) -> String {
        let mut out = String::new();
        for line in self.summary().lines() {
            out.push_str(line);
            out.push('\n');
            if line.contains("looser than the fab profile") && line.ends_with("not taken") {
                for rule in &self.looser {
                    out.push_str(&format!("    {}\n", rule.line()));
                }
            }
            if line.ends_with(Case::NetClassPatternRefused.describe().2) {
                for pattern in &self.patterns {
                    out.push_str(&format!("    {}\n", pattern.line()));
                }
            }
            for case in NAMED_CASES {
                if line.ends_with(case.describe().2) {
                    for named in self.named(case) {
                        out.push_str(&format!("    {named}\n"));
                    }
                }
            }
            for case in [Case::TrackRefused, Case::ViaRefused] {
                if line.ends_with(case.describe().2) {
                    for item in self.refused_copper_of(case) {
                        out.push_str(&format!("    {}\n", item.line()));
                    }
                }
            }
        }
        out
    }

    /// The full report with its private details, for the local file.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "format_version": self.format_version,
            "counts": self.counts,
            "rows": self.rows(),
            "looser_rules": self.looser,
            "refused_patterns": self.patterns,
            "refused_copper": self.refused,
            "placeholders": self.placeholders,
            "details": self.details,
        })
    }
}
