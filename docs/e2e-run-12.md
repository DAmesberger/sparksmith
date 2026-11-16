# Twelfth end-to-end run: the assistant chat and the datasheet workflow in the chat

On 16 November 2026 the lead agent tested the assistant chat (slices C1–C3) and the datasheet workflow in the chat (slice W2) in the macOS guest VM.

The run was planned as demo recording. The founder changed it to a **test run** part-way through: the website takes will be recorded again once the redesigned Settings › AI and cards are built. Phase B therefore looked for defects with the real model, under a 10 USD cap, and no clips were cut.

**Setup.**
- Build: `main` at df304165, built with the `ci` profile as `Sparksmith.app`, bundled as in run 11:
  - `library/` in `Contents/Resources/library`;
  - nix `libiconv`/`libcharset` in `Contents/Frameworks`;
  - the old guest bundle deleted before the deploy.
- Guest display: 1920 × 1080 at scale 1.0.
- Recording: the run 11 helpers (ScreenRec, Puppet).
- Datasheet: TI's public TLV757P datasheet (SBVS322C, 38 pages, 2.5 MB).
- Model: the Anthropic endpoint preset with `claude-opus-5-5`. The founder entered the key himself in Settings › AI.

## Outcome

| | Result |
|---|---|
| Opening the chat | **Works** from ⇧⌘A, View › Show Assistant, the application bar, the palette, the `/` menu, the Component start page and a PDF drop. |
| Old "Create from datasheet…" dialog | **Gone** from the File menu, the palette and the New gallery. The start-page button opens the chat. |
| Settings › AI | **Hard to use**: it was unclear where the API key goes (finding 1). A redesign is being built. |
| Stop while a card waits, at 1024 × 640 | **Works.** The W2 agent's report is not reproduced. |
| Datasheet workflow, first run (dropped PDF) | **Completed** end to end: Inputs → generator card → impact card Apply → result card → Open in editor → Undo run. 9 requests, 12 k output tokens, 3 min. |
| Undo run, redo | **Work**: one step each way. |
| Datasheet workflow, second run (PDF chosen with Choose…) | **Stuck** at "Reading…" (finding 9). With the PDF dropped instead, it completed like the first run. |
| Result card | Shows **raw JSON** and the escaped Markdown of the review (finding 5). |
| The chat building a small board | **Failed.** It made two passive components and placed U1 on the schematic, outside the sheet frame. Nothing reached the board, and the token limit stopped both turns (findings 12–15). |
| The chat pointing at things (`view.select`) | **Refused**: "no part, track, via, fill, keepout or wire has id 20" (finding 14). |
| Cost | Estimated at **about 5–9 USD**. The app shows no prices, so this is not measured; see [Cost](#cost). |
| Crashes | None. |

## A. Chat panel and entry points (no model calls)

- **Opening and closing.**
  - ⇧⌘A toggles the column.
  - View › Show Assistant (listed with ⇧⌘A) opens it.
  - The app-bar button opens it, and X closes it.
- **Folding, resizing and docking.**
  - The fold button folds it to a narrow strip with a status dot, and a click unfolds it.
  - Dragging its edge resizes it up to about 520 pt.
  - The header button docks it left or right.
- **`/` menu.** Typing `/` in the composer lists "Datasheet → component". Enter adds the workflow card:
  - "I can do this with a workflow";
  - "Datasheet → component v1.0" with the summary;
  - "It will ask for: Datasheets, Part name";
  - Just chat and Use it.

  Just chat folds the card to its title, marked "Just chat".
- **Palette.** "datasheet" finds one command, "Assistant: Datasheet → component", which adds the card.
- **Component start page.** "Create from datasheet…" opens the chat on the card, also when the chat was closed.
- **PDF drop.** Dropping the PDF from Finder on the canvas opens the chat, also when it was closed. The card reads:
  - "You dropped tlv757p.pdf."
  - "Datasheets tlv757p.pdf (38 pages · 2.5 MB)"
  - "It will ask for: Part name"
- **Without a key.** A message ends with the banner "failed: configuration: ANTHROPIC_API_KEY is not set" and the card "Ended early · nothing changed / Failed: configuration: ANTHROPIC_API_KEY is not set. / 0 requests · 0 tokens · 0 s". Use it ends the same way (finding 3).

## B. The datasheet workflow in the chat (real model)

Run 1, from the dropped PDF in full screen:

1. **Use it.**
   - The run block lists 14 steps: Inputs, Read, Generators, Package, Generator, Made, Drafts, Pinout, Preview, Import, Check, Report, Review, Done.
   - The footer reads "Inputs · 0 of 14" with Stop.
   - The input card asks for Part name, "Pages to read first" (optional) and Notes (optional), with Decline and Continue.
   - "TLV75733PDBVR" and the note "DBV package (SOT-23-5), fixed 3.3 V output" were typed, then Continue.
2. **Read and Generators** finished at once. Then "Package · 3 of 14" ran.
3. **Generator card** after about 45 s:
   - the meter read "2 of 40 requests · 2075 of 200 k tokens · 0 of 45 min";
   - the card is titled "Generator / Generator" and lists the generators as checkboxes (finding 6);
   - the model's choice, "Tabbed small-outline and power packages", is checked but scrolled out of view;
   - Change and Continue. Continue was pressed.
4. **Made, Drafts, Pinout and Preview** ran. About 50 s later the impact card read "Import / 4 function added · 5 function removed · 5 terminal changed / Findings: none", with Decline and Apply (finding 7). Apply was pressed.
5. **Check, Report, Review and Done** took about 60 s. The block ended "9 of 40 requests · 12 k of 200 k tokens · 3 of 45 min / Done".
6. **Result card** "Datasheet → component: TLV75733PDBVR":
   - Drafts, Findings and Review as raw JSON (finding 5);
   - Undo run and Open in editor buttons.
7. **Open in editor** showed the TLV75733PDBVR component, r1 draft:
   - symbol: IN 1, GND 2, EN 3 on the left; OUT 5, NC 4 on the right;
   - 3D tab: the SOT-23-5 model ("SOT-23 and SC-70 package", `tabbed-rows.solidcraft`) with pitch 0.95 mm, body 2.9 × 1.6 × 1.1 mm, standoff 0.075 mm, span 2.8 mm, lead 0.4 × 0.15 mm and foot 0.4 mm;
   - Issues 2: pinout unreviewed and footprint polarity missing.

   The review report cites pages 3, 6, 22, 25 and 31–33 for every value, including "land size | 1.1 mm x 0.6 mm (5X (1.1), 5X (0.6)) | DS p.32 Example Board Layout".
8. **Undo run** gave the toast "Undid: Datasheet → component: TLV75733PDBVR · Component". The component left the project, and the card kept only Open in editor. The toolbar's redo gave "Redid: …", and undo removed it again.

Run 2 ("redo by running again"):
- `/` → Use it showed a Datasheets row as well (the drop of run 1 is not reused), with "Drop PDFs here" and Choose….
- Choose… with the PDF stayed at "tlv757p.pdf Reading…" for over a minute, twice (finding 9).
- Stop ended that run.
- Dropping the PDF again gave a new card, and Use it ran the same way as run 1: generator card after 2 requests · 2264 tokens, impact card "4 function added · 5 function removed · 5 terminal changed", result after Apply.
- Run 2's review has the same values as run 1's (DBV0005A, 0.95 mm pitch, 1.1 × 0.6 mm lands, 2.6 mm row span).

**Stop at 1024 × 640.** In a 1024 × 640 window, Use it was pressed and the input card left waiting. The run block's Stop, in the block's footer next to "Inputs · 0 of 14", ended the run:
- the block read "Stopped; what completed stays as one undo step";
- the input card read "Ended";
- the header read "Stopped".

In that window the chat column kept 520 pt, and the run block and input card needed scrolling to reach Stop.

## C. The chat on a board (real model)

**Setup.**
- New project, PCB workspace, fab profile JLCPCB 2-layer.
- Before these messages, Settings › AI › Run limits was turned on to stay within budget: 30 model requests and 600 000 tokens, later 350 000.

**Message 1.**

> Build a small 3.3 V supply: put the TLV75733PDBVR on the schematic as U1 with a 1 uF 0603 capacitor from IN to GND and one from OUT to GND, and a 10k 0603 pull-up from EN to IN. Then update the PCB, place the parts close together on the board and route every net.

- **Calls.** The tool rows were:
  - `query project.inspect`, `find place a component on the schematic`, `describe component.place`;
  - `query library.search` twice **refused**, then four times done;
  - `find create a generic resistor or capacitor chip component (06…`, `describe library.generate_part`, `find list library folders or standard parts available`;
  - `query library.generators`, `query library.rescan`, `apply library.generate_part`;
  - `query project.inspect` refused once, then several times done.
- **End.** It ended with the banner "limit reached: 600000 tokens: 570860 used, and the next request could take 96934" and the card "Ended early · 1 undo step / Limit reached: 600000 tokens: 570860 used, and the next request could take 96934. / Assets: 3 added / Bindings: 2 added / Components: 2 added / Footprints: 2 added / Models: 2 added / Packages: 2 added / Symbols: 2 added / Project settings changed / 14 requests · 570 k tokens · 36 s".
- **Result.** Two components, "Chip capacitor 1.60 × 0.…" and "Chip resistor 1.60 × 0.8…", were in the project. Nothing was on the schematic or the board.

**Message 2.**

> Place U1 (TLV75733PDBVR) on schematic sheet 1 and select it so I can see where it is. Do nothing else.

- **Calls.** `apply component.place` was done. `show view.select {"ids": [20]}` was **refused**.
- **End.** "Ended early · 1 undo step / Limit reached: 350000 tokens: 282428 used, and the next request could take 97260. / Parts: U1 added / Nets: 5 added / Occurrences: 1 added / 2 requests · 282 k tokens · 5 s".
- **Result.** U1 is at (100, 80) mm, above the A4 sheet frame (finding 15). Its card shows "None · unresolved", "6" errors and "1" to-do.

The board scene stopped here: further turns would cost about 1 USD each, and the 10 USD cap was close.

## What worked well

- **Every entry point opens the same chat and card.** The old dialog is gone, and the drop works on any workspace with the chat closed.
- **The workflow runs end to end in about 3 minutes** with 9 requests. The deterministic steps are instant, the gates wait for the person, and the run block shows each step as it goes.
- **The drafted part is right**: pins, the SOT-23-5 package from the generator with the datasheet's land pattern, a 3D model and a cited review, all in the open project.
- **Undo run, redo and Stop** each act as one step and say so.
- **Run limits hold.** Both chat turns stopped before the request that would have passed the limit, with the numbers.

## Findings

1. **Settings › AI does not say where the API key goes.** The founder could not find it either; a "Connect a model" redesign is being built.
   - **Layout.** AI is one section of the general Settings sheet (about 390 pt wide), below Rules and Production.
   - **The endpoint form.** Add endpoint › "Choose a preset…" › Anthropic opens "NEW ENDPOINT" with Name, Provider "Anthropic Messages", Base URL, Model, Timeout "30 min", Key "None stored" and New key "Type to store or replace". The word "API key" appears nowhere, and the key field is the sixth of seven rows.
   - **No connection chosen.** After Save, "Assistant uses" stays "None" although Anthropic is the only connection; the chat then cannot run until it is chosen.
   - **Truncated row.** The saved row reads "Anthropic — claude-opus-5-5 · another computer, o…".
   - **One long scroll.** Permissions, Run limits, Profiles ("claude · built in · for claude-opus-5-5, clau…", "default · built in", each with Copy to edit) and Workflows ("Datasheet → component v1.0 · Shipped · available") follow on the same page.
2. **"Ready" without a model.** The chat's header shows a green dot and "Ready" with no connection chosen and no key stored.
3. **The missing-key error names an environment variable.** "failed: configuration: ANTHROPIC_API_KEY is not set" does not point to Settings › AI, and this banner (like every banner here) stays pinned above the transcript after later turns.
4. **The docked-left header is unclear.** The fold button is a down chevron ("Fold the assistant"), and the dock button is a sliders icon ("Dock the assistant right") that reads as settings.
5. **The run block and result card show raw data.**
   - **Step rows.** The rows show the result's keys or text, not a summary: "Read [{"document":"tlv757p.pdf","text":"=== page 1 ===\nTLV757P 1A, Low IQ, Small-Siz", "Package generator, params, preset, why", "Made can_redo, can_undo, changes, outcome", "Review accepted, markdown, next, pins".
   - **The result card.** Its items are JSON:
     - "Drafts {"ids":[2,3,6,4,5],"outcome":"drafts_created"}";
     - Findings, a 20-line object with `"code":"pinout.unreviewed"` and `"code":"footprint.polarity_missing"`;
     - Review, `{"accepted":true,"markdown":"# Datasheet import review: …` with `\n` and table pipes.
   - **Missing buttons.** There is no Show or Save review…, which the design's card has (assistant-panel.md §7).
6. **The generator gate looks like nothing is chosen.**
   - It is titled "Generator" twice and lists every generator as a checkbox (multiple-choice look for a single choice).
   - The visible rows were "Chip passives and LEDs" and "Grid arrays (BGA, LGA, WLCSP)", both unchecked. The model's choice ("Tabbed small-outline and power packages", checked) was only found by scrolling the list.
   - The card does not state the preset or why, although the plan's `why` exists.
   - Change was disabled while the card waited.
7. **The impact card's counts read oddly.** "4 function added · 5 function removed · 5 terminal changed / Findings: none" on a brand-new draft: singular nouns after plural counts, and "removed" functions the person never had. The design's "leave out rows…" choice was not shown.
8. **"Pages to read first" shows "1-5, 31" as if filled in.** It is a hint: the Given summary then reads "Pages to read first: —". It looks like a value the run will use.
9. **A datasheet chosen with Choose… never finishes reading.**
   - In the input card of a run started from `/`, Choose… › `tlv757p.pdf` showed "tlv757p.pdf Reading…" for over a minute.
   - Continue then refused with "still reading the documents".
   - Remove and choosing again did the same.
   - The same file dropped on the window read at once ("38 pages · 2.5 MB").
10. **Old cards stay live.** Two drops leave two identical offer cards, both with Use it. A finished run's card keeps "Open in editor" after Undo run, when its drafts no longer exist.
11. **Two meters disagree during a run.** The composer footer read "0 requests · 0 tokens · 0 s" while the run block counted "7 of 40 requests · 5047 of 200 k tokens".
12. **Board chat turns grow to about 100–140 k tokens a request.**
    - Message 1's 14 requests totalled 570 k tokens, and the next would have taken 96 934.
    - Message 2, a two-call request, took 282 k tokens in 2 requests, about 140 k each.
    - The first request of message 1 was about 13 k tokens. The size probably comes from the bulky tool replies (`project.inspect`, `library.search`) and the long workflow result kept in the conversation; this was not inspected.
    - At this size a short message costs about 1 USD, and compaction (K1) is not built.
    - The card's "36 s" and "5 s" are much shorter than the observed wall time, about 2.5 min and 1.5 min.
13. **The model mis-calls the verbs and then searches the library for generic parts.** `library.search {"text": "capacitor 0603"}` was refused twice: "unknown field `text`, expected one of `query`, `limit`, `include_drafts`". It then generated new chip-capacitor and chip-resistor components instead of placing from the Place palette's generated passives, and spent the turn there. It never reached the schematic wiring, Update PCB, placement or routing.
14. **`view.select` refuses the id the model just used.** After `component.place` it called `show view.select {"ids": [20]}` and got `view.unknown_object`: "Refused view.select: no part, track, via, fill, keepout or wire has id 20. Nothing was selected. Instead: Read the ids with query (layout.objects, project.inspect), or name a net, pad or sheet in objects." The placed schematic occurrence is not selectable by the id the place call returns, or the model passed the wrong one; the chat could not point at its own result.
15. **The chat places U1 outside the sheet.** `component.place` put U1 at (100, 80) mm, above the A4 frame of Sheet 1, where the person would not see it at the sheet's fit.
16. **The Browser panel disappears in narrower windows.** After a drop in a 1350 or 1024 wide window the Browser column was not shown; at 1700 it was.

## Speed

| Operation | Time |
|---|---|
| Opening the chat, the `/` menu, the palette entry, the PDF drop card | instant |
| Use it → input card | instant |
| Continue → generator card (Read, Generators, Package) | about 45 s, 2 requests |
| Continue → impact card (Made, Drafts, Pinout, Preview) | about 50 s |
| Apply → result card (Import, Check, Report, Review) | about 60–70 s |
| Whole workflow run | 3 min, 9 requests, 12 k output tokens (both runs) |
| Undo run, redo | instant |
| Board message 1 (stopped by the limit) | about 2.5 min, 14 requests |
| Board message 2 (stopped by the limit) | about 1.5 min, 2 requests |

## Cost

The app shows tokens, not prices. Opus 5.5 is 4 USD a million input tokens and 20 USD a million output tokens.

| Part | Counted in the app | Estimate |
|---|---|---|
| Datasheet runs (2) | 9 requests and 12 k output tokens each; input not shown | about 1.5–3 USD |
| Board chat (2 messages) | 852 k tokens, almost all prompt | about 2–3.5 USD (less if cached) |
| No-key attempts in phase A | 0 requests | 0 |
| **Total** | | **about 5–9 USD**, under the 10 USD cap |

Check the Anthropic console for the exact figure. The founder asked that the run stop before 10 USD; the board scene was ended early for that reason.

## Media

Under `~/dev/sparksmith-media/run12/` (not in the repository). No clips were cut, because this became a test run:
- `raw-videos/sceneA-chat-panel.mp4` (162 s): the entry points, fold, resize, dock, Settings › AI, the no-key error, the drop and the 1024 × 640 window.
- `raw-videos/sceneB1-datasheet-and-board-test.mp4` (213 s): the datasheet run with its cards, Open in editor and Undo run, and the board messages.
- `phaseA-notes.md`.
- `tools/`: the helper scripts used.

## Clean-up

- **Guest app.** Sparksmith was quit with Don't Save, and its Recovery folder is empty.
- **Guest files.** `~/Documents/e2e12` (the datasheet), the frame folders and the recording script were deleted.
- **Shared folder.** `results/run12/` (about 5.6 GB of frames and captures) and `artifacts/run12-ds/` were deleted.
- **API key: still stored.** The founder's Anthropic API key remains in the guest's keychain for `https://api.anthropic.com`. It was not removed, as instructed; it is removed with Settings › AI › Edit › "Remove key" when wanted.
- **Settings left changed.** The guest's Settings › AI keeps:
  - the Anthropic endpoint, chosen as "Assistant uses";
  - Run limits switched on at 30 requests, 200 000 output tokens, 350 000 tokens and 45 min.
- **Helper apps kept.** ScreenRec and Puppet stay installed with their grants; Puppet is still running.
- **Finder.** A Finder window may remain open on the guest desktop.
- **Display.** Left at 1920 × 1080, scale 1.0.
