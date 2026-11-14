# Seventh end-to-end run: KiCad import in the app, run 6's fixes, fab export

On 15 November 2026 the lead agent tested the fixes for run 6 in Sparksmith's desktop app, working as a designer would:
- importing a small KiCad project, written for this run, through each path the app now offers;
- run 6's other findings (saving into `~/Documents`, recovery, part search, the fab-profile prompt);
- fab export end to end on the imported board, once it was fully routed;
- placing header, QFN and DFN parts, which now come from generators, and viewing them in 3D.

**Setup.** The build was `main` at 4690eae5, built with the `ci` profile and run as `Sparksmith.app` in the macOS guest VM (Guest VM app), with `library/` copied into `Contents/Resources/library` and the bundle's `Info.plist`, which declares KiCad files with the Viewer role. The guest display was 1024 × 640 points at 2×.

**The KiCad project** (`glowtab`) was hand-written from the KiCad file-format documentation for this run; it copies no existing design. It has:
- a 2-layer board, 36 × 24 mm, with J1 (a 2-pin through-hole header plus an SMD tab, pad 3, that its symbol does not have), R1 (330), D1 (green LED) and C1 (1 µF), all three 0805. The R, LED and C footprints have different names (`Glowtab:R_0805`, `LED_0805`, `C_0805`) but the same pads;
- seven track pieces, two vias and a GND zone on B.Cu. The +5V branch to C1 pin 1 is drawn on the board, but the schematic leaves C1 pin 1 unwired, so the import refuses that track;
- a schematic with wires, +5V and LED_A labels and three GND power symbols;
- a `.kicad_pro` with a Default class (0.2 mm clearance and track), a Power class (0.4 mm track, 0.25 mm clearance) assigned by the pattern `+5*` and to GND explicitly, and board minimums looser than the fab profile (0.08 mm clearance and track, 0.15 mm drill, 0.1 mm hole clearance, 0.2 mm hole to hole, 0.6 mm text).

## Outcome

