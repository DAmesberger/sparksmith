# Agent harness: one harness for MCP clients and the built-in chat

**Status:** design for founder review, 23 November 2026. Nothing here is built. Estimates are agent hours. Related: [extension surface](../extension-surface.md), [MCP surface](../mcp-surface.md), [LLM integration](../llm-integration.md), [board geometry for an LLM](../llm-geometry.md), [assistant panel](../assistant-panel.md), [extension ABI](extension-abi-design.md), [generator plugins](../generator-plugins.md).

Founder (verbatim): "we need a harness that optimizes the agents (regardless if via MCP or the internal chat) and i think we need some neurosymbolic stuff to make it work good, but that means it needs to be modular and switchable, maybe even as a WASM plugin?"

Rules this design keeps: no default token or turn limits (a stopped turn wastes what it spent), fewer tokens come from a better harness, every provider including local models works, and no data is hard-coded.

## 1. What the harness is

The harness is everything between a model and the engine. The model proposes and the engine does the work. The harness decides what the model sees, in which form, what happens to its proposals before they reach the engine, and what it is told afterwards.

| Part | What it does | Today |
|---|---|---|
| **Context assembly** | What the model sees at the start of a message and in which encoding: `view.context`, open issues, what changed by hand, geometry views | `view.context` per message; geometry encodings designed in [llm-geometry](../llm-geometry.md), not built |
| **Surface view** | How the catalog is described: `find` hits, `describe` levels, reply shapes, refusals | `describe` returns the full contract: input schema, reply schema and their definitions |
| **Plan, verify, recover** | Check a proposal before it runs, run it, check the result, and turn failures into short, useful feedback | Dry runs, the one `Error` with `closest` and `instead`, rebasing of stale edits, `repeat_limit`, workflow step checks |
| **Condensing and caching** | Keep the prompt small across a long conversation, and keep its start stable so a provider's prompt cache can serve it | K1 in the chat (`chat::context`): shortened replies, stubs, condensing at 70 % of an 80 k window, summary; the nudge; cache breakpoints on Anthropic |
| **Per-model profiles** | The prompt additions and loop settings one model needs | `library/assist/profiles/*.json`: system prompt add or replace, `omit_tools`, `repeat_limit` |
| **Evaluation** | Measure a change before shipping it: task success, tokens, calls, time | Scripted board build (`chat::tests::board_chat`), recordings and replay (`model::record`), and a Python bench outside the repository |

### Where each part lives

There are two places a harness part can run:

- **On the surface** (`sparksmith-automation`, behind `Surface::call`). It works for every caller: MCP clients we don't control (Claude Code, Codex, OpenCode), ACP agents, the built-in chat and workflows.
- **In our loop** (`sparksmith-assist`). It only works where Sparksmith runs the loop: the built-in chat and workflow model steps. An external agent runs its own loop, condenses in its own way and has its own system prompt.

**Rule: put a part on the surface whenever it can live there.** Surface views, reply shapes, plan validation, verifiers, fact retrieval and failure feedback all fit there, and then MCP clients and the chat improve together. Only condensing, the nudge, cache breakpoints, per-model prompts and the request loop stay in our loop, because they need the conversation itself.

### The measured biggest win: the input view

The external review measured every entry's `describe` reply (`artifacts/review-8dea4ac3/probes/catalog-projection.tsv`, 290 entries):

