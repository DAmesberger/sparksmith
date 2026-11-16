# Eleventh end-to-end run: recorded demo takes at Full HD, run 10's fixes in the app

On 16 November 2026 the lead agent recorded Sparksmith's desktop app in the macOS guest VM as demo material for the website and README, and tested it as a designer would, covering:
- a USB-C powered 3.3 V sensor board from scratch: schematic with named parts, Update PCB, outline, placement, shove routing with layer changes, a GND pour, 3D, production check, fab export, save, quit and reopen;
- a KiCad project written for this run, imported and exported;
- the run 10 fixes.

The AI scene ("Create from datasheet…") was not part of this pass; it waits for an API key that only the founder enters.

**Setup.** The build was `main` at 4a8248e8, built with the `ci` profile and run as `Sparksmith.app` in the guest VM (Guest VM app). The bundle had `library/` in `Contents/Resources/library`, the bundle's `Info.plist`, and nix `libiconv`/`libcharset` in `Contents/Frameworks`. The guest display was 1920 × 1080 at scale 1.0, and the app ran in full screen for every take.

**Recording.** The driver's `start_recording` with `record_video` failed at once: "SCRecordingOutput failed to write the recording: Failed due to failure to process first sample buffer". This is probably because the guest has no hardware video encoder. `screencapture -v` over SSH failed with "could not create image from display", because SSH has no screen-recording grant. The takes were therefore recorded with two small helper apps written for this run:
- a ScreenCaptureKit recorder that writes 30 fps JPEG frames with timestamps;
- an input "puppet" that posts real HID mouse and keyboard events at a human pace (eased pointer glides, typing at 8–14 characters a second) and logs each action's time window.

Both got Screen Recording and Accessibility grants in the guest's System Settings. The videos were cut from the frames on the host with ffmpeg (H.264, 1920 × 1080, 30 fps). Waits for the agent between actions were removed, failed or retried actions were dropped, and each kept action keeps its real speed. Every on-screen time in the clips is the app's real response time.

**The board** (`usb-sense`):
- J1, the built-in GCT USB4105 USB-C receptacle;
- U1 "TLV75733PDBVR", a 3.3 V LDO in SOT-23-5 with pins IN, GND, EN, NC, OUT;
- U2 "LM75BD", a temperature sensor in SOIC-8 with pins SDA, SCL, OS, GND, A2, A1, A0, VCC;
- C1 and C2 1 µF and C3 100 nF, in 0603;
- R1 and R2, 5.1 k CC pull-downs, and R3, a 1 k LED resistor, in 0603;
- D1, a green 0603 LED;
- J2, a 1 × 4 header (3V3, SDA, SCL, GND).

Nets: VBUS, 3V3, GND, CC1, CC2, SDA, SCL and the LED net. The board is 40 × 30 mm on JLCPCB 2-layer.

**The KiCad project** (`beaconlet`) was hand-written for this run from the KiCad file-format documentation; it copies no existing design. It is a transistor-switched LED beacon:
- J1, a 1 × 3 through-hole header (VIN, GND, CTRL);
- C1 "1u", R1 "10k" and R2 "330", on `Beacon:Chip_0603`;
- D1 "red" (`Beacon:LED_0805`);
- Q1 "NPN" (`Beacon:SOT-23`).

The board is 30 × 20 mm with a B.Cu GND zone, F.Cu tracks, two GND vias and a silkscreen text. The schematic has wires, labels and VIN and GND power symbols. The `.kicad_pro` has Default and Power classes, the pattern `VIN` → Power and GND assigned to Power explicitly. The generator script is kept with the media.

## Outcome

| | Result |
|---|---|
| Time | About 55 minutes of app time for scenes 1–6, including retakes. |
| Named parts | **Possible, but only through the Component workspace** (finding 1). U1 and U2 carry real part numbers and pin names in the schematic, the board and the BOM. |
| Unrouted count after the GND pour | **Fixed.** After the Top GND fill, the status line read "0 airwires" and Issues listed no unrouted connection. The production check had no connectivity error. |
| Router and USB-C holes | **Fixed as far as seen.** CC1, CC2 and VBUS were routed between and beside J1's holes in several takes, and no hole-clearance error appeared. |
| Production check | **0 errors only by waiving J1's 6 built-in geometry errors** (finding 2). Everything else on the board was clean. |
| Fab export | "Wrote 17 production files", all named `usb-sense-*`. |
| Save, quit, reopen | **Identical.** The Issues report copied before quitting and after reopening was byte for byte the same (1,526 bytes). The Recovery folder was empty after the quit. |
| KiCad import | Report in under 2.5 s. Export named `beaconlet-*` before any save (**fixed**). 3 KiCad tracks were refused and GND stays unrouted (finding 4). |
| Run 10 fixes | Slow-save hint, unmapped terminals as to-dos, the wrapped 30° refusal and export naming are **fixed**. The precise clearance message was not reproduced. |
| Crashes | None. |

