# Assistant panel design

**Status:** revised proposal for founder review, 16 November 2026. It replaces the 9 November proposal, whose eleven recommendations the founder approved (with one exception, no fake ACP agent). Where this revision changes an approved point, it says so. **The chat (C1), its controls and cards (C2), Settings › AI (C3) and the datasheet workflow in the chat (W2) are built; ACP agents (A1) and the later slices are not.** Estimates are agent hours. Related: [extension surface](extension-surface.md), [extension ABI design](design/extension-abi-design.md), [MCP surface](mcp-surface.md), [LLM integration](llm-integration.md), [datasheet assistant](datasheet-assistant.md), [first MCP runs](datasheet-assistant-run-mcp-1.md), [board geometry for an LLM](llm-geometry.md).

## Founder direction (16 November 2026)

> Build the assistant panel. As we already defined, create from datasheet should not be separate but automatically invoked with the prompt. That needs a proper design pass first. If we detect a certain action where we have a refined process for it, we should prompt it inside the chat (e.g. also datasheet path, etc). AI should be modular such that the unified chat notices when we have an optimized process for it and then uses it, and optimized processes can have in-chat controls/inputs, etc.

In one line: **one chat; refined processes ("workflows") are data the chat finds, offers and runs, with typed controls inside the conversation.**

## What exists today

| Piece | State |
|---|---|
| Surface: one catalog, `Surface::call`, principals, grants with module ceilings, `Asker`, one error (ABI S0–S2b) | Built |
| MCP on `2026-07-28`: six verbs (`find`, `describe`, `query`, `apply`, `act`, `show`), headless and live (ABI S3) | Built. `show` serves `view.select` and `view.open` (S5a) |
| MCP prompt `datasheet_to_component` | Built. Generated from the workflow's data since W1: serves its `method.md`; title, description and arguments from `workflow.json` (`crates/automation/src/mcp/prompts.rs`) |
| Three Claude Code runs of that prompt | Done: footprints 0 nm against the sources, every pin right, about 17.5 USD ([run notes](datasheet-assistant-run-mcp-1.md)). Tooling defects T1–T11 found |
| In-app datasheet runner (`crates/assist`): model layer, Anthropic and OpenAI-compatible backends, keychain keys, limits, recording and replay, profiles, PDF text and page images | Built. Since S6 its tools are the six verbs, called through `Surface::call` as the local agent under a reach profile; it still runs in a headless session of its own, not on the open project |
| Workflow contract (W0): `workflow.json` format 1 with its load check, the ten controls, the `assist.*` signatures, the run's events; the datasheet workflow's `workflow.json` | Built ([workflow format](workflow-format.md)) |
| Workflow runner (W1): loader for shipped and user workflows, the step runner with `each`, `check`, `retries` and budgets, one turn per run, recordings naming the workflow's digest, `assist.workflows` and `assist.step_result` (`assist` 1.1), the datasheet workflow's `method.md`, MCP prompts from workflow data; the reach filter in the gate's `Grant` (`gate.reach`) | Built ([workflow format](workflow-format.md#the-runner-w1)). The chat runs it on the window's engine and `design-part` headless (W2) |
| Turns: `project.turn_begin`/`turn_end`, the envelope's `turn`, one labelled undo step per turn (ABI S4a); turns are not counted or capped | Built ([turns](extension-surface.md#turns-s4a)). MCP's verbs do not carry `turn` yet |
| The built-in agent on the surface (S6): the six verbs, `assist.document_text`, `assist.document_page`, `assist.pinout_edit` (with `assist.pinout_preview` and `assist.pinout_apply`), reach profiles as data, leniency in the verb reader | Built |
| Streaming loop (L1): both wire formats stream; Stop between chunks; the loop's events for the chat (text and reasoning pieces, tool-call rows with their state, a meter of requests, tokens and minutes) | Built |
| Jobs, events (ABI S4b); the window module's camera, layers and screenshot (S5b); elicitation (MCP P5) | Not built |
| Chat panel (C1): the column (⇧⌘A, View › Assistant, the palette, the application bar), one conversation per open project, the transcript (messages with their context line, bounded Markdown replies, folded reasoning, tool rows, Ask cards, the turn summary card with Show and Undo turn), the states; each message one turn of the built-in agent on the window's engine (`sparksmith_assist::chat`, `apps/sparksmith/src/assistant.rs`, `app/assistant_host.rs`); the chat's prompt and reach are data (`library/assist/chat-prompt.md`, `reach/chat.json`) | Built. No per-turn caps (founder, 16 November). The model, limits and permissions are Settings › AI's (C3): a question follows its kind's rule (edits, files, publishing to a library, moving the view); an Ask card offers Allow once, Allow for this session, Always allow (written to Settings › AI) and Deny; publishing drafts into the project runs unasked (`unasked` in the reach profile). Replies stream through the loop's events (L1) and Stop acts between chunks. Workflow and control cards (C2) are further transcript items |
| Settings › AI (C3): endpoints with keychain keys, agents as configuration, permissions per kind of action (ask every time, once a session, always allow, deny; publishing into the project never asks), optional limits on model runs, off by default so the chat can build a whole board, while a workflow brings its own budget, profiles (copy to edit, checked on save), workflows listed with why one can't run | Built. Stored in `<config>/assist/settings.json`, apart from the app's settings; starting values in `library/assist/settings.json`. A workflow run takes the assistant's endpoint from here and runs on the workflow's budget, within the person's limits when set. No agent is started yet (A1) |
| Controls and cards (C2): the ten controls, the workflow card (a compact confirmation of its inputs: Just chat, Accept and continue), a run's block (steps, meter, Stop), the `/` menu's rows, in `crates/ui` (`sparksmith_ui::chat_cards`); the impact card answers a plan (Apply, Decline, rows left out) or a call that asks (Allow once, Allow for this session, Always allow, Deny); `assist.offer` (`assist` 1.2), checked against the lent workflows and shown through the window (`WindowHost::offer`); `offer_when` signals (`workflow::offer`); palette entries generated from workflows; PDF drop | Built. The adapter (`apps/sparksmith/src/workflow_cards.rs`) keeps the cards; each is a transcript item of the chat (`Item::Workflow`), and a new one opens the chat. The `/` menu shows above the composer while it starts with `/`. The `@` list shows above it while an `@` is typed. Open in editor and Change… run in the window; Accept and continue, Just chat, a run's answers, Stop and Undo run go through the cards' outbox to the run host (W2). The chat lends its session the workflows, so the model can call `assist.offer` |
| Datasheet workflow in the chat (W2) | Built. Accept and continue starts the run off the UI thread (`apps/sparksmith/src/app/assistant_run.rs`, `assistant/worker.rs`) on the one runner: its calls and its work on the chat's session go to the window through a `Desk` (`sparksmith_assist::desk`) and run between frames on the window's engine, through the chat's gate and Ask cards, in the run's one turn. The run waits on the generator choice (Continue, or Choose another… and Change) and on the pinout's impact card (Apply or Decline); Stop ends it at the next step boundary; the result card opens the drafts in the editor and undoes the whole run as one step. Just chat and the run's end and result are told to the model with the person's next message. Accept and continue waits while a message's turn runs. The Component start page's Create from datasheet… opens the chat on the workflow card. `sparksmith design-part` runs the same workflow headless (Apply without asking, the model's generator stands); a recording that names no workflow (the reference conversations in `examples/`) replays as the conversation it holds. Ask cards use the impact card's ask mode |

