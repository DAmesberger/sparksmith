# Datasheet assistant: first MCP runs after part generation

15 November 2026. Three live runs of the [`datasheet_to_component`](../skills/sparksmith-datasheet-part/SKILL.md) workflow through `sparksmith mcp`, after the D2 and D3 slices of part generation. Every package family is now a WASM generator, so the workflow runs `library.generators`, `library.generate_part`, `library.draft_closure`, the pinout onto the drafts, and then the review. A fresh headless Claude Code did the work; no person or developer agent intervened. The runs are compared with the hand-made references, as [run 1](datasheet-assistant-run1.md) was.

The recordings and projects are in [`examples/tlv75733-mcp-1/`](../examples/tlv75733-mcp-1/), [`examples/esp32-s31-wroom-3/run-mcp-1/`](../examples/esp32-s31-wroom-3/run-mcp-1/) and [`examples/opa2333-mcp-1/`](../examples/opa2333-mcp-1/). Each folder holds the saved project, the review Markdown the model wrote, and `transcript.jsonl`. The transcript is Claude Code's `stream-json` output with page images, thinking signatures and the duplicated result fields removed, tool results over 4 000 characters cut to their first 1 500, and local paths replaced with `<run>`, `<repo>`, `<home>` and `<tool-results>`. The tool inputs, which are the model's output, are complete.

## Method

