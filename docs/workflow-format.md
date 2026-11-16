# Workflow format

**Status:** contract (slice W0 of the [assistant panel](assistant-panel.md#8-slices)), 16 November 2026. The types, the load check and the first instance are built; the runner (W1), the chat (C1) and the controls' widgets (C2) are not. Code: `crates/assist/src/workflow/`.

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

The first instance is `library/assist/workflows/datasheet-to-component/workflow.json`. Its `method.md` arrives in W1, when `skills/sparksmith-datasheet-part/SKILL.md` moves there (approved Q4); until then the folder lists as unavailable, and the tests check its headings against the skill.

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
| `files` | a list of paths | `accept` (media types), `min`, `max` (default 1, at most 16) | `datasheet` |
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

### Outputs

```json
"package_plan": {"help": "…", "fields": {"generator": {"type": "text"}, "preset": {"type": "text", "optional": true}, "params": {"type": "object"}}, "example": {…}}
```

Field types: `text`, `integer`, `number`, `boolean`, `object`, `list`, `any`. `assist.step_result` refuses a missing required field, a wrong type and a field the shape does not name, each by its pointer, with the `example` when there is one (`Output::check_value`).

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

Signatures only (`workflow::entries`); they join the catalog in the `assist` module with S6 and W1, declared with `declare_entries!` like every entry, and `cargo xtask surface` records them then.

| Entry | Kind, class | Parameters | Reply |
|---|---|---|---|
| `assist.workflows` | query, read | `{text?, limit?, unavailable?}` | `{workflows: [{id, version, title, summary, intents, source, inputs, why_not}]}` |
| `assist.offer` | show, show; headless `host.unavailable` | `{workflow, inputs?, why}` | `{card, missing}` |
| `assist.step_result` | query, read; changes no document | `{step, result}` | `{accepted, problems: [{at, message}], example?}` |

## Events

`workflow::event::WorkflowEvent` is what the chat receives about workflows: `offered` (by the model, a signal or the person), `started` (with the digest and the turn's label), `step`, `step_done`, `retry`, `card`, `meter`, `ended` (`done`, `stopped`, `declined`, `limit`, `failed`) and `declined` (Just chat). Text, reasoning and tool-call rows are the loop's own events (L1).