## What this revision supersedes

| 9 November design | Now | Why |
|---|---|---|
| §1: the endpoint loop is an in-process MCP client of `mcp::Server` through a "panel channel" | The built-in loop calls `Surface::call` in process as `LocalAgent`, with the six verb definitions, byte-equal to MCP's. ACP agents still connect over `mcp --live` on a per-conversation channel | ABI decision 2, approved 14 November |
| `Engine::join_since` and undo labels | `begin_group` / `end_group` with a label (ABI S4) | ABI design |
| `window.context` | `view.context` (ABI S5) | ABI design |
| Tools converted from `tools/list`; `instructions: short`, `tools: compact` | The six verbs; small models read the generated line form | ABI §2.2 |
| §4: workflows are MCP prompts written in the automation crate | Workflows are versioned data in the shipped library. The MCP prompt is generated from that data, and so is the chat's form | Founder, 16 November; no hard-coded data |
| Five prompts listed (`fix_issues`, `route_nets`, …) | One workflow ships first (datasheet → component). Others are data added later, each its own decision | Keep scope lean |
| `document.text`, `document.view_page`, `review.submit` | `assist.document_text`, `assist.document_page`, `assist.pinout_edit`, `library.review_check` (built) | ABI §2.7 |
| The datasheet dialog stays beside the panel | Removed; its run options move to Settings › AI | Founder, 16 November |
| §7 slices S1–S11, 47–72 h | §8 below | |

Everything else carries over: ACP launch, sign-in, streaming, cancel and approvals; endpoint settings and keys; the column, its fold and its states; attachments. Those sections are kept in shortened form in §6 and §7.

---

## 1. One chat, the only AI entry point

- **One conversation per open project,** in a docked column (§7). It is the only place in the app where AI runs. No other dialog starts a model.
- **Two connection kinds,** as approved: an ACP agent (Claude Code, Codex, Gemini CLI, OpenCode; their own login) or a model endpoint (Anthropic, OpenAI-compatible, local servers) through `crates/assist`.
- **The chat can do everything** the app can: "Claude Code for PCBs" (founder, 16 November). It plans, makes parts, places, wires the schematic, routes, checks and exports, through the six verbs and the same catalog the person's own actions use. Nothing is out of its reach by design; what it may do without asking is the person's permissions (§6.4).
- **Permissions per kind of action,** as a coding agent's harness asks: changing the design, files outside the project, publishing to a library, moving the view. Each kind is Ask every time, Ask once a session, Always allow or Deny; an ask offers **Allow once**, **Allow for this session**, **Always allow** and **Deny**. Publishing a part into the open project never asks; publishing to a library asks unless the person chose otherwise. The gate decides at the call, never the model's wording.
- **No turn limits.** A message may run as long as the work takes: a whole board in one conversation. What keeps a long conversation inside the model's context is compaction (§6.5), not a cap. The person's optional run limits (§6.3) and a workflow's own budget are the only stops besides **Stop**.
- **Plain chat** works on the board: the model reads `view.context` (what you see and have selected), uses the six verbs, and each of your messages is one undo step.
- **Workflows** are the refined processes. The chat offers one when it fits (§3), the person confirms, and the workflow runs inside the conversation with its own controls (§4). A workflow run is one turn and one undo step.
- **Every other entry point opens the chat.** The Component start page's "Create from datasheet…" button, the command palette and the `/` menu all open the chat with that workflow's card. Dropping a PDF on the chat offers it too.
- **External MCP clients** (Claude Code, Codex) get the same workflows as MCP prompts, generated from the same data, and the same inputs as forms where they support elicitation (§4.3).

## 2. Workflows as data

### 2.1 Where they live

```
library/assist/workflows/
  datasheet-to-component/
    workflow.json      what it is for, inputs, steps, budget, reach, controls
    method.md          the judgement guide for its model steps (today's SKILL.md text)
<config>/assist/workflows/<id>/   a person's own or changed workflows, same format (like run profiles)
```

