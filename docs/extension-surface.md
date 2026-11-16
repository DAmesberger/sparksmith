# The capability surface

The contract every caller of Sparksmith reaches: the UI, the CLI, MCP, the built-in agent and, later, plugins. It is built in slices from [the extension ABI design](design/extension-abi-design.md); [the founder's decisions](design/decisions-2026-11-14.md) win where the two differ. This page says what exists now (slices S0, S1, S2, S2b, S3 and S4a) and how to change it.

## Entries and modules

An **entry** is one capability with a stable dotted wire name (`layout.add_via`). Today's three families are one catalog: the core's `Operation` (through `project.apply`) and `Query` (through `project.query`), and the automation crate's `Request`. The Rust enums stay the typed implementation.

A **module** is the unit of identity and versioning. Each entry belongs to exactly one, named on its declaration. The modules are declared in `sparksmith_core::permission::MODULES`:

| Module | Holds |
|---|---|
| `project` | inspect, properties, undo, redo, the wrapper requests, BOM |
| `files` | new, open and save a project file |
| `schematic` | sheets, symbols, wires, labels, placed components (`component.*`, `instance.*`), `functions.suggest`, pin reports |
| `layout` | `layout.*`, `fill.*`, `keepout.*`, `board.*`, `diff_pair.*`, `length_group.*`, dimensions |
| `rules` | `rules.*`, `net_class.*`, `impedance.*` |
| `library` | definitions, drafts, publishing, folders and sources, updates (`project.preview_update`, `project.apply_update`, `update`), pinout import, markings |
| `generators` | `library.generators`, `library.generate`, `library.generate_part`, `library.package_from_generator` (1.1) |
| `checks` | `project.issues`, `fabrication.preflight`, the fabrication waivers |
| `fab` | fabrication output, the schematic PDF, firmware pin configuration |
| `model3d` | the exact 3D export, the board interface, the assembly link, part files |

The per-entry assignment is `crates/automation/surface.lock`, generated from the declarations: every entry appears there under exactly one module. Names were not changed; a rename is a breaking change the lock makes deliberate. `view`, `settings`, `assist` and `host` come with the slices that give them entries (S2, S5, S6).

## Declaring an entry

Every command enum lists its variants once, beside the enum, with `declare_entries!` (`crates/editor-core/src/permission.rs`):

```rust
declare_entries!(FillCommand in layout {
    Add "fill.add" Edit "Add a copper fill",
    Refill "fill.refill" Edit "Refill copper" [long],
});
declare_entries!(Request in project {
    ExportFabrication "project.export_fabrication" Ask "Export fabrication files" [in fab, long, link],
});
```

A line is `Variant "wire.name" Class "Title" [attributes]`. A variant without its line does not compile, because the macro also writes the exhaustive `class()` and `command()` matches. The attributes, comma separated, all optional:

| Attribute | Means | Default |
|---|---|---|
| `in module` | another module than the family's | the family's module |
| `short`, `long` | runs under 2 s; long enough for progress and cancel (`time`) | instant |
| `link` | the result can pass 2 KiB (`result`) | inline |
| `review` | the person reviews the impact before it applies (`person`) | none; every action adds approval |
| `reach(Files \| Settings \| Window \| Network \| Runtime)` | what it touches outside the document | none |
| `effect(Pure \| Revision \| World)` | what its answer depends on | `world` with a reach, else `revision` |
| `needs(Files \| Window \| Runtime \| Solidcraft)` | host features it needs | none; a reach of files adds `files` |
| `since(major, minor)` | the module version it arrived in | `0.1` |

The title is at most 40 bytes; the summary is the first sentence of the doc comment (for requests, the catalog's summary), at most 140 bytes. The kind is derived: Never reaches no model; Ask or a path makes an action; else a read is a query and an edit an operation.

The macro builds `ENTRIES: &[EntryDecl]` for each enum. Discovery, the MCP verbs' catalog (`find`, `describe`) and `surface.lock` all read these; no list elsewhere names an entry.

### Paths

A parameter that names a file or folder has the type `sparksmith_core::permission::FilePath`, a string on the wire whose schema carries `"x-kind": "path"`. The gate finds every path a request names by walking the request with its schema (`crates/automation/src/session/paths.rs`), so a request takes a path because its parameters say so. A request that names a path reaches `files`, is at least Ask and is limited to the run's folders. The mark is removed from every schema a caller is shown, so tool lists and `project.schema` replies are what they were.

## Descriptor types

The types are in the ABI crate, `sparksmith_generator_abi::surface` (`no_std` + `alloc`, so a plugin can read what the host writes; no generator module uses them, so module bytes do not change):

- `Class`: `read < show < edit < ask < never`. `show` (the window only: no revision, no undo step) has no entries until S5.
- `Kind`: `query`, `operation`, `show`, `action`, `event`.
- `Effect` (`pure`, `revision`, `world`), `Person` (`none`, `review`, `approval`), `Time` (`instant`, `short`, `long`), `ResultSize` (`inline`, `link`), `Reach`, `HostFeature`.
- `EntryDecl` and `ModuleDecl`: what a declaration says, built in `const` context. Reaches and needs are small bit sets.
- `ModuleInfo` and `EntryInfo`: a module and an entry as described to a caller.
- `Call<P>`: the envelope (`entry`, `params` of the host's JSON type, `project`, `expected_revision`, `dry_run`, `confirm`, `turn`, `job`), and `Error`, the one error shape (`code`, `message`, `at`, `rule`, `closest`, `instead`, `expected_revision`, `actual_revision`). The front door uses both.
- `Lock`, `LockedModule`, `LockedEntry`: the lock file.

## `surface.lock`

`crates/automation/surface.lock` is JSON, one line per entry:

```json
{"format": 1, "modules": [
 {"id": "checks", "version": "0.1", "digest": "sha256:…", "entries": [
  {"name":"fabrication.preflight","class":"read","schema":"3f…","fields":[["profile","9a…"]],"required":[]},
  …
 ]},
 …
]}
```

- **Module digest:** SHA-256 of the module's canonical description: its entries sorted by name, each with its kind, class, effect, reach, needs, person step, time, result size, `since` and the digest of its canonical input schema.
- **Canonical schema:** the entry's parameter schema with the definitions it reaches renumbered in the order they are first referenced, without `description`, `title`, `$comment`, `examples` and `$schema`. A renamed Rust type or an edited doc comment changes nothing; a changed field does. Titles and summaries are not contract.
- **Per entry:** the class, the first 16 hex digits of the schema digest, the same per top-level parameter, and the required parameters.
- **Beside it, `crates/automation/surface.schemas`:** each entry's parameter schema itself and every type it reaches, one line per entry and one per type (a type shared by several entries, such as an operation inside `project.apply` and `project.apply_all`, is recorded once and referenced by name), so the next change can be compared structurally and a changed type reads as one changed line ([what a change needs](#what-a-change-needs)). `--update` writes both files; `--check` fails when either is out of date.

`cargo xtask surface --check` (in `cargo xtask ci`) compares the catalog with the lock. It runs the automation test `session::surface::tests::the_lock_holds`, since the catalog is built from that crate's types. It fails when:

- a module's digest changed and its version did not;
- a version went back, or a module was removed;
- from 1.0, an entry was removed, its parameters changed incompatibly ([what a change needs](#what-a-change-needs)), or a class became stricter, without a major raise;
- an entry is new and its `since` is not the module's new version;
- the lock is otherwise out of date.

`cargo xtask surface --update` rewrites the lock, after the same rules pass. To change the surface, raise the module's version in `MODULES` (and `since` on a new entry), then update the lock.

**Versions.** Every module was `0.x` until the MCP cut-over (S3), where each started at `1.0` (decision 12; `permission::CUT_OVER`). From 1.0 a break raises the major and an addition the minor: a new entry raises its module's minor and declares `since` it (for example `since(1, 1)`). Versions recorded before the structural comparison (for example `project` 6.0, `layout` 2.0) stay as they are.

### What a change needs

Decided 2026-11-16: adding an optional field to an entry's parameters or reply is a minor change. The gate compares the recorded canonical schema of each entry with the current one, structurally and at any depth (`session::surface::compat`). A parameter schema is what a caller sends, so a change is compatible when every value the old schema accepted is still accepted with the same meaning:

| Change (at any depth, in a definition too) | Needs |
|---|---|
| a new entry | minor, declared `since` the new version |
| a new optional field (not required; `Option`, `#[serde(default)]` or skipped when absent) | minor |
| a required field made optional | minor |
| a wider type (`T` to `Option<T>`, `integer` to `number`), a removed or looser bound, a dropped `format`/`pattern` | minor |
| a new enum value or union variant (a new operation inside `project.apply`) | minor |
| a new default where none was | minor |
| an edited doc comment, a renamed type | no version (the digest is the same) |
| reordered fields or variants | minor (the digest changed, the contract did not) |
| a removed or renamed field, entry or variant | major |
| a new required field, an optional field made required | major |
| a narrower or other type, an enum value removed, a tighter bound, a new `format`, `pattern` or path mark | major |
| a changed or removed default (what an omitted value means) | major |
| unknown properties newly refused (`additionalProperties: false`) | major |
| a stricter class | major |
| any schema keyword the comparison does not know | major |

Before 1.0 any change raises the minor. Without a recorded schema for an entry (the first change after the schemas file was added), the old digest rule applies: any retyped parameter is major.

**Wrappers.** `project.apply`, `project.apply_all` and `project.query` carry the union of every operation or query, so a change inside one is also a change to the `project` module. Under these rules a new operation, or a new optional field in one, is a minor raise of `project` as well as of the operation's own module; a break in an operation is a major raise of both. The wrappers' schemas stay in the digest, so a caller of `project.apply` is told the truth about what the wrapper accepts.

**Enum values: widened inputs are minor, widened replies would be major.** A caller sends parameters, so a new enum value or variant takes nothing away from what it could send: minor. In a reply the direction turns: a new value can reach a client that matches every value it knows, so a reply's new enum value would be a break, while a reply's new optional field stays minor (clients ignore fields they do not know). Today the lock records parameter schemas only; replies have no schema in the catalog, so no reply change is gated yet. When reply schemas are recorded they compare with the direction turned for enums and variants.

## What S1 deleted

- `metadata::REVIEWED`, `LONG`, `LINKED` and `OUTSIDE_DOCUMENT`: now `review`, `long`, `link` and `reach(…)` on the declarations.
- `Request::NAMING_PATHS`, the 50-arm `Request::paths` match and the gate tests' `path_samples`: now `FilePath` and the schema walk.
- `declare_classes!` and its `CLASSES` and `TITLES` tables, `Operation::classes()` and `Operation::titles()`: now `declare_entries!`, `ENTRIES` and `Operation::entries()`.

MCP behaviour, the tool lists and their annotations are unchanged.

## The front door

Every call reaches the engine through one function, `Surface::call` (`crates/automation/src/front.rs`), implemented by `Session` (S2). `Session::respond`, `dispatch`, `execute` and `rehearse` are calls through it; the MCP server (headless and live), the live channel, the CLI and the built-in assistant all reach it that way.

- **The envelope.** A `Call` names any entry by its own name: an operation (`layout.add_via`), a query (`project.bom`) or a request (`project.save`), with its `params`. `expected_revision` is the envelope's. An entry takes it when it is an operation or its parameters name one (`EntryMeta::revision`, from the schema); such a call without one is refused (`malformed_request`), and one given in the parameters too must agree. The session checks it once, before any file is read or written, and the engine again. `dry_run: true` runs an edit on a fork and keeps nothing. `turn` names an open turn ([turns](#turns-s4a)). `confirm` and `job` are refused (`host.unsupported`) until S4b and the plan slices serve them; `project` is for a transport holding several sessions.
- **The wrappers.** `project.apply`, `project.apply_all` and `project.query` stay in the catalog as the shapes the JSON-lines transport sends; MCP sends calls by entry name, and its `apply` verb sends a batch as `project.apply_all`. The front door reads `{"command", "params"}` as a call (`front::from_request`), moving the revision to the envelope, and the gate weighs the operations and queries inside a wrapper, each in its own module, never the wrapper. `project.schema` is the catalog lookup the `describe` verb reads.
- **Who calls.** `Principal`: `Person` (the UI), `Operator` (the person's own script), `LocalAgent` (the built-in agent) and `RemoteAgent` (MCP and other model-facing transports). The person and the operator run everything. `Plugin` comes with S8. Transports still name a `Caller` (`Operator`, `Model`, `Allowed`), which maps to a principal and whether a person already allowed the request.
- **Grants.** `Grant` (formerly `Policy`): the mode, the folders and a ceiling class per module. A module absent from the grant is out of reach. `Grant::new(mode, folders)`, what the CLI and MCP flags give, has every module up to Ask; `Grant::local_agent` has the document's modules up to Edit and `files`, `fab` and `model3d` up to Ask.
- **One decision.** `gate::decide_entries`, the same for every principal: the person and the operator run; Never is refused (`gate.never`); a class above the grant's ceiling for the entry's module is refused (`gate.module`); a path outside the folders is refused (`gate.path_outside`); the mode decides Run or Ask. A path makes a call at least Ask; a batch takes its strictest member. An unknown entry is refused before (`catalog.unknown`, with the nearest names in `closest`). `front::tests::one_decision_over_every_entry_and_every_principal` holds it as a table over every catalog entry and every principal.
- **Asking.** What needs a person goes to the host's `Asker`: Allow runs, Deny answers `gate.denied`, and a host with no way to ask (`NoOne`) answers `gate.needs_person`, saying where a person can allow it. A dry run asks no one, since it keeps nothing.
- **One error.** Every failure is `sparksmith_generator_abi::surface::Error`: engine and file diagnostics keep their codes, and gate refusals carry `instead`. On the wire it is a superset of the old diagnostic, so replies keep their shape.

Not yet: the `review` person step does not ask (it comes with plans, `dry_run` and `confirm`), no entry needs a window yet (the MCP `show` verb answers `host.unavailable` headless), and the busy rule stays in the live channel until S5.

## What S2 deleted

- `Request::expected_revision` and its 50-arm match: the envelope's revision and `EntryMeta::revision`.
- `gate::Refusal` and `Policy`: the one `Error` and `Grant`.
- `Session::admit` and the gate's own path resolution: `Surface::decide` and `Session::gate`.

## The MCP cut-over (S3)

`sparksmith mcp` and `sparksmith mcp --live` serve the catalog on MCP `2026-07-28` only, through six verbs: `find`, `describe`, `query`, `apply`, `act` and `show` (`crates/automation/src/mcp/tools.rs`). `tools/list` is the verbs and nothing else (3,398 bytes, held under 3.5 KB by a test); every entry, the generator entries included, is data behind them. Each verb call becomes one `Call` through `Surface::call` as a remote agent:

- **Headless**, on the server's own sessions (project handles, `docs/mcp-surface.md` §3.2). Nobody can be asked, so a person-owned step is refused with `gate.needs_person`.
- **Live**, sent to the window as the same envelope (the live channel's `surface.call`); the window decides with its own grant (`live::grant`: every module but `files`, so opening, saving and starting a project stay the person's, refused with `gate.module`) and asks the person in the window.
- **`show`** steers the window. No entry has the show class yet (S5), so headless it answers `host.unavailable`.

The `datasheet_to_component` prompt serves `skills/sparksmith-datasheet-part/SKILL.md`. [MCP surface, as built: S3](mcp-surface.md#as-built-s3-the-cut-over-14-november-2026) has the detail.

## What S3 deleted

- The old MCP server, its 40 per-command tools, `tool_name`, `INSTRUCTIONS` and `LIVE_INSTRUCTIONS`, and its protocol versions.
- The list of requests MCP hid (the generator requests) and the 64 KiB `tools/list` test: the six verbs have a fixed size.
- `live_tools` and `live::WINDOW_ONLY`: the window's grant leaves out the `files` module.
- The live channel's `engine.execute` `{command, params}`: it carries a `Call`.
- `CommandInfo::permission_note` and the per-command tool schemas in `discovery`.

## Turns (S4a)

A **turn** is a labelled group of calls whose edits undo as one step: one message of the built-in agent, one workflow run, or a group a remote agent opens.

- **Open and close.** `project.turn_begin {label, changes?, calls?}` answers `{turn: "t_…", label, calls, changes}`; `project.turn_end {turn}` closes it and answers what it did. Both are reads of the `project` module (6.2): they change no document. A session holds at most 16 open turns; opening another closes the oldest.
- **A call in a turn** names it in the envelope's `turn`. An unknown or ended turn is refused (`turn.unknown`) and nothing runs.
- **One undo step.** The front door opens the engine's undo group around each of the turn's calls (`Engine::begin_group(label)`, then `continue_group(label, mark)` for the next call, `end_group()` after it). Every step the turn makes joins its one step, which keeps the document from before the turn as its undo target and reads the label (`Step::label`, shown by `Step::line`). A step anyone else makes in between (the person's own edit, an undo or redo) splits it: the turn's next edit makes a new step under the same label, so undo order stays what happened. A dry run keeps nothing and joins nothing.
- **Limits.** `Grant::limits` (`Limits {changes, calls}`, default 60 and 400 as in Settings › AI) bound every turn of a principal with a grant; `turn_begin`'s `changes` and `calls` can only lower them. Every call carrying the turn counts as a call, gate refusals included; a call that takes `expected_revision` is a change when it moves the revision. A call past either limit stops with `gate.turn_limit`, changing nothing. The person and the operator have no grant and no limits, but their turns group the same way. A call without a turn is not counted.

Not yet (S4b): jobs, `Progress` and events. MCP's verbs do not carry `turn` yet; the live channel's `surface.call` does, since it sends the envelope whole.

## Explicit context

Nothing about standards or materials is installed for the process (S2b). The host's data is one value, `sparksmith_core::library::standards::Context`: the standards (standard fields, marking codes, the generic-part grammar, symbol drawings) and the material library, or why that failed to load.

- **Made once by the host:** `sparksmith_io::standards::shipped_context()` reads the shipped library. The desktop app makes it in `App::new`, the CLI in `run`.
- **Handed to every engine:** `Engine::set_context`, kept across new and opened documents like the generator runtime and copied by `fork`. `sparksmith_io::new_project::{engine, profiled_engine, open}` take it; `Engine::showcase_with`, `Engine::showcase_hosted` and `session::showcase` take it.
- **Current while it is used:** an engine enters its context on the calling thread for each operation and query (`Engine::enter_context`); a session for each request, so a 3D build or export reads the same. Code outside an engine call reads the context current on its thread (`standards::get`, `looks::get`). The desktop app keeps its context current on the window's thread for the program's life; the CLI for its run.
- **Carried to workers:** a thread a host or a build starts runs its work through `standards::carried`, which enters the starting thread's context. Solidcraft's parallel builds and every worker the desktop app starts do.
- **Missing is an error:** with no context current, a standard or material is refused with a message that names the cause; nothing falls back to a value in code or another engine's data.
- **Tests:** test builds give every new engine the fixture context (the repository's standards and materials, compiled into the test build), as they give it fixture stackups; a test that calls below the engine enters `Context::fixture()` itself. The process-wide self-install is gone. `Engine::clear_context` (tests only) makes an engine without one.

Two engines in one process with different contexts answer the same request each by its own (`standards::tests`, `session::tests::sessions_answer_with_their_own_context`).

A material library is kept for the rest of the program once loaded, since built models keep references into it: a host makes one context at startup, not one per request.

Not moved in S2b: the compiled-in starter rows (a parse cache of tables built into the program, the same for every engine), and two caches of shipped files that do not differ between engines of one host: the new-project defaults (`sparksmith_io::new_project::shipped`) and the shipped part files (`session::shipped_parts::builtin_parts`).
