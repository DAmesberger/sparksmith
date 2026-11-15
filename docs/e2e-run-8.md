# Eighth end-to-end run: run 7's fixes, generated parts across families

On 15 November 2026 the lead agent tested the fixes for run 7 in Sparksmith's desktop app, working as a designer would:
- importing a small KiCad project, written for this run, and checking components, values, the import report and the refused-track stub;
- placing a QFN on the routed, imported sheet;
- placing one part of each generated family and viewing it in 3D;
- the toast's position while ⌘ is held.

**Setup.** The build was `main` at fd8f89e1, built with the `ci` profile and run as `Sparksmith.app` in the macOS guest VM (Guest VM app), with `library/` copied into `Contents/Resources/library` and the bundle's `Info.plist`. The guest display was 1024 × 640 points at 2×.

**The KiCad project** (`tinyglow`) was hand-written for this run from the KiCad file-format documentation; it copies no existing design. It has:
- a 2-layer board, 40 × 25 mm, with J1 (a 2-pin through-hole header), R1 (470), D1 (Red) and C1 (100n). R1, D1 and C1 have identical 0805 pads under three footprint names (`Tinyglow:R_0805`, `Tinyglow:LED_0805`, `Tinyglow:C_0805`). Values are set on both the footprints and the symbols;
- eight track pieces, two vias and a GND zone on B.Cu. The +5V branch to C1 pin 1 is drawn on the board, but the schematic leaves C1 pin 1 unwired, so the import should refuse that piece;
- a schematic with wires, an LED_A label, a +5V power symbol and three GND power symbols;
- a `.kicad_pro` with a Default class and a Power class. Power is assigned by the patterns `+5*` and `GND|VBUS`; the second is a regular expression. Board minimums are looser than the fab profile: `min_clearance` and `min_track_width` 0.08 mm, `min_through_hole_diameter` 0.15 mm, `min_hole_clearance` 0.1 mm, `min_hole_to_hole` 0.2 mm, `min_text_height` 0.6 mm.

## Outcome

