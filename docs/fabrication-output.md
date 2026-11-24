# Fabrication output (as built)

The founder asked for a full development cycle that ends in production files a board house can build from. This note records what was built: the formats, the conventions and the limits.

The Gerber writer follows Ucamco's public Gerber Layer Format Specification. The job file follows Ucamco's public Gerber Job File format, and the drill files follow the public Excellon description. All of the code is original. No other EDA tool's reader, writer or viewer was consulted, and the verification reader is original too.

## One command

- **Automation:** the `project.export_fabrication` request takes `{output, zip, profile, options}`. It runs the production check and then writes a new or empty folder, or one new `.zip`. It returns the pre-flight report, every file with its size, and the time each step took. It is a file effect: it adds no revision and no undo step.
- **CLI:** `sparksmith fab FILE.sparksmith OUT_DIR|OUT.zip [--zip] [--profile ID] [--date ISO|none]`. Files are named after the project file. `--profile` needs a value; without one the command is refused, never run under the saved profile. A blocked export prints the title of each unwaived error, up to 100 and then how many remain.
- **MCP:** the same request reaches MCP through the generic catalog.
- **Desktop:** PCB workspace → ribbon Output → Production, or "Production files…" in the command palette.
  - The dialog shows a profile picker, Folder or ZIP output, an output toggle for each group and the placement convention.
  - It lists the pre-flight report, computed on a worker, grouped by kind of finding. Tick findings (Shift-click ticks a run, a group's box ticks the whole group), or use a row's or a group's Waive…. The reason form opens under that row or group, and Waive applies one optional reason to every ticked finding as one undo step. A waived finding offers Restore.
  - Export runs on a worker and shows progress; the result lists the written files.
- **Shared path:** all of these call `sparksmith_io::fabrication::export`. The pure generation is `sparksmith_core::fabrication::generate`, the PDF drawings are `sparksmith_io::fab_drawings`, and `sparksmith_io::pdf` is a small generic PDF canvas that schematic printing can reuse.
- **One production snapshot:** an export resolves the project and the chosen profile once (`fabrication::ProductionSnapshot::resolve`), or not at all: the project with that profile in place of its own, the overrides and net classes kept, its choices carried and reconciled as a rules edit does it (`layout::restack::reconcile_rules`), and the active variant, the waivers, the stackup and the computed fills as the project holds them. The board is never changed to fit the profile: its colour option follows the board's solder mask colour, and its copper weight is the choice that orders the board's outer foils. A profile the design cannot take gives no snapshot (`fabrication.profile_refused`): a kept override conflicts with it, or none of its copper weights orders the board's foils (3 oz copper sent to a fabricator that offers 1 and 2 oz). The production check and every writer (Gerber, Excellon, the drill map and table, the job file, pick-and-place, both BOMs and both drawings) read that one project, so the job file's foils and design rules, the drill files' plated-hole sizes and the mask openings are the ones the check passed.
- **Writing:** a folder is written to a sibling staging folder and then renamed into place. A ZIP is written atomically, entries in set order with a fixed timestamp.

## Production check (`fabrication.preflight`)

The check reports every project issue under the chosen fabrication profile, on the [production snapshot](#one-command). Overrides and net classes are kept. A profile the design cannot take (a kept override conflicts with it, or it offers no copper weight for the board's outer foils) is refused with `fabrication.profile_refused` instead of a report: there is no finding to waive, so a waiver cannot turn the refusal into an export under the saved profile, and the export writes nothing. A profile for other boards (another copper layer count) or a chosen option whose prerequisites the board does not meet (thickness, finish) is an error too ([which boards a profile is for](board-rules-and-shape.md)). It adds production findings to those issues. The query's `profile` is an id of the user's profile library or a shipped profile (as for an export) or a whole profile; omitted, the project's own rules apply. A board without a fab profile is blocked (`rules.no_fab_profile`, not waivable).

**Errors** (they block export):
- unrouted connections and unplaced parts;
- stale fills, including fills computed under other rules;
- stored fill copper short of its clearance, off the board or in a keepout ([fill checks](board-rules-and-shape.md#copper-fills-and-keepouts));
- fills never filled;
- an unsolved outline;
- parts without a footprint;
- plated board slots that are not straight slots;
- every board design-rule error.

**Warnings:**
- parts with only an envelope 3D model;
- parts whose definitions have unpublished draft changes (the review gate for library changes);
- legend over a pad's mask opening;
- legend text past the board outline or over a cutout or slot (`fabrication.legend_outside`), one finding per text, naming it ("Designator J2 runs outside the board");
- legend text closer to those edges than the rules' silkscreen edge clearance, when one is set (`fabrication.legend_edge`). Mounting holes are not edges for either check. Legend lines are not checked: a part may overhang the board and draw its outline past the edge;
- purchasing to-dos such as a missing Part;
- component-area findings.

### What the board needs of its fab profile

A check whose limit is the fab profile's own rule (not an override's, a net class's or a class pair's) records what the board measured as a requirement (`diagnostics::Requirement`): a minimum rule at most the measured value (a clearance, a track width, a drill, an annular ring, a mask web, a copper angle, a sliver), a maximum at least it (the via aspect ratio), a copper layer count or a solder mask colour. Copper that touches or overlaps records none: no profile allows it.

- **What is needed.** The finding's `needs` (on `fabrication.preflight` items and `project.issues` rows) states the rule, the measured value, the profile's limit and the source the profile gives for it, and the requirement in neutral words: "Needs pad to unplated hole clearance ≤ 0.175 mm" (`rules::requirement::describe`). The words name the rule and the value, never a fabricator.
- **Suggestions are computed.** `fabrication.profile_suggestions {profile?}` checks the board under every installed profile (the user's library and the shipped ones; the session adds them to `candidates`, the desktop reads the same folders) exactly as an export would, through the [production snapshot](#one-command): a profile qualifies when the design can take it and no profile-limited finding is left. The answer lists the tightest need of each rule under the checked profile and the qualifying profiles, ordered by the fabricator hierarchy: the board's own fabricator's profiles first, then other fabricators', then profiles that name none, each group by name. With none qualifying it states only what is needed. It never changes the board.
- **Never a waiver.** A correctly drawn part is made by a profile that allows it, not waived (founder decision, 16 November 2026). A finding with needs offers no Waive… in the dialog or in Issues (`PreflightItem::offers_waiver`), and the engine refuses `fabrication.waive` of its key from any caller (`fabrication.not_waivable`, naming the need; decision 23 November 2026); the Production dialog lists the needs and each qualifying profile with "Switch to …", and an Issues row's Fix is "Switch to …" the first one. A switch is one `rules.set_fab_profile` built as the snapshot builds it (`fabrication::snapshot::switch`: overrides kept, the copper weight that orders the board's foils), so the board is then made exactly as it was checked; Export, while only such errors block it, says "Fix the design or switch to a fab profile that allows it first". The suggestions run on a worker, after the check.

**Waivers:**
- A plated board slot the drill file cannot route (`fabrication.slot_unroutable`) cannot be waived, a waiver of one saved in a file is ignored, and `fabrication::generate` refuses such a board: milled on the profile it would be unplated.
- A waiver accepts a design rule that is not kept: the files still show exactly the design. It never accepts files that would not be the design, nor a short. So these cannot be waived, and a saved waiver of one is ignored: a stale fill (`fill.stale`) and one never filled (`fabrication.fill_unfilled`), since the copper in the files would be another board's or missing; an outline that does not solve, is open or is not one board (`board.shape_unsolved`, `board.shape_open`, `board.shape_invalid`), since the last valid one would be made; a part without a footprint (`fabrication.no_footprint`) or that does not resolve (`instance.check_failed`), since its lands would be missing; and copper of two nets that touches (`board.overlap`, `fill.overlap`). A gap under its rule stays a `board.clearance` or `fill.clearance`, which can be waived; the waiver does not cover a later overlap of the same two objects. A row of one of these says what fixes it and "Cannot be waived"; Export, while only they block it, says how many to fix and that they cannot be waived.
- An export under another fab profile than the board's finds every fill stale, as their copper was computed under the board's rules: switch the board to that profile and refill.
- Incomplete board checks, unrouted connections and parts not on the board cannot be waived (`fabrication::preflight::NO_WAIVER`), nor can a lost or inexact dimension reference, a loop between the board and its linked design, or a board without a fab profile. A waiver of an unrouted connection would turn Issues to "No issues" while airwires remain and let export write a complete-looking job (e2e run 13, finding 37); a part to leave unassembled is placed and marked Not fitted in a variant. `fabrication.waive` refuses such a key, naming what resolves it; a waiver of one saved in a file is ignored, so the finding still blocks export, stays in Issues and adds no drawing note.
- An unwaived error blocks export, and nothing is written. Every underlying finding is checked separately, even when Issues groups several pad pairs or unrouted connections into one row. A waiver removes only its own finding before Issues groups the remaining findings.
- `fabrication.waive {keys, reason?}` waives one or more findings with one optional reason, and `fabrication.unwaive {key}` restores one. Each is a document operation and one undo step. A list with one empty or unprintable key is refused whole. The setting "Require a reason to waive errors" (Settings → Production, off by default) makes the engine refuse a waiver without a reason, from the dialog, the in-app assistant and headless CLI and MCP sessions alike: every host sets up its engine from the user's settings file the same way (`UserSettings::configure`).
- Waivers are saved in the project (`Project.production.waivers`, keyed by finding code plus the objects it names). A finding that changes identity needs a new waiver. Unfilled fills are keyed by fill ID; legend text findings name both the instance and the footprint graphic, so accepting one never accepts another.

## Files

`STEM` is the base name (the project file's stem in the CLI). In the desktop it is the project's name, which an Untitled project takes from its file on the first save. Until then an Untitled project's files take the name that save would give it: the KiCad board it was imported from (or the file a recovered session came from). A new project that is neither saved nor named has no name to give its files: Export is disabled with "Save or name the project first: the files take its name", rather than writing `Untitled-…` files.

| File | Content |
| --- | --- |
| `STEM-copper-lN-top/innerK/bottom.gbr` | `Copper,Ln,Top/Inr/Bot`, positive |
| `STEM-mask-top/bottom.gbr` | `Soldermask,Top/Bot`, **negative** (the image is the openings) |
| `STEM-paste-top/bottom.gbr` | `Paste`, positive; only when the side has paste |
| `STEM-legend-top/bottom.gbr` | `Legend`, positive; only when printed and not empty |
| `STEM-profile.gbr` | `Profile,NP`: outline, cutouts and non-plated slots with exact arcs |
| `STEM-plated.drl`, `STEM-non-plated.drl` | Excellon, plated and non-plated separately |
| `STEM-drill-map.gbr`, `STEM-drill-table.txt` | `Drillmap`: one symbol per tool; the text table |
| `STEM.gbrjob` | Job file: stackup, thickness, finish, outer design rules, files in layer order |
| `STEM-pos-top/bottom.csv` | Pick-and-place per populated side |
| `STEM-bom.csv`, `STEM-bom-parts.csv` | BOM grouped as `project.bom` groups it; one row per part |
| `STEM-fabrication.pdf`, `STEM-assembly.pdf` | Drawings, A3 landscape |

### Gerber X2 conventions

- **Format:** `%FSLAX46Y46*%` and `%MOMM*%` (1 nm), absolute coordinates, `G75` multi-quadrant arcs, `M02`. Unchanged coordinates are left out; `D01`/`D02`/`D03` are always written.
- **Apertures:** each is defined once, numbered from D10 in first-use order, with its `.AperFunction` set by `%TA` before its `%AD`.
- **File attributes:** `.GenerationSoftware`, `.CreationDate` (only when asked), `.ProjectId` (see [release identity](#release-identity)), `.SameCoordinates`, `.FileFunction` and `.FilePolarity`.
- **Copper order:**
  1. Computed pours on each net are united, keeping copper wherever any pour contributes it. The resulting fill regions are plotted largest first, each region dark and its holes clear (`%LPC`). An island inside another region's hole is smaller, so it is drawn after that hole is cleared; nothing drawn after the fills is erased.
  2. Tracks, as round-aperture `D01` draws, so their ends are exactly round.
  3. Vias (`ViaPad`).
  4. Plated mounting-hole rings (`WasherPad`).
  5. Pads (`SMDPad,CuDef`, `ComponentPad`, `FiducialPad,Local`).
- **Pads:**
  - Circles, axis-aligned rectangles and obrounds are standard apertures.
  - Rotated rectangles and obrounds, and rounded rectangles (any corner mask, bottom-side reflection applied), are literal-value macros of exact primitive-21 rectangles and primitive-1 circles.
  - Custom outlines are regions with exact `G02`/`G03` arcs; a grown custom outline follows the core's mitred offset.
- **Object attributes:** `.N` (net; empty for no net), `.P` (designator, pad number) and `.C` (designator).
- **Mask:**
  - Pads open by their mask rule; `Inherit` uses the effective design rules' mask expansion, not the footprint default.
  - Untented vias and mounting holes open by the same expansion.
  - Through-hole and NPTH pads open on both sides.
  - A notch that the growth closes inside a custom outline stays mask: the opening is a region with a cut-in, which clears nothing drawn before.
- **Paste:** `Inherit` shrinks by the rules' paste reduction, for surface pads without a hole; explicit reductions and windows are kept as authored. A custom outline that shrinks into several pieces prints every piece.
- **Legend:**
  - Footprint silkscreen strokes are drawn with a round pen, with exact arcs; filled shapes are regions.
  - Text uses Sparksmith's stroke font (`layout::legend`, now shared with the 3D silkscreen films).
  - The legend is not clipped at mask openings; the check warns instead.
  - Text an instance moved (`instance.set_text_placement`) prints where it was moved. The offset is in the footprint's own axes, so it turns and flips with the part; the turn is added in quarter turns. An offset is at most 1000 mm. The placement stays with the instance through new revisions of the same footprint, and goes when its footprint or text does.

### Release identity

Every Gerber and Excellon file (`.ProjectId,<name>,<guid>,<revision>`), the job file (`GeneralSpecs.ProjectId`) and both drawings' title blocks (Release, Project id) state one identity:

- **Name:** the project's name (`project.rename`).
- **GUID:** the project's own identity, `Project.production.project_id`, saved with the project. A project takes it when it is first saved: a digest of the file's name, the whole document at that moment and, in the desktop, random bytes, so two boards of one name are two projects. A session (CLI, MCP) adds no random bytes, so a replay saved under the same file name reproduces it byte for byte. It is kept through renames, edits, undo and redo, and a copy saved elsewhere keeps it. A project never saved has none; its files use the release digest in its place.
- **Revision:** the release, 16 hex digits of the SHA-256 of everything delivered, written without an identity (the creation date left out): every file of the set and, when the drawings are written, everything they print (`fabrication::drawing::DrawingData`, as canonical JSON, which the PDFs are laid out from). The same files give the same revision; any change to what a file or a drawing says changes it (a waiver's reason or a profile name only the drawing prints), and an edit that changes nothing delivered (a construction line) does not. A drawings-only export identifies its drawings. The CSV files carry no identity: their formats have no place for one.

### Excellon conventions

- **Header:** `M48`, X2 attributes as `; #@! TF.…` comments, `FMAT,2`, `METRIC`, and one `TnnCd.ddd` tool table per file, each tool preceded by its `TA.AperFunction` comment.
- **Body:** `%`, `G90`, `G05`, then decimal-point coordinates at 3 decimals, so no zero-suppression convention applies. The file ends with `T0` and `M30`.
- **Slots:** pad slots and plated straight board slots are canned `G85` slots with the tool's width. A board slot or circle is one only when its whole outline lies within 0.1 µm of the slot or circle read from its two half turns (`layout::physical::stadium`): arcs that bulge inwards, sides that are not parallel and open or non-tangent outlines are not.
- **Tools:** one per (plating, diameter to 1 µm, hit or slot, function), numbered from 1 across both files, plated first (T1–T3 plated, T4 non-plated), so a number names one tool in the drill table, the drill map, the fabrication drawing and either file. Excellon numbers tools per file, but a fabricator who loads both files into one CAM job, or reads the combined drill table, then never sees two tools called T1.

### Pick-and-place

- **Defaults:**
  - Each row gives the placement origin (the footprint datum) in mm from the board origin, the same origin the Gerber files use.
  - Rotation is degrees counter-clockwise seen from the top, with 0° the footprint as drawn in the library.
  - Bottom parts are written at their board angle.
- **Fab-specific conventions:**
  - Origin: the outline's lower-left corner or any point.
  - Bottom angle seen from below: `(180 − rotation) mod 360`, optionally with bottom x negated.
  - Rotation offsets by package-name pattern.
- **CSV format:** RFC 4180 quoting, LF line ends, UTF-8 without BOM.
- **Value column** (pick-and-place and both BOMs): the instance's value text, every typed field with a value as the instance shows it ("10 µF 16 V", "10 kΩ ±1%"), so every rating the part was specified with is kept. Without one: the chosen Part's MPN, else the component name for a part of more than two terminals (an IC's name is its identity), else empty. A passive's component name ("Chip capacitor 0603 (1608 metric)") is a package description the Package column already carries, so it is never written as a value; its Status reads "no part".

### Drawings

- **Fabrication drawing:**
  - the outline at a standard scale with overall dimensions;
  - every hole drawn with its tool's drill-map symbol;
  - the drill table and the stackup table;
  - notes from the effective rules: thickness, copper, minimum track and gap, drill, rings, mask, legend; and one per waiver;
  - a title block on every sheet ("Fabrication drawing, sheet k of n").
  - Nothing is cut: the tables and notes flow from the column beside the board onto further sheets (a column at the left and one over the title block), a table or the notes going on under a "(continued)" heading. A 24-layer board prints all 51 stackup rows and every note on two sheets (`a_24_layer_drawing_prints_every_stackup_row_and_note`). At most 64 sheets; more is refused, never cut.
- **Assembly drawing:** one page per populated side, with bodies, grey pads, designators and red pin-1 dots. The bottom page is viewed from below and is labelled as mirrored.

## Verification (measured 2 November 2026, macOS arm64, `ci` profile)

`fabrication::tests` holds an original Gerber/Excellon reader and a bit rasteriser that samples pixel centres. They are tested against hand-derived areas: circle and stadium flashes and draws, rotated macros, cut-in and clear regions, arcs, and Excellon slots.

`oracle` decides point by point what must be made, from the document's primitives alone: exact distances to lines and arcs, with no polygon library and none of the production geometry (pad growth, slot reading, placement, fill computation). `expected` paints it, and `compare` reads every written layer back, takes the XOR against it at 10 µm and allows no difference two pixels thick. Both boards below are compared.

`second` is a second opinion by another method, without the point oracle: every object of every file measured on its own and paired one to one with an object of the document whose area is a formula (a land, its grown or shrunk opening, a via, a track), the stored fill areas, the profile's contours and length, and the holes by size and plating. `layers` holds a four-layer board with two pours a layer drawn past its edge, the legend of both sides, the drill map and table against the drill files, and the job file against the files. Both have seeded faults.

`proof` holds one board with everything the files can hold to the oracle: each layer's image at points 3 to 60 µm either side of every edge, a two-way inventory of every file, hole, slot, contour and copper object, and the spacing of fill copper measured in the Gerber and Excellon files (`fabcheck::spacing`) against the rules' numbers. Every kind of output fault is seeded in the files, and each must be reported. What still shares code with production: the footprint a part resolves to, the effective rule values, the stored fill regions (their position and spacing are checked on their own), and the polygon library under `fabcheck::spacing`.

| Board | Layers | Largest XOR |
| --- | --- | --- |
| Showcase (`Engine::showcase`): 4 layers, bottom and inner fills | 4 copper, 2 masks, 2 pastes, profile, plated and non-plated drill | 0 mm² on every layer |
| Sketched fixture: 2 mm filleted corners, a round cutout, a non-plated and a plated slot, a plated ring hole and a bare hole, refilled fills | same | 0.0001 mm² on copper; 0.0009 mm² on the profile and plated drill (1 µm drill rounding) |

The 10,000-via plane (200 × 200 mm, one bottom fill) is generated in 0.12 s (23.8 MB, 13 files). Its bottom copper, top mask and plated drill read back with 0 XOR at 20 µm. The target was seconds.

The shipped showcase's whole CLI export, with the check, drawings and atomic write, takes 0.13 s; most of that is fsync.

The comparison window is offset by a non-round margin. Two holes of one fill can share an edge exactly on a grid-aligned pixel centre. An even-odd fill and clear-polarity regions then sample that edge differently: a measurement artefact of 0.1 mm² on the aligned grid, and 0 when offset.

Format tests check attributes, polarities, D-code reuse, macro text, escapes, determinism (same bytes twice; a date only when asked) and the job file's layer order and thickness. Automation tests export the shipped (sketched) showcase as a folder and as a ZIP. They also cover a refused used folder, and an export blocked under another profile (stale fills) with nothing written, then unblocked by waivers that survive save and reopen and undo one step each.

## Limits and follow-ups

- **Out of scope:** IPC-2581 and ODB++ are follow-ups.
- **Holes:** blind and buried vias are not modelled, so there are no drill-pair files; a document that held one is refused. A plated board slot that is not straight is refused by the check and by the writers.
- **Legend:** it is not clipped at mask openings or the outline (the fabricator clips; the check warns). The desktop's Designator section moves the silkscreen designator only; the command takes any footprint text.
- **Assembly variants:** the assembly files and drawing follow the active variant ([assembly variants](assembly-variants.md)); one export writes one variant.
- **Profile check:** the desktop dialog keys its check on the document, the check settings (Settings → impedance tolerance) and the picked profile's content. It reads the user's profiles when it opens and again at Export: a profile file edited since the check refuses that Export ("The fab profile's file changed since the check"), and the dialog checks the new rules first.
- **Drawings:** text widths use Helvetica's metrics; bold text is measured with the regular widths.
- **Desktop dialog:** the pre-flight time on large boards was not measured; the check runs on a worker. Cancelling during an export closes the dialog while the worker finishes writing.
