# Ninth end-to-end run: the D1 generator form, run 8's fixes, parts before a fab profile

On 15 November 2026 the lead agent tested Sparksmith's desktop app as a designer would, covering:
- making a resistor, a QFN and a header by hand from the Place palette;
- the generator gallery: each generator chip, editing parameters, a refused value, the module form's scrolling at a small window, and the WROOM-3 preset;
- *Change package…* in the part editor, and the palette after a cold start;
- run 8's fixes: Copy report, Copy BOM, long finding rows, the placement toast and a pin landing on a wire end;
- making a generated part in a new project before a fab profile is chosen (the open founder question).

**Setup.** The build was `main` at 292f2e40, built with the `ci` profile and run as `Sparksmith.app` in the macOS guest VM (Guest VM app). The bundle had `library/` copied into `Contents/Resources/library`, the bundle's `Info.plist`, and nix `libiconv`/`libcharset` in `Contents/Frameworks`. The guest display was 1024 × 640 points at 2×, and the app window was 1024 × 560 points, the smallest size this run used.

**The KiCad board** (`duo`) was hand-written for this run from the KiCad file-format documentation; it copies no existing design. It has two `Duo:R_1206` footprints (R1 "1k", R2 "2k2") on a 30 × 20 mm outline, nets VIN and MID, and one F.Cu track. Its `.kicad_pro` has looser minimums than the fab profile and the netclass pattern `V[A-Z]+|GND`.

## Outcome

| | Result |
|---|---|
| Time | About 35 minutes of app time, from first launch to the final quit. |
| Three makes from the palette | **Worked.** "resistor 0603" → Enter → the part was on the pointer in under 1 s (the first capture, 0.9 s after Enter, already showed it). "qfn-16" and "header 2x5" were the same. No busy state was caught in any capture: each make finished before the first capture. |
| Placement toast | **Fixed** (run 8 finding 5): "Placed R1, joining no net; 3 new issues" with **Undo** and **Show**; "Placed J1, joining no net; 11 new issues". |
| Pin on a wire end | **Names the net**: "Placed R2; R2 pin 1 joins N2". A first try a few pixels off the wire end read "Placed R2, joining no net; 3 new issues". |
| Copy report / Copy BOM | **Fixed** (run 8 finding 4). Issues' Copy report, the KiCad import report's Copy report and Resolve parts' Copy BOM all reached the guest pasteboard (`pbpaste`), by accessibility press and by a pointer click. |
| Generator gallery | One chip per generator, 9 in all. Each chip shows its generator's form ("… · version N"), and the list header read "GENERATORS 200". |
| WROOM-3 preset | **Offered and placed with 3D** (run 8 finding 1 fixed). The module generator's preset made "ESP32-S31-WROOM-3", placed as U2. In PCB 3D it shows a green module board, castellated pads and a light-grey can. |
| Refused value | A ground-array turn of 30° was refused in the module's words: "ground array is turned 30°; the module's part files turn it by a multiple of 45°" (finding 2). |
| Module form at 1024 × 560 | Scrolls. All sections, including the long row and marking lists, are reachable with the wheel; Cancel and Create component stay at the top. |
| Change package… | The gallery opens in package scope, and the replacement is one undo step ("Undid: Package from a generator · Component"). After replacing QFN-16 with QFN-20, the component had **101 errors** (finding 1). |
| Cold start | The window appeared 1 s after `open -a Sparksmith`. Three seconds later, "resistor 0603" already listed "FROM GENERATOR 1: Chip resistor 0201 (0603 metric)" next to "GENERIC PARTS 1: Chip resistor 0603" (finding 4). |
| Part before a fab profile | **Not refused** (run 8 finding 2 fixed): see the next section. |
| KiCad import | Import report: "Footprints 2 (0 on the back), 1 definition", "Pads 4 of 4 imported", "Copper 2 nets, 1 track piece, 0 vias, 0 fills", "RULES NOT TAKEN 13". This time a folder-access prompt came first (finding 5). |