| | Median | Largest | Sum |
|---|---:|---:|---:|
| Full contract (today's `describe`) | 30,627 B | 84,977 B (`project.query`) | 7.9 MB |
| Input view (call, input schema, class, person step; no reply schema) | 750 B | 22,247 B (`project.query`) | 0.27 MB |

Most of the full reply is the reply schema and its shared definitions: `project.undo` is 30 KB in full and 711 B as an input view. In two paired trials of one engineering change (fresh GPT-6.2 Astra agents, low effort, n = 1 per arm), all four agents saved byte-identical, independently verified boards. The tuned arm used **31 % and 34 % fewer total tokens**. Uncached input rose 11 % in the first pair and fell 67 % in the second, so treat the total-token saving as the steady result and the uncached change as noise for now.

The review did this on the client, with a Python proxy that removes `reply` and `$defs` from `describe` replies. Done there, it helps only that one client, and the first version also dropped the approval metadata from cached contracts. It should be done on the server: `describe` offers levels of detail, and every client, the chat included, gets the small one first (slice H1).

## 2. Neurosymbolic design

**The model proposes intent and plans. Symbolic parts do the exact work and check it.** A language model is good at choosing what to do: which parts belong together, which net goes where, which fix fits. It is bad at exact values: coordinates, clearances, pin numbers it has not read, units. The engine is the opposite. So each step of the loop goes to whichever side does it well:

```
person's goal
  │
  ▼
[model] intent + typed plan ──► [validate] names, fields, units, ids, net intent ──refused──┐
                                    │ ok                                                     │
                                    ▼                                                        │
                              [solve] router, placer, generators, value parser                │
                                    │                                                        │
                                    ▼                                                        │
                              [rehearse] dry run on a fork                                    │
                                    │                                                        │
                                    ▼                                                        │
                              [verify] ERC, DRC, preflight, intent diff, render check ─fail──┤
                                    │ pass                                                   │
                                    ▼                                                        ▼
                              apply in the turn (one undo step)            [feedback] one line per problem:
                                                                           code, object, rule, value, instead
                                                                           ──► back to the model
```

| Symbolic part | Where in the loop | Exists |
|---|---|---|
| Typed plan validation against schemas | Validate | Yes: every `apply`, `query` and `act` is checked against the entry's schema; a wrong field is refused naming the right one (`mcp::fields`); unknown names give `closest` |
| Unit and value parsing (`10mil`, `4k7`, `100nF`) | Validate | Units yes (`editor-core/src/units/text.rs`); values through the families' value fields |
| Exact fact retrieval | Before planning | Pins `schematic.pins`, pads `layout.pads`, datasheet text and pages `assist.document_text`/`document_page`, parts `library.search`, rules `rules.*` |
| Net-intent check: "these terminals should be one net" against what the copper graph and the schematic say | Validate and verify | **No.** The model checks this itself by reading pins and nets |
| Router (push-and-shove, `max_detour`, preview) | Solve | Yes: `layout.route_step`, `route_preview`, `route_pair` |
| Placement by constraints | Solve | **No.** `layout.place` takes coordinates the model chooses |
| Generators (packages from datasheet values) | Solve | Yes: `library.generate_part` |
| ERC, DRC, preflight | Verify | Yes: `project.issues`, `fabrication.preflight` |
| Findings since the last edit (new and resolved) | Feedback | Sketched in the ABI design's 3.4 (`findings: {new, open}`); edit replies do not carry it today |
| Workflow step checks | Verify | Yes: `assist.step_result` checks a model step's answer against its typed output |

The datasheet workflow already works this way: model steps write typed outputs (`package_plan`, `pinout_document`, `review_report`), each checked against its schema, and tool steps do the exact work (`library.generate_part`, `library.pinout_preview`, `library.review_check`). This design applies the same pattern to board work.

**Feedback stays small and exact.** A failure returns what the engine knows, in words the model can act on: `{code, at, rule, value, limit, instead}`, one line per problem, naming objects by designator and net. It does not return a dump of the board. A verifier answers with a difference ("2 new: clearance R3.1–GND 0.12 < 0.20 mm; 1 resolved"), not the full issue list. Repeated identical refusals already end at `repeat_limit`; the nudge handles long reading with no change.

### Three PCB examples

**Wire a sub-circuit** ("add the LDO with its two capacitors").
1. The model reads facts: `schematic.pins {designators: [U3, C7, C8]}`.
2. It states intent as data: `{VIN: [J1.1, U3.1, C7.1], VOUT: [U3.5, C8.1], GND: [U3.2, C7.2, C8.2]}`.
3. A net-intent check (new, read-only) resolves each terminal, compares it with the current connectivity and answers what is missing, what is extra and the operations that close the gap. Today the model does this comparison itself across several reads.
4. One `apply` of those operations, then ERC on the difference. A short or a wrong pin comes back as one line naming the terminal.

**Fix DRC** ("fix the clearance errors").
1. `project.issues` gives findings with object ids, rule and measured value.
2. The model picks a strategy per finding: move a part, reroute, change the layer. It may never widen a rule or add a waiver unasked. That is the person's decision, and the gate's classes already keep waivers behind the person.
3. The router does the geometry: `layout.route_preview` with `max_detour`, then `route_step` or `route_apply`.
4. A dry run plus the findings difference checks it. Anything left over comes back as one line per finding.

**Place by function** ("put the decoupling caps next to U1's power pins, the connector on the left edge").
1. The model states constraints, not coordinates: `near(C1, U1.VDD, ≤1.5 mm)`, `edge(J1, left)`, `group(U2, Y1, ≤5 mm)`, `side(top)`.
2. A constraint placer (new) solves them against courtyards, keepouts and the board shape, and returns positions, or the constraint it could not meet and by how much.
3. `layout.place` in a batch dry run, then courtyard and DRC checks.

The geometry work in [llm-geometry](../llm-geometry.md) (overview, window, route at goal, guided or path level) fits the same pattern and is a context-assembly strategy here.

**New symbolic parts are catalog entries.** The net-intent check and the placer are entries (`read` and `edit`), declared with `declare_entries!` like any other. MCP clients and the chat then get them through the same verbs, the gate holds, and `surface.lock` versions them. None of them is a private tool of the built-in loop.

## 3. Modularity

### Strategy slots

Each harness part is a **slot** with one stable interface. A **strategy** fills it. Every slot is a function from JSON to JSON with no hidden state, so it can be built in Rust or loaded as WASM behind the same interface:

| Slot | In | Out | Runs |
|---|---|---|---|
| `surface_view` | catalog entry, detail level | the description a caller sees | Surface |
| `reply_shape` | an entry's full reply, the caller's detail | the reply sent | Surface |
| `context` | `view.context`, issues, changes since the last call | the context block of a message | Loop (and an `assist.context` query for MCP clients) |
| `encode` | a geometry query result | text for the model (grid, list, overview) | Surface (as a query option) |
| `validate` | a plan, the document at R | ok, or refusals with `instead` | Surface |
| `verify` | before R, after R, the plan's intent | findings difference | Surface |
| `feedback` | an error or findings | the compact lines the model reads | Surface |
| `condense` | history, policy | history, kept record | Loop |

In Rust: one trait per slot, a registry of named strategies, and the active choice in a harness profile. The parts that exist (`chat::context`, the nudge, `repeat_limit`, `mcp::fields`, the leniency reader) become the first strategies of their slots. The code stays where it is, now selected by name.

**Never in a slot, and never pluggable:** the gate and grants, person steps and approvals, keys, the decision whether a call runs, and anything that changes the document outside `Surface::call`. A strategy may change how something is described. It may never change what is allowed or remove a person step or class from what a caller sees. A test holds that every `surface_view` strategy keeps `class`, `person` and `reach`, so the review's dropped-approval bug can't come back.

### WASM plugins or built-in strategies?

| | Built-in Rust strategies + data profiles | WASM strategies on the one plugin ABI |
|---|---|---|
| Switch without a rebuild | Yes, between shipped strategies; parameters are data | Yes, including new strategies |
| Third parties and experiments | Need a Sparksmith build | A bundle, approved by hash |
| Deterministic, replayable | By review and tests | By construction: effect `none` is a pure function; effect `read` runs on a frozen fork |
| Evaluation results tied to an exact strategy | Build revision | Bundle hash |
| Speed | Native | wasmi interpreter; effect `none` has 500 M fuel (about 0.5 s) per invoke. Fine for transforms, too slow for a router on a large board |
| Cost to build | Small | Needs S9a (the `call` import answered on a frozen fork), which isn't built; generators run on effect `none` only |
| Debugging | Normal | Harder; logs through `log` only |

**Recommendation: both, in order, with one interface.** Build the slots and the data profiles as Rust strategies now. Add WASM as a second way to fill the same slots once S9a lands, on the one plugin ABI ([extension ABI 2.6](extension-abi-design.md)), with no second harness ABI. A strategy is a plugin capability: transforms are effect `none`, verifiers and validators are effect `read`.

| Belongs in WASM | Stays native |
|---|---|
| Deterministic transforms: surface views, reply shapes, geometry encodings, context compilers, feedback formatting | Model calls, streaming, the network, keys |
| Verifiers and validators that read the fork: net-intent check, custom design checks, house rules | The request loop, Stop, scheduling, cache breakpoints (provider wire format) |
| Alternative or experimental solvers on a fork (effect `edit`): a third-party placer | The core router, copper graph, ERC and DRC: one source of truth in the engine, never duplicated in a plugin |
| | The gate, grants, approvals, files |

### Choosing strategies, as data

A **harness profile** is a file in the shipped library, `library/assist/harness/NAME.json`, which a user's copy can override like run profiles:

```json
{"format": 1, "name": "compact",
 "surface_view": {"strategy": "input", "full_on_request": true},
 "reply_shape": {"strategy": "shorten", "reply_chars": 12000},
 "condense": {"strategy": "stubs-then-summary", "keep_rounds": 2, "bulky_chars": 1500,
              "batch_chars": 12000, "trigger": 0.70, "target": 0.35, "window_tokens": 80000},
 "nudge": {"after_tokens": 250000},
 "recover": {"repeat_limit": 3}}
```

It is chosen at three levels. The more specific one wins, and none is detected automatically:

1. **Model:** a run profile (`profiles/NAME.json`, already chosen by model id) names its harness profile.
2. **Task:** a workflow's `workflow.json`, or one step of it, names one (a datasheet step may want a different reply shape than board wiring).
3. **Person:** Settings › AI › Advanced, per connection.

For MCP clients, the surface parts are chosen per call (`describe {detail}`, a query's `encoding`), and the server's default is the measured best. Detecting the client by its `clientInfo` is possible, but this design does not do it, matching the founder's "no model detection".

## 4. Evaluation harness (Rust, dev tooling)

The evaluation tool stays out of the shipped product. It is a dev crate and an `xtask` command, so it can use the real engine, the real verbs and the real reader of saved files.

- **Task:** data in `tests/agent-tasks/NAME/`: a starting project, the goal text, the grant, and an **independent verifier**. The verifier is a spec checked by reading the saved file with `sparksmith-io` (outline, net memberships, track widths and endpoints, placements, finding counts), never the model's own report.
- **Arm:** a harness profile × a run profile × a model connection (or an external client).
- **Runners:**
  - *Scripted:* a scripted model that takes its ids from replies, as `board_chat` does. Deterministic, no key, runs in CI.
  - *Recorded:* replays a recording (`model::record`), refused against another workflow version, as today.
  - *Live:* a real endpoint, hosted or local, with a budget the operator sets for that bench run (requests, tokens, time). This budget is the bench's, not a product default.
  - *External client:* `sparksmith mcp` with a server-side trace tap. Codex, Claude Code or OpenCode drives it, and the client's own usage counters are entered or marked "unavailable".
- **Metrics per run:** success from the verifier; tokens in, out, cached and uncached (from provider usage, `usage::Tally`); one estimate where a provider gives none, from a single shared estimator (today the chat uses 3 characters per token and the review 4 bytes per token); calls per verb, describes, refusals and retries; bytes per call (request, reply, model-visible); server time and wall time; the document revision and SHA-256 of the saved file.
- **A/B:** paired arms from fresh contexts, alternating order, N repeats, reporting median and spread. No recommendation from one pair: the two Astra pairs disagreed on uncached input.
- **CI gates on scripted runs:** each scenario must succeed, and its model-visible bytes and estimated tokens must stay under recorded ceilings. A ceiling moves only with a commit that says why. A per-entry budget for the input view joins these gates. Live runs stay on demand.

**From the Python bench, keep:** the per-call trace record (verb, entry, request, reply and visible bytes, round trip, result digest), a fresh private folder per arm with a folder-scoped grant, an independent verifier, fresh paired contexts, keeping wire bytes apart from model usage, and writing "unavailable" rather than guessing. **Replace:** the client-side proxy and contract cache (with `describe` detail levels and a trace tap in the server), `verify.py` (with a Rust verifier over `sparksmith-io`), the Unix-socket shim, and the copy of the method in `astra.md` (with one shared method text, below). The Codex tuner role can stay as a note in the verification skill on how to drive the external-client arm.

## 5. What exists and fits, and what is duplicated

**Fits as it is:**
- The six verbs, byte-equal for MCP and the chat, and one `Surface::call`: a surface strategy reaches both with no extra work.
- `EntryDecl` metadata (class, person, reach, effect, time, result size): what a view may shorten and what it must keep.
- The ABI design's three description levels (index, line, full). The input view is the missing level between line and full.
- Run profiles as data, reach profiles as data, workflows with typed model steps and checked outputs.
- K1 condensing, the nudge, `repeat_limit`, rebasing of stale edits, field and name suggestions, the leniency reader.
- `model::record` replay, `board_chat` and `board_trace` scripted runs that already print per-request sizes.
- The plugin ABI with declared effects, the generator host, fuel and memory limits.

**Duplicated or hard-coded, to fix while building this:**
- `Policy::CHAT` (12 000, 2, 1 500, 12 000, 0.70, 0.35, 80 000, 250 000) and the default `repeat_limit` are constants in Rust. They become the shipped `compact` harness profile (no-hard-coded-data).
- The method an agent should follow appears in four places: `chat-prompt.md`, `chat-playbook.md`, the MCP instructions (`ABOUT` in `crates/automation/src/mcp.rs`, text in code) and the review's `astra.md`. The generic part of Astra's guidance (describe before use, fetch definitions only when needed, carry ids and the revision forward, batch independent edits, read the error before retrying) is good for every model. It goes into one library file that both the MCP instructions and the chat prompt are built from. The MCP instructions are 940 B today, and Claude Code cuts them at 4,096 characters.
- Two trace formats: model replies (`model::record`) and wire calls (the bench's `trial.jsonl`). They become one run trace with both layers.
- Two token estimators (3 characters per token, 4 bytes per token) become one.
- The line form (designed, not built) and the input view (measured, client-side) answer the same question. Both become levels of one `detail` parameter.

## 6. Risks

- **Overfitting to one model and one task.** The evidence is two pairs on one change. Mitigation: a task set before changing defaults, paired repeats, and a local model in every comparison.
- **A view hides something the model needed.** The pinned cache lost the approval metadata. Mitigation: required fields kept by test; the full contract always one call away; refusals name what the caller should describe.
- **Prompt cache churn.** A strategy that changes the start of the prompt from request to request costs more than it saves. Mitigation: measure uncached tokens separately, and keep views deterministic per surface digest so clients can cache them.
- **Verifiers give false confidence.** In the review, a schematic placed outside its page passed ERC. Mitigation: verifiers include render and placement checks, and the bench verifier reads the saved file and renders.
- **Solver refusals loop.** A placer or router that keeps refusing can trap the model. Mitigation: refusals name the constraint and the amount, and `repeat_limit` plus the nudge already handle repeats.
- **Too many knobs.** Eight slots × strategies × profiles can become untestable. Mitigation: ship few strategies, each one earning its place through a measured A/B result, and delete a strategy that loses.
- **WASM speed and maturity.** wasmi is an interpreter, and S9a isn't built. Mitigation: native first, WASM for transforms and verifiers, never the core router.
- **Live-run cost and noise.** Mitigation: CI gates use scripted runs only; live runs are on demand with an operator budget.

## 7. Founder decisions

1. **`describe` gives the input view by default, the full contract on request (`detail: "full"`), for every client including the chat.** *Recommendation: yes.* It is the measured biggest win and costs one optional parameter (a minor version of `project`).
2. **Harness parts live on the surface wherever they can, so MCP clients and the chat get the same harness.** *Recommendation: yes.*
3. **Strategy slots with one interface: Rust strategies now, WASM strategies on the one plugin ABI after S9a, and no separate harness ABI.** *Recommendation: yes.*
4. **The gate, grants, person steps, keys and model calls are never pluggable, and no strategy may hide `class`, `person` or `reach`.** *Recommendation: yes.*
5. **Harness profiles are data (`library/assist/harness/`), chosen by model profile, then workflow or step, then the person, with no automatic detection of models or clients.** *Recommendation: yes.*
6. **New symbolic parts (net-intent check, constraint placer) are catalog entries, not private tools of the chat.** *Recommendation: yes.* The placer needs its own short design first.
7. **One shared method text in the library, from which the MCP instructions and the chat prompt are built, carrying the generic part of the Astra guidance.** *Recommendation: yes.* Model-specific remainders go into run profiles.
8. **The evaluation tool is Rust dev tooling (a dev crate and `cargo xtask agent-bench`), not shipped, and the Python bench retires once the Rust one reproduces its two trials.** *Recommendation: yes.*
9. **CI gates on scripted runs (success plus byte and token ceilings); live runs only on demand, with a budget you approve per batch.** *Recommendation: yes.* Please name the budget for the first live A/B batch (the trials so far used one model; I suggest a hosted model and a local one).
10. **Edit replies carry the findings difference (new and resolved since the turn's previous edit).** *Recommendation: yes*, as the ABI design's 3.4 sketched. It saves a `project.issues` call after most edits.
11. **The model-filled part of the kept record (goals, plan, remaining steps) through one `assist` entry, checked by schema.** *Recommendation: later, after H5*, once the bench can show whether it saves more than it costs.

## 8. Slice plan

At most 5 agents at once. Each slice is usable alone and is measured with the bench where one exists. H1 comes first because it is the measured win, and it needs nothing else.

| # | Slice | Delivers | Acceptance | Hours |
|---|---|---|---|---|
| **H1** | Server-side input view | `describe {name, def?, detail?: "input" \| "full"}`, default `input`: name, kind, class, title, summary, person, reach, `call`, `call_reply_entry`, the input schema and its definitions (paged as today), and a pointer to the full contract. `project.schema` takes the same `detail`; a minor version of `project`; `surface.lock` updated; `tools/list` within `TOOLS_BUDGET` | A test over every entry: the input view keeps class, person and reach, its examples pass the field check, and its size is printed and held under a recorded ceiling (median ≤ 1 KB). `detail: "full"` is byte-equal to today's reply. The chat's verbs still equal `tools/list`. `board_chat` re-measured before and after | 3–5 |
| **H2** | Harness profiles as data | `library/assist/harness/compact.json` from `Policy::CHAT` and `repeat_limit`; slot registry with today's code as named strategies; run profiles and workflows name a profile; the constants removed | `Policy::CHAT` no longer exists in code; `board_chat` gives identical requests; a broken profile is refused naming the file and field | 4–6 |
| **H3** | Rust bench, scripted and recorded | Dev crate plus `cargo xtask agent-bench`: task data, independent verifier over `sparksmith-io`, one run trace (model and wire layers), one token estimator, metrics as JSON lines; the board build and the two-header engineering change as the first tasks; CI ceilings | The engineering change from the review passes its verifier on a scripted arm; ceilings fail when a reply grows; the board build's numbers match `board_chat`'s | 8–12 |
| **H4** | Live and external arms | Endpoint arm with an operator budget; trace tap in `sparksmith mcp`; paired A/B report with median and spread; a note in the verification skill on the external-client arm | The review's two Astra trials reproduced, input view versus full, on the server-side view; one local-model arm runs | 4–6 |
| **H5** | One method text | A library file the MCP instructions and the chat prompt are built from; the generic Astra guidance in it; `ABOUT` out of code | Instructions under 4,096 characters; the chat prompt's text unchanged in meaning; one A/B on H4 | 2–3 |
| **H6** | Findings difference in edit replies | `findings: {new: […], resolved: n}` on `apply` replies, one line per finding | `board_chat` drops its separate `project.issues` reads; DRC-fix task on the bench | 4–6 |
| **H7** | Net-intent check | `schematic.check_intent {nets: {NAME: [terminals]}}` (read): missing, extra, wrong net, and the operations that close the gap | The sub-circuit wiring task completes in one read, one check and one apply; wrong intents refused naming the terminal | 6–10 |
| **H8** | Line form | The ABI design's line descriptions, generated, as `detail: "line"` and in `find` hits | Every line ≤ 160 B and names its required fields; a local model completes the board-build task on lines first | 6–8 |
| **H9** | WASM strategies | `surface_view`, `encode` and `verify` slots fillable by a plugin capability (effect `none` or `read`) | A WASM view gives byte-identical output to the Rust one; a trap leaves the reply unchanged and named. Needs S9a | 8–12 |
| **H10** | Constraint placer | Design first (2–3 h), then the entry | Place-by-function task on the bench; refusals name the unmet constraint | design 2–3, build 15–25 (estimate after the design) |

H1–H5 take about 21–32 agent hours and need nothing outside this plan. H1, H2 and H5 can run in parallel. H3 follows H2, and H4 follows H3. H6–H8 add about 16–24 hours and are gated on H3's numbers. H9 waits for S9a. H10 waits for its own design and your approval.
