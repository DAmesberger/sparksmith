# Generator modules

A generator is a plugin of the one plugin ABI ([extension ABI §2.6](../docs/design/extension-abi-design.md)) that offers the capability `generate-part`, of effect `none`: a pure function in a WebAssembly module, typed parameters in, a complete part body out (symbol, package, footprint, and the 3D model as one of its bundle's part files with values). Each directory here is one generator crate that builds to `wasm32-unknown-unknown`. The modules are loaded as data, never linked into the app; the host that runs them is `crates/generators` (`sparksmith-generators`).

Status: slices GP0, GP1 and GP2. A part can be made from a module through the command engine (`library.generate`, `library.generate_part`); the project embeds the module and checks the part against it on every open; the module names one of its bundle's `.solidcraft` part files and its values as the 3D model, which the host evaluates through Solidcraft like any part file ([generator plugins](../docs/generator-plugins.md)). The catalog, the gallery and the part editor make parts through the shipped modules. ABI 1 is open to change until the first release.

The stand-alone estimate for inherited pad mask expansion is in `standalone-mask-expansion-nm.json`, in nanometres per side. Its 50 µm value preserves the original Sparksmith footprint prototype and its measured generator fixtures. A board profile supplies its own sourced value through `project.mask.expansion`. The standard parameter `board_mask_expansion_nm` records that value with the part for offline regeneration.

## Layout

| Path | What |
| --- | --- |
| `crates/generator-abi` | `sparksmith-generator-abi`: the ABI 1 wire types (`v1`), the canonical JSON codec, the parameter check, and the `GeneratorRuntime` trait. `no_std` + `alloc`, no generator logic. |
| `crates/generator-sdk` | `sparksmith-generator-sdk`: what a generator is written with: `Generator` and `export_generator!`, the module's allocator and panic handler, typed parameters, symbol drawing, footprint drawing (courtyard, silkscreen broken round the pads, assembly), and the golden-test harness. |
| `crates/generators` | `sparksmith-generators`: the wasmi host, with `Limits` and the output checks. |
| `crates/generators-probe` | A test module: the host itself built for wasm32, so the native tests can show the wasm32 build gives the same results. Never shipped. |
| `crates/generators-fixture` | A test plugin (`sparksmith-gen-fixture`): `generate-part` beside an `edit` command, so it may import the host functions; during `generate-part` it calls all three and draws a 45° arc. Its golden case runs natively, under wasmi and under the probe. Never shipped. |
| `generators/<name>/` | One generator: `Cargo.toml` (cdylib), `build.rs` (the part files' SHA-256 as constants), `src/`, `presets.json`, `golden.json`, `parts/` (its part files, each `<stem>.solidcraft` with a `<stem>.part.json` sidecar in the shipped part files' format: title, summary, licence, provenance). |
| `library/generators/<name>/` | The shipped **bundle**, committed: `generator.json` (the manifest), `<name>.wasm` (the module; its SHA-256, which the manifest records, is the module's one hash) and `parts/*.solidcraft`. `build --ship` writes it, `check` verifies that the source builds to it. |

`cargo xtask layers` keeps it apart: a generator crate depends on the SDK and the ABI only, only generator crates depend on the SDK, nothing depends on a generator crate or on the probe, and the ABI crate depends on no workspace crate (so `sparksmith-core` can use its types and trait without an interpreter).

## ABI 1: the one plugin ABI

**Exports.** A module exports its memory as `memory` and four functions:

| Export | Signature | Meaning |
| --- | --- | --- |
| `sparksmith_abi` | `() -> u32` | Returns 1. |
| `sparksmith_alloc` | `(len: u32) -> u32` | A buffer for a request (or a reply the host hands back from `call`), or 0 when `len` is 0 or over 64 KiB. |
| `sparksmith_describe` | `() -> u64` | `(ptr << 32) \| len` of the canonical `Description`. |
| `sparksmith_invoke` | `(ptr: u32, len: u32) -> u64` | Reads the canonical `Invoke` (`{abi, capability, params, scope?}`) from the buffer and returns `(ptr << 32) \| len` of the response: a status byte (0 Ok, 1 Err), then the capability's result (a canonical `Generation` for `generate-part`) or `{"errors":[…]}`. |

**Imports.** At most the three functions of module `sparksmith`, all optional: `call (ptr, len) -> u64`, `log (level, ptr, len)` and `cancelled () -> u32`. Anything else, or one of these with another signature, is refused before the module runs. This host serves capabilities of effect `none` only: during such an invoke it answers `call` with the fixed refusal `plugin.pure` (`v1::pure_refusal`, written into a buffer the module allocates), drops `log` and answers `cancelled` with 0, so the result depends on the request alone. A plugin whose capabilities are all of effect `none`, a plain generator, may import nothing at all (`generator.has_imports`). The SDK's `host` module wraps the three; natively it answers as the host does in a pure invoke.

**Description**, one format for every plugin: `{abi, id, version, title, licence, deterministic, capabilities: [{name, effect, title?, params, rules, presets, part?}], needs: [{module, major, minor?, class}]}`. `effect` is `none`, `read` or `edit`; a generator's `generate-part` declares `none` and its `part` holds the families, scopes, search phrases and instance fields. A plugin offering any capability of effect `none` declares `deterministic: true`; a plugin whose capabilities are all of effect `none` has empty `needs`. The host refuses a description that breaks any of this (`generator.invalid_description`).

**Floats are refused.** The host validates every module without the WebAssembly float feature (`wasmi::Config::floats(false)`), so a plugin whose results are hashed contains no float instruction or type (`generator.float`). The ABI crate's codec has its own integer-only reader, writer and serde (de)serializer, so no float code reaches a module; the SDK computes arc ends with integer trigonometry (`draw::arc`, `draw::cos_sin`, a Taylor series in a 2^60 fixed point, exact at quarter turns).

The host checks a module before anything runs (size, a valid core module of the allowed features, imports, no start function, the four exports and the memory, memory within the limit), then gives every call a fresh instance with a fuel budget and a memory cap, decodes the reply with the canonical decoder and checks it: unique keys, every reference resolving (pads to package terminals, symbol pins and binding to component terminals), arcs whose ends lie on one circle about their centre within 1 nm, values in range, and the parts the scope requires present and no others.

**Canonical JSON.** Object keys sorted by their UTF-8 bytes, each once; no whitespace; integers only (`i64`/`u64`, shortest form, so no floats, exponents or `-0`); strings escaped as `serde_json` escapes them, otherwise literal UTF-8, and in Unicode NFC (checked by the host); absent optional fields omitted, never `null`. The decoder accepts only bytes the encoder would write. Lengths are integer nanometres and angles integer millidegrees.

**Request.** `{abi: 1, scope: component | package, params: {key: value}}`, sent inside the invoke envelope for `generate-part`. The parameters are complete: the host fills every default from the description first (`sparksmith_generators::params::complete`), so the part key never depends on a module's defaults. Parameters a project setting decides (the least chip land gap, the resistor symbol style) are declared as `context` parameters and frozen into the request.

**Generation keys (slice B1).** Every generated object has a key: terminals, units, symbol drawings (`Graphic.key`, unique within its unit), package terminals, pads, footprint drawings (unique across silkscreen and assembly). The host gives each key a local id and records the key-to-id map with the part; regenerating keeps every key's id and gives new keys new ids, so ids do not depend on the order a module emits things in (`docs/generator-plugins.md`, G4).

**Identity (slice B4).** Part key: the generator's `id` with the params hash, SHA-256 of the canonical `Request` (ABI, scope, complete parameters as recorded; `sparksmith_core::library::generated::key`). The version tells revisions of that lineage apart. Result hash: SHA-256 of the canonical `Generation` bytes (after the status byte), the integrity check. Module hash: SHA-256 of the module bytes, which the check on open runs. Bundle hash: SHA-256 of the manifest's bytes, the provenance a person approves. No part key depends on the module's bytes, so a part made from the same source is the same part on every host.

**Refusals** are `{code, field?, message}` with a sentence a person can act on: `param.unknown`, `param.missing`, `param.type`, `param.out_of_range`, `param.inconsistent` and `param.scope` for parameters; `capability.unknown` from the SDK; `generator.too_large`, `generator.invalid_module`, `generator.unsupported_feature`, `generator.float`, `generator.has_imports`, `generator.missing_export`, `generator.abi_mismatch`, `generator.out_of_fuel`, `generator.memory_limit`, `generator.trapped`, `generator.output_too_large`, `generator.request_refused`, `generator.invalid_output`, `generator.invalid_model` and `generator.invalid_description` from the host. Every host refusal ends in "It was not run." or "Nothing was changed."

**Approvals.** A person approves a plugin by its bundle hash, per user, outside any project (`sparksmith_io::plugin_approvals`, `plugin-approvals.json` in the config directory; `sparksmith plugin approvals | approve NAME | revoke SHA256`). A changed bundle of an approved plugin asks again and says what changed (version, capabilities and their effects, needs, licence, or only the files). Effect-`none` capabilities need no grant; GP9 will read the store before a third-party bundle runs on open.

## The 3D model and the bundle (GP2)

A part never exists without its 3D model, so every generation carries `model`, and the host refuses one without it (`generator.invalid_model`). The model is one of the generator's own part files with values:

```json
"model":{"part_file":{"asset":"<sha256 of the part file>","values":{"body_round":40000,"end_round":67500,"height":450000,"length":1600000,"overcoat":30000,"termination":300000,"width":800000}}}
```

- **The part file** is an ordinary parametric `.solidcraft` design that follows the part-file conventions ([part files](../docs/part-files.md)); it ships in the generator's bundle and the module names it by its SHA-256 (the crate's `build.rs` writes the hashes of `parts/` as constants). The module never runs Solidcraft; it decides which file and which values.
- **Values** are integers: nanometres for a length parameter, millidegrees for an angle, the count for a unitless one (the part file's parameter decides). The project stores them as integers too (`PartValue`: `{"nm": …}`, `{"mdeg": …}`, `{"count": …}`, `{"milli": …}`, file version 3); a part file reads each as an exact decimal (`1.6 mm`). The host refuses a value that is not a whole number, one beyond 1 m, or a name that cannot name a part parameter; at most 256 values.
- **The host** (`library.generate_part`) finds the file in the bundle (or, for a module the project embeds, in the project), evaluates it at the values through the part-file path (`sparksmith_solidcraft::models::part_file`), and refuses a blocking finding, bounds more than 10 µm off the generation's envelope, or a lead contact under no body of a lead material. `library.generate` then stores the file as a Model of format `solidcraft` (one per file, shared by every part that uses it) and the package as `PackageModel::PartFile`, so the 3D view, the exact export, markings and the load check treat it like any part file.
- **The bundle** (`library/generators/<name>/`) is the manifest `generator.json` (`format`, `name`, the module and each part file by path and SHA-256, with each part file's title, licence and provenance), the module and `parts/`. Its identity is the SHA-256 of the manifest's bytes (`library.generators` reports it as `bundle`), which names every other file by hash. `sparksmith_io::generator_bundle` reads a bundle and refuses a missing or changed file by name.

The header generator (phase C) ships `pin-header`, parametric in its row and pin counts. Its `style` parameter also makes female sockets (20 November 2026): `pin-socket`, a through-hole strip (presets: Würth WR-PHD 613xx11821, single row, oval lands), and `socket-smd-bottom`, a dual-row SMD socket entered from below through a non-plated hole per position (presets: Würth WR-PHD 6100xx243021, its recommended lands); each a black housing with a lead-in per position and gold two-leaf contacts. The gull-wing generator ships three, `gull-wing-two-sided` (with `gull-wing-two-sided-bevel`, the SOIC outlines' bevelled pin-1 edge, `pin1_bevel`) and `gull-wing-four-sided`, parametric in their lead counts.

The no-lead generator (phase C) makes DFN and QFN parts with or without an exposed pad and ships nine part files, one per outline class and construction, each parametric in its terminal counts: sawn (`dfn`, `dfn-ep`, `qfn`, `qfn-ep`, and `dfn-ep-wettable`, `qfn-ep-wettable` with wettable flanks), a land grid array (`lga`) and an open-cavity sensor with a top opening (`dfn-window`). Since version 6 a package no row pattern makes (L-shaped corner terminals, pins joined by lugs, pins at their own pitch) gives its terminals one by one as rectangles of leadframe (`terminal_numbers`, `terminal_x`, …, `terminal_lift` for a half-etched neck) and its land pattern land by land (`given_land_numbers`, …; `(N)` an unnumbered land of terminal N, round lands and vias); the ninth part file, `sawn-terminals`, places each rectangle in one of forty slots and parks and cuts away the rest. Presets after TI's DML0010A (TPS22990) and RXB0014A (LMQ66430-Q1) drawings.

The molded generator (phase C) ships eight part files: flat, bent and J-lead terminals and MELF caps, each with and without the polarity band. Since version 5 a polarised (tantalum) capacitor prints its pF code and voltage letter by default (`value_code`).
The through-hole generator (`tht`, phase C) ships six part files, one per lead topology (`tht-axial`, `tht-radial`, `tht-radial-formed`, `tht-dip`, `tht-to`, `tht-to-formed`); the module picks the file and gives every value.

The grid generator (phase C) makes BGA, WLCSP and LGA parts, full or with empty positions (`depopulated`, a list of ball names), and ships four part files: balls or lands, each full (`grid-ball`, `grid-land`) or with 16 rectangular cut-outs that remove the empty positions (`grid-ball-voids`, `grid-land-voids`).

The tabbed generator (phase C) ships three part files, one per outline class (`tabbed-rows`, `tabbed-tab-lead`, `tabbed-flange`), parametric in their lead counts, grid steps and lead profile. Since version 2 it takes a datasheet's land pattern as given (`lands: explicit`), as the datasheet skill asks.

The module generator (phase C) makes radio and processor modules: pad rows on up to eight sides' slots (castellated or LGA), a ground array turned by any multiple of 45°, single pads and the antenna keep-out as a footprint keep-out. It ships four part files (LGA or castellated, ground array along the axes or turned 45°) that park what a module lacks beyond its −x edge and cut it away.

Since version 2 the module generator's antenna meander (`antenna_trace`, `antenna_margin`, `antenna_turns`) and the shield can's stamped emboss groove round its label area (`can_emboss`, `can_emboss_inset`) are parameters, and it has an ESP32-S3-WROOM-1 preset (datasheet v1.8). Since version 4 the can can be notched at its +x +y corner (`can_notch_width`, `can_notch_length`) and the board can carry an antenna connector, a U.FL / MHF I receptacle (`connector`, `connector_x`, `connector_y`), both parked and cut away like the rest; the ESP32-S3-WROOM-1U preset uses them. Since version 7 the shield label is laid out as Espressif's laser-marking artwork (the -1U's FCC label exhibit) and gains the MIC, IC and CMIIT lines, the maker's name as installed artwork, the specification identifier and a Data Matrix (`label_mic`, `label_mic_id`, `label_ic_id`, `label_cmiit_id`, `label_company`, `label_status`, `label_matrix`).

The electromechanical families (18 November 2026) are original parametric recreations saved at the manufacturer drawing each cites (sources and dates in each part file's provenance); their lands are the manufacturer's, taken as parameters, and each lead contact lies on its land:

| Generator | Part files | What it makes | Prefix | Presets |
| --- | --- | --- | --- | --- |
| `tactile` | `tactile-mini`, `tactile-mini-ground`, `tactile-square` | Top-push tactile switches: mini (J-bent terminals, white POM push plate, nickel-silver cover, optional ground tab, soldered or unlanded) or square (gull-wing terminals, round plunger, cover with corner studs); `common` says which terminals are joined inside, `separate` keeps each its own pin | SW | Panasonic EVQP2 with and without ground terminal (EVQP2V02M also on the four switch lands, terminals separate), E-Switch TL3305A, Omron B3S-1000 |
| `reed` | `reed-smd` | Surface-mount reed switches: hollow borosilicate envelope with tapered seals, leads flattened into overlapping reed blades inside, formed feet | SW | Littelfuse MDSM-4 |
| `buzzer` | `buzzer-smd`, `buzzer-smd-label` | Magnetic buzzers: LCP case with bevelled orientation corner and sound port, gold end terminals, optional polyimide masking label | BZ | CUI Devices CMI-9653S-SMT |
| `electrolytic` | `electrolytic-smd`, `electrolytic-smd-vent` | Vertical-chip aluminium electrolytics: bevelled PA9T seat plate, terminals in its grooves, aluminium can with seal bead, rounded rim, black negative print and optional vent cross | C | Panasonic FK size codes B–G, 6.3 × 5.4 (CAPAE660X550N), KEMET EDK 6.3 × 7.7 |
| `power-inductor` | `inductor-molded`, `inductor-shielded`, `inductor-shielded-window` | Power inductors: moulded composite block with inlaid bottom terminals, or a ferrite sleeve round a drum core (epoxy ring, enamelled winding, optional winding windows) | L | Coilcraft XEL4030, Bourns SRR6028, an estimated 6 × 6 × 4.5 mm |
| `standoff` | `standoff`, `standoff-spigot` | SMD spacers with an internal thread: tin-plated tube, chamfers, optional locating spigot below the board; an annular land round a non-plated hole | H | Würth WA-SMSI M3 (9774015360R … 9774100360R) |
| `fiducial` | none | Fiducials: a bare copper dot (pad kind fiducial, no terminal, no paste) in a mask opening, its clear area the courtyard and a copper keep-out on its own side; optionally a second dot on the far side; no body (`Model::NoBody`), since the board draws the land | FID | JLCPCB 1.0 mm in 2.0 mm (one side, both sides, local), PCBWay 3R 1.0 mm in 3.0 mm |

The connector generators (18 November 2026) make surface-mount connectors whose lands are the maker's recommended layout, given as parameters, with each body placed on them by the drawing's offsets, so the lead tips stand on the lands (what a body fitted to an imported board's footprint by its pads relies on):

- **`wire-to-board`**: one-row shrouded headers with a metal tab in each end wall, side or top entry, the shroud natural (ivory), white or black (`pa-natural`, `pa-white`, `pa9t`); six part files (`wtb-side-*`, `wtb-top-*`, one per entry and colour), each with the mating cavity, a latch window, two polarising grooves, bent tin contacts and the tabs. Presets: JST SH (1.0 mm) and PH (2.0 mm) SMD headers, side and top entry.
- **`board-to-board`**: dual-row SMD terminal strips (gold posts, tin gull tails, grooved insulator with post pockets), optionally with two alignment pins; part files `b2b-dual-row` and `b2b-dual-row-pegs`. Presets: Samtec FTSH-1xx-01 (1.27 mm) and TSM-1xx-01/-03 (2.54 mm, -03 also with -A pins).
- **`microsd`**: a push-pull microSD socket (nickel-silver shell on tin solder feet, LCP insulator, eight gold spring contacts, card-detect feet); part file `microsd-push-pull`. Preset: Hirose DM3D-style.

The chip generator ships four part files (resistor, capacitor, ferrite or inductor, LED), authored by `cargo test -p sparksmith-solidcraft --test author_parts -- --ignored` (`author_chip`): end caps that stand proud of the body, a resistor's overcoat inset from its edges, an LED's die under its lens.

## ABI 1 changes

ABI 1 is open to change until the first release; each change is listed here, and module hashes and golden hashes are updated deliberately with it (`build --ship`).

| Date | Slice | Change | Effect |
| --- | --- | --- | --- |
| 11 November 2026 | GP0 | First version. | — |
| 14 November 2026 | GP2 | `Description.version` (required, from 1). `Generation.model` required; `Model::PartFile { asset, values }` defined (a bundled part file by SHA-256, integer values). Modules ship as bundles (`library/generators/<name>/`). New refusal codes `generator.invalid_model`, and `generator.invalid_description` for version 0. | A module built before GP2 is refused (no version, no model). The chip module's hash and every golden result hash changed; the lock was rewritten on macOS aarch64, which dropped the Linux x86_64 hash (the CI runner); record it again there. |
| 14 November 2026 | B1, B3, B4, S8 | The one plugin ABI: exports `sparksmith_abi`, `sparksmith_alloc`, `sparksmith_describe`, `sparksmith_invoke` (the `Invoke` envelope); the optional imports `sparksmith.call`, `log`, `cancelled`, answered as a pure invoke; the description format with capabilities, effects and needs (`id` and `title` replace `id_hint` and `name`). `Graphic.key`. `Primitive::Arc { center, start, end, clockwise }` with exact ends. Floats refused; the codec without `serde_json`. Part key by generator id. New refusal codes `generator.float`, `capability.unknown`. | Every older module is refused (`generator.missing_export`); file version 3 detaches parts recorded by one. The chip module shrank to 234 868 bytes; its hash and every golden result hash changed; the lock was rewritten on macOS aarch64, which dropped the Linux x86_64 hash: record it again there (it should now equal the macOS hash, see "Reproducibility"). |
| 15 November 2026 | Module (phase C) | Optional `Footprint.keepouts` (copper keep-outs: a name and a polygon) and `Pad.number` (the shown number when pads share one), both left out when empty. | Other generators' results and golden hashes unchanged; every module's bytes changed (the SDK's code) and every bundle was reshipped. A crate building `Footprint { .. }` or `Pad { .. }` adds `keepouts: Vec::new()` and `number: None`. |
| 18 November 2026 | Package realism | Optional `Generation.marking` (`lines`, `ink`, `pin1_dot`): the package's default top marking as data from the generator's parameters (`marking`, `marking_pin1_dot`; `sparksmith_generator_sdk::marking`), left out when absent. The host makes it the component's marking, the standard fields its lines name made the project's (`expand::marking`). | Every module's bytes changed (the ABI and SDK code) and every bundle was reshipped; the IC and discrete generators (gull-wing, tabbed, no-lead, chip, molded, grid) declare the `marking` parameter, so their golden requests and result hashes changed. |
| 19 November 2026 | Marking rules | `Marking` gains optional `colour` (printed ink of that sRGB), `surface` (a part file's marking surface by anchor name), `wrap` (`[start, span]` millidegrees round a cylinder), `polarity` (`{mark, at}`), `artwork` (SVG text and size, packed by the host under the generator's licence) and `decals` (further markings on other surfaces), all left out when absent (`limits::MARKING_DECALS`, `MARKING_ARTWORK`, `MARKING_SVG_BYTES`). Marking lines may use template alternatives and conditions (`{field:Resistance\|code4?field:Tolerance<=1%;field:Resistance\|code3}`) and `\|num:<prefix>`. | Every module's bytes changed (the ABI and SDK code) and every bundle was reshipped. The chip generator (version 4) marks resistors by the resistor code rule (`resistor_code`, by size) and LEDs' cathodes; the electrolytic generator (version 2) prints values on the can top; the through-hole generator (version 2) prints a radial polarised capacitor's sleeve. Chip golden requests gained `resistor_code` and their result hashes changed. |
| 19 November 2026 | Marking sizes | `Marking` gains optional `text_height` (the lines' capital height, nm; a line too long for the area is set smaller), left out when absent. The SDK's `marking_height` parameter (`marking::height_param`, 0 fits the area) carries it from the presets ([marking text height](../docs/generator-plugins.md)). | Every module's bytes changed and every bundle was reshipped. The marking generators declare `marking_height` (chip 6, gull-wing, molded, grid, tht and electrolytic 4, tabbed and no-lead 5), so their golden requests gained it (0); their presets give the sizes. The electrolytic's `-vent` part file scores the vent as a V-groove. |
| 20 November 2026 | Marking codes | `Marking` gains optional `codes` (orderable part number → the manufacturer's top-mark code, `limits::MARKING_CODES` entries of at most `MARKING_CODE_BYTES`), left out when empty; a line's `{code}` draws the placed part's code and nothing for a part number the table does not list. The SDK's `marking_codes` parameter (`marking::codes_param`, `MPN=CODE` entries) carries the table from the presets. | Every module's bytes changed (the ABI) and every bundle was reshipped. The no-lead generator (version 7) declares `marking_codes`, so its golden requests gained it (empty); its TI presets mark `{code}` from TI's package option addenda, and its ADI LGA-14 (CC-14-1) preset from the ADXL343 data sheet's branding table and ordering guide (343B on ADXL343BCCZ, -RL and -RL7). The module generator (version 5) gains `mask_colour` (the module board's own soldermask, green or black) and four black part files. |
| 20 November 2026 | Artwork templates | `Marking` gains optional `metal` (laser-etched into metal, the host's `laser_etch_metal` finish), left out when false. Marking artwork may be a template: `{…}` placeholders in its text, `data-if` conditions on its elements and `<use data-art>` naming installed library artwork ([artwork templates](../docs/generator-plugins.md)); a generator's artwork names standard fields as its lines do. | Every module's bytes changed (the ABI) and every bundle was reshipped. The module generator (version 6) declares `label_name`, `label_logo`, `label_fcc_id`, `label_ce` and `marking_codes`, so its golden requests gained them; its ESP32 presets carry shield labels. |
| 21 November 2026 | Fiducials | Optional `Keepout.side` (`top`: the part's own outer layer, `bottom`: the other; every copper layer when absent), left out when absent. `PackageGeometry.pin1` may be empty for a package without terminals (a fiducial). `Model::NoBody`: a part complete without a body of its own (a fiducial, whose land the board draws); the host evaluates nothing and the package's model is no body. | Every module's bytes changed (the ABI) and every bundle was reshipped; other generators' results and golden hashes unchanged. A crate building `Keepout { .. }` adds `side: None`. New generator `fiducial` (version 1). |
| 22 November 2026 | Pin tables | Optional `Terminal.drive` (`open_drain` or `open_source`, a driving direction's drive other than push-pull) and `Terminal.alternates` (further directions a terminal can be selected to, each once and none equal to `direction`, the design default; `limits::TERMINAL_ALTERNATES`), left out when empty. The SDK's `pin_table` parameter (`sparksmith_generator_sdk::pins`, every generator, `NUMBER=NAME:KIND` entries, `io` an input that can be an output) names and types the terminals from a datasheet's pin table ([pin table](../docs/generator-plugins.md)). Optional `Generation.orderable` (`{manufacturer, mpn}`, component scope, `limits::ORDERABLE`), left out when empty, from the SDK's `part_numbers` parameter (`MANUFACTURER=MPN` entries); the host publishes each as a manufacturer Part. | Every module's bytes changed and every bundle was reshipped; every generator's version rose by one and its description gained the parameter, so every parity digest changed; golden results unchanged. A crate building `Terminal { .. }` adds `alternates: Vec::new()` and `drive: None`. The module (8: ESP32-S3-WROOM-1, -1U and ESP32-S31-WROOM-3), no-lead (8: TI DML0010A and RXB0014A, Sensirion SHT4x) and tabbed (6: WS2812B) presets carry their datasheets' pin tables; the ESP32, TI and ADI presets their ordering codes. A crate building `Generation { .. }` adds `orderable: Vec::new()`. |
| 22 November 2026 | Unconnected pins | Optional `Terminal.unconnected` (`not_connected` or `reserved`: a pin the device declares without a function, the host's device NC or reserved terminal, never wired; such a terminal gives no `alternates` or `drive`), left out for an electrical terminal. The SDK's `pin_table` reads the kinds `nc` and `reserved` (review §9). | Every module's bytes changed and every bundle was reshipped; a generation of the same parameters is unchanged, so no generator's version rose. A crate building `Terminal { .. }` adds `unconnected: None`. The module (ESP32 pins 33–39 and 91) and no-lead (TI DML0010A pin 2, RXB0014A pins 2, 4, 5, 9 and 12) presets give their NC pins as `nc`, so those presets are new parameters. |
| 23 November 2026 | Net ties | A component may declare an assembly and independent tie groups. Each group has a stable positive id, a form and disjoint passive terminal keys. A footprint may declare copper bridges. Each bridge names its own pad keys, so the component binding identifies its group. | Ordinary parts omit these optional fields. The tie generator starts at version 1. Chip version 8 adds a zero-ohm resistor, and fiducial version 3 declares copper-only assembly. |

## Board ties and jumpers

In the Board features catalog, the tie generator makes copper ties with two to eight ends, Kelvin ties whose ends anchor on existing pads, solder or cuttable trace jumpers with two or three lands, and zero-ohm chip links. Set the land size, copper neck width and bridge length for a copper tie. A length of zero spans the centres of its outer lands. The project supplies the land gap. For a three-land jumper choose lands 1–2 or 2–3. Each jumper has an open or closed default that a variant can change. A closed solder jumper uses the bundle’s parametric solder bead. Zero-ohm links use the chip generator’s resistor size, land pattern, symbol style and native package body. Jumper presets record their published dimensional references and modelling assumptions. [Presets](tie/presets.json) · [Net ties](../docs/design/net-ties.md)

## Build and check

```sh
cargo xtask generators build         # build every module, print sizes and hashes, note any difference from the committed copy
cargo xtask generators build --ship  # … and write library/generators/<name>/ (the only way it changes)
cargo xtask generators check         # in `cargo xtask ci`: build, compare with the committed modules, golden cases three ways
cargo xtask generators reproduce     # two clean builds, compared with each other and the committed modules
cargo xtask generators measure       # the numbers below
```

Modules build in the `generator` profile (`opt-level = "s"`, LTO, one codegen unit, `panic = "abort"`, stripped, overflow checks on), each crate in its own cargo invocation with `--locked`, with the workspace and Cargo home remapped to `/src` and `/cargo` and rustc run through `xtask` as its rustc wrapper (`generators::wrap_rustc`). The caller's `RUSTFLAGS`, `RUSTC_WRAPPER` (sccache included), `CARGO_INCREMENTAL` and `CARGO_PROFILE_GENERATOR_*` are left out. The modules are `no_std` (the SDK supplies dlmalloc and a trapping panic handler), so no `std` code or host path enters them except panic locations, which keep their remapped source paths.

A module has one hash: the SHA-256 of its committed `library/generators/<name>/<name>.wasm`, which the bundle's manifest records. There is no separate lock file. A module's bytes depend on its source to the line and column, since panic locations are kept: any edit of a generator, the SDK or the ABI changes every affected module's hash, and `check` fails until `build --ship` writes the new module and it is committed.

**Golden cases.** Each crate's `golden.json` holds requests with complete parameters and the result hash of each. `check` runs them natively (the crate's own test), under wasmi natively, and under the probe, which is wasmi built for wasm32 running inside the native wasmi; all three must give the recorded hashes, and the probe must also report the same fuel as the native host.

**Module parity.** `golden.json` also keeps `outputs`, the digest of everything the generator says (`sparksmith_generator_abi::parity`): its description, byte for byte as `sparksmith_describe` returns it, and its reply (status byte and body) to the defaults and to every preset in every scope it makes. The crate's native golden test computes it from the native build (`golden::parity::outputs`), and the host's `module_outputs_match_the_native_build` from the module under wasmi; both must give the recorded digest, and a mismatch lists each entry's hash so the two reports can be compared. Any change to a description or a preset changes the digest: paste the one the native test prints. The parity code is native only (`cfg(not(target_arch = "wasm32"))`), so it does not change any module's bytes. It exists because wasmi 2.0.0 miscompiled a `select` fused with a zero test of a local (wasmi issue 2046, fixed in 2.0.1; the host pins 2.0.2): `if key.starts_with("row") { "rows" } else { "single pads" }` came out of the module generator as the group "sing" (the address of one string with the length of the other), while the native build, V8 and every golden part hash were right. `crates/generators/tests/interpreter.rs` keeps the minimal case.

## Reproducibility (14 November 2026)

The same source builds to the same module bytes on any checkout path, on macOS and on Linux. Measured with rustc 1.96.0 in the Nix shell from empty target directories: the `chip` module (234 868 bytes) is `82d5e6c7…` on macOS aarch64 from two checkout paths and on Linux x86_64.

What makes it so:

- **`-C metadata`.** Cargo's value hashes, among other things, `rustc -vV`, which names the host. It seeds each crate's stable id, and with it the order of the module's functions: before B4 the same source gave a different module on each host. `wrap_rustc` drops every `-C metadata` and passes one hash of what identifies the crate within the build (package name and version, crate name and types, the `--cfg` set and the target), none of which names the host or the path.
- **Paths.** The workspace and Cargo home are remapped (`/src`, `/cargo`), so neither the checkout path nor the registry location enters a panic location; the standard library is precompiled with `/rustc/<commit>` paths. The target directory lies inside the workspace, so the build script's `OUT_DIR` is remapped too.
- **Stale artifacts.** Every mismatch measured on 14 November (`b84e4dd4…` in the main checkout, `53781caf…` on Linux) came from a target directory holding crates compiled before the wrapper existed. Cargo's fingerprint ignores `RUSTC_WRAPPER`, so it reused them with cargo's own metadata. Shown directly: one build without the wrapper followed by `generators build` recompiled nothing and gave `9a7bcea3…`. There are two guards now. The build's flags carry a cache key, `-Cmetadata=wrapper-<SHA-256 of the xtask binary>`: cargo fingerprints the flags, so a new wrapper recompiles everything, and the wrapper drops the key before rustc sees it. And `check`, on a mismatch, deletes `target/generators` and builds once more from empty, failing only if the clean build still differs.
- **Windows** is not measured and not compared: the remap leaves backslashes after `/src` in panic locations, so its bytes are expected to differ. There `check` verifies the bundles and runs the golden cases only (`COMPARE_BYTES` in `xtask/src/generators.rs`); no per-host hash is kept. `.gitattributes` keeps the bundles and part files byte-exact on checkout.

Part identity has not depended on these bytes since B4: a part key is the generator's id and parameters, and a module rebuilt to other bytes at the same version makes the same part (G15). The module hash names the embedded asset; the bundle hash is provenance.

## Measurements and limits (11 November 2026)

Measured with `cargo xtask generators measure` on an Apple M-series Mac, macOS, host built in the `ci` profile (wasmi at opt-level 2, portable dispatch), median of 21 runs.

| Module | Bytes | Features used |
| --- | --- | --- |
| chip | 281 021 (274 KiB); 283 970 naming its part files (GP2); 234 868 without `serde_json` or floats (B3, 14 November 2026) | sign-extension, bulk-memory, reference-types (floats refused since B3) |
| header (14 November 2026) | 232 533 | as chip |
| gull-wing (15 November 2026) | 248 095 | as chip |
| no-lead (14 November 2026) | 245 883 | as chip |
| molded (14 November 2026) | 228 961 | as chip |
| tht (14 November 2026) | 261 313 | as chip |
| grid (15 November 2026) | 237 788 | as chip |
| module (15 November 2026) | 266 517 | as chip |
| tabbed (14 November 2026) | 233 846 | as chip |
| probe (host for wasm32, test only) | 1 772 010 | — |

The feature list is measured: each allowed feature is switched off in turn and the module validated without it. Mutable globals, multi-value and saturating float-to-int are allowed but unused; SIMD, relaxed SIMD, threads, memory64, multi-memory, tail calls, extended constants, custom page sizes, wide arithmetic and GC are refused.

| Call | Fuel | Time |
| --- | --- | --- |
| load (check and compile all functions) | — | 4.2 ms |
| describe (24 presets, 14 241 bytes) | 6 033 459 | 8.7 ms cold (load included) |
| generate, package scope (1 178 bytes) | 1 047 731 | 5.2 ms cold, 0.85 ms warm |
| generate, resistor 0603 component (2 117 bytes) | 1 479 896 | 6.0 ms cold, 1.3 ms warm |
| generate, largest case: LED 0603 (2 726 bytes) | 1 825 471 | 6.2 ms cold, 1.4 ms warm |

Requests are 248–258 bytes. Linear memory ends at 1 152 KiB on every call (the 1 MiB stack plus heap). Fuel runs at about 1.1 G per second; it grows by about 460 per response byte.

Limits (`sparksmith_generators::Limits::DEFAULT`, `v1::limits`), set from these numbers:

| Bound | Note's target | Set | Why |
| --- | --- | --- | --- |
| Module size | 2 MiB | 1 MiB | 3.7 × the chip module |
| Fuel per call | 50 M | 500 M | A response at the 1 MiB bound needs about 0.5 G; 83 × describe; about 0.5 s natively |
| Linear memory | 64 MiB | 32 MiB | 28 × the measured 1.1 MiB |
| Request | 64 KiB | 64 KiB | 250 × the measured requests; room for pin-name lists |
| Description | (1 MiB) | 256 KiB | 18 × the chip description |
| Response | 4 MiB | 1 MiB | 380 × the largest chip response; a 1 024-pad grid array projects to about 0.3 MiB |
| Per-kind counts | — | pads ≤ 1 024, terminals ≤ 1 024, graphics ≤ 4 096 per list, points ≤ 1 024, units ≤ 64, coordinates ≤ 1 m | `v1::limits` |
| Wall time | 2 s | none | Fuel ends every call after the same work on every machine. `sparksmith_generators::worker::Worker` runs each call on its own thread; whoever waits cancels it |

The fuel limit and the response bound are tied: raise them together. The projections for larger families are estimates until those generators exist (GP3, GP4).

## Adding a generator

1. `generators/<name>/` with a `Cargo.toml` like `chip`'s (crate type `cdylib` + `rlib`, `unsafe_code = "deny"` because the export macro allows it for `#[unsafe(no_mangle)]`, the SDK as the only dependency) and `#![cfg_attr(target_arch = "wasm32", no_std)]`.
2. Add it to the workspace members.
3. Implement `Generator` and call `export_generator!`.
4. Write `golden.json` (its cases and `outputs`), run `cargo test -p <crate> golden` (a mismatch prints the hash or digest produced), check the geometry by hand-derived tests, then `cargo xtask generators build --ship`.
