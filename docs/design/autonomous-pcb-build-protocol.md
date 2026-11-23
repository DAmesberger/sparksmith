# Autonomous PCB build protocol

**Status:** design for founder review, 23 November 2026. Nothing here is built unless it says so. Estimates are agent hours and are estimates. Related: [USB-C multi-tool](usb-c-multitool.md) (the first board this protocol is for), [agent harness](agent-harness.md), [LLM integration](../llm-integration.md), [MCP surface](../mcp-surface.md), [fabrication output](../fabrication-output.md), [development](../development.md#live-mcp).

## 1. What we want to prove

That an AI, using Sparksmith, can take a board from written requirements to an order-ready fabrication package with no human design work, and that the boards it produces work. And that anyone can check this claim from what we publish, without trusting us.

The claim is only as strong as its record. So this document is mostly about what is recorded, how it is replayed, and how it is checked by tools we did not write.

## 2. Roles, and what "autonomous" means

### Who does what

| Role | Who | Allowed actions |
|---|---|---|
| Requirements author | A person (the founder) | Writes the requirements before the run. They are frozen and hashed; nothing is added during the run |
| Gate approver | A person | Answers yes or no to a permission-gated request the AI makes (an `ask`-class command, a purchase, publishing). Each answer is logged. A "no" is allowed; an answer with extra instructions is not |
| Buyer | A person | Pays for and places the order the AI prepared. Relays fabricator questions to the AI word for word and the AI's answers back |
| Hands | A person | Receives boards and carries out the physical test procedure the AI wrote, step by step, recording readings. Does not diagnose or fix |
| Operator | A person | Starts the run with the published commands, keeps the machine running, runs the independent checks after the run |
| Designer | **The AI only** | Everything else: analysis, part choice, sourcing, components, schematic, rules, layout, checks, fixes, fab package, BOM, test procedure |

### Definition

A run is **autonomous** when, between freezing the requirements and the AI declaring the fab package ready, no person gave the AI any information or made any design choice, except gate answers (yes/no) and verbatim relays of third-party messages.

Everything else a person does is an **intervention**. Every intervention is logged and counted, never hidden. Categories:

| Code | Intervention | Example |
|---|---|---|
| N | Nudge without information | The fixed text "Continue. The requirements are final; decide yourself." after the AI stops or asks |
| C | Clarification | Answering a question about the requirements |
| H | Hint or correction | "Use the other connector", "pin 3 is wrong" |
| E | Direct edit | A person changes the design, a file or the library |
| T | Tool fix | Sparksmith changed during the run (a bug fix). The tool commit is pinned, so this is always an intervention |
| R | Restart | The run is restarted with a changed prompt, model or tool |

A run with zero interventions of any kind is **fully autonomous**. A run with only N interventions is **autonomous with nudges** and reports their count. Any C, H, E, T or R makes it **assisted**, reported with the list. A gate "yes" is not an intervention, but approvals are counted too, because a design that needs many approvals is less autonomous in practice.

The physical stage (tests on real boards) is outside the definition: hands are needed there. What counts is that the procedure and the pass criteria were written by the AI before the boards arrived, and that the person followed them without changing them.

## 3. Tooling, pinned

Everything that could change the result is pinned in `run/manifest.json` before the run and published.

| Item | What is pinned | How |
|---|---|---|
| Sparksmith | Git commit SHA; `cargo build --locked --profile ci -p sparksmith-cli`; the binary's SHA-256 | `git rev-parse HEAD`, `shasum -a 256` |
| Library | The library folder's commit SHA (or the `SPARKSMITH_LIBRARY_DIR` tree digest); fab profile file digest (`library/profiles/jlcpcb-4-layer.json`); method text digest (`library/assist/workflows/datasheet-to-component/`, whose workflow digest already covers `workflow.json` and `method.md`, [workflow format](../workflow-format.md)) | Digests in the manifest |
| Agent | **Recommended: an external agent over MCP**, Claude Code headless with `--output-format stream-json`, exactly as the existing datasheet runs (`examples/tlv75733-mcp-1/transcript.jsonl` records `"model": "claude-opus-5-5"` and `"claude_code_version"`). Alternative: the built-in chat with its provider profile (`library/assist/profiles/*.json`) and `model::record` | Model id, client version, effort setting, system prompt or kickoff prompt text, tool allow-list |
| MCP server | `sparksmith mcp --permission-mode auto-allow-edits --allow-folder run/` (headless, gated; ask-class requests are refused with `gate.needs_person` and become gate requests) | Command line in the manifest |
| Environment | OS and version, CPU, `nix develop` flake lock, date and time zone | Manifest |
| Budget | An explicit token or dollar ceiling, chosen by the operator for this run, or "none". Sparksmith itself sets no default limits ([agent harness](agent-harness.md)) | Manifest |
| Prices | The model's per-token prices on the run date, with their source URL | Manifest |

Why the external agent first: its transcript format already exists in the repository, it shows every prompt, tool call, reply and token count, and it tests the surface every outside user will use. The built-in chat is the second run.

## 4. The record

One folder per run, published as is (after redaction, section 8).

```
run-YYYY-MM-DD-NN/
  manifest.json          pins, digests, start and end times, budget
  requirements.md        frozen requirements; its SHA-256 is in the manifest
  kickoff.md             the exact prompt that started the run
  transcript.jsonl       client stream: every prompt, model output, tool call, tool result, token usage, timestamp
  wire.jsonl             every MCP JSON-RPC message in and out of `sparksmith mcp`, byte for byte, timestamped
  decisions.md           every design decision: question, options, choice, rationale, sources (AI-written)
  sources/               index.json: every datasheet and web page used (URL, date read, SHA-256, licence note);
                         per-part review reports (the datasheet workflow's `*.review.md`)
  checks/                every ERC, DRC and preflight result as JSON, numbered in run order
  findings.md            every finding: first seen, how resolved (fixed by which call, waived with reason, accepted)
  approvals.md           every gate request and answer, with time and approver
  interventions.md       every intervention, coded as in section 2; "none" if none
  usage.json             tokens per request (input, cache write, cache read, output), totals, cost at manifest prices
  timing.json            wall-clock per phase; model time vs tool time where the transcript shows it
  replay.jsonl           engine replay derived from wire.jsonl (section 5)
  out/                   the .sparksmith project, fab ZIP, BOM, pick-and-place, schematic PDF, 3D model
  verification/          outputs of the independent checks (section 6)
  physical/              test procedure (AI-written, dated before delivery), photos, readings, instrument list
  failures.md            everything that went wrong, including runs that were abandoned
  scorecard.md           section 9, filled in
```

What exists today and what does not:

| Record part | State |
|---|---|
| `transcript.jsonl` | **Exists**: Claude Code's stream-json, as in `examples/*-mcp-1/transcript.jsonl` |
| Per-part review reports | **Exists**: the datasheet-to-component workflow writes them |
| Model recordings for the built-in chat | **Exists**: `model::record`, replayable offline |
| `wire.jsonl` | **Not built.** The server-side trace tap is harness slice H4 ([agent harness](agent-harness.md)). Until then a small wrapper that copies stdin and stdout of `sparksmith mcp` to a file with timestamps does it (P1) |
| `decisions.md`, `findings.md` | **Convention only.** The kickoff prompt requires the AI to keep them; findings are also derivable from `checks/` |
| `usage.json`, `timing.json` | Derived from the transcript by a script (P4) |
| `replay.jsonl` | **Needs a converter** from MCP verb calls to `stdio` command lines (P2) |

Rules for the record:

- Nothing is deleted from it. A wrong turn stays in, with its fix.
- Every datasheet number the AI uses cites a source in `sources/index.json` and, for parts, a row in the part's review report (document, page, value, confidence).
- Every check run is kept, not only the last one, so the count of findings found and fixed is visible.
- Secrets never enter it: API keys are not recorded ([LLM integration](../llm-integration.md)), and the redaction pass (section 8) checks.

## 5. Determinism and replay

A language model's output is not deterministic, so "re-run the build" means two different things:

**Engine replay (deterministic).** The tool calls that changed the design are replayed into the engine, without any model. Sparksmith already does this for the ESP32 example: `examples/esp32-s31-devboard/regenerate.sh` pipes 605 JSON-lines requests through `sparksmith stdio --operator`, fails on any refused request, then writes the fab package with `sparksmith fab … --date none`, which is byte-identical run to run ([fabrication output](../fabrication-output.md#release-identity)). A session adds no random bytes to the project's identity, so a replay saved under the same file name gets the same project id, and every Gerber, drill and job file carries the release revision: 16 hex digits of the SHA-256 of everything delivered.

So a third party:

1. Clones Sparksmith at the pinned commit and builds the CLI (`nix develop`, then `cargo build --locked --profile ci -p sparksmith-cli`).
2. Sets `SPARKSMITH_LIBRARY_DIR` to the pinned library.
3. Runs `sparksmith stdio --operator < run/replay.jsonl` and checks that no reply has `"ok":false`.
4. Runs `sparksmith fab out/board.sparksmith fab/ --profile jlcpcb-4-layer --date none`.
5. Compares the release revision in any Gerber's `.ProjectId` (or every file's SHA-256) with the published package. **Equal means the published package is exactly what the recorded calls produce.**

**Model re-run (statistical).** The same kickoff prompt, pins and requirements, run again from scratch. The result will differ. What we publish is the number of attempts, how many succeeded, and the spread of tokens, cost and time. One success out of one attempt is reported as exactly that.

**Datasheets and web pages change.** The record keeps the URL, date and SHA-256 of every document read. Where a licence allows, a copy is kept privately; the public record keeps the hash, so a third party can check they read the same document.

## 6. Independent verification

"Independent" means a tool or person that is not the run's AI and not Sparksmith's own checker. Sparksmith's checks are part of the run; they are recorded but do not count here.

| Check | How | Pass |
|---|---|---|
| **Gerber and Excellon read-back** | Load the package in at least two viewers not written by us: Ucamco's online Reference Gerber Viewer (from the authors of the Gerber format) and the fabricator's own DFM viewer (JLCPCB's upload preview). Optionally a permissively licensed open-source Gerber library (**licence to check before use**; no GPL or LGPL code, per AGENTS.md). Compare layer count, outline size, drill count per tool against the drill table, and a visual overlay per layer | Every layer present and readable; outline and drill counts match the fabrication drawing; no unexplained difference |
| **Netlist versus requirements** | (1) Extract the netlist from the Gerber X2 object attributes (`.N` net, `.P` pad, `.C` component, which Sparksmith writes) with a short script written by someone other than the run's AI (P3). (2) Independently extract copper connectivity from the rasterised copper layers plus drills, and check it agrees with the `.N` attributes. (3) Compare with a connection table derived from the requirements and the datasheets: power rails, every IC pin's net, connector pinouts. A second, fresh model may write that table blind from the requirements and datasheets, then a person reviews the differences | Geometry and attributes agree; every required connection present; every difference explained in `verification/netlist.md` |
| **BOM versus purchasable parts** | For every BOM line: the MPN found at JLCPCB/LCSC (or another named distributor) on the check date, lifecycle active, stock at least the order quantity, price recorded; the LCSC part number for assembly | All lines purchasable, or an alternate named and checked |
| **DRC under the fab profile** | Sparksmith's `fabrication.preflight` under `jlcpcb-4-layer` is part of the run. Independently: the fabricator's own DFM check on upload, and the fabricator's engineering questions (EQ) after ordering | Zero unwaived errors; every waiver listed with its reason; fabricator DFM passes; every EQ logged with its answer |
| **Controlled impedance** | The fabricator's impedance calculator on the exact stackup, for each impedance class, compared with the widths and gaps on the board | Within the fabricator's stated tolerance |
| **Physical first article** | The fabricator's electrical test of bare boards where offered (**JLCPCB's practice not verified here**); then the AI-written procedure: bring-up, measurements with calibrated instruments (instrument models and calibration dates recorded), functional tests, photos of both sides | Every test in the procedure passes, with readings; failures go to `failures.md` with the AI's diagnosis |

For the [USB-C multi-tool](usb-c-multitool.md), the physical tests are T1–T12 of its test plan, with the USB4 SSD benchmark as the acceptance test.

## 7. Approvals during the run

The headless server is gated ([development](../development.md#live-mcp)): with `--permission-mode auto-allow-edits`, reads and edits run; anything that needs a person is refused with `gate.needs_person`. The AI then writes a gate request into `approvals.md` (what, why, the exact call), and the gate approver answers yes or no. On yes, the operator runs that one call (`stdio --operator` with the exact line, logged) or re-runs the server with a wider mode for that step only, logged. The AI is told the answer in a fixed sentence.

Expected gates on a board build: none for design work. Probably: writing the fab package outside the run folder (avoided by keeping everything inside), any library-source change, and publishing. Ordering and payment happen outside Sparksmith and are the buyer's job.

## 8. Publication

A new public repository per board (for example `autonomous-build-usb-c-multitool`), containing:

- The record folder (section 4), redacted.
- The design: `.sparksmith` project, schematic PDF, 3D model.
- The fab package exactly as sent to the fabricator, with its release revision.
- The order record: quote, order date, fabricator EQs and answers (prices may be kept).
- Verification outputs, photos of bare and assembled boards, all measurements with instruments, the failures log and the scorecard.
- A README that states the result in one paragraph, including the intervention count, and how to replay it.

**Licensing (decision P-L).** Recommendation: hardware design files under CERN-OHL-P v2 (permissive, made for hardware); text, photos and measurements under CC BY 4.0; scripts under MIT OR Apache-2.0 like Sparksmith. Datasheets are **not** republished (their licences rarely allow it); the record keeps their URL, date and hash.

**Privacy.** Generic only:

- Nothing from any private design, not even as an example or a comparison. The requirements are written fresh for the public board.
- Before publishing, a redaction pass replaces home paths, user names, host names, e-mail addresses and tokens in every file, and a scan confirms none remain. The redaction script and its report are published too.
- Publishing is a gated step: a person approves the redacted repository before it goes public.
- Trademarks (USB4, USB Type-C) are used only to describe; no USB-IF logos and no compliance claims.

## 9. Success criteria and scorecard

**The claim succeeds when all of these hold:**

1. **S1 Autonomy:** zero interventions of codes C, H, E, T, R (section 2). Nudges (N) are reported.
2. **S2 Order-ready:** the fabricator accepts the package; no EQ needs a design change.
3. **S3 Independent checks:** every check in section 6 passes, or each difference is explained and harmless.
4. **S4 Replay:** a third party reproduces the published package byte for byte from `replay.jsonl` (section 5).
5. **S5 Working boards:** the first-article tests pass, including the board's acceptance test.
6. **S6 Complete record:** every model call and every tool call is in the record; `wire.jsonl` and `transcript.jsonl` agree on the call count.

**Scorecard** (published as `scorecard.md`):

| Area | Measure | Result |
|---|---|---|
| Autonomy | Interventions by code (N, C, H, E, T, R); gate approvals | |
| Outcome | S1–S6 pass or fail | |
| Cost | Tokens: input, cache write, cache read, output; dollars at the manifest's prices | |
| Time | Wall clock from start to "package ready"; per phase | |
| Effort | Tool calls; refused calls and their codes; check runs | |
| Quality | Findings found and fixed; waivers with reasons; fabricator EQs; DFM warnings | |
| Hardware | Board and BOM cost per unit at 5 and 50; tests passed out of run; acceptance result | |
| Reproducibility | Third-party replay: identical yes/no; model re-runs: successes out of attempts | |

Measured numbers only. Anything not measured is written "not measured", never estimated after the fact.

## 10. Run book

**Before the run (operator)**

1. Freeze the requirements: the founder writes `requirements.md` (function, interfaces, limits, cost target, fabricator, acceptance test). Record its SHA-256. No edits after this.
2. Pin the tooling: check out the Sparksmith commit, build the CLI, record its hash, pin the library and the model. Write `manifest.json`.
3. Create the run folder with the empty record files. Start the MCP wrapper that writes `wire.jsonl` (P1).
4. Write `kickoff.md`: the requirements path, the rules of this protocol (keep `decisions.md` and `findings.md`, cite every number, ask for a gate instead of acting, never ask a person for design input), the fab profile, and where to put outputs. Publish it unchanged.

**The run (AI; operator watches only)**

5. Start the agent with `kickoff.md` and the pinned model; note the start time.
6. Phase A, requirements analysis: block diagram, power budget, interfaces, open questions answered by the AI itself in `decisions.md`.
7. Phase B, part choice and sourcing: candidates, datasheets into `sources/`, availability and price snapshot.
8. Phase C, components: each part through the datasheet-to-component workflow; review reports kept.
9. Phase D, schematic: sheets, ERC to zero errors; the AI's own connection table checked against the schematic.
10. Phase E, rules: the fabricator's impedance stackup entered, fab profile set, net classes with impedance targets, clearances.
11. Phase F, layout: outline, placement, routing, fills; DRC to zero unwaived errors.
12. Phase G, release: `fabrication.preflight`, fab package, BOM and pick-and-place, schematic PDF, 3D model, the physical test procedure.
13. Phase H, self-review: a fresh agent, same pins, reviews the package against `requirements.md` without the first agent's conversation; its findings go back to the run agent. This is still AI work.
14. The AI declares "package ready" in a fixed sentence. Note the time. The autonomous part ends here.

**After the run (operator, buyer, hands)**

15. Derive `replay.jsonl` (P2), replay it on a clean checkout, confirm the release revision matches.
16. Run the independent checks (section 6) and write `verification/`.
17. Gate: the founder approves ordering. The buyer uploads the package, records the quote and the DFM result, and orders. Fabricator EQs are relayed to the AI verbatim; answers go back verbatim.
18. Hands run the physical procedure as written; photos and readings into `physical/`.
19. Fill in `failures.md` and `scorecard.md`.
20. Redact, scan, gate, publish.

## 11. What must be built first

| # | Item | Why | Estimate |
|---|---|---|---|
| P1 | MCP wire recorder: a wrapper now, the H4 trace tap later | `wire.jsonl` | wrapper 1–2 h; H4 per the harness plan |
| P2 | Converter from recorded MCP verb calls to a `stdio` replay, keeping only accepted edits and file effects | Engine replay (section 5) | 3–5 h |
| P3 | Gerber X2 netlist extractor and copper-connectivity cross-check, written outside the run, permissive licence | Independent netlist check | 4–6 h |
| P4 | Usage, timing and scorecard script over the transcript | Cost and time totals | 2–3 h |
| P5 | Redaction script with a leak scan | Publication | 2 h |
| P6 | Kickoff and method text for whole-board builds (one method text, harness H5) | Same rules for every run | 2–3 h |

Board-specific gaps (USB4 85 Ω target, stackup entry, straddle footprints, no-via rule, current check) are listed in the [multi-tool design](usb-c-multitool.md#15-what-this-board-asks-of-sparksmith). Any of them the AI cannot work around becomes a finding of the run, not a reason to help it.

## 12. Decisions for the founder

| # | Decision | Recommendation |
|---|---|---|
| P-A | Is a nudge (N) allowed in a run called "autonomous"? | Allowed but counted; the headline claim "fully autonomous" only with zero |
| P-B | First run with an external agent over MCP, or the built-in chat? | External agent first (record format exists); built-in chat second |
| P-C | Run budget | An explicit ceiling in the manifest; reaching it ends the run as a failure, reported |
| P-D | How many model re-runs before publishing | At least one more full run after the first success, to show it was not luck |
| P-L | Licences for the public repository | CERN-OHL-P v2, CC BY 4.0, MIT OR Apache-2.0 |
| P-E | Self-review by a second agent (step 13) part of the autonomous run? | Yes: it is AI work and it is recorded |
