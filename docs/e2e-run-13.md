# Thirteenth end-to-end run: a whole board in the macOS guest, with the assistant throughout

On 22 November 2026 a test agent drove Sparksmith's desktop app in the macOS guest VM as a designer would, from a new project to fab output, save, reopen and crash recovery. It used the in-app assistant (Claude API) at each stage. No code was changed.

**Setup.**
- Build: `main` at **97bd4a76** ("Merge main (8adffdb1): Solidcraft repin"), built with `cargo build --release -p sparksmith` in a detached worktree with its own target directory (20 min).
- Bundle: `library/` in `Contents/Resources/library`, the artwork SVGs from the library repository in `Contents/Resources/library/artwork/`, nix `libiconv`/`libcharset` in `Contents/Frameworks`, ad-hoc signature. The old guest bundle was deleted before `bin/guest deploy`.
- Guest display: 1920 × 1080 at scale 1.0. The window was 1900 × 960, not full screen.
- Model: the stored Anthropic connection with `claude-opus-5-5`. The key was already in the guest keychain. The new ad-hoc signature made macOS ask for the login password once on first use, and "Always Allow" was chosen.
- Datasheet: TI's public TLV757P datasheet (SBVS322).
- Driver note: background clicks and keys do not reach the egui canvas, so every action used foreground delivery.

**The board** (`usb-sensor`, 60 × 40 mm, JLCPCB 4-layer, black mask, ENIG):
- U1, ESP32-S3-WROOM-1 from the module generator;
- J1, the shipped GCT USB4105 USB-C receptacle;
- U2, TLV75733PDBVR 3.3 V LDO, made with the datasheet workflow in the chat;
- C1–C3 1 µF, C4–C5 100 nF, R1–R2 5.1 k CC pull-downs, R3 10 k EN pull-up, R4–R5 4.7 k I²C pull-ups and R6–R7 1 k LED resistors, all 0603;
- C6, a 22 µF 10 V case-B tantalum;
- D1–D2, green 0603 LEDs;
- SW1 (reset) and SW2 (boot), E-Switch TL3305A tactile switches;
- J2, a 1 × 4 header (3V3, SDA, SCL, GND);
- FID1–FID3, global fiducials.

There is no I²C sensor on the board: neither the shipped library nor the generators have one under a name a designer would search for, and the chat budget went to the LDO and the module pins instead. J2 carries the I²C bus out.

## Blockers and production gaps (ranked)

| # | Severity | Finding |
|---|---|---|
| 1 | **Blocker (AI)** | Asking the chat to wire sub-circuits on the schematic ended with "nothing changed" after 10 requests and 791 k tokens: it called `project.inspect` 8 times and never a wiring verb (finding 22). |
| 2 | **Blocker (AI)** | A datasheet workflow run stops at the default token limit after 4 requests, with no end card, no undo and no way to continue. It leaves an orphan draft (finding 17). |
| 3 | **Production gap** | Generic passives, LEDs, switches and headers have no orderable Part: Resolve parts shows "Fitting parts 0" for every group, so the BOM has an MPN only for J1. Nor can a variant fit a substitute part (findings 9, 40). |
| 4 | **Production gap** | Online library: the default address is `http://localhost:8787` (`crates/io/src/settings.rs:57`). With `https://parts.sparksmith.io` set and no sign-in, the palette shows only the sign-in note, never a result (finding 3). |
| 5 | **Production gap** | The module generator's ESP32-S3-WROOM-1 symbol has 41 pins numbered 1–41, with no names and all passive. The board cannot be wired by function without the datasheet. Naming them through the chat cost about 2.4 M tokens (findings 7, 19). |
| 6 | **Production gap** | The LDO made by the datasheet workflow has a symbol with pin numbers only (no IN, GND, EN, NC, OUT) and nothing in the BOM's Value, Manufacturer or MPN columns. The module has the same empty columns (findings 16, 41). |
| 7 | **Production gap** | Once the unrouted connections are waived, the Issues badge shows a green "No issues" while 38 airwires remain. Export then writes a complete-looking job (finding 37). |
| 8 | **Production gap** | The shipped USB4105 footprint fails the shipped JLCPCB 4-layer profile: pads 17 and 18 are 0.175 mm from the GND lands, and the rule needs 0.2 mm. This is open since run 11 (finding 29). |
| 9 | **Production gap** | Push-and-shove did not move the USB diff pair. A 3V3 track routed across it stopped short and left a dangling stub, with no message (finding 32). |
| 10 | **Production gap** | The diff pair routed at 83 Ω (USB needs 90 Ω) with no impedance warning. The 4-layer stackup preset is 1.74 mm thick, not 1.6 mm, also without a warning (findings 2, 31). |
| 11 | **Major (AI)** | The chat cannot share the project: while the person edits, its applies are refused ("the project had already moved on"). The turn's undo step and change summary then claim the person's own edits (finding 18). |
| 12 | **Major (AI)** | Asked to explain a finding and fix it, the chat showed a good question card. After the answer it spent 754 k tokens without changing anything and never gave the explanation (finding 24). |
| 13 | **Major** | Fiducials and other board-only parts can only be placed from the schematic: the PCB workspace has no place-part entry, and P does nothing there (finding 30). |