## Template/generated part before a fab profile (founder question)

This was done in File › New Project, with "Don't Save" for the open work.
1. In the Schematic, I searched the Place palette for "led 0805 red". It offered "GENERIC PARTS 1: Chip LED 0805", chipped "not yet created", "imperial", "2 × 1.25 mm" and "Red". The panel said "Not a component yet: choosing it makes one with its generator, then you place it".
2. Enter showed the toast "Created Chip LED 0805: click the sheet to place it, Esc to cancel". The Place component panel read "Chip LED 0805 · r1 (latest)", "Has sym · fp · 3D · no MPN".
3. A click on the sheet placed it: "Placed D1, joining no net; 3 new issues" with Undo and Show.
4. "bga 36" + Enter then made "BGA-36 0.5 6×6 4×4" the same way ("Created BGA-36 0.5 6×6 4×4: click the sheet to place it, Esc to cancel"). Neither make showed the fab-profile sentence.
5. Switching to the PCB workspace opened the **Fab profile** card ("A board's design rules come from its fabricator's profile", JLCPCB 2-layer preselected, Cancel / Use profile). The Issues list showed one to-do, "Not placed, 1"; the Board panel's Rules row read "No fab profile".

In the first project, which also had no profile, **Update PCB** applied the schematic to the board while the Rules row read "No fab profile". The board's issues were then checked against "0.2 mm clearance and courtyards", the defaults the Issues footer names. So no step of making, placing or sending a part to the board waits for a profile. Only the PCB workspace asks for one, with a card that can be cancelled.

## Run 8's fixes

| Run 8 finding | Now |
|---|---|
| 1. WROOM-3 not offered as a template | **Fixed.** The module generator's ESP32-S31-WROOM-3 preset is offered, makes the part and places it with 3D. |
| 2. Part creation in a new project refused with the fab-profile sentence | **Fixed.** See above. |
| 3. A long finding row is cut by Waive…, and its tooltip ends in "…" | **Not rechecked.** This run produced no finding long enough to reach the buttons. The longest rows ("R2 pad 1 is 0.1 mm from the board edge", "The footprint has 107 pads and no pin-1 polarity pad") fitted. The imported board had no refused-track stub. |
| 4. Copy buttons do not reach the clipboard | **Fixed** for Copy report (Issues, KiCad import report) and Copy BOM. |
| 5. Placing over wires gives no feedback | **Fixed.** Each placement shows the toast, with joined nets or "joining no net" and the new-issue count. |

## What worked well

- **Making a part is one keystroke and under a second**, for the resistor, the QFN, the header and the WROOM-3 module. The part arrives on the pointer, and placing it is the same undo step.
- **The toast says what the placement did.** It names the pin and net it joined, gives the new-issue count, and offers Undo and Show.
- **One form for every generator.** The nine chips each give their own form, with sections, units and a preset menu. The module's refusal arrives in the module's words.
- **The clipboard works.** Each report pastes as readable text with the finding code under each line, for example "× R1 pin 1 unconnected / instance.terminal_unaccounted: R1 terminal ‘1’ is neither connected nor marked no-connect.". The BOM pastes as tab-separated text with a header row.
- **A new project no longer blocks on the fab profile.** The profile is asked for where it matters, in the PCB workspace.

## Findings

1. **Change package… leaves the footprint and package disagreeing.**
   - Pressing *Change package…* on the ESP32-S31-WROOM-3 draft opened the gallery in package scope. A double-click in it, meant for the browser behind it, applied a package. After that, the QFN-16 component's Issues read "97 errors, 1 warning": "Package terminal ‘3’ has no pad" through "Package terminal ‘99’ has no pad", and "The footprint has 17 pads and no pin-1 polarity pad".
   - On that component, footprint tab › From generator › "qfn 20" › **Replace package** gave "101 errors, 1 warning". This was the 97 terminal errors plus "Pad 17 lands a package terminal the package does not have" for pads 17 to 20, with "The footprint has 21 pads and no pin-1 polarity pad".
   - Two ⌘Z ("Undid: Package from a generator · Component", each time) restored the drafts.

   The exact path into the first replacement was not clean (an unintended double-click), so the 99-terminal package's origin is not certain. The second step was deliberate. Replacing a 16-terminal package with a 20-terminal one by number left the 21 new pads pointing at terminals the package did not have, while 97 terminals had no pad. Either the package and the footprint were not both replaced, or the old terminals were kept.