- **No workflow logic in the app.** The app has one generic runner and one closed set of controls. Which inputs, which entries, in which order, with which gates and budgets is all in `workflow.json`.
- **Format 1, checked on load.** A workflow that fails the check is listed as unavailable with the reason and never runs. Unknown fields are refused, naming them.
- **Versioned.** `version` is `major.minor`. A recording names the workflow id, version and the digest of its files, so a replay against another version is refused clearly.
- **`needs`** names the surface modules and versions it uses (`{"module": "library", "major": 1, "class": "edit"}`), the same shape as a plugin's. A host that can't meet them lists the workflow as unavailable, naming both versions. `cargo xtask surface` already makes module versions honest.
- **`reach`** is the workflow's ceiling per module. The effective grant of a run is the smaller of the chat's grant and the workflow's reach, so the datasheet workflow can't publish, save or touch settings, whatever the model is told.
- **Single source for the method.** `skills/sparksmith-datasheet-part/SKILL.md` moves to `method.md` and the skills folder points to it (Q4). The MCP prompt serves `method.md`, as it serves the skill today.

### 2.2 What a workflow says

| Field | Meaning |
|---|---|
| `id`, `version`, `title`, `summary` | Identity, and the one line the chat and `prompts/list` show |
| `intents` | Three to six example requests, so the model can match ("make a part from this datasheet") |
| `offer_when` | Structured signals that offer the card without a model (§3.3): an attachment type, a workspace. Never text matching |
| `needs`, `reach` | Modules and versions it requires; its per-module ceiling |
| `budget` | Requests, output tokens, total tokens, minutes. A run gets the smaller of this and the person's run limits, when set |
| `inputs` | A typed schema: each input's type, title, help, bounds, default and control (§4) |
| `steps` | The ordered steps (§2.3) |
| `result` | What the result card shows and which `show` actions it offers |

### 2.3 Steps

Five kinds, run in order by the generic runner. No loops or expressions beyond what is listed; anything more complex is either an entry in the catalog or a model step.

| Kind | Runs | Example |
|---|---|---|
| `input` | Shows controls for some inputs; waits for the person | Datasheets, part name |
| `tool` | One `Surface::call` by entry name, params filled from inputs and earlier results (JSON pointers). `each` repeats it over a list input. No model involved | `assist.document_text` per datasheet; `library.generate_part`; `component.check_drafts` |
| `model` | The model works on a goal with the verbs, within the run's reach and a step budget. Its guide is a section of `method.md`. It ends by calling `assist.step_result`, which checks its answer against the step's `output` schema | Read the drawings and choose generator, preset and values; write the pinout document; write the review report |
| `review` | A person gate. `confirm` waits (impact preview, Allow/Deny); `notify` shows a card with a Change button and goes on | The generator choice (notify); the pinout import (confirm) |
| `result` | The result card | Drafts, counts, open findings, Open in editor |

A `tool` or `model` step may name `check`: a following tool step whose refusal sends the run back to that step with the refusal's text, at most `retries` times (default 3). That is how "preview until no blocking finding" and "review_check until accepted" are expressed.

### 2.4 The datasheet workflow, as data (shortened)

```json
{"format": 1, "id": "datasheet-to-component", "version": "1.0",
 "title": "Datasheet → component",
 "summary": "Draft a component, symbol, package, footprint, binding and 3D model from manufacturer datasheets, every value cited, for you to review and publish.",
 "intents": ["make a part from this datasheet", "create the component for TLV75733", "add this chip to my library"],
 "offer_when": [{"attachment": "application/pdf"}],
 "needs": [{"module": "library", "major": 1}, {"module": "generators", "major": 1},
           {"module": "checks", "major": 1}, {"module": "assist", "major": 1}],
 "reach": {"library": "edit", "generators": "edit", "checks": "read", "assist": "edit", "view": "show"},
 "budget": {"requests": 40, "output_tokens": 200000, "minutes": 45},
 "inputs": {
   "datasheets": {"type": "files", "accept": ["application/pdf"], "min": 1, "max": 4, "control": "datasheet", "title": "Datasheets"},
   "part":       {"type": "text", "max": 64, "title": "Part name", "help": "The manufacturer part number"},
   "notes":      {"type": "text", "optional": true, "multiline": true, "title": "Notes", "help": "Package variant, what to leave out"}},
 "steps": [
   {"id": "ask",     "kind": "input",  "inputs": ["datasheets", "part", "notes"]},
   {"id": "read",    "kind": "tool",   "call": "assist.document_text", "each": "/inputs/datasheets", "params": {"document": "/item/title"}},
   {"id": "gens",    "kind": "tool",   "call": "library.generators", "params": {}},
   {"id": "plan",    "kind": "model",  "method": "method.md#find-a-generator", "output": "package_plan", "check": "make"},
   {"id": "pick",    "kind": "review", "gate": "notify", "control": "choice", "show": "/steps/plan/generator"},
   {"id": "make",    "kind": "tool",   "call": "library.generate_part", "params": {"generator": "/steps/plan/generator", "preset": "/steps/plan/preset", "params": "/steps/plan/params", "name": "/inputs/part", "scope": "component"}},
   {"id": "drafts",  "kind": "tool",   "call": "library.draft_closure", "params": {"component": "/steps/make/component"}},
   {"id": "pinout",  "kind": "model",  "method": "method.md#procedure", "output": "pinout_document", "check": "preview"},
   {"id": "preview", "kind": "tool",   "call": "library.pinout_preview", "params": {"target": "/steps/drafts/component", "document": "/steps/pinout/document"}},
   {"id": "import",  "kind": "review", "gate": "confirm", "control": "impact", "plan": "/steps/preview", "then": {"call": "library.pinout_apply"}},
   {"id": "check",   "kind": "tool",   "call": "component.check_drafts", "params": {}},
   {"id": "report",  "kind": "model",  "method": "method.md#the-review-report", "output": "review_report", "check": "accept"},
   {"id": "accept",  "kind": "tool",   "call": "library.review_check", "params": {"report": "/steps/report/report"}},
   {"id": "done",    "kind": "result", "show": ["drafts", "unreviewed", "uncertain", "findings"], "open": "view.open"}]}
```

Field names are illustrative; the contract slice (W0) fixed them in [workflow format](workflow-format.md), and the shipped file is `library/assist/workflows/datasheet-to-component/workflow.json`. The deterministic parts (reading text, listing generators, generating, drafting, previewing, checking) run without the model, which is what makes the flow cheaper and steadier on small models than the free-form prompt: the model only does the three judgement steps.