| | Result |
|---|---|
| Time | About 38 minutes of app time, from first launch to the quit. |
| Import | `open -a Sparksmith tinyglow.kicad_pcb` (the same path as Finder's Open With) imported in under a second. This time there was no folder-access prompt, because run 7's grant remained. With unsaved work open, the app first asked "Save changes before continuing?". |
| Components | **Four distinct**: "Tinyglow:PinHeader_1x02", "Tinyglow:R_0805", "Tinyglow:LED_0805" and "Tinyglow:C_0805". The report says "Footprints 4 (0 on the back), 4 definitions". Run 7 found 2 definitions here. |
| Values | **Carried**. The BOM reads "1,1,C1,100n,Tinyglow:C_0805 package,,,no part", "2,1,D1,Red,Tinyglow:LED_0805 package,…", "3,1,J1,Conn_01x02,…" and "4,1,R1,470,Tinyglow:R_0805 package,…". The placement file carries the same values and packages. Values in Resolve parts were not quoted in this run. |
| Import report | "Pads 8 of 8 imported", "Copper 3 nets, 7 track pieces, 2 vias, 1 fill", "Schematic 1 sheet; board nets equal 2, split 1, joined 0". "Rules not taken 30", now including `min_clearance` for each copper pair ("track to track clearance … KiCad 0.08 mm, profile 0.1 mm (min_clearance)") and "min silkscreen text height KiCad 0.6 mm, profile 1 mm (min_text_height)". "Not imported 2": the refused pattern by name, "GND\|VBUS — a regular expression; only * and ? wildcards are read", and "1 — tracks the engine's checks refused". `+5*` is no longer refused. |
| Refused-track stub | "Track on +5V (Top) stops at (10, 7) mm: import refused the rest (schematic keeps a pad off the net)", under "Track ends joining nothing", with Locate and Waive… (finding 3). |
| QFN on the routed imported sheet | **Placed, not refused.** QFN-12 3×3 0.5 EP 1.65 was created from its template and placed twice: once in a clear area, and once with its pin rows over the +5V and LED_A wires. Neither was refused, so the named refusal and the hover-to-keep toast did not appear. |
| Fab export | Production export of the imported board wrote 15 files: Gerbers, `.gbrjob`, drill file and table, BOM, parts BOM, placement, assembly and fabrication PDFs. The drill table lists "T1 0.400 plated via 2" and "T2 1.000 plated component 2". |
| Generated parts | Each family was created from its template, placed and sent to the board with Update PCB. All show in 3D, which opens at once. The ESP32-S31-WROOM-3 module is not offered as a template (finding 1). |
| ⌘ hold | Not reproduced. A synthesized 5–6 s ⌘ hold (HID event tap) with a toast up showed neither the key badges nor the keys panel, and the toast did not move. |

## Run 7's fixes

| Run 7 finding | Now |
|---|---|
| 1. Footprints with equal pads merge under the first one's name | **Fixed.** Four definitions; R, LED and C each keep their own name in Components, the BOM and the placement file. |
| 2. KiCad values dropped | **Fixed** in the BOM and placement files (470, Red, 100n, Conn_01x02). |
| 3. `+5*` refused unnamed; `min_clearance` missing | **Fixed.** `+5*` is taken; a refused pattern is named with its reason. `min_clearance` appears for each copper pair with both values. |
| 4. Stub reason "clearance" | **Fixed.** The stub now reads "…import refused the rest (schematic keeps a pad off the net)". |
| 5. QFN refused on the imported sheet, net unnamed, Details unreachable | **Could not be reproduced.** The QFN placed both times, so the named refusal and the hovered toast were not seen. |
| 6. Part creation in a new project refused with the fab-profile sentence | **Still present** (finding 2). |
| 7. Menu label capitalization | Not rechecked in the in-app menu. The macOS menu path File › "Import KiCad Project…" works. |
| 8. DFN-8 terminals highlighted as circles in 3D | Not rechecked. |

## Generated parts

One of each was placed in a new project, after choosing JLCPCB 2-layer as the fab profile. Each was then sent to the board and viewed in 3D.

| Template | 3D look |
|---|---|
| Chip LED 0805 (2012 metric) | White body with a clear lens, and a white **pin-1 dot** in silkscreen beyond pad 1. |
| SOIC-8 3.9 mm | Black body, gull-wing leads on two sides. |
| SOT-23-5 | Small black body, three leads on one side and two on the other. |
| DIP-8 300 mil | Black body, through-hole pins in plated rings. |
| QFN-16 3×3 0.5 EP 1.7 | Black body, terminals flush at the edges (no leads). |
| BGA-36 0.5 6×6 4×4 | Black body with its silkscreen outline. The ball grid underneath was not looked at. |
| Pin header 2×5 2.54 mm | Gold pins on a black body. |
| SOD-123 | Black molded body, cathode band and a pin-1 dot. |
| Custom module (18 × 25 × 3.2 mm, 38 + 1 pads) | Green module board, castellated pads, a light-grey can and the printed antenna meander. |

None looked wrong, and none took visible time: creating a part from a template and seeing its symbol attached to the pointer took about 0.6 s, the module included.

## What worked well

- **The import keeps what the designer typed.** Distinct footprints and values come through to the BOM and placement files. A designer orders a resistor, an LED and a capacitor.
- **The report names everything that was not taken.** Each looser rule appears with KiCad's value, the profile's value and the KiCad key. The refused pattern appears with the reason.
- **The stub tells the truth.** "schematic keeps a pad off the net" points the designer to the schematic, not to spacing.
- **Generated parts look like their packages** across all nine families, and 3D opens at once.

## Findings

1. **The ESP32-S31-WROOM-3 module cannot be placed from a template.**
   - "wroom" lists only "BUILT-IN 3D PARTS 1: ESP32-S31-WROOM-3 module body · Built-in · a package's 3D body".
   - "module" and the Modules filter list only "Custom module · Modules · Built-in · custom values". It has 18 × 25 × 3.2 mm, "38 + 1 (pitch 1.27 mm)" and "Lands: estimated template defaults".

   The module generator's ESP32-S31-WROOM-3 preset, which `docs/generator-plugins.md` describes, is not offered. The generated module was therefore checked with Custom module.
2. **Creating a part in a new project is still refused with the fab-profile sentence.** In File › New Project's schematic, pressing Enter on "Chip LED 0805" left the sheet empty. Creating BGA-36 then showed, in red under the search field: "this board has no fab profile; choose one (Board › Rules) before routing, filling or making production files". The step was creating a part, and the sentence starts in lower case. The "Fab profile" card ("Use profile") appeared only in the PCB workspace. This is run 7's finding 6.
3. **The stub's full text is hard to read.** In the Issues list the row is cut by the Waive… button. The hover tooltip also ends in "…": "Track on +5V (Top) stops at (10, 7) mm: import refused the rest (schematic…". The full reason could be read only from the row's wrapped second line.
4. **Copy buttons do not reach the macOS clipboard.** "Copy report" in the KiCad import report and in Issues left the guest pasteboard unchanged (`pbpaste` still returned a test string set beforehand). A BOM copy in Resolve parts did the same. The workspace builds eframe without its `clipboard` feature (`Cargo.toml`: `features = ["default_fonts", "wgpu", "x11", "wayland", "accesskit"]`), and egui-winit was compiled without it. That makes `ctx.copy_text` a no-op on the desktop, which is a plausible cause. A person who presses Copy report gets nothing to paste.
5. **Placing over wires gives no feedback.** The QFN placed with its pin rows across the routed +5V and LED_A wires was accepted silently; only the error count rose (14 → 27). This may be right if no pin end touches a wire. It also means the run-7 refusal could not be triggered by an ordinary placement.

## Speed

| Operation | Time |
|---|---|
| KiCad import by Open With | under 1 s |
| Template → part on the pointer (each family, module included) | about 0.6 s |
| Update PCB and open in 3D | instant (drawn in the first capture, 0.3 s after the click) |
| Production export of 15 files | instant to the eye |

## Clean-up

The app was quit in the guest without saving the test projects; its Recovery folder was left empty. The following were deleted:
- from the guest: the KiCad copies, the saved `tinyglow.sparksmith` and the production files under `~/Documents/e2e8`, and a key-hold helper in `/tmp/e2e8`;
- from the shared `results/` folder: the KiCad source files, the helper source and the temporary screenshot.

The guest pasteboard holds the test string "probe". The guest display was left as found (1024 × 640 at 2×).