2. **The refused ground-array turn appears only after Create.** Typing 30 in the turn field and pressing Tab marked nothing. The sentence "ground array is turned 30°; the module's part files turn it by a multiple of 45°" appeared after **Create component**. The D1 notes say a value the module would refuse is marked on its own field. Here the field accepted 30 without a mark until the make was attempted. (Not confirmed: whether the sentence was drawn under the field or as the form's general refusal after Create.)
3. **A pointer click on *Change package…* did not visibly open the gallery** in two tries at 1024 × 560, while an accessibility press on the same button and the Footprint tab's **From generator** did. The button sits low in the 3D tab's scrolled panel. The click may have landed on the panel's scroll edge, so this needs a recheck before any fix.
4. **"resistor 0603" also offers the 0201 resistor.** After a cold start, the palette listed "Chip resistor 0603" (generic, "imperial", "1.6 × 0.8 mm") and, under FROM GENERATOR, "Chip resistor 0201 (0603 metric)". This is a correct match on the metric code, but a designer typing 0603 almost always means imperial. The metric match is listed as a separate group, so a careless Enter still takes the right part.
5. **The KiCad import asked for Documents access again.** `open -a Sparksmith ~/Documents/e2e9/duo.kicad_pcb` showed "Save changes before continuing?", then the macOS prompt "“Sparksmith” would like to access files in your Documents folder." The import stayed at "Importing KiCad project…" until Allow was pressed. This is expected for a re-signed ad-hoc bundle (run 8 kept run 7's grant), not an app fault.
6. **The import report lists no refused netclass pattern** for `V[A-Z]+|GND`, a regular expression assigned to the Default class. Run 8 listed such a pattern under "Not imported". Here the pattern names only the Default class, so dropping it changes nothing, and silence may be intended. The report's sections were "exact 4 pads mapped one to one", "exact B1 1 board outline imported as a board sketch", "exact D1 1 project rules (default class, board minimums) set over the fab profile as the project's own rule set", "report L1 1 footprints imported project-only (never publishable to a shared library)" and "report D1 13 KiCad rules looser than the fab profile, not taken (the fab limit kept)".
7. **The Issues list is hard to open in the PCB workspace at 1024 × 560.** With the Board panel open, the Issues strip sat at the panel's bottom edge, and its rows (filters "PCB only", "Every kind") were pushed below the window. Dragging the divider did not enlarge it. The rows could be read only through accessibility ("Too close to the board edge, 2", "R2 pad 1 is 0.1 mm from the board edge" with Locate and Waive). The Schematic and Component workspaces showed their rows.

## Speed

| Operation | Time |
|---|---|
| Palette Enter → generated part on the pointer (resistor, QFN-16, header 2×5, LED, BGA-36) | under 1 s each (done by the first capture) |
| Gallery Create component → ESP32-S31-WROOM-3 made and opened | under 2 s (done in the capture 0.35 s after the click) |
| Update PCB and open 3D with the WROOM-3, QFN, header and resistors | about 1.8 s from the click to a drawn capture, including the capture itself |
| KiCad import after Allow | under 2 s |
| Cold start to window | 1 s; the palette listed generated parts about 3 s after launch |

## Clean-up

The app was quit in the guest without saving the test projects ("Don't Save"), and its Recovery folder is empty. The following were deleted:
- from the guest: the KiCad copies in `~/Documents/e2e9`;
- from the shared `results/` folder: the KiCad source files and the temporary screenshot.

The guest pasteboard holds the test string "probe". The guest display was left as found (1024 × 640 at 2×), and the driver session was ended.
