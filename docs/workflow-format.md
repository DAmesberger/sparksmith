# Workflow format

**Status:** contract (slice W0 of the [assistant panel](assistant-panel.md#8-slices)) and runner (W1), 16 November 2026. The types, the load check, the first instance with its method, the runner, the `assist.workflows` and `assist.step_result` entries and the MCP prompts are built; the chat (C1) and the controls' widgets (C2) are not. Code: the contract in `crates/automation/src/workflow/` (below the runner, since the catalog's entries and the MCP prompts are made from the same data; `sparksmith_assist::workflow` re-exports it), the runner in `crates/assist/src/workflow/runner.rs`.

A workflow is a refined process the chat finds, offers and runs, with typed controls inside the conversation. It is data, never code: the app has one generic runner and one closed set of ten controls, and a workflow only chooses among them.

## Where workflows live

```
library/assist/workflows/<id>/        shipped
  workflow.json                       what it is for, inputs, steps, budget, reach, controls
  method.md                           the judgement guide its model steps read
<config>/assist/workflows/<id>/       a person's own, same format; replaces a shipped one of the same id
```

- **Checked on load** (`workflow::load::load_folder`). `workflow.json` passes the format's rules, its `id` is the folder's name, and `method.md` has every heading a model step names. A folder that fails is listed as unavailable with the reason, naming the file and the field, and never runs.
- **Available here** (`Workflow::availability`). Each `needs` module is in this host's catalog with the same major and at least the minor; each entry a tool step or a confirm gate calls exists, belongs to a needed module and has a class within `reach`. Otherwise the workflow is listed as unavailable, naming both versions or the entry. A result card's button whose entry the host lacks (`view.open` headless) is left off the card instead.
- **Digest.** `sha256:` over `workflow.json` and `method.md`, each preceded by its name and length (`load::digest`). A recording names the workflow's id, version and digest.
- **Bounds.** Each file at most 256 KiB; at most 64 workflows a folder.

- **Listed per host** (`load::host`): the shipped library's folder and the person's `<config>/assist/workflows/`, against this program's catalog.

The first instance is `library/assist/workflows/datasheet-to-component/`: `workflow.json` and `method.md`, moved there from `skills/sparksmith-datasheet-part/SKILL.md` in W1 (approved Q4); the skill and `AGENTS.md` point to it.

## `workflow.json`, format 1

Unknown fields are refused, naming them. A field that does not apply to an input's `type` or a step's `kind` is refused too ("is for tool steps, not model").

| Field | Type | Rule |
|---|---|---|
| `format` | `1` | Another number is refused, naming both |
| `id` | text | `a–z`, `0–9`, `-`; at most 64; starts with a letter; the folder's name |
| `version` | `"major.minor"` | |
| `title` | text | 1–60 bytes: the card, the palette, the `/` menu |
| `summary` | text | 1–200 bytes: the chat's index and `prompts/list` |
| `intents` | list of text | 3–6 example requests, each at most 120 bytes |
| `offer_when` | list of signals | At most 8: `{"attachment": "<media type>"}`, `{"workspace": "<name>"}`, `{"issue": "<rule code>"}`. Never text matching |
| `needs` | list | `{"module", "major", "minor"?}`, 1–16, each module once, major ≥ 1 |
| `reach` | object | Module → class (`read`, `show`, `edit`, `ask`). A run's grant is the smaller of the chat's and this; `never` is refused |
| `budget` | object | `requests` (1–1000), `output_tokens`, `total_tokens`? (≥ `output_tokens`), `minutes` (1–1440). A run gets the smaller of this and Settings |
| `inputs` | object | Name → input (below), at most 24 |
| `outputs` | object | Name → the answer shape of a model step (below), at most 16; each used by a step |
| `steps` | list | 1–32 steps (below); the last is the one result step |

The JSON Schema of the format is `workflow::schema()`.

### Inputs

| `type` | Holds | Fields beside `title`, `help`, `optional`, `control`, `default` | Control |
|---|---|---|---|
| `text` | text | `max` bytes (required, at most 64 KiB), `multiline` | `text` |
| `length` | integer nanometres | `min`, `max` | `number` |
| `angle` | integer millidegrees | `min`, `max` | `number` |
| `number` | a finite number | `min`, `max` | `number` |
| `integer` | a whole number | `min`, `max` | `number` |
| `enum` | one option's value, or a list with `multiple` | `options: [{value, title}]` (1–64), `multiple` | `choice` |
| `enum_from` | as `enum`, options from a step's result | `from: {list, value, title?}`, `multiple` | `choice` |
| `files` | a list of paths; in a run, the attached documents (below) | `accept` (media types), `min`, `max` (default 1, at most 16) | `datasheet` |
| `url` | an `https://` URL | | `url` |
| `page_ranges` | `"1-5, 31, 32"` | `of`: the `files` input whose pages | `page_ranges` |

`control` may be omitted (the column on the right); a control that cannot show the type is refused. A `default` is checked like a value. Lengths and angles are integers in the file; the `number` control converts what the person types (`0.25mm`, `10mil`).

### Steps

Every step has `id` (unique; `a–z`, `0–9`, `-`, `_`), `kind` and `title` (its row in the run's step list). Five kinds, and nothing else: anything more complex is a catalog entry or a model step.

| `kind` | Fields | Rule |
|---|---|---|
| `input` | `inputs` | Each input is asked for by exactly one input step, before any pointer reads it. The card shows only what the offer did not fill |
| `tool` | `call`, `params`?, `each`?, `check`?, `retries`? | One `Surface::call`, no model. `each` points at a list; the call repeats per item, read as `/item` |
| `model` | `goal`, `method`, `output`, `requests`?, `check`?, `retries`? | `method` is `method.md#<heading anchor>`; `output` names an `outputs` entry; the step ends with `assist.step_result`. `requests` (≤ the budget) caps the step |
| `review` | `gate: "confirm"`: `control` (`impact`), `plan`, `then: {call, params}` | Waits through `Asker::review`; Apply makes `then` |
| | `gate: "notify"`: `control` (an input control), `show`, `options`?, `change`? | A card with Change, and the run goes on. `options` (with `choice`) lists alternatives; `change` names the earlier model step a change sends the run back to |
| `result` | `items: [{title, from}]`, `actions: [{title, call, params}]` | The last step. Actions are `show` entries (Open in editor) |

`check` names a later tool step whose refusal sends the run back to this step with the refusal's text, at most `retries` times (1–10, default 3).

### Pointers

Strings in `params`, `each`, `show`, `plan`, `from` and `options.list` that start with `/` are pointers:

- `/inputs/<name>…`: an input asked for by an earlier step;
- `/steps/<id>…`: an earlier step's result. Into a model step, the next segment must be a field of its output; into a tool step's reply, the path is resolved at run time;
- `/item…`: the current item of `each`, in that step only.

`{"literal": "/text"}` writes a string that starts with `/` as itself. A pointer to a value that is absent leaves the parameter out.

A `files` input is attached to the run's session as its documents when its input step completes (their text read on this computer), and from then on a pointer reads it as the attached documents, `[{"title", "pages"}]`, never as paths: the datasheet workflow reads with `assist.document_text {"document": "/item/title"}`, which stays out of the `files` module.

### Outputs

```json
"package_plan": {"help": "…", "fields": {"generator": {"type": "text"}, "preset": {"type": "text", "optional": true}, "params": {"type": "object"}}, "example": {…}}
```

Field types: `text`, `integer`, `number`, `boolean`, `object`, `list`, `any`. `assist.step_result` refuses a missing required field, a wrong type and a field the shape does not name, each by its pointer, with the `example` when there is one (`Output::check_value`).

## The runner (W1)

`sparksmith_assist::workflow::runner::run_workflow` drives a workflow's steps in order on a session; its model steps run on any model backend, its cards and events reach the host through `Person`.

- **One turn.** Before its first step after the inputs, the run opens a turn (`project.turn_begin`) labelled with the title and its first text input ("Datasheet → component: TLV75733PDBVR"); every call, the runner's and the model's, carries it, so the run undoes as one step. The turn closes however the run ends; what completed stays.
- **No rights of its own.** Every call is the caller's principal under the caller's grant held to the workflow's `reach` (`Grant::narrowed`), through `Surface::call`. An entry that takes a revision gets the session's current one.
- **input** asks for the step's inputs the run was not given (a card for each, in the step's order); values are checked like a default; Decline and Stop end the run.
- **tool** fills its pointers and makes the call (`each`: once per item, the replies a list).
- **model** is one conversation: the step's goal, its output's fields, the inputs, the earlier steps' results (each cut at 120,000 characters) and its section of `method.md` (`method_section`), with the six verbs. The session holds the step open (`Session::open_step`) and `assist.step_result` checks the answer; an accepted answer is the step's result. A `requests` cap holds the step to that many more requests.
- **review**, `confirm`: an `impact` card from the plan (its `change_counts`, or the length of each list; its findings); Apply makes `then`, Decline ends the run. `notify`: a `choice` card; Change sends the run back to `change` with the person's choice.
- **result**: the result card; a button whose entry the host lacks is left off.
- **check and retries.** A tool step refuses when its call fails; one that a `check` names also refuses when its reply says `"accepted": false` or lists a finding that blocks (`blocks` or `blocking`). The run goes back to the checking step with the refusal's text in its brief, at most `retries` times, then fails.
- **Budget.** The smaller of the workflow's `budget` and the configured limits, counted over all model steps; a `meter` event after each model step.
- **Refused before anything runs:** a workflow this host cannot run, an input it does not have or that does not pass, and a replay recorded with another workflow, version or digest.
- **Recordings** of a run start with one line naming the workflow (`{"workflow": {"id", "version", "digest"}}`, `Recording::for_workflow`); a replay reads it and the runner compares it.

## MCP prompts

`prompts/list` and `prompts/get` are generated from the host's available workflows (`mcp::prompts`): the prompt's name is the id with `_` for `-`, its title and description the workflow's `title` and `summary`, its arguments the inputs in the order the input steps ask for them (required unless `optional`; the description is the input's title and help, and how to write its type as text). `prompts/get` serves `method.md`, then a task: the summary and each given input. A missing required argument, one the workflow has no input for, and text past its input's `max` are refused, naming it. A test holds the order of the tool steps' entries in `method.md`'s procedure.

## `method.md`

Markdown. Each model step reads one section, named by its heading's anchor (GitHub's: lowercase, spaces as `-`, other punctuation dropped; headings inside code fences do not count). The same file is what `prompts/get` serves to an external MCP client (§2.5 of the design), so a section is written for a model with only that section, its goal and its inputs.