## A. The board from scratch (scenes 1 and 2)

1. **New project and fab profile.** File › New project. The Fab profile card listed "JLCPCB 2-layer" and "JLCPCB 4-layer"; Use profile set it.
   - Before the clean redeploy, the guest's `~/Applications/Sparksmith.app` kept two stale `*-1oz.json` profiles from an older bundle, because the deploy copies over without deleting. The card then showed four JLCPCB entries.
2. **Parts.**
   - The USB-C receptacle came from the Place palette (P, "usb-c", Enter).
   - The passives, the LED and the header came from the palette with one keystroke each: "1u 0603", "100n 0603", "5.1k 0603", "1k 0603", "led 0603 green" and "header 1x4". Each was on the pointer at once.
   - The two ICs were made in the Component workspace › New gallery. "sot-23-5" showed the SOT-23-5 generator form: "Tabbed small-outline and power packages · version 2", Name, Preset "SOT-23-5" and the lead, body and tab fields. "TLV75733PDBVR" was typed in Name, then Create component. The pins were then named one by one in the Pins tab. The same steps made "LM75BD" from the SOIC-8 form.
   - Publish, then Place in schematic.
3. **Schematic.**
   - Wires (W) and power ports (⇧P): GND by default, plus VBUS and 3V3 from the Common list.
   - Net labels SDA, SCL, VBUS, CC1 and CC2, named in the side panel before placing.
   - No-connects on the unused J1 pins, U1 NC and U2 OS. U1 EN was tied to IN.
   - Values were per instance (1u, 1u, 100n, 5.1k, 5.1k, 1k), after adding the standard Capacitance and Resistance fields to the two passive components and updating the placed instances.
4. **Update PCB** (⇧⌘U) listed the changes and applied them. The parts landed beside the default board.
5. **Outline.** Shape opened the board sketch with its width and height parameters. Width 40 and height 30, then Done, redrew the board at once (clip `board-shape`).
6. **Placement.** All 11 parts were dragged onto the board; R3 and D1 were turned 180°.
7. **Routing.** Route (R) with Shove and 45°, routing CC1, CC2, VBUS, 3V3, the LED net, SCL and SDA.
   - SDA changed layer with V and continued on Bottom to J2 (clip `layer-change-via`).
   - A GND track routed straight up beside U2 pushed the already-routed SCL track aside as the pointer moved (clip `shove-routing`). That take was not kept on the board, because its far end crossed J2.
   - The USB D+/D− pairs (A6–B6, A7–B7) were tied under J1 with vias on Bottom.
8. **GND pour.** Fill (P), net GND, four clicks at the board corners, Enter. The fill drew at once. Issues then showed 8 Info rows under "THERMAL SPOKES REMOVED 8", for example "GND fill on Top: 2 thermal spokes of J1 pad A1B12 removed; they would end without reaching GND copper". The status line read "0 airwires".

## B. 3D (scene 3)

- **3D opened at once**, drawn in the first frame after the tab click, with the pour and 11 parts.
- **Orbit** with a right-drag was smooth at 1920 × 1080.
- **Close-ups** (wheel zoom to the pointer):
  - the LM75BD SOIC-8 with gull-wing leads seated on gold lands, the pin-1 dimple and the silkscreen designator;
  - the USB-C shell in brushed metal with its tabs in plated slots.
- **Layers** (view menu): hiding Top solder mask and Components showed the bare copper, the pour and the tracks; the bottom view (6) showed the SDA track and the header pads on Bottom.
- **Materials** match run 10. No visual regression was seen.

## C. Production check and fab export (scene 4)

1. **Check.** The Production panel read "6 errors · 12 warnings". Export was disabled, with the hover text "Fix or waive the 6 errors first".
   - Errors (all J1, the built-in USB-C part):
     - "Pad holes too close to copper 2": "J1 pad 17: hole is 0.175 mm from J1 pad A1B12 on GND" and "J1 pad 18: hole is 0.175 mm from J1 pad B1A12 on GND";
     - "Thin plated-hole annular rings 4": 4 × "J1 pad S: annular ring 0.2 mm".
   - Warnings: "No pin-1 marking 2" and "No Part chosen 10".