| | Result |
|---|---|
| Time | About 35 minutes of app time, from first launch to the last quit. |
| Import paths | **All three work.** File › "Import KiCad project…" (the macOS menu reads "Import KiCad Project…"), the command palette ("kicad" lists the import and "KiCad Import Report"), and `open -a Sparksmith glowtab.kicad_pcb` while the app runs (as Finder's Open With does). With unsaved work open, Open With first asks "Save changes before continuing?". |
| Importing indicator | "Importing KiCad project…" shows in the title bar while the import runs. The first import from `~/Documents` (and later one from `~/Desktop`) waited about a minute on macOS's "“Sparksmith” would like to access files in your Documents folder." prompt. The indicator stayed up meanwhile. Once access was granted, an import took under a second. |
| Import report | Opens by itself after the import and fits the 1024 × 640 screen, with scrolling. "Footprints 4 (0 on the back), 2 definitions", "Pads 9 of 9 imported", "Copper 3 nets, 6 track pieces, 2 vias, 1 fill", "Schematic 1 sheet; board nets equal 2, split 1, joined 0". "Rules not taken 20", each named with both values, for example "track to plated hole clearance: KiCad 0.1 mm, profile 0.35 mm (min_hole_clearance)" and "min track width: KiCad 0.08 mm, profile 0.1 mm (min_track_width)". "Not imported": 1 net class pattern, 1 refused track. It ends with "Copper a refused track left is named in Issues: delete it or route on." |
| Reopening the report | Window › KiCad Import Report and the palette both reopen it, also after the project was saved. |
| Refused-track stub | Named: "Track on +5V (Top) stops at (8, 6) mm: import refused the rest (clearance)", with Locate and Waive (finding 4). |
| Components | KiCad footprint names: "Glowtab:Header_1x02_Tab" and "Glowtab:R_0805". **D1 and C1 are also listed as Glowtab:R_0805** (finding 1). |
| Save | Save proposed `glowtab.sparksmith` in the KiCad project's folder. The project name changed from "Untitled" to "glowtab". |
| Fab export | **Reached and works.** With every net routed, the production check gave 0 errors and "9 warnings". Export was enabled once a folder was chosen, and wrote 15 files ("Wrote 15 production files"). |
| Generated parts | QFN-16 3×3, DFN-8 2×2 and Pin header 2×5 were created from templates, placed and sent to the board with Update PCB. All three show in 3D, which opens at once. A QFN could not be placed on the imported sheet (finding 5). |

## Run 6's fixes

| Run 6 finding | Now |
|---|---|
| 1. KiCad import has no path in the app | **Fixed**: File menu, palette and Open With all import. |
| 2. Import report not shown in the app | **Fixed**: the report opens after the import and reopens from Window and the palette. |
| 3. Looser rules counted but not named | **Fixed**: 20 rules named, each with KiCad's value, the profile's value and the KiCad key. KiCad's `min_clearance` is still missing from the list (finding 3). |
| 4. Saving into `~/Documents` warns and stays unsaved | **Fixed.** ⌘S showed "Saved /Users/agent/Documents/e2e7/glowtab/glowtab.sparksmith", and the title's dot cleared. After quitting, the Recovery folder was empty and the next launch offered no recovery. A forced kill with unsaved edits did offer recovery, as it should. The toast's placement while ⌘ is held could not be checked: a synthesized ⌘ hold did not bring up the keys panel in the guest. |
| 5. Refused import track leaves an unnamed stub | **Fixed**: the stub is named as coming from the import. The reason given is misleading (finding 4). |
| 6. Footprint names lost; red ✕ without hover; project stays Untitled | **Fixed**: KiCad names, and the project takes the file's name on save. Hovering J1's red ✕ in the Browser shows "Glowtab:Header_1x02_Tab / J1 pin 3 unconnected · and 2 more". In Resolve parts the imported groups showed "○ None" in the Part column, with no red mark to hover. |
| 7. Fill outline clicks lost | Not rechecked. |
| 8. Fab-profile card behind the recovery prompt | **Fixed**: launching with a project, with and without a recovery prompt, showed no fab-profile card. |
| 9. Shipped library parts not found | **Fixed.** "esp32" lists "BUILT-IN 3D PARTS 1: ESP32-S31-WROOM-3 module body · Built-in · a package's 3D body". Choosing it says "ESP32-S31-WROOM-3 module body is a package's 3D body: choose it in a component's 3D Body section." "heatsink" opens Place 3D part on the PCB with finned-heatsink selected: "finned-heatsink.solidcraft · 4 KiB · MIT OR Apache-2.0 · The check found nothing to fix · Bodies 1 · aluminium ×1 · 20 × 20 × 10 mm · Evaluated in 21 ms". |

## Fab export

The imported board was finished as a designer would:
1. Deleted the refused-track stub.
2. Drew a wire from C1 pin 1 to +5V in the schematic.
3. Ran Update PCB.
4. Routed C1 pin 1 to R1 pin 1. The route tool took 0.4 mm, the Power class width, for +5V.
5. Moved J1 off the board edge ("J1 pad 3 is 0 mm from the board edge" was a real error from the KiCad placement).
6. Marked J1 pin 3 no-connect.

PCB issues then read "No issues". Production check with "JLCPCB 2-layer · this board": "9 warnings", in three groups: "Envelope model 4", "J1 pin 3: pin added by import" and "No Part chosen 4". There were no errors.

The export wrote `glowtab-production/` with the following files:
- copper L1/L2, mask top/bottom, paste top, profile and drill map as Gerber X2 (`TF.FileFunction`, `TF.ProjectId`, `TA.AperFunction` attributes) and a `.gbrjob`;
- `glowtab-plated.drl` (finished sizes T1 0.300 via, T2 1.000 component; 4 hits) and a drill table;
- `glowtab-pos-top.csv`, `glowtab-bom.csv`, `glowtab-bom-parts.csv`;
- assembly and fabrication PDFs.

The bottom copper carries the GND pour. The files are complete. Their content shows findings 1 and 2: the BOM line reads "3,C1 D1 R1,,Glowtab:R_0805 package,,,no part", with no values.

## What worked well

- **The import is now a designer's path.** One menu item, then a report that names every case, then the board. The report's rows say what happened ("the profile's value stays", "a pin added by import", "schematic kept; each a finding").
- **The schematic's view wins and the app says so.** "Nets the imported board joined, 1", "C1 pin 1 unconnected", "J1 pin 3: pin added by import", each with Locate and "Mark no-connect" where it applies.
- **Net classes carry into routing.** +5V routed at 0.4 mm without touching the width.
- **Saving next to the KiCad files** with the KiCad project's name is what a designer expects.
- **Production output is readable.** Gerber attributes, a drill table that explains finished sizes, and a single "Wrote 15 production files" toast.
- **3D opens instantly**, and the generated QFN, DFN and header bodies look right: header pins gold on a black body, QFN and DFN with terminals at the edges.

## Findings

1. **Footprints with equal pads merge under the first one's name.** D1 (`Glowtab:LED_0805`) and C1 (`Glowtab:C_0805`) have the same pads as R1 (`Glowtab:R_0805`). The import made one definition of the three ("4 (0 on the back), 2 definitions"). The result:
   - Components lists only "Glowtab:Header_1x02_Tab" and "Glowtab:R_0805";
   - Resolve parts groups "C1, D1, R1 · Glowtab:R_0805 …" as one part;
   - the BOM reads "3,C1 D1 R1,,Glowtab:R_0805 package,,,no part";
   - the placement file lists the LED and the capacitor as "Glowtab:R_0805 package".

   A designer would order three of one part for a resistor, an LED and a capacitor.
2. **KiCad values are dropped.** R1 330, D1 green and C1 1u are set in both the schematic and the board. Resolve parts shows "VALUES None", and the Value columns of the BOM and placement files are empty.
3. **Two KiCad project rules are not imported, and the report does not say which.**
   - The class pattern `+5*` is refused. The report says "net class patterns that are not plain wildcards or name no imported class 1" but does not name the pattern. The importer rejects any pattern containing `+` (`crates/kicad/src/import.rs`), which is common in power-net names such as +5V and +3V3. +5V still reached the Power class here, apparently through an explicit assignment.
   - KiCad's board `min_clearance` (0.08 mm, below the profile's 0.1 mm) is not among the 20 rules not taken: `project.rs` does not read that key.
