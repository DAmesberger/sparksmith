# Mechanical parts and the part-file chooser

Parametric 3D parts are placed directly on the PCB from a [part file](part-files.md): solderable SMD nuts and standoffs (they have pads) and spacers and heatsinks (purely mechanical). The desktop's part-file chooser picks a file, checks it, edits its parameters and previews it; the same operations serve the CLI and MCP.

Footprints stay Sparksmith's: the part file supplies only the 3D body, never the land pattern.

## Variants

A directly placed part is an ordinary component instance. Its component, symbol, package, footprint and binding (the **mechanical part set**) are generated library definitions, published into the project snapshot as revision 1 by `library::publish` over a scratch draft set, so the user's drafts are never touched (`crates/editor-core/src/library/mechanical.rs`, `make_variant`). The package's model is the part file (`PackageModel::PartFile`).

- **Variant key.** Each variant (a part file revision, its values, its lands, the pad use and the assembly flags) is its own set, identified by `MechanicalPart.key`: SHA-256 over the canonical JSON of those (`variant_key`). The core computes it; a request never supplies it. The placement reuses the component lineage whose latest revision's package has that key (`find_variant`), so equal specs never make two sets. Equal values share one native definition and one BOM line; an M3 x 4 mm and an M3 x 6 mm nut are different variants.
- **Record.** `PackageDefinition.mechanical: Option<Box<MechanicalPart>>` holds `key`, `name`, `source` (a built-in name or a file name, display only), `pads`, `bom` and `pick_and_place`. Electrical rules never read it.
- **Editing values** selects the variant with the new key or creates it, and re-points the instance (`layout.set_part`). A newer revision of the part file is not applied to placed parts automatically: choosing the file again makes a new variant. The generated set can be edited in the Component workspace like any component; the edited package keeps its `mechanical` record, so the key names the variant's origin, not a guarantee that its footprint is unchanged.
- **The model.** The part file is a Model of format `solidcraft`, shared by asset hash: one Model per file revision in a project, whatever the number of variants (`PartModelSource::File` reuses an existing Model with the same hash; `::Model` names one).

## The generated set

| Definition | Content (`mechanical.rs`) |
|---|---|
| Package | The envelope is the part's derived bounds size (at least 0.01 mm a side); model `PartFile { model, values, derived }`; the `mechanical` record; the part file's declared datums. Terminals: one per copper land when `pads` is `terminals`, else none. |
| Footprint | One pad per land (below) and the standard footprint graphics over the body's xy bounds (`add_standard_graphics_over`): assembly outline, silkscreen outline, courtyard (`COURTYARD_EXCESS_MM` = 0.25 mm) and designator. |
| Binding | Package terminal n to component terminal n, or no mappings. |
| Symbol | One unit `A`: a body with the designator above and the component name below, one pin per terminal on the left. |
| Component | The designator prefix (1 to 8 uppercase letters), passive terminals, the symbol and binding, and the name `<name> <param value>...` (for example `SMD nut height 4 mm`), cut at `MAX_LABEL`. |

An instance placed on the board has the symbol's unit **unused** (off-sheet, `instances::place_off_sheet`), no occurrence and a `Placement`. A part whose pads land terminals can still be placed on a sheet and wired, for example to GND; connectivity keeps a single source, the schematic. Without a designator the core numbers the instance under the prefix.

A package may have no terminals (a fiducial, a mounting hole, a logo or a mechanical part; `library/package.rs:185-187`), which is what a purely mechanical set uses.

## Lands

A land is Sparksmith data in the footprint frame: `Circle { diameter, hole? }`, `Rect { size, rotation_deg }` or `Hole { diameter }` (`MechanicalLand`). At most 64 lands (`MAX_LANDS`); names are 1 to 16 bytes of `[A-Za-z0-9_-]`, unique within a part; sizes 0.05 to 100 mm (`LAND_MIN_NM`, `LAND_MAX_NM`) with a hole smaller than its land (`PartSpec::validate`).

- A copper land is a `PadKind::Mechanical` pad on the top side. With `pads: mechanical` (the default) it lands no terminal; with `pads: terminals` each copper land lands its own terminal `1`, `2`, ... and can join a net.
- A circle with `hole` has a round non-plated hole at its centre (an annular SMD-nut land) and four paste windows of 70 degrees on the ring, so the stencil never covers the hole; other lands inherit the footprint's paste.
- A `Hole` is a `PadKind::NonPlatedHole` pad with no terminal.

The host chooses the lands from one of these sources (`layout.place_part_file`, `session/mechanical_parts.rs`):

1. **Shipped land rules.** A shipped part's sidecar (`library/parts/NAME.part.json`) gives rules over its own parameter values: an `annulus` (a round copper land around a non-plated hole at the origin) or a `hole`, each diameter `scale x value(param) + add_mm` (`scale` defaults to 1; `session/shipped_parts.rs:14-25`). The SMD nut is an annulus of `outer_d + 0.6` around a hole of `thread_d + 0.2`; the round spacer a hole of `inner_d`; the finned heatsink has none.
2. **From a footprint** (`lands_from`): each pad of a project footprint becomes a land (position, circle or rectangle, hole; terminal dropped). Other pad shapes are refused (`footprint_lands`).
3. **Explicit** `lands` in the request. Not together with `lands_from`.
4. None: a purely mechanical part.

