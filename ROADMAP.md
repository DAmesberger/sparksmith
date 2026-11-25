# Sparksmith roadmap

The [product plan](docs/sparksmith-plan.md) defines release scope. This file tracks execution; unchecked work is not implemented.

## Workflow support, 25 November 2026

What a person can do today, by workflow, with the evidence and the limits that matter. "Built" means implemented and tested in this repository; it is not a production-release claim, and no board has been built from its output. The external codebase review of 21 November 2026 found false passes in checking and fabrication and in file ownership. Fixes and regression tests for its probes are on `main`; that they close every case is not claimed.

| Workflow | Status | Evidence | Main limits |
| --- | --- | --- | --- |
| Component authoring | Built: Symbol, Footprint, Mapping and 3D modes, immutable revisions, multi-unit symbols, pin functions, generator families in WASM, datasheet-to-component through MCP | [editor evidence](docs/editor-evidence.md), [generator plugins](docs/generator-plugins.md) | Acceptance scenarios CE-01 to CE-07 remain open; typed field definitions need lower-level authoring |
| Schematic capture | Built: multi-sheet hierarchy, net labels, power ports, annotation, ERC, PDF printing | [schematic capture](docs/schematic-capture.md) | No buses |
| Schematic-to-board update and placement | Built: reviewed impact previews, one undo step | [editor evidence](docs/editor-evidence.md) | — |
| Routing | Built: manual, walkaround and push-and-shove, track drag with shove | [push-and-shove](docs/push-and-shove.md) | Through vias only (no blind, buried or laser vias); corner drags not built; no autorouter; a pair does not change layer around a pad in its way |
| Differential pairs, length tuning, impedance | Built | [high-speed routing](docs/high-speed-routing.md) | No return-path verification; pad and package delay excluded |
| Pours | Built: fills computed on a worker, thermal relief | [editor evidence](docs/editor-evidence.md) | The fill audit checks stored fills against the rules; the export read-back does not recompute their regions |
| Board checks and production check | Built | [fabrication output](docs/fabrication-output.md) | A via that a pour edit leaves on one pour only is not reported |
| Manufacturing output | Built: Gerber X2, Excellon, job file, BOM, pick-and-place, fabrication and assembly drawings; the ESP32 example re-exports byte for byte | [fabrication output](docs/fabrication-output.md) | No IPC-2581, ODB++ or panels; the example board is not reviewed by a person or built |
| Assembly variants | Built: population and fitted part per variant, feeding BOM, placement, drawings and 3D | [assembly variants](docs/assembly-variants.md) | One variant per export |
| 3D view and native export | Built: board plan meshed live, exact `.solidcraft` export, parity gate | [board plan and interface](docs/architecture/board-plan-and-interface.md) | A cold 3D open of a 10,000-via board takes seconds ([large board](docs/editor-evidence.md#large-board-benchmark)); its exact export is far from that ([build cost](docs/pcb-realism.md#build-cost-measured)) |
| Linked mechanical designs | Built: published datum slots, linked-design preview and publication (slices B0–B4, B6) | [board interface](docs/architecture/board-interface.md) | Live update of a linked design is not built |
| KiCad import | Not in the product (founder decision, 25 November 2026): the translator of board, schematic, rules and zones stays as a building block of a coming AI-assisted import | [development](docs/development.md#kicad-import-comparison) | No menu, palette or command-line import |
| Firmware pin configuration | Built: JSON, C header, Rust and Zephyr emitters; the C header is compiled in tests | [component model](docs/component-model.md) | Pin configuration only, no clocks or registers |
| Automation and the assistant | Built: CLI, JSON lines and MCP through one front door; the built-in assistant uses the same verbs | [extension surface](docs/extension-surface.md), [assistant panel](docs/assistant-panel.md) | — |
| Files and recovery | Built: atomic saves, recovery snapshots, offline reopen | [development](docs/development.md) | A save that finds the file changed keeps both versions; another program writing in place at the same moment is not excluded |
| Platforms | Linux CI on every push; Windows CI on `main` ([CI](https://github.com/DAmesberger/sparksmith/actions/workflows/ci.yml)); Linux driven in the window on 24 and 25 November 2026; macOS driven in the VM, last full run 22 November 2026 ([run 13](docs/e2e-run-13.md)) | [development](docs/development.md#build-and-run) | No macOS CI job for open, save and quit; Windows not driven by hand |

## Milestone 0 — prove the foundation (open items as of 22 November 2026)

- [x] Inspect and pin the Solidcraft research baseline.
- [x] Record product, library, native integration and copper-performance requirements.
- [x] Adapt repository agent rules and task skills with upstream attribution.
- [x] Bootstrap the Rust workspace and executable checks.
- [x] Create native board/component geometry with retained placement expressions; test regeneration in a fresh upstream engine session.
- [x] Prove native parametric Solidcraft model composition and independent reopening for the board/component fixture.
- [ ] Prove volumetric copper, shared 2D/3D selection and measured local updates.
- [ ] Prove library identity/mapping updates and routing transactions on a small fixture.

Implement in the order and against the acceptance evidence in [Milestone 0](docs/milestone-0.md). An isolated prototype is not a shipped capability; record remaining shared-engine gaps explicitly.

The native fixture proof is complete; see [foundation evidence](docs/foundation-evidence.md). The copper sample, desktop shell and first component/schematic/PCB editors are implemented; see [editor evidence](docs/editor-evidence.md). Editor development proceeded in parallel at the user’s request. Milestone 0 remains open: scaling/local-invalidation benchmarks and complete library-update/routing proofs are not yet satisfied.

## Working editor foundation

- [x] Shared Solidcraft UI dependency, source-referenced shell, fonts, controls and renderer.
- [x] Initial component forms for symbol pins/body, rectangular footprint pads and mappings, with immutable local snapshots; complete authoring scenarios remain pending below.
- [x] Single-sheet terminal wiring, schematic properties and basic electrical checks.
- [x] Multi-sheet capture: net labels, power ports, sheets with a hierarchy and repeated child definitions, annotation across sheets, sheet furniture with a title block, label/hierarchy/power ERC and schematic PDF printing ([schematic capture](docs/schematic-capture.md)). Buses remain.
- [x] Explicit schematic-to-board sync, placement, manual routing and physical-island checks.
- [x] Parametric PCB/pad/trace/envelope `.solidcraft` generation and independent engine regeneration tests.
- [x] Atomic `.sparksmith` save, offline reopen, shared command engine and undo/redo.
- [x] One typed engine for UI, CLI and MCP; persistent net identity; engine-owned definition drafts; file dialogs, background saves and crash recovery.
- [x] Manufacturing output: Gerber X2, Excellon, job file, pick-and-place, BOM, fabrication and assembly drawings, a production check with saved waivers, and one export command for UI, CLI and MCP ([as built](docs/fabrication-output.md)).
- [ ] Complete first-release editors, library updates and manufacturing workflow.

## Component-editor acceptance

The [accepted authoring contract and scenarios](docs/part-library-design.md#accepted-authoring-contract) define target behavior, not current capability. New Component is the entry point; component categories and templates are optional. Basic multi-unit symbols and structured multi-function IOs with instance-level selection belong to the first complete editor. Establish unit/function identities against CE-04–CE-06 before implementing the CE-01 UI, then deliver the scenarios in order within the component-editor phase. The [component model](docs/component-model.md) is the implementation contract for the definition, typed-field and terminal-function slices that precede CE-01. Slice 1 (revisioned definitions, drafts, exact references and multi-unit symbol occurrences) is implemented, and the headless CE-04/CE-05 steps listed in the [editor evidence](docs/editor-evidence.md#verification) pass through the engine. Slice 2 (revisioned quantity kinds and typed fields, component field selections and typed instance values with reviewed field updates) is implemented headlessly, with text editing of symbol-shown fields in the schematic inspector; fields are not yet authored in the component editor. Slice 3 (terminal function arrays, device-NC and reserved classes, shared resources with capacity and constraints, per-instance function selections, availability per implementation and function-aware ERC) is implemented headlessly, with the headless CE-06 function steps listed in the editor evidence; since 1 November 2026 the desktop authors multi-function pins, peripherals and Rhai pin rules in the Pins tab and assigns them in the schematic pin-mux panel ([pin functions as built](docs/component-model.md#pin-functions-peripherals-rules-pinout-import-and-firmware-c1c7-as-built)). The scenario boxes below stay open until their UI, automation and native-model evidence exists.

- [ ] CE-01: generic resistor, reusable typed fields, alternate symbols and independently revisioned physical choices.
- [ ] CE-02: non-polarized/polarized capacitors, tolerance semantics, polarity and manufacturer requirements.
- [ ] CE-03: inductor, derived fields, generator recipes, manual overrides and offline regeneration.
- [ ] CE-04: dual transistor, independently placed units and one physical component.
- [ ] CE-05: dual amplifier, explicit power unit and unused-unit handling.
- [ ] CE-06: MCU-style IC, structured pin-function arrays/directions, resource constraints, repeated supply connections and exposed-pad obligations.
- [ ] CE-07: connector, electrical shield, mechanical holes, package variants and model datums.

Open decision before the first release: a symbol at every graphics limit encodes to about 1.4–1.7 MB of pretty JSON (corner radii included), so only ten to twelve such revisions fit the 16 MiB project file. Choose a compact encoding, pruning of unreferenced revisions or lower vertex limits; see [encoded size](docs/component-model.md) in the component model.

All scenarios require the common UI/automation, transactional, offline and native-model evidence in the spec. The [Rhai integration proof](docs/part-library-design.md#rhai-integration-proof) passes its bounded arithmetic/generation tests, but does not complete any full scenario. Buses, automatic gate/pin swapping and firmware generation beyond pin configuration (clocks, registers, HAL) remain separate work; the firmware pin-configuration export is release 1 scope.

## Subsequent milestones

1. Component authoring, project snapshots, single-sheet schematic and ERC.
2. Schematic-to-board updates, placement, routing, DRC and integrated 3D.
3. Pours, manufacturing output, native Solidcraft delivery, updates and recovery; complete the first-release workflow.
4. Release 2: extend features according to real board needs. Push-and-shove routing is built ([push-and-shove](docs/push-and-shove.md)).

Candidates, not yet scheduled:

- **ODB++ export** (user decision, 23 November 2026), from the same production snapshot as the Gerber set. Professional users name its absence as a reason to rule a tool out (comments on Hackaday's KiCad 9 review, 2025).
- **Mechanical parts as a generator.** The nut, spacer and heatsink are shipped part files with land rules in their sidecars (`library/parts/*.part.json`); a `mechanical` generator would compute the lands in the module and retire the sidecar rules. `layout.place_part_file` with a person's own file and explicit lands stays as it is.
- **The envelope as a part file.** A shipped one-box part file would replace `PackageModel::Envelope`.

## Current hardening sequence

At the user's request, execute in this order:

1. **Component and schematic updates:** revision-bound impact previews, explicit affected-copper handling and one-step undo. Implemented, including live airwire preview during component dragging; tests cover stale/cancel/locked/failed updates.
2. **File handling and recovery:** native dialogs, unsaved-work transitions, portable unpublished drafts and crash recovery. Implemented; Cmd+Q and the app menu's Quit ask about unsaved work. Dock › Quit and a logout on macOS do not (the recovery snapshot has the work).
3. **Performance:** measure representative boards, then remove whole-model reconstruction from local edits. Keep timings and remaining limits explicit.

Parametric design is an additional user requirement: author footprints and board geometry using Solidcraft's parameter/expression facilities. Placement references must support board edges and other components, with independently selectable placement origin, body center and pad-center anchors. Rotation and bottom-side transforms must be explicit. Persist dependencies and stable electrical identities, reject cycles/missing references, and preview footprint-driven routing changes. This follows the hardening sequence; numeric editor fields alone do not satisfy it.