2. **Waive.** The two error groups were selected and waived as one action ("Waive 6 findings") with the reason "built-in USB-C part: vendor land pattern". The result read "12 warnings · 6 waived".
3. **Export** to `~/Documents/e2e11` wrote "Wrote 17 production files" into `usb-sense-production/`:

   `usb-sense-assembly.pdf`, `-bom-parts.csv`, `-bom.csv`, `-copper-l1-top.gbr`, `-copper-l2-bottom.gbr`, `-drill-map.gbr`, `-drill-table.txt`, `-fabrication.pdf`, `-legend-top.gbr`, `-mask-bottom.gbr`, `-mask-top.gbr`, `-non-plated.drl`, `-paste-top.gbr`, `-plated.drl`, `-pos-top.csv`, `-profile.gbr` and `usb-sense.gbrjob`.

   The drill table reads "T1 0.300 plated hit via 1", "T2 0.600 plated slot component 4", "T3 1.005 plated hit component 4", "T4 0.650 non-plated hit mechanical 2" and "Holes: 11 (9 plated, 2 non-plated), of which 4 routed slots".
4. **BOM.**
   - J1 resolved: "4,1,J1,USB4105-GF-A,…,GCT,USB4105-GF-A,resolved".
   - The passives carry their values: "1,2,C1 C2,1 µF,Chip capacitor 0603 package,,,no part" and "6,2,R1 R2,5.1 kΩ,…".
   - The ICs: "8,1,U1,,TLV75733PDBVR package,,,no part" and "9,1,U2,,LM75BD package,,,no part" (finding 1).

## D. KiCad import (scene 5)

File › Import KiCad project…, then `beaconlet.kicad_pro`; "Save changes before continuing?" was answered with Save. The report was open about 2 s after Open:
- "Footprints 6 (0 on the back), 5 definitions", "Pads 14 of 14 imported", "Copper 6 nets, 8 track pieces, 2 vias, 1 fill", "Schematic 1 sheet; board nets equal 6, split 0, joined 0";
- "NOT IMPORTED 3": "3 tracks the engine's checks refused";
- "IMPORTED AND LISTED 9":
  - "5 footprints imported project-only (never publishable to a shared library)";
  - "1 board-level drawings, texts and dimensions, not imported (slice K7)";
  - "3 pins or labels on a wire's middle without a junction (joined to it)";
- "EXACT 40":
  - "14 pads mapped one to one" and "1 board outline imported as a board sketch";
  - "1 project rules (default class, board minimums) set over the fab profile as the project's own rule set";
  - "1 net classes imported" and "1 nets assigned to their class explicitly (no pattern gives it on our names)";
  - "11 labels and power ports placed", "10 wires drawn";
  - "1 zone layers imported as fills, filled by our engine";
- the toast: "Imported beaconlet.kicad_pro; save it to keep it".

Issues: "TRACK ENDS JOINING NOTHING 2" ("Track on VIN (Top) stops at (4, 16.8) mm: import refused the rest …", the same at (14.4, 16.8)) and "UNROUTED CONNECTIONS 3" ("VIN: 3 pads unrouted", "GND: 3 pads unrouted", "CTRL: 2 pads unrouted"; CTRL is unrouted in the source). In 3D the board shows its tracks under green mask and the parts as black envelope boxes, as in run 10.

Export before saving: the production check read "3 errors · 14 warnings" (Unrouted connections 3, Track ends joining nothing 2, Envelope model 6, No Part chosen 6). With the 3 unrouted errors waived for the test, the export wrote `beaconlet/beaconlet-production/beaconlet-*.gbr` and so on (15 files). **The unsaved import now names its files after the KiCad project.**

## E. Save, quit, reopen (scene 6)

- ⌘S showed "Saved /Users/agent/Documents/e2e11/usb-sense.sparksmith".
- ⌘Q quit with no prompt and an empty Recovery folder.
- `open -a Sparksmith ~/Documents/e2e11/usb-sense.sparksmith` showed the drawn board with "Opened /Users/agent/Documents/e2e11/usb-sense.sparksmith" in the first capture, about 3 s after the command.
- The Issues report after reopening was identical to the one before quitting: "8 notes. Connectivity, outline, courtyards and clearances (track to track 0.1 mm, JLCPCB 2-layer)".
- The app reopened as a window, not in full screen; the take ends after ⌃⌘F.