### 2.5 The steps for an external MCP client

An external client has its own loop and no runner. `prompts/get datasheet_to_component` returns `method.md` plus the inputs as a task, exactly as today, so Claude Code keeps the text that produced the 0 nm results. A test checks that every entry a `tool` step names appears in `method.md`'s procedure, in the same order, so the two can't drift silently.

## 3. How the chat notices a workflow

### 3.1 The model chooses (default)

- **The index is in context.** The chat's system text carries the workflow index: id, one-line summary and the intents, about 60 tokens a workflow, held under 1 KB by a test. Past that size the index is left out and the model calls `query assist.workflows {text}`, which ranks by the same fields.
- **The model offers, the person decides.** When a request fits, the model calls `show assist.offer {workflow, inputs, why}` with every input the conversation gives: the message's files and the values it states or a file name carries (a part number). Without a model, the chat fills each `files` input from the latest files the person attached (`@`, a typed path, a drop, a pick) whose type it takes (`Workflow::fill_files`). The chat draws one compact confirmation:

  ```
  ┌ Datasheet → component ─────────────────────────────── v1.1 ┐
  │ You asked for a part from its datasheet.                   │
  │ Datasheets  [tlv757p.pdf · 38 pages · 2.5 MB ⌄]  [Change…] │
  │ Part name   [TLV75733PDBVR                    ]            │
  │                 [Just chat]  [Accept and continue]  (Enter)│
  └────────────────────────────────────────────────────────────┘
  ```

  Each row is the workflow's start input (the inputs of the input steps before its first other step), changed in place: ⌄ lists the files attached in this session and the project folder's, Change… opens the picker, a dropped file joins the value. A required input nothing filled is an empty inline field, and the keyboard goes to it; an optional one nothing filled is left out. When the card appears it takes the keyboard from the composer, so Enter accepts. A later offer of the same workflow fills the waiting card instead of adding one, leaving what the person changed.
- **Accept and continue** starts the run with those values; its input step asks nothing more. **Just chat** returns to free conversation and tells the model so.
- `assist.offer` is a `show` entry: it changes no document and needs a window, so headless it answers `host.unavailable`.
- No keyword matching in code. What the model reads is the workflow's own data.

### 3.2 Explicit entry

- **`/` in the composer** lists the workflows (title and summary). Choosing one shows its card, filled from the conversation.
- **Command palette:** one entry per available workflow, generated from its data ("Assistant: Datasheet → component").
- **Component start page:** "Create from datasheet…" now opens the chat with that card.
- **MCP clients:** `prompts/list` (Claude Code shows prompts as slash commands).

### 3.3 Small and local models

- **Signals from data, not words.** `offer_when` lists structured signals: a dropped file's type, the workspace, an Issues row. When one matches and the model hasn't offered anything, the chat shows the card itself. Dropping a PDF therefore always offers the datasheet workflow, on any model.
- **The `/` menu and the palette** need no model at all.
- **Profile option `workflow_choice: "classify"`.** One extra small request before the turn: the message and the index, answered as a JSON enum of workflow ids or `none`. Measured on Flash Next and `qwen3.8:27b` before it is offered (slice D1).
- **Inside a workflow,** the deterministic steps carry the load, and each model step sees only its own goal, its section of `method.md` and its inputs, which keeps a 32 k context workable.

## 4. In-chat controls

### 4.1 One closed set of controls

The widgets are code, built once in `crates/ui` from `ui_kit::Container` and the shared tokens (`cargo xtask ui-containers` holds that). A workflow only chooses among them and fills their fields. A workflow naming a control the app doesn't have fails the load check.

| Control | Shows | Input type in `workflow.json` |
|---|---|---|
| `text` | One line or multiline, with a byte limit | `text` |
| `number` | A number with its unit; accepts `0.25`, `"0.25mm"`, `"10mil"` (the command wire) | `length`, `angle`, `number`, `integer` with `min`, `max` |
| `choice` | Single or multiple choice, titled options; options may come from a step's result (generators and presets from `library.generators`) | `enum`, `enum_from` |
| `datasheet` | File picker and drop target; shows title, page count and size; copies the file into the conversation's folder, which is in the grant | `files` with `accept`, `min`, `max` |
| `url` | A URL, fetched only through an Ask (network reach) | `url` |
| `page_ranges` | `"1-5, 31, 32"` against the file's page count (the datasheet workflow has none: its Package step finds the pages) | `page_ranges` |
| `impact` | A plan's counts, findings and changed rows; a multi-select of rows to leave out; **Apply** / **Decline** | `review` steps with `gate: confirm` |
| `progress` | Current step, done of total, **Stop** | every running step |
| `meter` | Requests, tokens and minutes against the run's limits | always, for a running workflow |
| `result` | What was made, counts, open findings; **Show**, **Open in editor**, **Undo run** | `result` steps |

### 4.2 How a control reaches the person

- **In the chat,** an input step's controls are one compact confirmation in the conversation, as the offer's (§3.1). Enter accepts, Esc declines; the keyboard goes to the first input to fill in.
- **Gates go through `Asker`.** A `review` step with `gate: confirm` calls `Asker::review(plan)`; the chat's `Asker` draws the `impact` card. The same trait answers Ask-class calls (an Allow/Deny card). So a workflow's gate and the surface's gate are one mechanism.
- **Typing in the chat isn't "busy".** The busy rule refuses agent edits while the person types on the canvas or in a dialog; the composer and the chat's cards are excluded.

### 4.3 The same inputs for an MCP client

| Control | MCP `2026-07-28` | Without elicitation |
|---|---|---|
| `text`, `number`, `choice` | Form elicitation: string, number, titled enum | Prompt arguments (strings) |
| `datasheet`, `url` | String path or URL, with the allowed folder in the help | Prompt argument |
| `page_ranges` | String with a pattern | Prompt argument |
| `impact` | `input_required` with the counts as the message and a titled multi-select of rows (MCP P5) | Headless: `gate.needs_person`. Live: the window's review |
| `progress`, `meter` | `notifications/progress`; the client's own usage display | — |
| `result` | Text and a resource link; the model may call `show view.open` when live | Text |

