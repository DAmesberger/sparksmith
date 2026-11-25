# Sparksmith

**Parametric PCB design software with integrated 3D.**
Parts, schematic, board layout and 3D in one app, on [Solidcraft](https://github.com/tangency-labs/solidcraft)'s CAD engine. Alpha: see [Status](#status) before you use it for a real board.

Formerly Pcbcraft. Renamed to avoid confusion with tangency-labs' PCBCraft, a separate project.

<img src="docs/media/highlight.webp" width="960" alt="Highlights: a board designed from schematic to 3D, a board parameter changed with everything following, a part dimensioned in 3D, the realistic view, and a track dragged with push-and-shove">

## Why it's different

**Parametric boards.** A board is a design with named parameters and expressions, not a fixed drawing. Width, height and corner radius drive the outline, and placements, fills and keepouts can be tied to its named edges. Footprints are constrained sketches. Part bodies can be parametric Solidcraft models, and common generic parts (chip R, C and L, LEDs, diodes, headers) are made on demand from what you type.

**3D is the same design, not an export.** Copper, pads, plated holes, vias and fills have their real thickness. Layout and 3D are two views of one board, with one selection, one undo history and one set of properties. Pick a track in 3D and it is selected in the layout. The whole board also saves as a native, parametric `.solidcraft` model, built by Solidcraft's engine.

**The whole workflow is there.** Multi-sheet schematics with ERC, push-and-shove routing, differential pairs, length tuning, copper fills, design rules, and production files behind a check. Schematic to Gerbers in one app, with the [limits below](#known-limits).

**Scriptable, and AI if you want it.** Every change, from a wire to a published part, is a typed command on one engine. The desktop app, the command line and the MCP server send the same commands, and a refused command leaves the design untouched. Nothing needs an AI model. If you connect one, an assistant can work on a file in the background or, when you allow it, live in the open window, on your undo history. One use is drafting a part from its datasheet: the model cites every value, Sparksmith checks the citations, and you review and publish.

## What it does today

<table>
  <tr>
    <td width="50%"><img src="docs/media/parametric.webp" alt="Board parameters changed: the outline, the parts held to its edges, their tracks and the fills follow"><br><b>Parametric boards.</b> Change a parameter; the outline, the parts held to it, their tracks and the fills follow.</td>
    <td width="50%"><img src="docs/media/3d-dimension.webp" alt="A face of the USB-C receptacle picked in 3D and dimensioned to the board edge; the part moves to the typed value"><br><b>Dimensions in 3D.</b> Pick a part's face and a board edge, type a value, and the part moves.</td>
  </tr>
  <tr>
    <td><img src="docs/media/ai-part.webp" alt="A model reads a regulator datasheet and drafts the part with the app's own tools; every value cites its page"><br><b>Parts from datasheets.</b> A model drafts symbol, typed pins, footprint and 3D, citing every value; you review and publish.</td>
    <td><img src="docs/media/shove.webp" alt="A route and a dragged track push neighbouring tracks aside live"><br><b>Push-and-shove.</b> Route or drag a track; other nets make room live, in one undo step.</td>
  </tr>
  <tr>
    <td><img src="docs/media/realistic.webp" alt="The board in the realistic studio view, orbiting"><br><b>Realistic 3D.</b> The same model in a studio look, one key away.</td>
    <td><img src="docs/media/3d-layers.webp" alt="The 3D board in the realistic view, peeled layer by layer down to the laminate"><br><b>Real copper.</b> Peel the stackup layer by layer, in the realistic view too; copper, holes and fills are solids.</td>
  </tr>
  <tr>
    <td><img src="docs/media/timelapse.webp" alt="A new design from schematic to a placed, routed board in 3D, sped up"><br><b>End to end.</b> Schematic, placement, routing, fills and 3D in one app (sped up).</td>
    <td><img src="docs/media/components.webp" alt="The ESP32-S31 module's pins with their typed function lists"><br><b>Typed pins.</b> Pins carry typed function lists, peripherals and pin-mux rules.</td>
  </tr>
  <tr>
    <td><img src="docs/media/production.webp" alt="The production check reports no issues and the export writes the production files"><br><b>Production.</b> A pre-flight check, then Gerber, drill, pick-and-place, BOM and drawings.</td>
    <td><img src="examples/esp32-s31-devboard/images/app-pcb-2d.png" alt="The ESP32-S31 dev board in the PCB layout view, routed, with its top copper fill"><br><b>The reference board.</b> A 4-layer ESP32-S31 dev board, from parts to Gerbers.</td>
  </tr>
</table>

- **Components.** Symbol, footprint, pin functions, mapping and 3D in one workspace. Publishing creates immutable revisions. A project keeps an exact snapshot of its library, so it reopens offline, and a library change reaches a board only through a reviewed update.
- **Schematic.** Several sheets with a hierarchy, net labels, power ports, title blocks, annotation, ERC and vector PDF output. The Place palette searches with units: `res>=10k 0603`.
- **PCB.** 2 to 64 copper layers with a typed stackup. Interactive routing that shoves or walks round other nets. Differential pairs at a gap derived from the stackup's impedance, and length meanders. Fab profiles, net classes, copper fills with thermals, and parametric board outlines.
- **3D.** One key from the layout. Peel the stackup layer by layer. Generated part models with markings, and imported STEP, GLB and `.solidcraft` models.
- **Production.** Gerber X2, Excellon, a job file, pick-and-place, BOMs, and fabrication and assembly drawings. Tests read every Gerber layer back and compare it with the design.
- **Automation.** MCP (`2026-07-28`: six verbs over the whole catalog) and JSON lines over the same commands as the UI, including live MCP into the open window.

The reference design is a 4-layer [ESP32-S31-WROOM-3 dev board](examples/esp32-s31-devboard/README.md): USB-C, a 90 Ω USB pair, an LDO, 20 parts on the BOM. An AI agent designed it from parts to Gerbers through the command engine. ERC is clear, and the production check reports no errors and ten warnings. No person has reviewed it, and it has not been built.

## Where it's going

The goal is one workflow from idea to product: design, simulate, test without hardware, then manufacture, with an AI assistant for the parts of it you want to hand over. Everything in this section is **planned**, not built.

- **Circuit simulation.** Analog simulation in the style of LTspice, run from the same schematic.
- **The board as a digital twin.** Emulate the microcontroller running your real firmware: for example, an ESP32-S31 running compiled Rust. UART, GPIO, LEDs, displays and buttons work on the 3D board. Press a button in 3D and watch an LED blink. Write and test firmware before the board exists.
- **Smart parts.** Parts that carry electrical and behavioural models next to their symbol, footprint and 3D body, mapped pin by pin, so they simulate both as circuits and in software.
- **From a description to a board.** An assistant that takes a description to parts, schematic, layout, virtual testing and the production files, with every step open to your review.

Nearer steps, also planned: build and test the ESP32-S31 reference board; buses. The [roadmap](ROADMAP.md) tracks the work, and the [product plan](docs/sparksmith-plan.md) defines the first release.

## Status

Alpha. It is a working editor, not a manufacturing-ready release. **No board has been built from its output yet.** The board files are not stable: a file written today may be refused by a later version, so keep your own copies and do not rely on it for work you cannot redo.

The built-in fab profiles hold limits from fabricators' published pages (JLCPCB, PCBWay, and for the neutral Standard profile also OSH Park and Eurocircuits). Each rule names its source and each profile the date its pages were read; defaults the author chose say so. Check them against your fabricator's current page before you order.

What was checked, and how:

| System | Automatic checks | Driven by hand |
| --- | --- | --- |
| Linux | `cargo xtask ci` on Ubuntu for every push; see [CI](https://github.com/DAmesberger/sparksmith/actions/workflows/ci.yml) | Most of the workflow driven in the window on 24 and 25 November 2026 (one machine, one desktop, AMD graphics) |
| Windows | The same checks on pushes to `main`; see [CI](https://github.com/DAmesberger/sparksmith/actions/workflows/ci.yml) | Not driven by hand |
| macOS (Apple silicon) | None in CI | Developed there; last full run in the window on 22 November 2026 ([run 13](docs/e2e-run-13.md)) |

Driven by hand means a test agent using the window as a designer would, not a manufacturing test. [Editor evidence](docs/editor-evidence.md) records the checks made during development.

### Known limits

- No board has been built or reviewed by a person from its output, the ESP32-S31 reference board included. Its production check reports ten warnings.
- Only through vias. No blind, buried or laser vias, and no autorouter.
- A differential pair does not change layer around a pad in its way: it refuses, and you go another way.
- Exporting a board with thousands of vias as an exact `.solidcraft` model is slow: about 3 minutes for 1,000 vias on a shared machine, and one run with 10,000 took 36 minutes. The live 3D view does not need that model: a 10,000-via board took about 4 seconds to open in 3D, measured once on one Mac.
- A via left on one pour only, after a pour is edited, is not reported.
- Dragging one corner of a track is not built, and neither are buses in the schematic.
- Lengths are in millimetres. There is no setting for units or language.
- No ODB++, IPC-2581 or panels. KiCad import lists in its report what it left out or approximated.
- Windows, exFAT drives, network shares, OneDrive folders and other desktops or graphics cards on Linux have not been tried by hand.
- On macOS, quitting from the Dock ends the app without asking about unsaved work; the recovery snapshot has it.
- There is no installer: you build it from source.

Open source under MIT OR Apache-2.0. The part models, the exact 3D export and the viewport come from [Solidcraft](https://github.com/tangency-labs/solidcraft), a parametric CAD application in Rust, and the interface code started from its MIT or Apache sources. Its native file format holds the saved 3D model, and its interface conventions shape this one. We are grateful to its authors.

## Quick start

Needs Rust (the toolchain is pinned in `rust-toolchain.toml`) or Nix.

```sh
nix develop    # optional: the pinned toolchain without rustup
cargo run --locked --profile ci -p sparksmith -- examples/esp32-s31-devboard/esp32-s31-devboard.sparksmith
```

Open the PCB workspace, press **D** to switch between layout and 3D, and **Ctrl+P** (⌘P on macOS) to find any command. **F1** opens the help. Start with the guide [Make your first board](library/help/topics/guides/first-board.md).

## Documentation

- [Development](docs/development.md): build, checks, crate layout, command line, JSON lines and MCP.
- [Architecture and product plan](docs/sparksmith-plan.md), and the [component model](docs/component-model.md).
- [MCP and AI integration](docs/llm-integration.md), including [live MCP](docs/development.md#live-mcp) and the [datasheet assistant](docs/datasheet-assistant.md).
- [Keyboard shortcuts](docs/keyboard-shortcuts.md), [push-and-shove routing](docs/push-and-shove.md), [high-speed routing](docs/high-speed-routing.md), [fabrication output](docs/fabrication-output.md).

## Licence and attribution

Licensed under the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE), at your option, like Solidcraft. Third-party notices are in [NOTICE](NOTICE) and [ATTRIBUTION.md](ATTRIBUTION.md).

This is an independent clean-room implementation. It uses public specifications and permissively licensed dependencies, and contains no code or library data from GPL-family or commercial EDA tools. Manufacturer CAD data is used only like a datasheet, with its source and licence recorded.