4. **The refused track's reason says "clearance".** The issue row reads "Track on +5V (Top) stops at (8, 6) mm: import refused the rest (clearance)". The track was refused because the schematic keeps C1 pin 1 off +5V, not because of a clearance. A designer would look for a spacing problem that is not there.
5. **A QFN could not be placed on the imported sheet.** On the imported project, after routing, QFN-12 3×3 0.5 EP 1.65 was created from its template. Placing it failed twice with "Can't place a component: This connectivity change affects routed copper". The toast names no net. Its Details button vanished with the toast before it could be clicked; the hover reads "Click for Messages". A Pin header 1×3 placed on the same sheet a moment earlier without trouble, and QFN-16 placed fine in a new project. A plausible cause is the exposed pad joining GND, which has routed copper and a pour. This was not confirmed.
6. **Creating a part in a new project before choosing a fab profile is refused with an unrelated reason.** In File › New Project's schematic, choosing the QFN-16 template showed "this board has no fab profile; choose one (Board › Rules) before routing, filling or making production files". The step was placing a part, not routing, and the sentence starts in lower case. The fab-profile card ("Use profile") appeared only after switching to the PCB workspace.
7. **The import's menu label differs in capitalization.** The macOS menu reads "Import KiCad Project…"; the in-app File menu and the palette read "Import KiCad project…" (`apps/sparksmith/src/menu_bar.rs`).
8. **In 3D, the DFN-8's selection draws circles over its rectangular terminals** (a row of blue rings along each side). The other parts were not selected in 3D, so this may be how terminals of any part are highlighted. It needs a look by hand.

## Speed

| Operation | Time |
|---|---|
| KiCad import in the app, once folder access was granted | under 1 s |
| First import from a protected folder | about 60 s, waiting on macOS's access prompt |
| Open in 3D | instant (drawn in the first capture, 0.65 s after the click) |
| Production check and export of 15 files | instant to the eye |
| Relaunch with a project file | about 3 s to a drawn board |

## Clean-up

The app was quit in the guest; its Recovery folder was left empty. The following were deleted:
- from the guest: the KiCad copies, saved projects and production files under `~/Documents/e2e7` and `~/Desktop/e2e7-glowtab`, and temporary helpers in `/tmp`;
- from the shared `results/` folder: the KiCad source files, a helper source and the temporary screenshot.

The guest's Documents and Desktop access grants for Sparksmith remain, as do the macOS Recent Items entries. The guest display was left as found (1024 × 640 at 2×).