- **Build.** `sparksmith-cli` at `c5628bd0`, built with `cargo build --profile ci -p sparksmith-cli`.
- **Server.** `sparksmith mcp --permission-mode allow-all --allow-folder <run>` started with an empty project, with paths limited to the run folder. `allow-all` was needed only so the model could save the project, because `project.save` is in the ask class ([defect T10](#tooling-defects)). The model was told not to publish, and did not.
- **Client.** `claude -p` (Claude Code 2.1.325) with these options:
  - `--model claude-opus-5-5 --effort high`;
  - `--restricted --tools Read Write --strict-mcp-config --mcp-config …`;
  - `--allowedTools mcp__sparksmith Read Write --permission-mode dontAsk`;
  - `--max-budget-usd` (8, 17 and 8 for the three runs) and `--output-format stream-json`.

  The model had no shell, no web access and no user settings. It read the PDFs only with `Read` (page images and text) and could write files only inside the run folder.
- **Task.** The server's own `prompts/get datasheet_to_component` text, with these arguments:
  - `part` and `datasheets` (the files, given relative to the run folder);
  - `notes` naming the package variant, with one sentence saying where to save the project and the review Markdown once `library.review_check` accepts.
- **Datasheets.** All are public manufacturer PDFs, downloaded into a scratch folder:
  - TI TLV757P, SBVS322C, revised March 2024: 38 pages. The DBV0005A drawing is on p.31 and its land pattern on p.32; the earlier copy had 41 pages with them on pp.34–35;
  - Espressif ESP32-S31-WROOM-3 & -3U datasheet v0.7 (73 pages) and ESP32-S31 series datasheet v0.5 (110 pages), the same revisions run 1 used;
  - TI OPA333/OPA2333, SBOS351E, revised December 2015: 49 pages, D0008A on pp.46–47.
- **Comparison.** Scripts compared the drafts in the saved projects:
  - footprint pads by number (position, size, rotation, shape);
  - package model values;
  - pin functions normalised as run 1 did: the key is (table, code, direction), labels are compared case-insensitively without a `_PAD` suffix, and the reference's invented codes are mapped (`LP_Fn` to LP IO MUX Fn, `ANALOG` to analog).

  `sparksmith review-diff` was run against the WROOM-3 reference. Every disagreement was checked in the PDFs. Page numbers below are PDF pages.

## Outcome

| | TLV75733PDBVR (tabbed, SOT-23-5) | ESP32-S31-WROOM-3 (module) | OPA2333AIDR (gull-wing, SOIC-8) |
|---|---|---|---|
| Result | Drafts and review, accepted on the 1st `review_check` | Drafts and review, accepted on the 1st `review_check` | Drafts and review, accepted on the 1st `review_check` |
| Generator path | `tabbed` with the `tabbed-sot-23-5` preset, every drawing value and TI's land given (`lands: explicit`) | `module` with the `module-esp32-s31-wroom-3` preset, every value restated from Figs. 10-1 and 11-1 | `gull-wing` with drawing midpoints. The estimated lands were replaced with TI's land pattern by `library.footprint_from_pads` |
| Pinout import | **Blocked** ([T1](#tooling-defects)); entered with `library.edit` | **Blocked** (T1); entered with `library.edit` | **Blocked** (T1); entered with `library.edit` |
| Footprint against the source | 0 nm against the dev board's TLV75733 (TI DBV0005A land) | 0 nm against the reference, 107 of 107 pads | 0 nm against TI's D0008A land (p.47: 8 × 0.6 × 1.55 mm, 1.27 mm pitch, 5.4 mm between row centres) |
| Pins | 5 of 5 right (Table 4-1 p.3) | 99 of 99 terminals, 305 functions (reference: 305) | 8 of 8 right (Pin Functions p.4) |
| Published | No | No | No |
| Unreviewed / uncertain pins | 5 / 0 | 99 / 0 | 8 / 0 |
| Remaining findings | `pinout.unreviewed` | `pinout.unreviewed`, `footprint.polarity_missing` | `pinout.unreviewed` |
| API requests (turns) | 33 (39) | 48 (61) | 37 (44) |
| Refused tool calls | 8 | 9 | 4 |
| Wall time | 255 s | 1 911 s | 312 s |
| Output tokens (of which thinking) | 23 914 (5 538) | 265 858 (41 493) | 32 194 (9 552) |
| Cache writes / cache reads | 176 532 / 3 734 556 | 505 014 / 11 061 062 | 213 512 / 4 902 100 |
| Cost (`total_cost_usd`) | **2.6377112** | **11.5698684** | **3.3326920** |

## Comparison with the references

### TLV75733PDBVR

The reference is the dev board's TLV75733PDBVR. It was made through the same generator with TI's lands (`examples/esp32-s31-devboard/authoring/build_lib.py`).

| Item | Run | Reference | Verdict (source) |
|---|---|---|---|
| Lands | 5 × 0.6 × 1.1 mm, pitch 0.95, rows 2.6 apart; pad 1 at (−0.95, −1.3) | Same | **Identical**, 0 nm (p.32 DBV0005A example board layout) |
| 3D values | Body 2.9 × 1.6 × 1.1 mm, span 2.8, lead 0.40 × 0.15 mm, foot 0.40 mm, standoff 0.075 mm | Same (the preset) | **Identical.** The run took the drawing's midpoints (p.31). Its foot is 0.40 mm, not the 0.45 mm midpoint, because the generator refused 0.45 ([T6](#tooling-defects)). The run said so in a low-confidence value with an open question. |
| Pins | IN 1, GND 2, EN 3 (digital input), NC 4 `device_nc`, OUT 5, each citing p.3 Table 4-1 | Same | **Identical** |
| Rules | None | None | Equal. The datasheet states no pin restriction. |
| Polarity | Pad 1 with the generator's dot, set by `library.edit` after `footprint.polarity_missing` | None | Run better ([T8](#tooling-defects)) |
| Review | 18 values with pages and confidences, 4 open questions; it reports the `pinout_apply` workaround | — | Good. It also cites the package marking 1FKF (p.25). |

### ESP32-S31-WROOM-3

The references are the hand-made [`examples/esp32-s31-wroom-3/`](../examples/esp32-s31-wroom-3/) and [run 1](datasheet-assistant-run1.md). Sources: WROOM-3 = module datasheet v0.7; S31 = chip datasheet v0.5.

| Area | MCP run | Reference / run 1 | Verdict (source) |
|---|---|---|---|
| Terminals, numbers, classes | 99 rows; EPAD 95; corner pads 96–99; 8 NC as `device_nc` | Same | **Identical** (WROOM-3 Table 3-1 p.14) |
| Pads, ground array, corner pads, keep-out | Every value restated from the drawing on top of the preset | Same | **Identical**, 0 nm on 107 pads, keep-out identical (WROOM-3 Figs. 10-1, 11-1 pp.64–67) |
| 3D model | The `module` generator's part file at those values; envelope 22 × 30 × 3.5 mm | The example's own part file (`esp32-s31-wroom-3.solidcraft`, six values) | **Equivalent.** The D-slice parity test found the same 110 bodies within 1.3e-7 mm ([parity](generator-plugins.md#parity-at-retirement)). |
| Functions | 305, with `mux_table` on every coded function (IO MUX, LP IO MUX, analog) and labels as printed | Reference: 305 with invented codes `LP_Fn`/`ANALOG` and renamed labels. Run 1: no `mux_table` | **Run best.** On (table, code, direction) it matches the reference on 90 of 99 pins. The other 9 are below. |
| LP SPI on IO2–IO5 | bidirectional | Reference: output/input. Run 1: bidirectional | **Run right** (S31 Table 2-5 p.26, I1/O/T), as run 1 was |
| REF_GMAC_CLK on IO35 | output | Reference: bidirectional | **Run right** (S31 Table 2-3 p.24, type O) |
| USB_DP/DM, USB1P1_N0/P0 | passive | Reference: bidirectional | A judgement call: these are analog USB pads. Neither is wrong. |
| DAC_CH0/CH1 on IO4/IO5 | Missing | Reference: missing. Run 1: present | **Run 1 right** (S31 Table 2-8 p.31). Model omission. |
| Defaults | SPI2 on IO9–IO19, JTAG on IO54–57, UART0, USB1P1 on IO33/34; single-function pins have none | Same choices | **Equivalent** (S31 Table 2-3 note 1) |
| Pulls | `pull` on 53 GPIOs; none on IO37 | 54 | **Reference better.** The run read "Floating" in WROOM-3 Table 4-1 p.21 as "no internal pulls". That is the reset state, not a missing pull. |
| Function sources | All 305 cite WROOM-3 Table 3-1 p.14 | Reference: S31 tables | **Reference better.** Table 3-1 lists the function names, but the mux codes come from S31 Tables 2-3, 2-5 and 2-7. The check accepts any page, so this passes. |
| Peripherals | 18. LCD is split into `LCD` and `LCD_DATA_HI` | 19 (one `LCD`; also `LP_PROBE`, `SD2`, `SDIO`) | **Tooling fault**: the schema says "1–16 roles" ([T5](#tooling-defects)) |
| Pin rules | 7 rules covering strapping (IO36; IO37; IO60 and IO61 together), USB Serial/JTAG, JTAG, UART0, and LP UART flow control excluding LP I2C (forbid). All evaluate; none fails in `check_drafts`. | 8, with the same intents | **Equivalent in coverage.** The three strapping rules always warn, where the reference warns only when the pin is used otherwise. That is noisier but not wrong. Run 1's rules could not run at all. |
| Marking, polarity | No marking; polarity pad left unset (`footprint.polarity_missing`) | Reference: marking; polarity pad 1 | Marking is optional. The polarity finding is a tooling gap ([T8](#tooling-defects)). |

Against run 1 (in-app runner, 2 November), every run 1 defect is gone: rules evaluate, `mux_table` is set, rows carry source pages, GPIO is split into input and output, and no matrix-only peripheral is declared. The geometry is unchanged. The pinout reached the drafts through `library.edit`, not the import, because of T1.

### OPA2333AIDR (new family: gull-wing)

The repository has no reference for this part, so the run is checked against the datasheet.

| Item | Run | Datasheet | Verdict |
|---|---|---|---|
| Lands | 8 × 0.6 × 1.55 mm, pitch 1.27, centres at y = ±2.70; pad 1 at (−1.905, −2.70), counter-clockwise | p.47 D0008A: 8X (0.6), 8X (1.55), 6X (1.27), (5.4) | **0 nm.** The generator's own lands were 0.6 × 1.44 at ±2.5775 (an estimate); the model replaced them ([T7](#tooling-defects)). |
| Body / leads (3D) | 4.905 × 3.895 mm, span 5.995, lead 0.41 × 0.19 mm, foot 0.84 mm, standoff 0.18, body 1.57 mm (1.75 max overall) | p.46: 4.81–5.00, 3.81–3.98, 5.80–6.19, 0.31–0.51, 0.13–0.25, 0.41–1.27, 0.10–0.25, 1.75 max | Midpoints, as the skill asks. The standoff minimum was read as 0.11 mm (it is 0.10); the model flagged this itself. The leads still stand on the new lands. |
| Pins | OUT A 1, −IN A 2, +IN A 3, V− 4, +IN B 5, −IN B 6, OUT B 7, V+ 8; analog in and out, power in; each cites p.4 | p.4 Pin Functions (SOIC column) | **Right.** The output drive (push-pull) is inferred and listed as an open question. |
| Marking | "O2333A" text and a pin-1 dot | p.26 package option addendum: O2333A | Right |
| Polarity | Restored through `library.edit` (pad 1 and dot) after `footprint_from_pads` dropped them | — | Right, at the cost of two extra calls (T7, T8) |

## Tooling defects

Each defect is the tooling's fault, not the model's. Each one is reported, not fixed.

| # | Defect | Effect in these runs | Suggested fix |
|---|---|---|---|
| T1 | **`library.pinout_apply` cannot be called from a JSON client.** `base` is a `u64` (`crates/editor-core/src/library/pinout.rs:273`, `:331`) that is usually above 2^53. Claude Code's transport turns it into a double, so `1339620378277300805` arrives as `…700`, which is refused as `pinout.stale_base`. A string is refused as "expected u64". The in-app runner already accepts text; the command surface does not. | All three runs. The documented import step never ran. Each model confirmed the rounding, then wrote terminals, functions, sources and `review.unreviewed` into the component draft with `library.edit`. So the import's review state is now authored by the agent, and the person's safety net depends on the agent setting it honestly. On the WROOM-3 this cost a 69 KB failed apply, three 92 KB `library.edit` bodies (one only to fix `drive`) and three previews: most of its 266 k output tokens. | Make `base` a string (decimal or hex) in the plan and in `pinout_apply`, or accept both. Alternatively, apply by base against the server-held previewed document (next row). Also add a JSON-client test that sends the base through `f64`. |
| T2 | **`pinout_apply` over MCP needs the whole document again.** The in-app runner applies the previewed document by base only. | 69 KB sent a second time on the WROOM-3 (and refused, T1) | Keep the last previewed document per project handle, and make `document` optional when `base` matches. |
| T3 | **Large replies spill to files.** `library.generators` is 142 KB, with no filter parameter. The WROOM-3 `pinout_preview` reply is 63 KB and `project.inspect /drafts` is 50 KB. All exceed Claude Code's MCP output limit, so Claude Code saves them to a file the model must `Read`. The compact preview that run 1 introduced exists only in the in-app runner. | 1–3 extra `Read` calls per run, plus their context | `library.generators`: a summary by default (name, families, presets by key) and `{generator}` for one generator's parameters. `pinout_preview`: counts, findings, review items and rule checks by default, with `detail` to opt in. |
| T4 | **`library.generators` exposes a local path.** Its reply carries `shipped_library: "/Users/…/library"` (`crates/automation/src/session/generators.rs:377`). | The host's home path reaches the model and its transcript | Leave it out of the MCP reply, or make it relative. |
| T5 | **Stale "1–16 roles" text.** `MAX_ROLES` is 64, but the `Resource.roles` doc comment, which the schema shows (`crates/editor-core/src/functions.rs:135`), and the validation message (`:645`) say 1–16. | The WROOM-3 run split LCD into `LCD` and `LCD_DATA_HI` | Write both from the constant. |
| T6 | **The tabbed generator refuses TI's nominal SOT-23-5.** Its bend rule is `(span − body width)/2 − foot length > lead thickness`, and the DBV midpoints (2.8, 1.6, 0.45, 0.15) give exactly 0.15. | One refusal; the foot length moved to 0.40 mm (3D only) | Use `≥` or a small tolerance, and have the refusal name the largest foot length that passes. |
| T7 | **`gull-wing` (and the other generators except tabbed and module) cannot take a datasheet's land pattern.** There is only `land_width_extra`. | OPA2333: the model rebuilt the pads with `library.footprint_from_pads`, which dropped the generator's pin-1 dot and polarity. That cost two more calls to restore them. | `lands: explicit` with `land_width`, `land_length` and `land_span`, as tabbed v2 has, for gull-wing, no-lead, chip and molded. That is the D3 pattern. |
| T8 | **Generated footprints have no polarity record** (the GP1 limit listed in phase C). | `footprint.polarity_missing` on every run. Two runs set it by hand; the WROOM-3 left it. | Let generators emit `Footprint.polarity` (the ABI field), the pad numbered 1 with the dot as its marker. |
| T9 | **The `project.inspect` `at` syntax can't be discovered.** The skill says `{at: <footprint draft>}`, but the path is `/drafts/<index>/draft/definition/body/…`, where the index is a list position, not the draft id. | Models tried `draft:5`, `{draft: 5}`, `/drafts/items/5`, `/project/drafts` and `/drafts/4/footprint/pads/0`: 1 refusal in the TLV run, 5 in the WROOM-3 run | Accept `draft:<id>` (and `draft:<id>/pads/<n>`), and give the syntax in the skill and in the `at` schema description. |
| T10 | **Headless saving needs `allow-all`.** `project.save` is in the ask class, so the default `auto-allow-edits` refuses it with `gate.needs_person`. `allow-all` also allows `library.publish`. | This harness had to allow publishing in order to allow saving. The model did not publish. | Treat a save inside an allowed folder as allowed under `auto-allow-edits`, or add a mode that allows files but not publishing. |
| T11 | Small schema friction: a generator can't be `describe`d by name (`catalog.unknown` for `gull-wing`), `library.marking_surfaces` requires `package`, and `library.inspect` takes `lineage`, not `id`. | 1–3 refusals per run | Point `describe <generator>` to its `library.generators` entry, and give the required fields in the summaries. |

### Model defects

- **WROOM-3:**
  - DAC_CH0/CH1 are missing (S31 Table 2-8 p.31);
  - every function cites the module's pin table rather than the chip's mux tables;
  - IO37 has no pull option, from a misreading of "Floating";
  - the strapping rules always warn.
- **TLV757:** none beyond the foot length, which the tool forced (T6).
- **OPA2333:** the standoff minimum was misread (0.11 for 0.10 mm), which the model flagged itself, and the output drive is inferred (an open question).

All three runs followed the skill:
- they read the datasheets themselves and cited every value with a page and a confidence;
- they listed open questions;
- they never published or cleared the review state;
- they reported the T1 workaround to the person in their final message and their review.

## Cost

The client reported the cost of each run as `total_cost_usd`, at list price (Claude Code's `costBasis: list`), billed through the logged-in Claude Code account.

| Item | USD |
|---|---|
| MCP connectivity check (Haiku, one `project.inspect`) | 0.00139145 |
| Run 1: TLV75733PDBVR | 2.63771120 |
| Run 2: ESP32-S31-WROOM-3 | 11.56986840 |
| Run 3: OPA2333AIDR | 3.33269200 |
| **Total** | **17.54166305** |

The budget was 30 USD. Each run had a hard cap (`--max-budget-usd` 8, 17 and 8), set so that the cap of the next run still fit under the total. No run reached its cap.

Where the tokens went:
- The WROOM-3 run cost 4.4 times the TLV run. Its context grew to about 250 k tokens of PDF pages and replies.
- On that context, the T1 workaround's large `library.edit` calls and the second document copy (T2) dominate the output: about 266 k tokens, against run 1's 150 k.
- Fixing T1 to T3 would remove about four 70–92 KB tool inputs and two spilled replies.

By the same token accounting, that should bring the WROOM-3 run to about half its cost.