`prompts/get` without a required argument answers `input_required` with the form built from `inputs` (MCP §3.7). Forms and prompt arguments are generated from the workflow data, so nothing is written twice.

## 5. How it sits on the surface

### 5.1 Callers and grants

| Who | Principal | Grant |
|---|---|---|
| Chat on an endpoint (our loop) | `LocalAgent`, in process | `Grant::local_agent`: document modules to Edit; `view` to Show; `files`, `fab`, `model3d` to Ask; `settings` read |
| Chat on an ACP agent | `RemoteAgent` over `mcp --live` on the conversation's channel | The window's grant, as approved (§6) |
| A workflow run, either kind | The same principal | The above, narrowed by the workflow's `reach` |
| The workflow runner's `tool` steps | The same principal as the chat | Same: the runner has no rights of its own |

Every call, the runner's included, is one `Surface::call` with the envelope's `turn`. Nothing in the chat calls the engine directly.

### 5.2 Turns, previews, undo

- **A message is a turn; a workflow run is one turn.** The chat opens it with `begin_group(label)`: "AI assistant: route the USB pair", "Datasheet → component: TLV75733PDBVR". It undoes as one step. A person's own edit in between splits it, as approved.
- **Previews are dry runs.** An `impact` card is the dry-run reply (`dry_run: true`). Apply sends the same call at the preview's revision, and the plan digest (`confirm`) once the plan slice serves it. A moved revision gives a fresh preview, never a silent apply.
- **Stop** ends the turn: the runner stops at the next step boundary, a long entry gets its cancel through `Progress`, and what completed stays as one undo step with **Undo run** on the card.
- **A turn groups undo and nothing else.** The surface does not count or cap a turn's calls or edits, on either connection kind.

### 5.3 The window

- `query view.context` at the head of each message: workspace, view, selection with ids, visible layers, steps made by hand since the last turn.
- `show view.select`, `view.open` (a workspace, a sheet, the Component editor on a draft) and `view.camera`. The result card's **Open in editor** is a `view.open`. Shown changes take no revision and no undo step.
- Events (`project.changed`, `view.selection`) refresh the chat's context line and its "changed by hand" note.

### 5.4 Prerequisites

| Slice | From | Needed for the first demo? | What for |
|---|---|---|---|
| S4a turns: `begin_group`/`end_group`, labels | ABI plan | **Yes** | One undo step per message and per run |
| S4b jobs, `Progress`, events | ABI plan | Partly: `Progress` for Stop on a long entry; events can follow | Stop, "changed by hand" |
| S5a `WindowHost`, `view.context`, `view.select`, `view.open` | ABI plan | **Yes** | "Talk to the board", Open in editor |
| S5b camera, layers, screenshot, files through the window's prompts | ABI plan | No | Later |
| S6 built-in agent on the surface, `assist.*` entries, leniency in the verb reader | ABI plan | **Yes** | The chat loop and the document entries |
| Plans with `confirm` digests | ABI S4 / MCP P5 | No: the demo binds a preview to its revision and the pinout plan's `base` | Digest-bound review |
| P5 elicitation (`input_required`) | MCP plan | No | Forms in external clients |
| P4 prompts and skills from data | MCP plan | Only the prompt part, in W1 | |

The ABI plan's hours for S4–S6 are counted there, not again below. This plan splits S4 and S5 so the demo needs only their first halves.

## 6. Connections (kept from 9 November, shortened)

### 6.1 ACP agents