## F. Run 10's open items

| Run 10 item | Now |
|---|---|
| 1. GND pour does not join the GND pads | **Fixed.** "0 airwires" after the Top fill, no unrouted connection in Issues or in the production check. |
| 2. Router ignores hole clearances at J1 | **Fixed as far as seen.** No hole-clearance error from any routed track or via near J1 across about ten routing takes. J1's own pad-to-hole errors remain (finding 2). |
| 3. "0.2 mm apart; needs 0.2 mm" | **Not reproduced.** The D+/D− ties under J1 with four vias gave no via-to-via finding this time. J1's message now reads "0.175 mm" (run 10: 0.177 mm). |
| 4. No hint while a save waits on the Documents prompt | **Fixed.** The first ⌘S into `~/Documents/e2e11` wrote the file at once. After 3 s the title bar read "Saving… waiting for access to e2e11: check for a system permission prompt". Allow finished the save. |
| 5. QFN-16 → QFN-20 terminals 17–20 as errors | **Fixed.** Change package… › "qfn 20" › QFN-20 3×3 0.4 EP 1.7 › Replace package gave "Package, footprint and model taken from the generator". Issues: "UNMAPPED PACKAGE TERMINALS 4", rows "Package terminal ‘17’ has no electrical terminal" … ‘20’ with to-do circles, and "○ 4 on the mapping". No red "Making the part…" box was seen; the viewport showed an "Updating" chip. Publish then refused with "Can't publish drafts: Not published: 4 issues block publishing". |
| 6. 30° ground-array refusal cut off | **Fixed** at 1920 wide. Under the field it wraps in full: "The ground array is turned 30°; the module's part files turn it by a multiple of 45°." |
| 7. Exports of an unsaved project named "Untitled" | **Fixed** (section D). |

## What worked well

- **Speed throughout.** Every generated part, Update PCB, the outline change, the fill, 3D, the check, the export, the KiCad import and the relaunch responded at a speed that reads well on video, with no wait to cut.
- **The shove router is demonstrable.** A track pushed past a routed neighbour moves it aside live.
- **The parametric outline.** Typing 40 and 30 in the board sketch redraws the board at once.
- **The KiCad report is clear and complete**, now also saying what it refused.
- **Save and reopen are exact**, and the save hint names the folder it waits for.

## Findings

1. **A real part name and value takes the long way, and the BOM then reads oddly.** The Place palette's one-keystroke path ("sot-23-5", Enter) makes and places a component named after the package ("SOT-23-5"), with no name field. To give U1 a real name the designer must go to Component › New gallery, type the name in the generator form's Name field, create, publish, and only then place it. Pin names (IN, GND, EN, NC, OUT) are typed one per row in the Pins tab; the Pins tab's Paste table was not used. Other effects:
   - The package takes the component's name. The BOM's Package column reads "TLV75733PDBVR package" and "LM75BD package" instead of SOT-23-5 and SOIC-8, and the Value column of U1, U2, D1 and J2 is empty.
   - Passive values need the standard fields added to the component first ("Add standard fields" › Capacitance or Resistance, Update placed instances); the palette's "1u 0603" made a component named "Chip capacitor 0603" without a value.

   A designer would expect the palette to offer a name and value at the moment of placing, and the package column to keep the package.
2. **The built-in USB-C part blocks a clean production check.** Without any routing near them, J1's own footprint gives 6 errors: "J1 pad 17: hole is 0.175 mm from J1 pad A1B12 on GND" (and pad 18), and 4 × "J1 pad S: annular ring 0.2 mm" under JLCPCB 2-layer. 0 errors needed waivers again, as in run 10. These are the library's GCT USB4105 data against the fab profile; whether the part's land pattern or the profile's rule should change is a library decision.
3. **The module generator form shows raw numbers in two rows.** In "Radio and processor modules (castellated or LGA)", the rows "First pad centre x of each row" and "First pad centre y of each row" read "-8400000, -5715000, 8400000" and "5500000, -11900000, -11010000": unitless integers, apparently nanometres, where every other field reads in mm. The section header above "Side of each pad row" reads only "SING".
4. **The KiCad import refused 3 VIN tracks and left GND unrouted.** "NOT IMPORTED 3: tracks the engine's checks refused" and the two "Track on VIN (Top) stops at …: import refused the rest …" rows do not say which check refused them. "GND: 3 pads unrouted" remained although the B.Cu GND zone was "imported as fills, filled by our engine", and J1 pad 2, C1's via and Q1's via all lie inside it. The project file classes set Power (0.4 mm, clearance 0.25 mm) on VIN and GND; the cause was not investigated.
5. **A new generated component's 3D tab looked empty until Fit view.** After Create component for QFN-16, the 3D tab showed only the triad and grid. The fit button drew the model.
6. **Deploys keep stale library files.** `bin/guest deploy` copies over the existing `~/Applications/Sparksmith.app` with `ditto`. Two `*-1oz` profiles removed from `library/` stayed in the guest bundle until the bundle was deleted first. This is a test-rig issue, not an app issue: the profile list showed both.
7. **Reopen leaves full screen.** Relaunching with the project file opened a normal window, not the full-screen state the app was quit in. Minor; noted because it shows in the reopen take.

