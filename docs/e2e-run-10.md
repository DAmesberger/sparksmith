# Tenth end-to-end run: a USB-C board from scratch, a KiCad import, run 9's fixes on the new Solidcraft pin

On 16 November 2026 the lead agent tested Sparksmith's desktop app as a designer would, covering:
- a new USB-C powered 3.3 V board from scratch: schematic, generated parts, Resolve parts, Update PCB, outline, placement, shove routing with layer changes, a GND pour, 3D, production check, fab export, save, quit and reopen;
- a KiCad project written for this run, imported, routed, checked and exported;
- the run 9 fixes that had not yet been checked in the app;
- the general feel on the new Solidcraft pin (fork `6a3b7e607`, upstream v0.42.0).

**Setup.** The build was `main` at cdecce83, built with the `ci` profile and run as `Sparksmith.app` in the macOS guest VM (Guest VM app). The bundle had `library/` copied into `Contents/Resources/library`, the bundle's `Info.plist`, and nix `libiconv`/`libcharset` in `Contents/Frameworks`. The guest display was 1024 × 640 points at 2×. The app window was 1024 × 560 points throughout (a resize to 610 points high did not take), so every check below was made at the small size.

**The board** (part A): a USB-C receptacle (J1, the built-in GCT USB4105), an LDO in SOT-23-5 (U1), a sensor in SOIC-8 (U2), three 0603 resistors (two 5.1 k CC pull-downs and an LED resistor), three 0603 capacitors, a green 0603 LED (D1) and a 1 × 4 header (J2). Nets: VBUS, 3V3, GND, CC1, CC2 and the LED net.

**The KiCad project** (`sensetap`, part B) was hand-written for this run from the KiCad file-format documentation; it copies no existing design. It has J1 (a 1 × 3 through-hole header, "Conn_01x03"), R1 "10k" and R2 "22k" (`Sensetap:R_0603`), C1 "100n" (`Sensetap:C_0603`, the same pads as the resistors under another name) and TP1 "TP_OUT" (a 1.5 mm round test pad), on a 30 × 20 mm outline. Nets are +3V3, OUT and GND. Two +3V3 track pieces are on F.Cu, and the GND path is a B.Cu track with a via and an F.Cu stub to R2. OUT and C1's GND are left unrouted. The schematic has wires, five OUT labels, two +3V3 and three GND power symbols. The `.kicad_pro` has Default and Supply classes, the pattern `+3V3` → Supply, GND assigned to Supply explicitly, and board minimums looser than the fab profile.

## Outcome

| | Result |
|---|---|
| Time | About 30 minutes of app time, from first launch to the last quit. |
| Generated parts | **Each one keystroke, under 1 s**: "resistor 0603", "1u 0603 16V", "100n 0603", "sot-23-5", "soic-8", "led 0603 green" and "header 1x4" each showed "Created …: click the sheet to place it, Esc to cancel" in the first capture after Enter. "usb-c" offered "USB-C receptacle, USB 2.0, 16 contacts, top mount (GCT USB4105)". |
| Schematic | Power ports (GND by default, renamed VBUS and 3V3: "Same names join"), wires, CC1/CC2 net labels and no-connects. The schematic ended at "10 to do". |
| Board | Update PCB, outline set to 35 × 28 mm with Edit shape, parts dragged into place. |
| Routing | Shove router with 45° mode. Layer changes with V placed vias and continued on Bottom. The router laid copper the checks then rejected next to J1's holes (finding 2). |
| GND pour | Filled on Top. The new Info findings appeared, for example "GND fill on Top: 4 thermal spokes of U1 pad 2 removed; they would end without reaching GND copper", grouped as "THERMAL SPOKES REMOVED 5". GND still read "GND: 16 pads unrouted" (finding 1). |
| 3D | **Opens at once and looks right** (see section D). |
| Production check | 0 errors only after waiving the built-in USB-C part's geometry errors and the GND/via errors (finding 1). |
| Fab export | 17 files (below). |
| Save, quit, reopen | **Identical.** The Issues report copied before quitting and after reopening was byte for byte the same: "7 errors, 1 to do, 5 notes. Connectivity, outline, courtyards and clearances (track to track 0.1 mm, JLCPCB 2-layer)". The first save waited on the macOS Documents-access prompt (finding 4). |
| KiCad import | **Values and footprints carried.** "Footprints 5 (0 on the back), 4 definitions"; R_0603 and C_0603 stay separate. Routed to "No issues", production check "10 warnings", "Wrote 15 production files". |
| Run 9 fixes | Four of five **fixed** as described. The QFN-16 → QFN-20 replacement leaves only the four unmapped terminals, but as errors (finding 5). |
| Crashes | None. |