## Outcome

| | Result |
|---|---|
| Time | About 2 h 10 min wall clock (07:00–10:05), including about 1 h 20 min of tester waits on chat turns. Hands-on time was about 50 min; the 30-minute budget for a small board was not met. |
| Project settings | New project, fab profile, 4-layer stackup and black mask **work**; values are truncated in the Board card. |
| Online library | **Not usable** (gap 4). The offline library and generators were used instead. |
| Generators and Place palette | ESP32-S3-WROOM-1, passives, tantalum, LEDs, tactile switches, header and fiducials **work**. Generator results show no preview, and "1u 0603" picks a resistor. |
| Schematic | Wired by hand: real wires, ground and 3V3 ports, labels only for USB_DP/DN, SDA/SCL and BOOT, 31 no-connect flags. ERC shows **0 errors** and 20 "No part chosen" to-dos. |
| Layout | Placement by coordinates, GND and 3V3 inner planes, vias to the planes, walkaround routing, a coupled USB pair and 3 fiducials **work**. Shove of the pair **failed**. The board was left part-routed (34 airwires). |
| 3D | Standard and Realistic open **instantly**. Resistor codes (472, 512), the tantalum's "226 A", the module's Espressif label with CE mark and QR code, and the black mask are **correct**. |
| Variants | A DNP variant **works** in layout, 3D, BOM, placement and the assembly drawing. A substitute part is **not possible** (gap 3). |
| Production | Export is gated on errors (good). Waivers, then "Wrote 19 production files" in about 1 s. |
| Save, quit, reopen | **Identical.** All 19 files exported again after reopening were byte-identical. |
| Crash recovery | **Works.** After `kill -9`, the next launch offered "Recover an earlier session?" with the unsaved AI tracks, and Restore brought them back. |
| Assistant | Datasheet workflow run 2 **completed**. Module pin naming **completed** at high cost. SDA/SCL routing **completed** with clean undo and redo. Schematic wiring and the footprint fix **failed**. |
| AI spend | About **4.6 M tokens, estimated 19 USD** (upper bound, uncached; see [Cost](#cost)). |
| Crashes | None. |

## 1. Start, new project, project settings (07:00–07:13)

1. **Launch.** The app opens straight into an Untitled PCB workspace with the Fab profile card open; there is no start screen. File › New Project does the same.
2. **No profile.** Esc on the card leaves the board with "No fab profile". The Route panel then shows the via drill and pad as red "0 mm" with "Enter 0.1 to 6 mm", an error state the person did not cause (finding 1).
3. **Profile and stackup.**
   - Project renamed to `usb-sensor` in the Board card.
   - Rules › JLCPCB 4-layer: the review lists 64 rule changes and one new issue, "JLCPCB 4-layer is for 4, 6, 8, … 30 or…" (truncated). It does not offer to set the stackup.
   - Stackup › 4-layer preset: "1.74 mm over copper" (finding 2).
4. **Solder mask.** Black updates "Solder mask colour: Black or white (follows…)" in the rules, as the colour-dependent rules intend.
5. **Settings › AI** is clearer than in run 12:
   - Connection "Anthropic";
   - "API key: Key stored in system keychain", with Replace and Remove;
   - Model and Test connection;
   - Run limits in a collapsible section.

   Settings opens as an overlay, behind the command palette when both are open.
6. **Settings › Libraries.** The online library address defaulted to `http://localhost:8787`. Set to `https://parts.sparksmith.io`, it reads in red "Sign in to the online library … Searching works without an account". No sign-in was made (no account changes).

## 2. Library, generators, Place palette (07:13–07:27)

- **P in the schematic** opens the palette.
  - "esp32" lists three generator presets and the shipped S31 body.
  - Enter creates ESP32-S3-WROOM-1 and puts it on the pointer, and a click places it.
  - The toast reads "Placed U1, joining no net; 42 new issues"; 41 are errors at once (finding 5).
- **No preview for generator results.** Generator rows have none, while in-project components show symbol, footprint and 3D in the side card (good).
- **The shipped USB4105.** Its highlighted row shows "Type to find a component" in the side card. Enter places J1 at once, unlike generator items, which attach to the pointer first.
- **"ldo 3.3"** answers "Ignored: ldo" and lists a WS2812B LED, a B3S-1000 switch and the USB4105. There is no regulator in the library.
- **"1u 0603".** The top hit is the chip *resistor* already in the project, because "1u" is read as "Resistance = 1 µΩ or Rated power = 1 µW or 1u". The batch click placed two resistors (finding 6).
- **Other parts by one search each:**
  - "100n 0603", "5.1k 0603", "10k 0603", "4.7k 0603", "1k 0603";
  - "led 0603 green" (Chip LED 0603);
  - "tactile" (TL3305A);
  - "header 1x4";
  - "tantalum", case B, then 22 µF and 10 V typed in the part card; its marking updates to "226 A".
- **Mirror** flips vertically. Pins facing right need Rotation 180° plus Mirror.
- **Placement noise.** Every placement adds "joining no net; N new issues", and the error count reached 88 before any wire was drawn (finding 5).

## 3. Schematic (07:27–09:31, of which about 35 min hands-on)

**Datasheet workflow in the chat (LDO).**
1. `/` › Datasheet → component › Choose… › `tlv757p.pdf` read at once (41 pages): run 12 finding 9 is **fixed**.
2. The generator gate now shows the choice with the full reasoning (run 12 #6 **improved**).
3. **Run 1 stopped at "Pinout · 5 of 14"** with "Limit reached: 350000 tokens: 255202 used, and the next request could take 102697". That was after 4 requests and 2.7 k output tokens; each request resends about 64 k tokens. Nothing ends the run and it cannot continue (finding 17).
4. With the limits raised, **run 2 completed** in about 7 minutes and 9 requests.
   - Impact card: "New part: 5 pins, SOT-23-5 / 4 functions added · 5 functions removed · 5 terminals changed / Findings: none". The plurals are fixed, but "removed" on a new part remains (run 12 #7 partly fixed).
   - The result card is readable: Drafts, Pages used, Review summary, Show review, Undo run and Open in editor (run 12 #5 **fixed**).
5. **The LDO symbol shows pins 1–5 without names** (finding 16). The aborted run 1 draft remains as a second identical "TLV75733PDBVR" in the palette.

**Module pins through the chat.**
- **Turn 1.** "Give U1's component the pin names from the datasheet" (432 k tokens, 3 min).
  - The chat drafted a correct table from its own knowledge and said so.
  - `library.pinout_apply` was refused three times with "the project had already moved on, from revision 50 to 63…", because the tester kept arranging parts.
  - The turn's summary then listed the tester's edits ("Parts: U2 added; Nets: 5 added…") as its own undo step (finding 18).
- **Turn 2.** "Save it, mark reviewed, publish, update U1": U1 got its pin names, but the turn used **33 requests and 1.93 M tokens** and ended "limit reached" (finding 19).

**Wiring sub-circuits through the chat.** The request covered reset, boot, the LEDs, I²C with labels and decoupling. It ended "nothing changed" after 10 requests and 791 k tokens (finding 22).

**Wiring by hand.**
- **Nets drawn:**
  - VBUS → C6, C1 and U2 IN, with EN tied to VBUS;
  - U2 OUT → C2 and a 3V3 port;
  - GND ports;
  - CC1 and CC2 to R1 and R2;
  - USB_DP and USB_DN by net labels at J1 and U1 IO20/IO19;
  - reset: R3, C3 and SW1 on EN;
  - boot: SW2 through a BOOT label to IO0;
  - LEDs on IO2 and IO38;
  - SDA (IO8) and SCL (IO9) by labels to R4, R5 and J2;
  - C4 and C5 at U1;
  - SHIELD to GND;
  - 31 no-connect flags.
- **The wire tool is pleasant.** Click a pin, click the corners, click the end pin; junction dots appear on their own and the toast reads "Drew the wire ×4". Net labels draw as neat flags away from the pin.
- **Result:** ERC (continuous) shows 0 errors and 20 "No part chosen" to-dos (finding 9).
- **First save** opened the native panel in the home folder: `/Users/agent/usb-sensor.sparksmith`.

## 4. Layout (09:31–09:40)

- **Update PCB.** The review is clear:
  - "Adds 21 placements";
  - parts are staged beside the board;
  - the two J1 hole errors are named before Apply;
  - "83 new issues, 21 resolved".

  Apply was instant.
- **Placement.** Typed into Position and Rotation for each part: fast and exact, but the fields have no X/Y captions, and Y counts up from the bottom edge, which is shown nowhere (finding 27). "Too few fiducials" appears once parts are on the board (profile-driven, good).
- **Fills.** GND on Inner 1 and 3V3 on Inner 2, each 4 clicks and Enter; filled at once.
- **Vias to the planes.** The Via tool on pads, with net chosen from a dropdown, took the airwires from 61 to 38. Via-in-pad on 0603 gets no DFM note.
- **Walkaround.** R6 → D1 → U1 IO2 found its way around D1's pad first time.
- **Diff pair.** Board › High speed › Suggested pairs found "USB_D" from the net names; Add. Routing from one pad routed both tracks coupled ("Routing pair USB_D · gap 0.09 mm · 83.1 Ω") to U1 pads 13/14 (finding 31).
- **Shove.** A 3V3 track routed across the pair stopped short with no shove and no message, leaving "Track ends joining nothing" (finding 32).
- **Undo.** Cmd+Z after a multi-corner route removed one segment at a time ("Undid: Apply a routed path" ×3) (finding 33).
- **Fiducials.**
  - Placing them from the PCB was not possible. P does nothing, and typing "fiducial" fired single-key tools and switched to 3D.
  - Placed instead from the schematic palette: generator "Global fiducial Ø1.0 mm, 2.0 mm opening", 4 presets. Their labels overlap at default spacing.
  - After Update PCB they were moved to three corners. The clear-area overlay shows, and "Too few fiducials" was resolved (finding 30).
- **Counts.** The status line read "65 airwires" while Issues read "15 unrouted connections" (finding 34).

## 5. 3D (09:41–09:45)

- **Standard view** opens instantly.
  - The black mask follows the board setting.
  - The module carries Espressif's label with CE mark and QR code.
  - USB-C, header, switches, passives, LEDs and the fiducial lands are all present.
- **Realistic view** has no labelled button; it is reached through the palette ("Realistic look", ⇧R) or an unlabelled icon in the bottom bar. Resistor codes read 472 for 4.7 k and 512 for 5.1 k, correct EIA-3 codes.
- **Variant view.** With variant Lite active, D2's body disappears and its bare pads stay.

## 6. Variants (09:46)

- **Manager.** Board card › Variants › Edit… opens Assembly variants. Add variant gives "Variant 1"; it was renamed Lite and D2 unticked.
- **Active = Lite:**
  - D2 is drawn crossed and dimmed in the layout;
  - a "Lite variant" pill shows over the canvas;
  - 3D drops D2's body.
- **Substitute part.** Each part's Part dropdown offers only "Default part", because there are no Parts for generic components (finding 40).
- **Table.** It shows "8 of 21" with Show all, and its headers are truncated to "DEF…" and "IN L…".

## 7. Checks, preflight, fab output, assembly drawing (09:47–09:53)

- **Production check:** "76 errors · 22 warnings":
  - Pad holes too close to copper: 2;
  - Unrouted connections: 74;
  - warnings "U2 uses a definition with…", "Designator U1 runs outsid…" and "5 imported terminals hav…", truncated;
  - No Part chosen: 19.

  Issues shows "Unrouted connections 12" for the same board (finding 34).
- **Waiving** works by group: checkbox › Reason › "Waive N findings". The Reason field came pre-filled with the previous waiver's reason (finding 36). After waiving, the Issues badge turned to a green "No issues" (finding 37).
- **Export** wrote 19 files in about 1 s:

  | Group | Files |
  |---|---|
  | Gerber X2 | 4 copper layers, mask top and bottom, paste top, legend top, profile |
  | Drill | plated and non-plated `.drl`, drill map and table |
  | Job | `.gbrjob` |
  | BOM and placement | BOM grouped and per part, `pos-top.csv` |
  | Drawings | fabrication and assembly PDFs, A3 |
- **Files checked:**
  - The job names project, GUID, revision, ENIG and the 11-layer material stackup.
  - The fabrication PDF has a drill table, the stackup and numbered notes, one per waiver: 76 notes (finding 38).
  - In variant Lite, D2 is left out of `bom.csv` and the placement file, `bom-parts.csv` marks it "Fitted: no", and the assembly drawing reads "24 parts. 1 not fitted (crossed out)". The variant's name appears in no file (finding 39).
  - The BOM has an MPN only for J1 (findings 9, 41).
  - The assembly drawing uses a third of the A3 sheet, and 0603 designators are too small to read at that scale (finding 42).
- **Re-export into the same folder** is refused with a truncated "…exists and is not an…", and there is no Replace (finding 43).

## 8. Save, close, reopen, recovery (09:54–10:06)

- **Inventory of the saved file:**

  | Kind | Count |
  |---|---|
  | Instances and placements | 24 |
  | Wires | 58 |
  | Labels | 38 |
  | Junctions | 5 |
  | Nets | 47 |
  | Tracks | 31 |
  | Vias | 17 |
  | Fills | 2 |
  | Waivers | 76 |
  | Diff pairs | 1 |
  | Variants | 1 |
  | Drafts | 10 |

  The drafts are leftovers of the datasheet runs (finding 44).
- **Quit** from the menu gave no prompt, since the project was saved, and left Recovery empty.
- **Reopen** from Finder: the Lite variant was still active, the waivers were kept, 38 airwires remained and the project was not dirty. Re-export gave **19 of 19 files byte-identical** to the first export.
- **Recovery.**
  - After the AI routing, the app was killed (`kill -9`) with unsaved changes.
  - The next launch showed "Recover an earlier session? usb-sensor · 2 min ago · 24 instances", with Discard, Keep for later and Restore.
  - Restore brought back the unsaved AI tracks, toast "Recovered usb-sensor; save it to keep it".
  - Quitting then asked "Save changes before quitting?". Don't Save left Recovery empty.

## 9. The assistant across the run

| Turn | Requests | Tokens | Result |
|---|---|---|---|
| Datasheet workflow, run 1 | 4 | 255 k | Stopped by the limit at Pinout; orphan draft. |
| Datasheet workflow, run 2 | about 9 | about 160 k (estimate) | Done; symbol without pin names. |
| Name the module's pins | 18 | 432 k | Applies refused while the tester edited; the summary claimed the tester's edits. |
| Publish the pins and update U1 | 33 | 1 928 k | Done, but stopped by the limit. |
| Wire reset, boot, LEDs, I²C and decoupling | 10 | 791 k | **Nothing changed.** |
| Route SDA/SCL on top (after reopening) | 18 | 247 k | **Done:** 13 tracks, clean, 2-sentence summary, one undo step. |
| Explain the J1 finding, fix it, ask first | 26 | 754 k | Good ask card; then **nothing changed** and no explanation. |

- **Undo turn and redo** each act as one step ("Undid: AI assistant: On the PCB, route…"; ⇧⌘Z "Redid: …").
- **Compaction now exists:** "Condensed 6 earlier tool replies · 12 k → 6.5 k".
- **The ask card is the right shape:** "Fix the J1 peg-hole clearance? … Trim the two GND lands (recommended) / Keep the waiver / Something else", with Skip and Answer.
- No approval prompt appeared for ordinary edits. Changes are applied directly and undone per turn.

## What worked well

- **Save, reopen and recovery are trustworthy.** Re-export after reopening was byte-identical, and recovery after a hard kill restored the unsaved work.
- **The export gate holds.** It will not write files while errors stand, and every waiver goes into the fabrication drawing with its reason.
- **Variants carry through** the layout, 3D, BOM, placement file and assembly drawing.
- **3D is fast and convincing**, with correct markings.
- **Diff pairs are easy.** The pair was suggested from the net names and routes coupled in one gesture.
- **Schematic wiring by hand is quick and tidy:** automatic junctions, flag labels and a fast no-connect tool.
- **The assistant can do real layout work** when the request is concrete (SDA/SCL), and its undo and redo are clean.
- **Since run 12:** Choose… reads PDFs, the result card is readable and Settings › AI is clearer.

## Findings

Severity in brackets: **B** blocker, **P** production gap, **M** major UX, **m** minor.

**Project setup**

1. [m] **Zero rules without a profile.** With no fab profile, the Route panel shows red "0 mm" via fields with "Enter 0.1 to 6 mm" before any input.
   - Repro: New Project, Esc on the Fab profile card, then press Route.
   - Expected: fields disabled, or a note to choose a profile.
2. [P] **The 4-layer stackup preset is 1.74 mm thick** (1.2 mm core + 2 × 0.2 mm prepreg + copper), not the 1.6 mm standard. The job file and fabrication drawing say 1.74 mm, with no warning against the JLCPCB 4-layer profile.
   - Repro: Board › Stackup… › 4-layer preset.
3. [P] **The online library cannot be used.** The default address is `http://localhost:8787` (`crates/io/src/settings.rs:57`). With `https://parts.sparksmith.io` set, searches show only "Sign in to the online library … Searching works without an account" and no results.
   - Repro: Settings › Libraries, set the address, then P › "resistor".
   - Expected: the public address by default, and results without an account as the note promises.
4. [m] **Truncated Board card values:** "4 layers · …", "Black · ENI…", "JLCPCB 4-la…", "Rectan…", with no hover expansion seen.

**Library and Place palette**

5. [M] **Unconnected pins count as errors while drafting.** Each placement reports "joining no net; N new issues", and the schematic showed 88 errors before the first wire.
   - Expected: pending pins as to-dos until ERC is asked for, or until the part has been wired once.
6. [M] **"1u 0603" finds a resistor first.** "1u" is read as "Resistance = 1 µΩ or Rated power = 1 µW or 1u", and the chip resistor already in the project ranks above the capacitor.
   - Repro: place a 0603 resistor, then P › "1u 0603" › Enter.
   - Expected: µ-values without a unit to mean farads first; at least, a capacitor above a 1 µΩ resistor.
7. [P] **The module generator's ESP32-S3-WROOM-1 symbol has 41 unnamed passive pins** ("1" … "41"), although the generator cites Espressif's datasheet.
   - Expected: named pins with electrical types, as for the shipped USB4105.
8. [m] **Inconsistent palette rows.** Generator rows have no preview, and the shipped USB4105 row shows "Type to find a component" in the side card. Enter places the shipped part at once but attaches a generator part to the pointer.
9. [P] **No orderable Parts for generic parts.** Resolve parts says "12 of 13 groups unresolved"; Choose… for 1 µF 0603 shows "Fitting parts 0 — No part of this component meets these values yet". Every passive, LED, switch, header, the LDO and the module lead to "No part chosen" to-dos and empty MPNs in the BOM.
10. [m] **Batch placement too tight.** R1 and R2 placed by batch clicks touch pin to pin.
11. [m] **Mirror flips vertically.** A symbol facing right needs Rotation 180° plus Mirror.
12. [m] **Fiducial labels overlap** when the three are placed one after another on the schematic.

**Schematic editing**

13. [m] **Clicking inside a symbol body does not select it.** Only the outline and pins pick it, while a nearby value label picks its own part. Drag aimed at C1 grabbed C6 by its label.
14. [m] **The Power tool keeps the last style.** The first 3V3 port was drawn as a ground symbol. A label placed before its name was typed became "NET" silently.
15. [m] **Close-packed ports.** GND and 3V3 ports on adjacent module pins 1–2 stack tightly, and the GND ports on the module's left pins draw sideways.

**Datasheet workflow and assistant**

16. [P] **The workflow's LDO has no pin names.** It shows pins 1–5 only, where run 12 produced IN, GND, EN, NC and OUT. The run's Pinout step reported done.
    - Repro: `/` › Datasheet → component with `tlv757p.pdf` and "TLV75733PDBVR", Continue, Apply, then place the result. (`f-ldo-symbol-no-pin-names.png`)
17. [B] **The workflow stops at the default limit and cannot continue.** Run 1 stopped at "Pinout · 5 of 14", 255 k of 350 k tokens, after 4 requests: about 64 k tokens a request, the whole datasheet each time.
    - The run block has no end card, no Undo run and no Continue.
    - The header reads "Ready".
    - A half-made "TLV75733PDBVR" draft stays in the project and the palette, next to the later complete one, with nothing to tell them apart. (`f-workflow-limit-no-end-card.png`)
18. [M] **The chat and the person cannot work at the same time.**
    - While the tester placed U2 and moved parts, every chat apply was refused: "the project had already moved on, from revision 50 to 63, then 64, then 66. Something else is editing it right now".
    - The turn's change summary then listed the tester's edits ("Parts: U2 added; Nets: 5 added; Occurrences 1 added, 5 changed") as its own, so Undo turn would revert them.
    - Expected: the chat's own operations rebased onto the person's edits, and the summary showing only what the chat did. (`f-chat-turn-claims-user-edit.png`, `f-chat-apply-refused-revision-moved.jpg`)
19. [P] **Naming 41 pins took 33 requests and 1.93 M tokens**, about 8 USD. It ended "limit reached" although the change was made, and the header read "Stopped early". (`f-chat-pin-rename-1.9M-tokens.jpg`)
20. [M] **The assistant column did not come back.** After closing it with X while docked left and a run was going, ⇧⌘A, View › Show Assistant and the toolbar button each failed to reopen it; an accessibility press on the button finally did. This was reproduced once only.
21. [m] **The dock button reads as settings.** It is still a sliders icon ("Dock the assistant left"), as in run 12 #4.
22. [B] **The chat could not wire the schematic.** Asked to wire the reset, boot, LED, I²C and decoupling sub-circuits with real wires, it called `query project.inspect` 8 times (about 80–100 k tokens each). `show assist.ask` was then refused twice, and the 800 k limit ended the turn: "Ended early · nothing changed". (`f-chat-wiring-nothing-changed-791k.png`)
23. [m] **Wrong meters.** The footer timer shows "0 s" or "60 s" during turns. The "Undo turn" button stays disabled after a redo.
24. [M] **No explanation, then no fix.** Asked to "explain in plain words … and fix it … ask me before you change anything", the chat asked a good question. After the answer it spent 26 requests and 754 k tokens on footprint-editing lookups, and ended "nothing changed" without the explanation. (`ok-chat-ask-card.png`, `f-chat-footprint-fix-limit-nothing-changed.png`)
25. [m] **The `/` menu picked another workflow.** Down then Enter chose "Datasheet → 3D body" on the second use, although the same keys had chosen "Datasheet → component" before.

**Layout**

26. [m] **Settings toggles silently** through the menu and opens behind the command palette.
27. [m] **Position fields have no X/Y captions**, and Y counts up from the bottom edge, which is not shown.
28. [m] **LED pads are numbered "1/2" on the canvas** but the nets say "D1 A" and "K", so the anode cannot be found on the canvas.
29. [P] **The shipped USB4105 fails the shipped profile:** "J1 pad 17: hole is 0.175 mm from J1 pad A1B12 on GND; needs 0.2 mm (JLCPCB 4-layer: pad to unplated hole clearance)", and the same for pad 18. Open since run 11.
30. [M] **No place-part entry in the PCB workspace.** P does nothing there, and typing a search word fires single-key tools: "fiducial" switched to 3D. Fiducials, a board-only part, had to be placed on the schematic and pushed with Update PCB.
31. [P] **No impedance warning.** The USB pair routed at "gap 0.09 mm · 83.1 Ω" against the 90 Ω USB target.
32. [P] **Push-and-shove did not move the pair.** A 3V3 track routed onto the coupled USB pair stopped at it, with no shove and no message, and left a stub ("Track ends joining nothing").
    - Repro: route a coupled pair, then route another net across it with the default router mode.
33. [m] **One route is several undo steps.** Cmd+Z after a multi-corner route removes one segment at a time. A click on a pad of another net silently becomes a corner.
34. [M] **Three counts for unrouted copper.**
    - The status line said "65 airwires" while Issues said "15 unrouted connections".
    - The production check said "Unrouted connections 74" while Issues said "Unrouted connections 12".
    - Issues rows read "USB_DN: 3 pads unrouted" when 2 of the 3 pads were joined. (`f-production-check-74-vs-issues-12.png`)
35. [m] **No route-to-via key.** Pressing 2 or V while routing does not drop a via. The Via tool on a pad works.

**Production**

36. [m] **The waiver Reason field is pre-filled** with the previous waiver's reason.
37. [P] **A waived unrouted board looks finished.** After waiving the unrouted connections, the Issues badge and panel show a green "No issues" with 38 airwires left. Export then writes a complete-looking job.
    - Expected: unrouted waivers stay visible as a count, and are never shown as "No issues".
38. [m] **One note per waived pair.** The fabrication drawing gets a numbered note for each waived pad pair, 74 notes for the unrouted connections.
39. [m] **The variant name is missing from the outputs.** Lite's effect is correct, but neither the files, the drawings nor the folder name say which variant they are.
40. [P] **No substitute parts in variants.** The Part dropdown offers only "Default part" (follows from 9).
41. [P] **Empty BOM fields for the generated ICs.** U1 and U2 have no Value, Manufacturer or MPN; only the package is filled.
42. [m] **The assembly drawing is hard to read.** The board fills about a third of the A3 sheet, and 0603 designators are unreadable at that scale. It has no BOM table.
43. [m] **Re-export into the same folder is refused** with a truncated message ("…exists and is not an…") and no Replace.
44. [m] **Leftover drafts.** Ten drafts from the datasheet runs stay in the saved project after publishing.

## Speed

| Operation | Time |
|---|---|
| Launch, Update PCB, apply, fills, refill | Instant |
| 3D Standard and Realistic, variant switch | Instant |
| Export of 19 files | About 1 s |
| Reopen with 24 parts (from Finder) | A few seconds (not timed) |
| Datasheet workflow, run 2 | About 7 min (with gates) |
| Chat: route SDA/SCL | 2 min, 18 requests |
| Chat: name and publish module pins | 3 + 3 min, 51 requests |
| Chat: wire sub-circuits (failed) | 45 s, 10 requests |
| Chat: J1 fix (failed) | 2 min, 26 requests |
| Whole run, hands-on | About 50 min (tester waits on chat turns excluded) |

## Cost

The app shows tokens, not prices. The estimate uses the rate in run 12 (Opus 5.5: 4 USD a million input tokens, 20 USD a million output tokens). It assumes no prompt caching, so it is an upper bound.

| Part | Tokens (app's count) | Estimate |
|---|---|---|
| Datasheet workflow, 2 runs | about 0.42 M | about 1.8 USD |
| Module pins, 2 turns | 2.36 M | about 9.6 USD |
| Schematic wiring (failed) | 0.79 M | about 3.2 USD |
| SDA/SCL routing | 0.25 M | about 1.0 USD |
| J1 explain and fix (failed) | 0.75 M | about 3.1 USD |
| Output tokens, all turns | about 40 k | about 0.8 USD |
| **Total** | **about 4.6 M** | **about 19 USD**, under the 45 USD stop |

Check the Anthropic console for the exact figure. AI use was stopped after the J1 turn: the remaining scenes needed no model.

## Clean-up

- **Guest app quit.** The second instance was quit with Don't Save; Recovery is empty. No Sparksmith process is running in the guest.
- **Guest files kept:**
  - `~/usb-sensor.sparksmith`, the saved test board;
  - `~/Documents/e2e13/`: the two datasheets, `fab/` and `fab-first/`.
- **Shared folder:** `results/e2e-night/` holds:
  - the finding screenshots (`f-*`) and reference screenshots (`ok-*`);
  - `fab/usb-sensor-production/`, the first export;
  - `usb-sensor-before-reopen.sparksmith`;
  - `ds/`, the public TI datasheets.
- **Settings changed in the guest:**
  - Settings › Libraries online address `https://parts.sparksmith.io`;
  - Settings › AI run limits raised to 800 000 tokens and 50 requests;
  - the Anthropic key stays in the keychain, with "Always Allow" granted to this build's signature.
- **Host:**
  - the worktree is under the session scratchpad (`e2e13-wt`), with the bundle and notes in `e2e13/`;
  - the target directory is `target/e2e13`, 4.1 GB.

  Neither was removed.

## Recheck of the assistant fixes (22 November 2026, afternoon)

**Setup.**
- Build: `main` at **ac116c6e** ("Merge main (8cba819a); open-drain pin kinds, sole part number fitted at placement"), release build, bundled and deployed as above.
- Limits: the guest's AI run limits were reset to the shipped defaults (40 requests, 200 000 output tokens, 4 000 000 tokens, 45 min); the run 13 settings were backed up as `settings.json.bak-run13`.
- Keychain: the new ad-hoc signature again made macOS ask for the login password on first AI use.
- Project: new, unsaved. It holds U1 (ESP32-S3-WROOM-1 from the generator), R3 10 k, C3 1 µF, SW1 (TL3305A) and D1–D2 green 0603 LEDs.

**Results so far.**

| Check | Result |
|---|---|
| Module pin names | **Fixed.** The generator's ESP32-S3-WROOM-1 now has named pins (GND, 3V3, EN, IO4 … IO0, TXD0, RXD0 …), and Pin functions shows 41. |
| Wire the reset circuit (exact prompt) | **Fixed.** See below. |
| Edit while a turn runs | **Fixed.** See below. |
| Explain and fix | **Fixed.** See below. |

**Wire the reset circuit.** The exact prompt was "Wire the reset circuit on the schematic: R3 10k from EN to 3V3, C3 1u from EN to GND, and SW1 from EN to GND. Use real wires."
- The turn ran `query schematic.pins`, stated a plan and applied once.
- Result: "Done · 1 undo step", with Wires: 6 added; Nets N1, N2, N4 changed; 6 nets removed; Issues: 0 new, 9 resolved.
- Cost: **4 requests, 31 k tokens, 18 s** (22 s wall time). Run 13 had needed 10 requests and 791 k tokens and changed nothing.
- The wires were correct: R3 from EN to 3V3, and C3 and SW1 from EN to GND.
- **Undo turn removed all 6 wires in one step**, and redo restored them.

**Edit while a turn runs.** The prompt asked to wire D1/D2 anodes to IO4/IO5 and the cathodes to GND. While the turn ran, the tester moved D2, C3 and R3 and renamed D2's value.
- One apply was refused for a real geometric reason: "wire from D2 pin A to U1 IO5 would run over U1 pin TXD0".
- The chat then asked a good question (net labels or wires; series resistors), did a dry run and applied.
- Result: "Done · 1 undo step", with Labels 7 added and Wires 7 added.
- The summary has a separate **"Changed outside this turn"** block listing the tester's edits: Wires 1 added, Nets N1 changed, Occurrences 3 changed, Wires 4 changed, Nets N4 removed.
- Cost: 19 requests, 378 k tokens, about 2 min.
- **New product finding (Major).** Moving C3 and R3 by their Position fields dragged their attached wires so that **EN merged into GND, with no warning and no new issue**. The chat noticed it, named the revisions (19–21) and explained the cause. (`r-concurrent-changed-outside.png`)
- **Minor.** The question card appeared below the visible transcript. The panel did not scroll to it; only the header's "Waiting for you" showed it.

**Explain and fix.** The prompt was "EN is now shorted to GND. Explain in plain words what that finding means for the board, and fix it properly. Ask me before you change anything."
- **The explanation came first:**
  - EN is the "on" pin, so the chip is held in reset and never boots;
  - R3 wastes about 0.33 mA;
  - C3 and SW1 do nothing;
  - nothing is damaged.
- It then found the wires, ran a dry run and asked "Fix the EN–GND short? Delete C3.1→U1 GND and SW1.2→C3.1 wires, and wire SW1.2 to C3.2 (GND)", with Apply the fix and Change something.
- After Apply: "Done · 1 undo step". It read the pins back, confirmed EN is its own net, and listed the 36 remaining errors (unused U1 pins).
- Cost: 11 requests, 389 k tokens, 53 s of model time.
- One `describe wire.remove` was refused (no such verb) before it found `schematic.disconnect`. (`r-explain-fix-done.png`)

**Datasheet workflow with the shipped limits:** running (started 16:35).

**Spend so far:** about 0.80 M tokens, about 3.3 USD (upper bound, uncached).