## Commands

**Core**, one undo step each, atomic (`crates/editor-core/src/layout/part.rs`):

| ID | Payload | Effect |
|---|---|---|
| `layout.place_part` | `{ part: PartSpec, position, rotation_deg, layer, designator? }` | Finds or makes the variant, places a new instance with its unit unused; result `Outcome::PartPlaced`. Refused, changing nothing, on an invalid spec, a non-finite position or rotation, or any instance or library rule. |
| `layout.set_part` | `{ instance, part: PartSpec }` | Re-points a mechanical instance to the spec's variant, keeping its id, designator and placement. Refused for an instance that is not a mechanical part, a locked placement, or a terminal change while the unit is on a sheet or a terminal is connected. |
| `layout.remove_part` | `{ instance }` | Removes a mechanical instance that has no occurrence, with its placement. Refused if the part is on a sheet or connected, or locked. |

`PartSpec` carries `name`, `designator_prefix`, `source`, `model`, `values`, `derived`, `lands`, `pads`, `bom` and `pick_and_place` (default: true when there is copper). The core never evaluates the part file: the host sends what it evaluated.

**Automation** (`session/mechanical_parts.rs`; MCP tools `library_part_files`, `layout_place_part_file`, `layout_set_part_file_values`):

| ID | Payload | Result |
|---|---|---|
| `library.part_files` (query) | `{}` | `{ builtin: [...], project: [...] }`: the shipped part files and the project's part-file Models. |
| `layout.place_part_file` | `{ path or builtin or model, values?, position, rotation_deg?, layer?, pads?, bom?, pick_and_place?, lands?, lands_from?, name?, prefix?, designator?, licence?, redistribution?, expected_revision }` | Reads and checks the file (blocking findings refuse), evaluates the land source, builds the `PartSpec` and applies `layout.place_part`. |
| `layout.set_part_file_values` | `{ instance, values, pads?, bom?, pick_and_place?, expected_revision }` | Evaluates at the new values with the instance's file and land source and applies `layout.set_part`; omitted flags keep the instance's. A shipped `source` re-evaluates its land rules; any other variant keeps its lands. |

`sparksmith part list` lists the shipped part files; `sparksmith part check` also prints shipped lands. The builders `use_part_file_operation`, `set_part_values_operation`, `place_part_operation` and `set_part_operation` only read the engine and return the operation and the check report; the desktop applies the operation through its own engine, so UI, CLI and MCP issue the same typed command.

## Desktop

- **Part-file chooser** (`apps/sparksmith/src/part_chooser.rs`): a sheet with a source list (**Shipped**, **This project** (Models of format `solidcraft`), **Recent files** (at most eight, per user) and **Open file...**) and a detail pane with the check report, the parameters as a form with units, the bodies and materials. Evaluation runs on a worker thread.
- **Component workspace, 3D tab, Model section.** **Part file...** opens the chooser and **Use** applies `use_part_file_operation`. A package that uses a part file shows its file, licence, size and a parameters form; each committed field (Enter or focus loss) is one `library.set_part_values`, one undo step. **Use envelope box** returns to the package's envelope. The Component 3D view previews the part file (`project::preview_part`).
- **PCB Layout.** The command `pcb.place-part`, **Place 3D part...**, has the key **M** in PCB Layout (while routing, M cycles the router instead). It opens the chooser in Place mode and places the part at the view centre on the side being viewed, then selects it; from then on it moves, rotates and changes side like any footprint.
- **Properties of a placed mechanical part.** The placement card has a **3D part** section: the source, the parameters form (each committed edit is one `layout.set_part`), **Pads** (mechanical or terminals), **In BOM**, **In pick-and-place** and **Remove from board** (`layout.remove_part`).

## Outputs

- **BOM and pick-and-place.** An instance is in the BOM when its component is a bought one and its package's `mechanical.bom` is true (or it has no mechanical record) and it is not excluded on the instance; pick-and-place likewise with `pick_and_place` (`library::mechanical::in_bom`, `in_pick_and_place`, `instances.rs:159-171`).
- The courtyard, assembly outline and silkscreen outline are the generated footprint's, so courtyard checks and the assembly drawing treat the part like any footprint.

## Shipped parts

Original parametric designs in `library/parts/`, authored through Solidcraft's commands by `cargo test -p sparksmith-solidcraft --test author_parts -- --ignored`, MIT OR Apache-2.0. Their dimensions are generic and illustrative (verify against the chosen part's datasheet). Each has a sidecar with title, prefix, use and land rules.

| Part | Prefix | Lands |
|---|---|---|
| `smd-nut` | H | annulus, `outer_d + 0.6` around a hole of `thread_d + 0.2` |
| `round-spacer` | H | hole `inner_d` |
| `finned-heatsink` | HS | none |

The shipped list also holds the ESP32-S31-WROOM-3 part file and the USB-C receptacle `gct-usb4105`, a library component with a part file; `sparksmith part list` shows each parameter.