## The ten controls

`workflow::Control`; the widgets are built in `crates/ui` (C2). `ControlCard` is the descriptor a card is drawn from and an MCP form is built from (M1); `ControlAnswer` is what the person sends back (inputs, Change, Apply with rows left out, Decline, Stop, Undo run).

| Control | Shows | Takes input |
|---|---|---|
| `text` | one line or multiline, with a byte limit | yes |
| `number` | a number with its quantity (length, angle, number, integer) and bounds | yes |
| `choice` | single or multiple choice among titled options | yes |
| `datasheet` | file picker and drop target; title, page count and size of each file | yes |
| `url` | a URL, fetched only through an Ask | yes |
| `page_ranges` | pages against the file's page count | yes |
| `impact` | a plan's counts, findings and rows to leave out; Apply, Decline | no |
| `progress` | current step, done of total; Stop | no |
| `meter` | requests, tokens and minutes against the run's limits | no |
| `result` | items, findings; Show, Open in editor, Undo run | no |

## The `assist.*` entries

Parameters and replies in `workflow::entries`. `assist.workflows` and `assist.step_result` are in the catalog since W1 (`assist` 1.1); `assist.offer` comes with the chat's cards (C2).

| Entry | Kind, class | Parameters | Reply |
|---|---|---|---|
| `assist.workflows` | query, read | `{text?, limit?, unavailable?}` | `{workflows: [{id, version, title, summary, intents, source, inputs, why_not}]}`; the workflows the host lent the session (`Session::lend_workflows`), else `assist.no_workflows` |
| `assist.offer` | show, show; headless `host.unavailable` | `{workflow, inputs?, why}` | `{card, missing}` |
| `assist.step_result` | query, read; changes no document | `{step, result}` | `{accepted, problems: [{at, message}], example?}`; refused without an open step (`assist.no_step`) or for another step |

## Events

`workflow::event::WorkflowEvent` is what the chat receives about workflows: `offered` (by the model, a signal or the person), `started` (with the digest and the turn's label), `step`, `step_done`, `retry`, `card`, `meter`, `ended` (`done`, `stopped`, `declined`, `limit`, `failed`) and `declined` (Just chat). Text, reasoning and tool-call rows are the loop's own events (L1).
