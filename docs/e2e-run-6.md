# Sixth end-to-end run: KiCad import and a 555 blinker

On 14 November 2026 the lead agent tested two flows in Sparksmith's desktop app, working as a designer would:
- importing a small KiCad project, written for this run, and checking the result in the app;
- designing a new board from an empty project: a 555 LED blinker (NE555-style timer in SOIC-8, three 0603 resistors, a 10 µF 0805 and two 0603 capacitors, a red 0603 LED and a 1×2 header).

It also checked what part search shows for generated parts and whether run 5's fixes still hold.

**Setup.** The build was `main` at b5f66cde, built with the `ci` profile and run as `Sparksmith.app` in the macOS guest VM (Guest VM app) with `library/` copied into `Contents/Resources/library`. The guest display was 1024 × 640 points at 2×. The KiCad import and file checks used the `sparksmith` CLI on the host.

**The KiCad project** (`blinkpad`) was hand-written from the KiCad file-format documentation for this run; it copies no existing design. It has:
- a 2-layer board, 40 × 30 mm, with J1 (a 2-pin through-hole header plus an SMD tab, pad 3, that its symbol does not have), R1, D1 and C1 in 0805, seven track pieces, two vias and a GND zone on B.Cu;
- a schematic with wires, two net labels (VIN, LED_A) and three GND power symbols. C1 pin 1 is left unwired on purpose, while the board puts it on VIN;
- a `.kicad_pro` with a Default class (0.15 mm clearance, 0.25 mm track), a Power class (0.5 mm track, 0.2 mm clearance) assigned by the pattern `V*` and to GND explicitly, and board minimums (0.1 mm track and clearance, 0.2 mm drill).

## Outcome

| | Result |
|---|---|
| Time | About 25 minutes of app time for both flows, from first launch to the last reopen. |
| KiCad import | **No path in the desktop app.** The File menu has no Import entry, the command palette finds nothing for "kicad" or "import", and opening a `.kicad_pcb` with Sparksmith (`open -a`) does nothing. The import ran through `sparksmith import-kicad` (0.11 s). The app opened the result. |
| Import report | 4 footprints, 9 of 9 pads, 3 nets, 7 track pieces, 2 vias, 1 fill, 1 sheet. Shown: board nets equal 2, split 1; "9 KiCad rules looser than the fab profile were not taken"; 1 unmapped pad (a pin added); 1 unconnected pin; 2 pins or labels on a wire's middle; **1 track refused** (clearance). |
| Imported schematic | All four symbols at KiCad's places, with wires, labels and power ports. "2 errors, 2 warnings, 4 to do". The rows are "J1 pin 3 unconnected", "C1 pin 1 unconnected" and "Nets the imported board joined, 1", with the hover text "VIN and N8 joined on the imported board". |
| Imported board | Outline, parts, tracks and vias as drawn in KiCad. The VIN track to C1 pin 1 is missing (the refused track), which leaves a stub: "Track on VIN (Top) ends at (10, 10) mm joining nothing". Net classes: "1 class" (Power, with GND assigned and the `V*` pattern). |
| Imported 3D | Opens at once. The board, the bottom GND pour, the vias, and generic boxes for the KiCad footprints. |
| 555 schematic | 9 parts from search ("ne555 soic-8" → SOIC-8 template, "10k 0603", "47k 0603", "10uF 0805", "10nF 0603", "470 0603", "led 0603 red", "100nF 0603", "header 1x2"), wires, GND and VCC power ports and THR net labels. 7 nets. |
| 555 board | Update PCB placed 9 parts beside the board. Parts were dragged onto it and a top GND pour drawn. The shove router routed every net except THR and N7 inside the SOIC-8. VCC crosses under U1 on the bottom layer through two vias, using the layer key while routing. |
| 3D | Opens at once. SOIC-8 with gull-wing leads, chip parts, gold header pins, green mask over the pour. The hint reads "Right-drag to orbit · middle-drag to pan · wheel to zoom". |
| Production check | First 4 errors and 9 warnings, at the end 2 errors ("THR: 4 pads unrouted", "N7: 3 pads unrouted") and "No Part chosen 9". **Export stays disabled while errors remain, correctly.** Fab export itself was not reached in this run. |
| Save and reopen | Both projects reopen identically: imported board byte-identical through the app's save; 555 board with 9 placements, 29 tracks, 2 vias, 1 fill, 20 wires, 15 labels, 7 nets and the same 2 to-dos. **But every save into `~/Documents` warned and left the title marked unsaved** (finding 4). |
| Generated parts | Search lists the generic families under "GENERIC PARTS" with a "not yet created" badge. The shipped part files (ESP32 module, heatsink, spacer, nut) are not found (finding 9). |

## What worked well