## A. The board from scratch

1. **Parts.** The Place palette made every part from its generator with Enter. The panel said what would happen before Enter: "Not a component yet: choosing it makes one with its generator, then you place it", with "Chip passives and LEDs generator v2 · cited values (unverified)" and the chips "imperial: Size codes read as imperial; 0603M names the metric 0603" and "1.6 × 0.8 mm". After the first make, a second search read "In this project, r1 · version 2". The made part's card read "Chip resistor 0603 · r1 (latest)", "Chip resistor 0603 package · 1.6 × 0.8 × 0.45 mm", "Chip resistor 0603 footprint · 2 pads", "sym · fp · 3D · no MPN" and "Lands are estimated from the body (termination + 0.6 mm by width + 0.2 mm); confirm them against the part's recommended land pattern." SOT-23-5: "SOT-23-5 package · 2.9 × 2.8 × 1.175 mm", "SOT-23-5 footprint · 5 pads". "header 1x4" listed "Pin header 1×4 2.54 mm" with the note "2.54 mm (default): No pitch typed: add one, such as 2 mm", then the 1×40 and the 2.0 mm variants.
2. **Schematic.** Power ports attach to a pin in one click. Each net label was named in the side panel before placing. The status read "10 to do in the schematic". Resolve parts opened from the palette ("resolve"); the USB-C receptacle resolved to "GCT USB4105-GF-A".
3. **Board.** The fab-profile card preselected JLCPCB 2-layer. Edit shape set the board to 35 × 28 mm. The parts were dragged onto the board.
4. **Routing.** I routed VBUS, 3V3, CC1, CC2 and the LED net with Route (Shove, 45°). Pressing V during a route placed a via and continued on Bottom; the next click ended the route on the far pad. USB D+/D− were left as the receptacle's auto-named nets N27/N28 and tied A6–B6 and A7–B7 under J1 with vias, then undone (they are not used on this board).
5. **Pour.** A GND fill on Top over the whole outline. The new Info findings were listed under "THERMAL SPOKES REMOVED 5" with a blue "i" count in the Issues header ("i 5"). They never counted as errors.
6. **Production check and export.** See finding 1 for the waivers. The export wrote, into `~/Documents/e2e10/Untitled-production/`: `Untitled-assembly.pdf`, `-bom-parts.csv`, `-bom.csv`, `-copper-l1-top.gbr`, `-copper-l2-bottom.gbr`, `-drill-map.gbr`, `-drill-table.txt`, `-fabrication.pdf`, `-legend-top.gbr`, `-mask-bottom.gbr`, `-mask-top.gbr`, `-non-plated.drl`, `-paste-top.gbr`, `-plated.drl`, `-pos-top.csv`, `-profile.gbr` and `Untitled.gbrjob` (17 files). The drill table lists "T1 0.300 plated hit via 5", "T2 0.600 plated slot component 4" (J1's shell slots), "T3 1.005 plated hit component 4" and "T4 0.650 non-plated hit mechanical 2", "Holes: 15 (13 plated, 2 non-plated), of which 4 routed slots." The BOM reads "3,1,J1,USB4105-GF-A,…,GCT,USB4105-GF-A,resolved"; the generated passives, D1, J2, U1 and U2 read "no part" with empty values (none were typed).
7. **Save and reopen.** ⌘S opened the Save panel, and Return saved `~/Documents/Untitled.sparksmith` (317 KB). Quit left the Recovery folder empty. `open -a Sparksmith ~/Documents/Untitled.sparksmith` showed the drawn board in the first capture, about 3 s after the command. The Issues report was identical to the one before quitting.

## B. KiCad import

`open -a Sparksmith ~/Documents/e2e10/sensetap/sensetap.kicad_pcb` first asked "Save changes before continuing?" ("The project or component draft has unsaved changes.", Cancel / Don't Save / Save). After Don't Save, the KiCad import report was open in the next capture (under 2 s). There was no Documents-access prompt this time; the grant from the save in part A held.

The report (Copy report reached the pasteboard):
- "Footprints 5 (0 on the back), 4 definitions", "Pads 10 of 10 imported", "Copper 3 nets, 4 track pieces, 1 via, 0 fills", "Schematic 1 sheet; board nets equal 3, split 0, joined 0";
- "RULES NOT TAKEN 10", each with both values, for example "track to plated hole clearance: KiCad 0.25 mm, fab profile 0.35 mm (min_hole_clearance)" and "min silkscreen text height: KiCad 0.8 mm, fab profile 1 mm (min_text_height)";
- "exact D1 1 net classes imported" and "exact D1 1 nets assigned to their class explicitly (no pattern gives it on our names)";
- "exact K5 10 labels and power ports placed", "exact K5 7 wires drawn";
- "report G4 5 footprint text and drawings, not imported in this slice";
- "report - 1 board-wide settings (mask margin, via tenting) the fab profile refused".

The Components list shows four distinct footprints: "Sensetap:PinHeader_1x…", "Sensetap:R_0603", "Sensetap:C_0603" and "Sensetap:TestPoint_Pad…". C1 kept its own name although its pads equal R_0603's. Issues read "Unrouted connections 2".

I routed OUT (J1 pin 2 → R2 pin 1, R1 pin 2 → R2 pin 1, R1 pin 2 → TP1, C1 pin 1 to the OUT track) and C1's GND (C1 pin 2 → the GND via). The route tool took 0.25 mm for OUT and 0.4 mm for GND (the Supply class). Issues: "No issues". Production: "10 warnings" ("Envelope model 5", "No Part chosen 5"), no errors. Export: "Wrote 15 production files". The BOM and placement file carry the KiCad values and names: "1,1,C1,100n,Sensetap:C_0603 package,,,no part", "2,1,J1,Conn_01x03,…", "3,1,R1,10k,…", "4,1,R2,22k,…", "5,1,TP1,TP_OUT,…"; the drill table lists "T1 0.300 plated hit via 1" and "T2 1.000 plated hit component 3". In 3D, all five parts are plain black envelope boxes (the header has no pins), as the "Envelope model 5" warning says.

## C. Run 9's fixes

| Run 9 finding | Now |
|---|---|
| 1. Change package… leaves the footprint and package disagreeing | **Mostly fixed.** On the QFN-16 3×3 0.5 EP 1.7 component, the gallery said "Replaces the package, the footprint's 17 pads and the model; terminals matched by number; one undo step". "qfn 20" › QFN-20 3×3 0.4 EP 1.7 › **Replace package** gave the toast "Package, footprint and model taken from the generator" (with Undo), and Issues "4 errors, 1 warning": "Package terminal ‘17’ needs exactly one electrical terminal (0 mapped)" through ‘20’, and "The footprint has 21 pads and no pin-1 polarity pad". Run 9 had 101 errors here. ⌘Z showed "Undid: Package from a generator · Component". See finding 5. |
| 2. Refused ground-array turn appears only after Create | **Fixed.** Typing 30 in "Ground array turn" and pressing Tab drew the field red with "The ground array is turned 30°; the module's part files …" under it, the same sentence in a red banner above the form ("… turn it by a multi…"), and **Create component** disabled. See finding 6. |
| 3. A pointer click on *Change package…* does not open the gallery | **Fixed.** One pointer click on the button, low in the 3D tab's scrolled panel at 1024 × 560, opened the gallery in package scope. |
| 4. "resistor 0603" also offers the 0201 | **Fixed.** The palette lists only "Chip resistor 0603" with the chip "imperial: Size codes read as imperial; 0603M names the metric 0603". |
| 7. Issues rows hidden in PCB at 1024 × 560 | **Fixed.** With the Board panel open, the Issues card showed its filters ("PCB only", "Every kind"), "THERMAL SPOKES REMOVED 5", "WAIVED 8" and the row "J1 pad 17 · J1 pad A1B12". The footer "Connectivity, outline, court…" shows its full text on hover: "Connectivity, outline, courtyards and clearances (track to track 0.1 mm, JLCPCB 2-layer)" / "What these checks cover". |
| Footer without a fab profile | **Fixed.** File › New project (Don't Save), Cancel on the Fab profile card; Issues › Copy report: "No issues. Connectivity, outline and courtyards; no fab profile, so no clearance is checked". |

## D. Feel on the new Solidcraft pin

- **3D opens at once.** The PCB 3D tab was drawn in the first capture after the click, for the 13-part board with a pour and for the imported board. Zooming in was smooth.
- **Materials look right.** Green solder mask over the pour with the traces visible through it; ENIG pads and exposed copper gold; the header's pins gold on a black body; the USB-C shell brushed metal with its tabs; SOIC-8 and SOT-23-5 black bodies with gull-wing leads seated on gold lands; 0603 parts with metal end caps; white silkscreen designators beside the parts; a tan laminate edge. The QFN-16 in the part editor's 3D tab: black body, pin-1 dimple, terminals at the edges. No visual regression was seen against runs 8 and 9.
- **No crash** in either session.

## What worked well

- **Generated parts are instant and explain themselves.** Every family the board needed came from a typed search and Enter, with the generator, the citation status and the land-pattern caveat on the card.
- **The shove router and V for a layer change** behave as a designer expects: the via drops where the pointer is and the route continues on the other side.
- **The spoke notice is clear and harmless.** "… thermal spokes of U1 pad 2 removed; they would end without reaching GND copper" names the pad, the fill and the reason, is grouped, and never blocks.
- **Save and reopen are exact.** The copied Issues report matched byte for byte.
- **The KiCad import keeps what the designer typed**: footprint names, values, labels, power ports and net classes, and it says what it did not take.
- **Run 9's form and layout fixes are in.** The refusal is on its field, Create is disabled, and the Issues card is usable at 1024 × 560.

## Findings

1. **The GND pour on Top did not join the GND pads, and the board reached 0 errors only by waiving.** After filling GND on Top over the whole 35 × 28 mm outline, Issues still read "○ GND: 16 pads unrouted" ("Unrouted: R2 pad 2 (GND)", "… U2 pad 1 … J1 pad S …", 19 pad lines), next to five Info rows such as "GND fill on Top: 3 thermal spokes of J1 pad B1A12 removed; they would end without reaching GND copper". Refill did not change it. The pour may have been cut into islands by the routing, so the spokes found no GND copper. This was not confirmed: I did not inspect the fill's islands. The production check could only be cleared by waiving: J1's built-in geometry errors with the reason "built-in USB-C part geometry" (4 × "J1 pad S: annular ring 0.2 mm; needs at least 0.25 mm", "J1 pad 17: hole is 0.177 mm from J1 pad A1B12 on GND; needs 0.2 mm", the same for pad 18), and the remaining errors with "e2e10 test export". A designer would expect a full-board GND pour to connect the GND pads, or to be told which pads it left out and why.
2. **The shove router lays copper the checks then reject at J1's holes.** Routing VBUS under the USB-C receptacle produced "J1 pad S: hole is 0.306 mm from a track on VBUS; needs 0.35 mm (JLCPCB 2-layer: track to plated hole clearance)", "J1 pad 17: hole is 0.04 mm from a track on VBUS; needs 0.2 mm (… track to unplated hole clearance)" and the same for pad 18 (0.042 mm). A via it placed gave "A via on VBUS: drill is 0.186 mm from a track on CC2; needs 0.2 mm (JLCPCB 2-layer: track to via hole clearance)". The router seems to keep copper-to-copper clearance but not hole clearances. I deleted and rerouted those tracks; the via error stayed until it was waived.
3. **A spacing error says the spacing is the minimum.** Two vias tying J1's A6 and B6 gave "× Vias on N27 and N27: drills are 0.2 mm apart; needs 0.2 mm (JLCPCB 2-layer: via hole to via hole clearance)", and the same for N28. The measured value is probably just under 0.2 mm and is rounded in the message, which then reads as no error.
4. **Saving into `~/Documents` waits on macOS's access prompt with "Saving…" up.** The first ⌘S wrote `Untitled.sparksmith` (317 KB) at once, but the title bar kept "Saving…". A sample of the process showed the save thread in `sparksmith_io::persistence::sync_directory` → `open()`, blocked on the prompt "“Sparksmith” would like to access files in your Documents folder." (shown by UserNotificationCenter, behind the app window, so not visible at first). After Allow, the save finished. This is expected for a re-signed ad-hoc bundle (runs 7 and 9). The noteworthy part is that the file is already written before the prompt, and the app gives no hint of what it waits for.
5. **After QFN-16 → QFN-20, the four new terminals are errors, not to-dos.** The replacement now does what it says (17 → 21 pads, terminals matched by number). The four new terminals 17 to 20 have no electrical terminal, which is expected because the symbol has 16 pins. They are listed as "× Package terminal ‘17’ needs exactly one electrical terminal (0 mapped)" under "UNMAPPED PACKAGE TERMINALS 4", red, and the component shows "4 errors". Whether an unmapped terminal right after a package change should be a to-do (the next step of the work) rather than an error is a product decision. Also, while the replacement ran, the form showed "Making the part…" in a red error box with a ✕ icon for a moment before the toast. A progress message styled as an error reads like a failure.
6. **The ground-array refusal is cut off in both places at 1024 × 560.** Under the field: "The ground array is turned 30°; the module's part files …"; in the banner: "The ground array is turned 30°; the module's part files turn it by a multi…". The full sentence ("… by a multiple of 45°") could not be read in the form; a hover on the field's message was not tried.
7. **Production files of an unsaved project are named "Untitled".** Both exports wrote `Untitled-*.gbr` and so on, including the imported `sensetap` board (into `sensetap/Untitled-production/`). Run 7 noted that the project takes the KiCad name on save; an export before the save does not. A fabricator receives files named "Untitled".
8. **The imported board's 3D is all envelope boxes.** All five imported footprints are drawn as black boxes, and the 1 × 3 header has no pins. The production check says so ("Envelope model 5"), and the import report says footprint drawings are not imported in this slice. Noted for completeness; not a regression.

Not a finding: the KiCad report lists the class pattern `+3V3` neither as taken nor as refused. +3V3 already had 0.4 mm tracks from KiCad and was not rerouted, so whether the pattern took effect was not seen.

## Speed

| Operation | Time |
|---|---|
| Palette Enter → generated part on the pointer (resistor, capacitor, SOT-23-5, SOIC-8, LED, header, USB-C) | under 1 s each (done by the first capture) |
| Change package QFN-16 → QFN-20 | under 2 s to the toast |
| Open PCB 3D (13 parts and a pour) | instant (drawn in the first capture) |
| Fill / Refill | instant to the eye |
| Production check and export (17 and 15 files) | instant to the eye |
| Save | instant for the file; the "Saving…" state lasted about a minute, waiting on the access prompt |
| Relaunch with the project file | about 3 s to a drawn board |
| KiCad import after Don't Save | under 2 s to the report |

## Clean-up

The app was quit in the guest (the imported project with "Don't Save"); its Recovery folder is empty and no Sparksmith process runs. The following were deleted:
- from the guest: `~/Documents/e2e10` (both production folders and the KiCad copies) and `~/Documents/Untitled.sparksmith`;
- from the shared `results/` folder: the `e2e10` folder (the KiCad source files and a guest helper script).

No screenshot files were written. The guest pasteboard holds the last copied Issues footer text. The guest's Documents-access grant for Sparksmith remains. The guest display was left as found (1024 × 640 at 2×).