- **Setup:** Settings › AI › Agents: name, command, arguments, environment; no shell. Presets are editable templates (`claude-agent-acp`, `codex-acp`, `gemini --acp`, OpenCode through its own config); nothing is probed or detected. **Test** sends `initialize` and shows `agentInfo`, `authMethods` and capabilities.
- **Process:** its own process group; Stop, closing the panel or window and quitting end it the same way (close stdin, TERM, KILL).
- **`initialize`:** protocol `1`; no file system and no terminal offered, because the agent works through our tools.
- **Sign-in:** the agent's own. On `-32000` the chat shows its `authMethods` (a command with Copy, a button for `authenticate`, or the variable to set). Sparksmith never holds an ACP agent's key.
- **`session/new`:** `cwd` is an empty folder of ours; `mcpServers` has one stdio entry, `sparksmith mcp --live --port P --token-file F`, under a random server name. The token file is private and fresh per conversation. Starting a conversation is the consent for its own channel; Help › Connect stays separate for external clients.
- **Options and system text:** `configOptions` through `session/set_config_option`, remembered per agent; system text through `systemPrompt` when advertised, else the first prompt block. Adapter-specific options (Claude's `disallowedTools`) live in the agent's profile.
- **Streaming:** `agent_message_chunk`, `agent_thought_chunk` (folded), `tool_call` rows (folded into our own row for the same call), `plan`, `usage_update` for the meter.
- **Cancel:** `session/cancel`, open permission requests answered `cancelled`, held Asks declined, the turn closed; **End agent** after 10 s.
- **Approvals:** questions about our own tools are answered `allow_once` (our gate decides when the call arrives); questions about the agent's own tools show its options; our gate's Ask shows as a card.
- **Workflows on an ACP agent.** Our runner drives the steps. A `model` step is one `session/prompt` with the step's goal, its `method.md` section and its inputs; the agent calls our verbs over its channel and ends with `assist.step_result`. `tool` steps and gates run in our runner as for an endpoint. So the controls, gates and budgets are the same on both kinds (Q6).

### 6.2 Endpoints

- **Settings › AI › Endpoints:** provider, base URL, model id, key (masked, in the system keychain for that base URL, refused over plain HTTP to another computer), timeout, and declared capabilities (tool calling, vision, native PDF, context, output limit). Nothing is detected.
- **Loop:** the datasheet runner's loop, generalised in S6 and given streaming (L1), so Stop acts between chunks rather than after a whole request.
- **Profiles** per model (`library/assist/profiles/`, user copies in `<config>/assist-profiles/`): system additions, `omit_tools`, repeat limit, `workflow_choice`, per-workflow `method` additions. Edited in Settings › AI › Profiles as text, checked on save.
- **No vision:** images refused with the reason; PDFs reach the model as text through `assist.document_text`, and page images only where vision is declared. **No tool calling:** the endpoint is refused for the chat, with the reason.
- **Full context:** compaction (§6.5) condenses older turns before the context fills. Only a single request that cannot fit even after compaction stops, with the numbers.

### 6.3 Run limits

Settings › AI, the same for every model, **off by default** so the chat can build a whole board. When the person turns them on they hold every model run, the chat's and the workflows'; a workflow's `budget` sets its run's limits, held within the person's when set.

| Limit per run | Starts at, when turned on | Counted |
|---|---|---|
| Model requests | 40 | Endpoint loop |
| Output tokens | 200 000 | Endpoint loop, worst case checked before each request |
| Prompt plus output tokens | 4 000 000 | Endpoint loop |
| Time | 45 min | Loop and chat |

An ACP agent's tokens are its own; the meter shows its `usage_update`, and the time limit still holds. There is no cap on edits or calls. The product shows no prices (decision of 9 November).

### 6.4 Settings › AI

One page, in the shared settings containers. First, **Connection** (founder, 16 November: connecting a model must be obvious and take seconds): **Use** picks Anthropic, OpenAI, a model server on this computer, or an agent installed on this computer (Claude Code, Codex, Gemini CLI, OpenCode; the choices and their order are `library/assist/settings.json`'s presets). A hosted model shows one **API key** field (masked; **Paste**; **Show** while typing only), the **Model** (Anthropic's starts at `assist.json`'s default model; OpenAI's is typed, since its ids change), **Test connection** and **Save**; no address or other server detail. A model server on this computer asks for its **Server address** and no key. An agent is one click (**Use Claude Code**), with no command form. **Test connection** sends one request of a few tokens (at most 16 in reply, no tools, no reasoning) off the UI thread (`model::check_connection`) and says which model answered. **Save** keeps the key in the system keychain and makes the connection the chat's; afterwards the row reads "Key stored in the system keychain · Replace · Remove". The chat's not-connected state says "No model connected" with one button, **Connect a model…**, which opens Settings › AI with Connection in view and the key field focused; after Save the chat is ready.

Folded below it: **Advanced** (every connection, with its server address, timeout, declared capabilities, context size, and an agent's command, arguments and environment), **Permissions** (per kind of action, §1; an optional override per agent), **Run limits**, **Profiles**, and **Workflows** (read-only list: id, version, shipped or user, available or why not). Visible labels use plain words; the exact terms (base URL, endpoint, ACP) are in tooltips and here. Keys, endpoints, modes and grants are Never-class: no agent can read or set them.

### 6.5 Context compaction

No turn limits (founder, 16 November): instead, as a conversation approaches the model's context window, older turns are condensed so the chat can go on building.

- **When.** At a turn boundary, when the estimated prompt passes 70 % of the context the connection declares (`context_tokens`; an ACP agent compacts its own and reports it). It condenses down to about 35 %, so it runs rarely and the prompt cache is rebuilt rarely. The person's latest turns stay word for word.
- **What is kept.** The person's goals and decisions (including what they declined and why); what was built, with object ids, part and library ids and file paths; open issues; the current plan and its remaining steps; the last `view.context`.
- **What is dropped.** Bulky tool output: query replies, listings, check reports, document pages. The design can be re-queried through the verbs at any time, so a dropped reply leaves a stub naming the call that made it (`query layout.nets {…}`, revision 41) and the model asks again if it needs it.
- **Visible in the chat.** A divider row, "Condensed 14 earlier turns · 182 k → 61 k tokens", which unfolds to the kept record. The transcript the person sees is unchanged; only what the model is sent shrinks. **Undo** is untouched: compaction changes no document.
- **Inside one long turn** (a whole board from one message) the turn's own older tool results are cleared the same way between requests; its record and plan stay.

**Approaches.**

| Approach | How | For | Against |
|---|---|---|---|
| A. Summarise history | One model request condenses older turns into prose | General; no schema | Costs a request; lossy where it matters most (ids, values); varies by model |
| B. Structured state | A typed record: goals, decisions, built objects (ids, files), open issues, plan and steps. The host fills what it knows from the turn summaries (changes by collection and id, issues new and resolved, files written); the model fills goals, decisions and plan through one `assist` entry, checked against the record's schema | Ids and files exact; small; the same on every model | A schema to keep; the model must keep its part current |
| C. Clear old tool results | Replace tool replies older than the kept turns with a stub naming the call | Deterministic, no model request, biggest saving (tool output is most of the prompt); loses nothing the verbs can't give back | Does not shrink the conversation's prose |

**Recommendation:** C always, then B as the kept record, and A only for the conversation's remaining prose, written into the record's notes. C and B need no model request for their deterministic parts, which matters on small local models. Provider features (server-side context editing) may replace C on a connection that declares them; the result must read the same.

**As built (K1, 16 November 2026, after E2E run 12 finding 12).** `sparksmith_assist::chat::context`, policy `Policy::CHAT`:
- **Small replies.** A tool reply over 12 000 characters is shortened as JSON: lists keep their first items and count the rest, long texts and deep objects are cut, and `_shortened` says how to ask for less. Before, a reply could be 120 000 characters.
- **No re-sending.** A reply of 1 500 characters or more that is older than the last 2 rounds of calls is replaced by a stub naming its call. These replies are cleared together once 18 000 characters have piled up, so the prompt cache is rebuilt rarely. An edit's stub keeps the ids and names its reply gave (`It gave: instance 12, designator "U1"`). This also runs inside one long turn, between requests.
- **Condensing at 70 %.** Every earlier reply over 400 characters is cleared the same way. If, at a turn boundary, the prompt is still above 35 %, the turns before the latest are summarised. The summary request uses `library/assist/condense-prompt.md`. It replaces those turns with the kept record and the summary, put in front of the latest message.
- **The kept record (B)** is built from the history alone. It holds what the person asked, every edit with the ids and names its reply gave, the last revision seen and the count of refused calls. The model-filled part, the `assist` entry for goals and the plan, is **not built**; the summary carries them as prose.
- **The divider.** `Event::Condensed` draws the divider row ("Condensed 6 earlier tool replies · 31 k → 12 k tokens"), which unfolds to the record.
- **Caching.** The Anthropic backend already marks the system prompt (tools and system) and the conversation tail for caching, for the chat and the workflows alike.

## 7. The chat UI (kept, with the new cards)

- **Placement:** its own docked column at the right edge, resizable 300–560 pt, can dock left, folds to a 30 pt strip with a state dot (approved Q1). Opened by `assistant.toggle` (⇧⌘A / Ctrl+Shift+A through the shortcut registry), View › Assistant, the palette and an AI button in the application bar.
- **One conversation per open project,** in memory only (approved Q8).
- **Items:** your message with its context line and its files as `@` chips; reply text (bounded Markdown); folded reasoning; one row per tool call (entry, one-line summary, state; expands to input and compact reply); Ask cards; the **workflow card** (§3.1); **control cards** (§4); a **turn summary card** at the end of each turn (what changed by collection and name, issues new and resolved, requests and tokens, **Show**, **Undo turn**).
- **A running workflow** is one block in the conversation: its title, a step list with the current step, the `meter`, **Stop**, and its cards in order.

```
┌ Datasheet → component · TLV75733PDBVR ──────────────── v1.0 ┐
│ ✓ Inputs     tlv757p.pdf · TLV75733PDBVR                    │
│ ✓ Read       tlv757p.pdf · 38 pages                         │
│ ✓ Package    SOT-23-5, TI land pattern on page 32           │
│ ✓ Made       Added: components, symbols, packages, …        │
│ ◌ Pinout     preview 2 of 3 · 1 blocking finding            │
│ · Import  · Check  · Report                                 │
│ 14 requests · 61 k tokens · 6 min of 45           [Stop]    │
└─────────────────────────────────────────────────────────────┘
┌ Import pinout into the drafts? ─────────────────────────────┐
│ 5 pins · 5 functions · 0 rules · 0 uncertain                │
│ Findings: none blocking                                     │
│ [ ] leave out rows…                 [Decline]  [Apply] ⏎    │
└─────────────────────────────────────────────────────────────┘
┌ Done · 1 undo step ─────────────────────────────────────────┐
│ Drafts: component, symbol, package, footprint, binding      │
│ 5 pins unreviewed · 0 uncertain · open: pinout.unreviewed   │
│ Review accepted (18 values cited)    [Save review…]         │
│ [Show]  [Open in editor]  [Undo run]                        │
└─────────────────────────────────────────────────────────────┘
```

- **States** as in the 9 November design: not set up, connecting, sign-in needed, ready, running, waiting for you, stopped or limit reached, agent ended, endpoint unreachable.
- **Attachments:** `@` in the composer lists the files the session attached, then the project folder's and its subfolders', matched fuzzily; ↑↓ and Enter (or Tab, or a click) put the one chosen in the message as an `@name` chip. A path typed or pasted becomes a chip once a space follows it (or when sent), and a drop attaches too. The message carries its chips: the model is told each file's path after the text, and a workflow's files input is filled from them. PDFs are copied into the conversation's folder; an ACP agent also gets them as `resource` or `resource_link`; images only where vision is declared. Documents are untrusted data, never instructions.

## 8. Slices

At most five agents at once. Each slice ships with its tests. Nothing runs the desktop app on the host: UI evidence is in the macOS VM.

| # | Slice | Delivers | Hours | Needs |
|---|---|---|---|---|
| **W0** | Contract | `workflow.json` format 1 as types with its load check; the control vocabulary; `assist.workflows`, `assist.offer`, `assist.step_result` signatures; the chat's event types | 2–3 | — |
| S4a | Turns *(ABI plan)* | `begin_group`/`end_group`, labels | *in ABI plan* | — |
| S5a | Window, first half *(ABI plan)* | `WindowHost`, `view.context`, `view.select`, `view.open`, busy in the surface | *in ABI plan* | — |
| S6 | Built-in agent on the surface *(ABI plan)* | The loop on the six verbs; `assist.document_text`, `assist.document_page`, `assist.pinout_edit`; leniency in the verb reader; `assist::tools` deleted | *in ABI plan* | S3 (built) |
| **L1** | Streaming loop | Streaming on both wire formats; Stop between chunks; loop events for the chat | 3–5 | — |
| **W1** | Workflow runner | Loader for shipped and user workflows; the step runner (input, tool, model, review, result; `each`, `check`, `retries`); budgets; recordings name the workflow and its digest; `assist.workflows` and `assist.step_result`; the datasheet workflow as data with `method.md` moved from `skills/`; `prompts/list` and `prompts/get` generated from workflow data (hard-coded arguments in `prompts.rs` deleted); the method-order test | 8–12 | W0, S6, S4a |
| **C1** | Chat panel | Column, fold, transcript, tool rows, Ask card, turn summary card, states, ⇧⌘A, context line from `view.context`; endpoint backend | 10–14 | S4a, S5a, S6, L1 |
| **C2** | Controls and cards | The ten controls in `crates/ui`; the workflow card and `assist.offer`; `/` menu; palette entries from workflows; `offer_when` signals; PDF drop | 8–12 | W0; C1's shell |
| **C3** | Settings › AI | Connections (endpoints with keychain keys, agents), permissions, limits, profiles, workflows list | 5–7 | W0 |
| **W2** | Datasheet workflow in the chat ★ | The workflow end to end in the chat on an endpoint; result card with Open in editor; `sparksmith design-part` runs the same workflow headless; **the "Create from datasheet…" dialog deleted**, its button opens the chat | 5–8 | W1, C1, C2, C3 |
| A1 | ACP backend | Agents in the chat, workflows driven step by step (§6.1) | 6–9 | C1, W1 |
| M1 | Forms for MCP clients | `input_required` from workflow inputs and `impact` gates (with MCP P5) | 4–6 | W1, P5 |
| D1 | Small-model detection | Measure the index, `offer_when` and `classify` on Flash Next and `qwen3.8:27b`; set the profile defaults from the result | 3–4 | W2 |
| E1 | Evidence | VM runs: plain chat editing the demo board and the datasheet workflow on a local endpoint; an ACP agent on a local model | 2–3 | W2, A1 |
| K1 | Context compaction (**built** without the `assist` entry, §6.5 "As built") | §6.5: the kept record's schema and its `assist` entry; clearing old tool results with stubs; the summary of remaining prose; the trigger and the divider row; tests on recorded long conversations (ids and plan kept, dropped replies re-queried) | 6–9 | C1, L1 |
| R1 | Review on a copy | Accept or reject a large run as a whole (`review-diff`) | 4–6 | C1 |

★ **First demo** after W2: a chat on a local model that answers about the open board, selects and edits through the verbs with one undo step per message, and runs Datasheet → component in the chat with its inputs, the generator choice, the pinout impact card and the result card.

**Order.**

| Round | Agents |
|---|---|
| 1 | W0, S4a, S5a, S6, L1 |
| 2 | W1, C1, C2, C3 |
| 3 | **W2 (demo)**, A1 |
| 4 | K1, D1, M1, E1, R1, and the ABI's S4b and S5b |

**Hours.** This plan's own slices to the demo (W0, L1, W1, C1, C2, C3, W2): about **41–61 h**. After the demo (A1, K1, M1, D1, E1, R1): about **25–37 h**. Total **66–98 h**, plus the ABI plan's S4a, S5a and S6. The critical path is S6 → W1 → W2. More workflows (fix issues, review a schematic) are data: about 2–4 h each, each with its own decision.

### Testing without paid calls

- **Replays.** `crates/assist` already records and replays model conversations. Every workflow test runs a recorded conversation against a headless session; the 16 Flash Next recordings and the TLV and WROOM-3 replays must grade as before (W1, W2).
- **Scripted model in unit tests.** A model backend that answers from a script, for the runner's step logic, gates, Stop, budgets and compaction. It is a test double for the endpoint, not a fake ACP agent (founder, 9 November).
- **Deterministic steps need no model.** The `tool` steps of the datasheet workflow run in tests with fixed `model` step outputs.
- **Local models live:** Flash Next on vLLM and `qwen3.8:27b` on Ollama for the demo and D1. ACP evidence with a real agent on a local model (OpenCode with Ollama).
- **UI:** the accessibility harness with a replayed backend; the VM for end-to-end runs.
- **Claude Code or the Anthropic API** only with the founder's go and a stated budget.

### Risks

- **Two texts for one workflow** (`steps` for the runner, `method.md` for external clients). Mitigation: the order test in W1; one `method.md` section per model step.
- **The step language growing into a programming language.** Five kinds, `each`, `check` and `retries`, and nothing else; anything more becomes a catalog entry or a model step.
- **`assist.step_result` on weak models.** It is one more call to get right. Its refusals name the field and show an example, as `library.review_check` does.
- **Tooling defects T1–T11** cost the external runs most of their tokens. The runner holds the pinout document by handle, which avoids T1 and T2 in the chat, but the MCP route needs the fixes too.
- **ACP agent timeouts** shorter than our Ask wait: measure per agent; decline before the agent's timeout.
- **`sparksmith` must ship beside the app** for ACP agents; the chat says so when it is missing.

## 9. Open questions for the founder

1. **Offer, then confirm.** Should the chat always show the workflow card and wait for **Accept and continue**, or start a workflow directly when the model is sure? **Recommendation:** always the card. One click, and nothing long starts without the person.
2. **Gates in the datasheet workflow.** **Recommendation:** the generator choice as a non-blocking card with **Change**; the pinout import as a blocking impact card; publishing never inside the workflow.
3. **"Token/cost meter".** **Recommendation:** requests, tokens and minutes, against the budget or the person's limits when set; no prices, as decided on 9 November.
4. **One method text.** Move `skills/sparksmith-datasheet-part/SKILL.md` to `library/assist/workflows/datasheet-to-component/method.md` and point the skill and AGENTS.md there? **Recommendation:** yes.
5. **Workflows a person writes.** Load them from `<config>/assist/workflows/`, like run profiles, from the first release, with no editor in the app? **Recommendation:** yes. They are data and can never widen a grant.
6. **Workflows on an ACP agent.** Our runner drives the steps, or the agent gets the whole prompt as an external client would? **Recommendation:** our runner drives, so controls, gates and budgets are the same on every connection.
7. **Time limit.** The person's optional time limit for a run. The WROOM-3 run took 32 minutes on Opus, and a local model is slower. **Recommendation:** raise the default to 45 minutes; the datasheet workflow's budget asks for 45 and 40 model requests (its deterministic steps need no model requests; Flash Next took 11–16 on small parts).
8. **Signals without a model.** Should dropping a PDF always offer the datasheet workflow? **Recommendation:** yes. It is a card, not an action, and it works on any model.
9. **The dialog's removal.** Delete "Create from datasheet…" in W2, before ACP agents arrive? **Recommendation:** yes, once Settings › AI holds the endpoint. An endpoint is enough for the demo.
10. **Where the workflow writes.** As approved: the open project, as one turn. The CLI's `design-part` keeps writing a new project. **Recommendation:** unchanged.
11. **More entry points.** "Ask about this" and "Fix this" on objects and Issues rows. **Recommendation:** after the demo, as workflows with `offer_when` signals, each decided on its own.