- **The import is fast and honest.** 0.11 s for the whole project. The report counts every case and says what it did with each, such as "schematic kept; each a finding" and "a pin added by import".
- **Schematic wins, as designed.** C1 pin 1, left unwired in the schematic, stays off VIN. The app names the disagreement ("VIN and N8 joined on the imported board") and offers "Mark no-connect" on each unconnected pin.
- **The added pin is visible.** J1 pin 3 (the tab with no symbol pin) appears on the symbol and as "J1 pin 3 unconnected", not silently dropped.
- **Net classes arrive.** Power keeps its 0.5 mm / 0.2 mm values, its `V*` pattern and its explicit GND assignment. KiCad's Default class becomes the project's own rule set over the fab profile.
- **The shove router** took the R2–C1 and N3 tracks around existing copper. Where it could not, it stopped with a clear message: "Route stopped short: Blocked by D1 pad 1 (GND). The track ends there and the route goes on from its end; Undo takes the step back."
- **Changing layer while routing** (the `2` key, then `1`) places vias and continues, which made the VCC crossing under the SOIC-8 easy.
- **Update PCB** shows a review ("Adds 9 placements: U1, R1 and 7 more … 9 parts staged beside the board") before applying, and reports "24 new issues after this update" with a Show button.
- **Net pickers** name unnamed nets by their pins ("N7 · R1 1, R2 1, U1 7"), as in run 5.
- **3D opens instantly** on both boards.

## Findings

1. **KiCad import has no path in the desktop app.** The CLI imports well, but a designer in the app finds nothing. The File menu has New project, Open…, Save, Save as… and Quit. The command palette shows nothing for "kicad" or "import". Opening a `.kicad_pcb` with Sparksmith (`open -a`, as Finder's Open With does) does nothing visible: no toast and no error.
2. **The import report is not shown in the app.** After opening the imported project there is no trace of the report's counts: looser rules, the refused track, the added pin. Only the resulting findings remain (unconnected pins, joined nets, the dangling track).
3. **The "looser rules" are counted but not named.** The summary says "9 KiCad rules looser than the fab profile were not taken", and the detail file (`NEW_FILE.kicad-import.json`) lists only the refused track. Neither says which nine rules, or their KiCad and fab values. The project's 0.1 mm track and clearance equal the JLCPCB profile's 0.10 mm, so it is not obvious which rules were looser.
4. **Saving into `~/Documents` warns and leaves the project marked unsaved.** Every ⌘S showed: "…: The file was written, but synchronizing /Users/agent/Documents/e2e6 failed; power-loss durability is unconfirmed: Operation not permitted (os error 1)". The left end of the toast was hidden behind the keyboard-hint overlay.
   - The file was complete and reopened correctly, but the title kept its unsaved dot ("blinker555 •").
   - On quit, the session therefore stayed in Recovery, and the next launch asked "Recover one of 2 earlier sessions?" for a project that was saved.
   - The directory sync (`sync_directory` in `crates/io/src/persistence.rs`) fails in the macOS-protected Documents folder.
5. **A refused import track leaves a dangling stub and no named reason in the app.** The VIN segment to C1 pin 1 was refused ("tracks the engine's checks refused", path `clearance`) because the schematic keeps C1 pin 1 off VIN. The rest of the VIN branch remains as "Track on VIN (Top) ends at (10, 10) mm joining nothing". The board needs the user to delete the stub, and nothing in the app says it came from a refused import track.
6. **Imported footprints lose their KiCad names.** The Components list shows "KiCad footprint 1 compo…", "KiCad footprint 2 compo…" and so on, not `Blinkpad:R_0805`. The instance rows carry a red ✕ with no hover text explaining it. Both projects also keep the project name "Untitled" while the file is `blinkpad-imported` or `blinker555`.
7. **Fill outline clicks were lost when two clicks came close together.** In the Fill tool, a second corner clicked about 0.8 s after the first was ignored. Enter then closed a triangle from the remaining three points. Slower clicks drew the rectangle. The Route tool did not lose clicks at the same pace. This may be the driver's event timing and needs a check by hand.
8. **A new project's fab-profile card appears behind the recovery prompt when the app is opened with a file.** Launching with `blinker555.sparksmith` showed the "Fab profile … Use profile" card for an empty Untitled project under "Recover an earlier session?". It went away once the file opened.
9. **Shipped library parts are not found by part search.** "esp32" and "heatsink" give "No component or template matches", although `library/parts/` ships the ESP32-S31-WROOM-3, finned heatsink, round spacer and SMD nut. The generic families do appear: "resistor 0402 4k7" lists "Chip resistor 0402 … not yet created · imperial · 1 × 0.5 mm · 4.7 kΩ" and "2 pins · cited (unverified) values", with "Not a component yet: choosing it creates one, then you place it". As briefed, generated parts have no UI path to create one yet; nothing was built.

## Run 5's fixes

| Run 5 finding | Now |
|---|---|
| 1. Unknown search word fails the query | Holds: "ne555 soic-8" found the SOIC-8 template. |
| 2. Internal file name in provenance | Not rechecked. |
| 3. Parameter fields need a double-click | Not rechecked: the Board card's Size row is a summary, and no parametric field was edited. |
| 4. Saved files owner-only | Holds: saved projects are `-rw-r--r--`. |
| 6. Interface datum shown in 2D by default | Holds: neither board showed interface lines in 2D. |

## Speed

| Operation | Time |
|---|---|
| `import-kicad` of the 3-file project (CLI) | 0.11 s |
| Any edit, route step, fill, Update PCB | instant to the eye |
| Open in 3D | instant |
| Reopen in the desktop app | about 4 s from launch to drawn board |

## Clean-up

The app was quit in the guest. The test projects, the KiCad files and the recovery snapshots were deleted from the guest, and the test files and the temporary screenshot from the shared `results/` folder. The guest display was left as found (1024 × 640 at 2×).