## Speed

| Operation | Time |
|---|---|
| Palette Enter → generated part on the pointer (USB-C, chip passives, LED, header) | under 1 s each |
| Generator form Create component (SOT-23-5, SOIC-8, QFN-16) | about 1 s to the toast |
| Change package QFN-16 → QFN-20 | under 1 s to the toast |
| Update PCB apply | instant |
| Board outline 40 × 30 by parameter | instant |
| Fill (whole-board GND, Top) | instant |
| Open PCB 3D (11 parts and a pour) | instant (first frame) |
| Production check and export (17 files) | instant |
| Save | instant for the file; the hint appeared at 3 s while the access prompt was open |
| Relaunch with the project file | about 3 s to a drawn board |
| KiCad import after Save | about 2 s to the report |

## Clips

Under `/Users/amd/dev/sparksmith-media/run11/` (not in the repository):

| File | What it shows |
|---|---|
| `clips/generator-form.mp4` (15 s) | The SOT-23-5 generator form, typing the name TLV75733PDBVR and creating the component, symbol, footprint and 3D model in one step. |
| `clips/shove-routing.mp4` (12 s) | A GND track routed beside U2 pushes the routed SCL track aside as the pointer moves. |
| `clips/layer-change-via.mp4` (11 s) | SDA leaves U2, drops a via with V and continues on Bottom to the header. |
| `clips/board-shape.mp4` (9 s) | The board outline resized to 40 × 30 mm by its parameters. |
| `clips/gnd-pour.mp4` (13 s) | A whole-board GND fill drawn with four clicks; the airwires disappear. |
| `clips/3d-orbit.mp4` (18 s) | PCB 3D opening at once, then a slow orbit of the finished board. |
| `clips/3d-closeup.mp4` (15 s) | Zoom to the SOIC-8 sensor, then hiding mask and parts to show the copper. |
| `clips/kicad-import-report.mp4` (17 s) | Import KiCad project…, the import report, expanded to its exact items. |
| `clips/fab-export.mp4` (22 s) | Waiving the USB-C part's findings with a reason, Export, the 17 written files. |

Each clip has a `.webp` preview (960 wide, 12 fps). `shove-routing`, `3d-orbit`, `generator-form`, `kicad-import-report` and `fab-export` also have a `.gif` (800 wide, 10 fps). The edited scene takes are in `raw-videos/` (`scene1-schematic.mp4` 5:41, `scene2-pcb-routing.mp4` 3:12, `scene3-3d.mp4`, `scene4-production.mp4`, `scene5-kicad-import.mp4`, `scene6-save-reopen.mp4`), with 11 stills in `stills/`. The recorder, input and cutting tools are in `tools/`, and the KiCad project in `kicad/`.

## Clean-up

The cleanup is partial: the AI scene is still to come.
- **Left in place for the AI scene:**
  - the app running in the guest with `usb-sense` open;
  - `~/Documents/e2e11` in the guest (the project, both production folders and the KiCad copy);
  - the frame folders in the shared `results/run11/raw/` (about 15 GB), kept for possible re-cuts;
  - the helper apps `~/Applications/ScreenRec.app` and `~/Applications/Puppet.app` in the guest, with their Screen Recording and Accessibility grants.
- **Deleted:** the guest's local frame folders (`~/Movies/rec/scene1`–`scene6`).
- **Display:** left as found (1920 × 1080 at 1.0).
- **Pasteboard:** the guest pasteboard holds the last Issues report.
