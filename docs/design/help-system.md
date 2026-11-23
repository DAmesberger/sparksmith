# Help system design

Status: design for review, 22 November 2026. Nothing here is built; building needs the founder's separate go.

**Requirement (founder).** One source of truth for help, generated automatically as far as possible, with one view for people and one for agents. Typed data, not strings. Better than other tools.

**In one paragraph.** Help is a set of typed entries with stable ids, shipped as library data and versioned with the app. Most entries are generated from registries the code already has: the command catalog, finding codes, the shortcut registry, settings, fab profile rules, generators and the glossary. People write only what a registry cannot know: guides, workflows and the longer explanation of a finding. A gate like `cargo xtask surface` fails when something has no entry, a link does not resolve, or prose was written against facts that have since changed. The same entries feed an in-app help panel, the website and two read verbs on the surface (`help.search`, `help.topic`) that MCP and the built-in assistant share.

## 1. What exists today

| Content | Where | Problem |
|---|---|---|
| Entry titles and summaries, 290 entries | `declare_entries!` lines and doc comments; `surface.lock` | Good, typed, but only agents see it (`find`, `describe`) |
| App commands, 159 | `apps/sparksmith/src/commands.rs`: title, context, default key, `keywords` | Second command list beside the catalog. A command doesn't link to the entry it ends in |
| Ribbon hover help | `icon_button_help(…, help, …)` strings in the shell | Inline strings that nothing else reads |
| Finding codes, 256 | `issues/rules.rs`: `KNOWN_CODES`, `severity`, `group_title`, `fix` as string `match` tables | Codes are strings. A finding has a group title and a fix line but no cause or explanation. Prose lives in code |
| Settings | `UserSettings` doc comments (`crates/io/src/settings.rs`); `library/defaults/settings.json` `sources`; labels in `settings_window.rs` | Three places, with no help to show |
| Fab profile rules | `library/profiles/*.json`, each rule with its source row, URL and date | Already the best-sourced data we have. Not shown as help |
| Generators | `library/generators/*/generator.json` and the parameters `library.generators` returns | `library/assist/system-prompt.md` lists the generators again in prose ("module for radio and MCU modules, tabbed for SOT-23…") |
| Assistant text | `library/assist/system-prompt.md`, `chat-prompt.md`, `chat-playbook.md` (17 KB) | Explains the verbs, the build order and the generators. Part of that is help |
| MCP guides | `describe {name: "guide:<topic>"}` (`mcp/tools.rs`) | A stub today: "this server serves no guides yet". The skills the MCP design names were never served |
| Workflows | `library/assist/workflows/*/method.md` and `workflow.json` | Already data. Served as MCP prompts |
| Glossary | `GLOSSARY.md` | Developer vocabulary, marked "proposed". Not in the app |
| Keymap docs | `docs/keyboard-shortcuts.md` (default keymap table) | Copies the registry by hand |
| Design docs | `docs/` (about 70 files) | Developer and design records, not user help. Mostly stays as it is |
| Website | A landing page, kept outside this repository | One landing page. No docs yet |

The pattern: the facts are mostly typed already. What's missing is one shape that collects them, prose next to them, and a check that keeps the two together.

## 2. Source model

**An entry** has an id, a kind, a title, a one-sentence summary, a body of typed blocks, typed facts for its kind, links, and `since`, the app version it arrived in.

**Ids** are `kind/name` and reuse the names the registries already use:

- `entry/layout.add_via`
- `finding/board.via_annular_ring`
- `command/route.width-next`
- `setting/impedance_tolerance_pct`
- `rule/jlcpcb-2-layer/via_drill`
- `generator/no-lead`, `generator/no-lead#pitch`
- `term/symbol-unit`
- `guide/first-board`
- `workflow/datasheet-to-component`
- `panel/issues`

A renamed registry name is a renamed id. The gate makes that deliberate, the same way the lock does for entries.

**Typed facts per kind** (Rust types in the core; JSON in the library):

- **Entry:** module, class, kind, schema digest, and examples (validated calls, below).
- **Finding:** severity, the stage it blocks, group title, fix line, cause, explanation, the entries that fix it (for example `layout.set_via`), and the rule or setting it checks.
- **Command:** context, default key, the entry or panel it reaches. Keys are shown as the person's effective keys, never as text.
- **Setting:** type, unit, range, default, where the default came from.
- **Profile rule:** value, unit, source document, row, URL, date, and whether it's a limit or the author's choice.
- **Generator:** each parameter's key, type, range, default and presets.
- **Term:** definition, the words to avoid, related terms.

**Body blocks** are a closed enum: paragraph, list, table, note, image (alt text required), example call, and inline references. Inline references are `entry`, `finding`, `command`, `setting`, `term` and `key(command id)`. They are written in the Markdown sources as `[[finding/board.clearance]]` and `{key:route.width-next}`, and parsed at load into the enum. A reference that doesn't parse or resolve fails the gate. No free-text URLs inside the app's own content.

**Layout in the library:**

- `library/help/generated/*.json`: one file per registry, written by `cargo xtask help --update` and committed, like `surface.lock`.
- `library/help/topics/**/*.md`: hand-written prose. Frontmatter holds the id, kind, `covers` and `since`.
- `library/help/media/`: images.
- `library/help/<locale>/`: later (§6).

**What is generated:** entries (from `ENTRIES` and the schemas), the facts of findings, commands and shortcuts (from the registry, through an app-crate test, as `the_lock_holds` does for the catalog), settings (schemars schema and doc comments of `UserSettings` plus `defaults/settings.json`), profile rules, generators with their parameters, and the glossary.

**What is written by hand:** guides (first board, libraries, routing, fab output), the panel pages, the cause and explanation of each finding, and longer notes on an entry where its summary isn't enough. Workflows stay where they are and are indexed as `workflow/…` entries, not copied.

**Typed finding codes.** "Typed, not strings" is weakest here. A `declare_findings!` macro, shaped like `declare_entries!`, would turn the 256 codes into an enum with severity, stage and group title on the declaration line. The wire name stays the same string. The prose (title, fix, cause, explanation) moves from `rules.rs` into `library/help/topics/findings/`. The issues card then reads its fix line from the help entry, so the row and the help page can't disagree. See decision 1.

**Staleness.** A topic lists the ids it `covers`, each with the digest of the facts it was written against, for example `entry/layout.route@3f2a…` (the schema digest from the lock) or `finding/board.clearance@…`. When a covered fact changes, the gate names the topic and the fact, and someone rereads it and stamps it again with `cargo xtask help --stamp <topic>`. This is how "nothing stale" gets checked by a machine, not by memory.

## 3. The gate: `cargo xtask help --check` (in `ci`)

It fails when:

- a catalog entry, finding code, registry command or in-place key, setting, profile rule, generator parameter or glossary term has no entry;
- a finding has no cause or fix (explanation optional);
- an id is duplicated, or a reference or link doesn't resolve;
- a topic covers a fact whose digest changed and wasn't stamped again;
- an example call doesn't validate against its entry's current schema (run through `project.schema`, never executed);
- a title or fix breaks `issues::style::check`, or an image has no alt text;
- the generated files are out of date (`--update` rewrites them).

**No-builtin-data.** Help is read at run time from the shipped `library/` folder (`sparksmith_io::shipped`), the same way as fab profiles and workflows. Nothing is `include_str!`ed, so `cargo xtask no-builtin-data` stays as it is. If the help folder is missing or broken, the panel says which file and the app keeps working. The MCP skill prompt that's allowlisted today becomes a help topic, and its allowlist entry goes.

## 4. The view for people

- **Help panel.** A docked sheet built from `kit` containers and tokens, like the Keyboard shortcuts sheet. It has a search field, results grouped by kind, and a reading pane with back and forward. It works offline. Search uses the palette's scorer (`commands.rs`) over titles, summaries, terms and keywords, so both searches rank the same way.
- **Context.** F1 (a registry command, so it can be rebound) opens the entry for what has focus or is under the pointer: a ribbon button, a panel, a setting row, a property, a selected object's finding. Each issue row's disclosure gets "?", which opens `finding/<code>` with the cause, the fix, the Fix button and the rule's source. A setting row's "?" shows the default and where it came from. The palette gets a "Help: …" result group.
- **Effective keys.** `{key:…}` renders the person's current binding, so a rebinding changes the help too. The Keyboard shortcuts sheet (⌘/) becomes one generated help view, and the hand-kept table in `docs/keyboard-shortcuts.md` is deleted.
- **Values in the person's units.** Profile limits and setting defaults go through the units layer.
- **Website.** `cargo xtask help --site <out>` turns the same entries into static HTML: one page per entry, a search index, a folder per version (`/docs/1.0/`, `/docs/latest/`). The infra repo only serves the output. Publishing it is an outward step and needs the founder's go each time.

## 5. The view for agents

- **Verbs.** A new `help` module with two query entries, class read and effect pure, declared like any other, so they pass through `Surface::call`, the gate and the lock:
  - `help.search {text, kinds?, limit?}`
  - `help.topic {id, budget_bytes?, sections?}`
- **MCP resources.** `sparksmith://help` (the index) and `sparksmith://help/{id}` (a template, with completion on `id`) carry the same bytes as the verbs.
- **Guides.** `describe {name: "guide:<topic>"}` serves `guide/<topic>`, which finally fills today's stub.
- **The built-in assistant** calls the same verbs in process. The parts of `system-prompt.md` and `chat-playbook.md` that explain the product, such as the generator list and the build order, become topics the prompt names by id instead of copying them.
- **Answer shape.** The answer is typed: facts first, then prose excerpts cut to `budget_bytes`. We have no offline Claude tokenizer, so the budget is in bytes. Each answer says whether it was cut and how to get the rest.

**Example.** A model sees a finding and asks what it means:

```json
{"tool": "query", "arguments": {"name": "help.topic",
  "params": {"id": "finding/board.via_annular_ring", "budget_bytes": 900}}}
```

```json
{"ok": true, "result": {
  "id": "finding/board.via_annular_ring", "kind": "finding", "since": "1.0",
  "title": "Thin via annular rings",
  "summary": "The copper ring round a via's drill is thinner than the fab profile allows.",
  "facts": {"severity": "error", "blocks": null,
    "checks": "rule/jlcpcb-2-layer/via_annular_ring",
    "fixed_by": ["layout.set_via", "rules.set"],
    "fix": "Use a larger via pad or a smaller drill"},
  "excerpt": [{"p": "Drills wander. A thin ring can break out at the hole edge, which opens the connection…"}],
  "truncated": true, "next": {"sections": ["explanation"]},
  "related": ["rule/jlcpcb-2-layer/via_drill", "guide/fab-profiles"],
  "uri": "sparksmith://help/finding/board.via_annular_ring"}}
```

In a live session, the profile rule in `checks` is the board's own profile, taken from the open project. Headless, it's left out unless a project handle is passed.

## 6. Translation, screenshots, accessibility

- **i18n readiness.** Only prose gets translated. Generated facts (keys, values, schemas) are the same in every language. English is the source. A translation in `library/help/<locale>/` records the digest of the English text it translated: when the English changes, the page falls back to English and is flagged. English only until the first release.
- **Screenshots.** A scripted VM scenario (mac-vm) opens fixed example projects and captures each image a topic names. Images carry the app version they were taken on, and the release checklist regenerates them. Never on the host desktop, and not in CI. Alt text is required, and the gate checks it.
- **Accessibility.** AccessKit roles for headings, lists and links. The whole panel works from the keyboard (search, arrows, Enter, Back, Esc). Text follows the UI scale, colours come from tokens with checked contrast, and severity shows as a glyph, never by colour alone.

## 7. What other tools do (public documentation; recheck before citing externally)

- **Large commercial EDA suites** keep a separate online documentation site, and F1 opens a web page. The help needs a network and can describe another version than the one installed.
- **Open-source EDA suites** ship a manual written apart from the code, with offline packages. Rule and error texts in the app and the manual are kept in step by hand.
- **Blender** links every button to its online manual page and shows the Python path in tooltips. The manual is a separate repository, so it can lag the build.
- **VS Code** generates its settings UI and settings docs from the JSON schema each extension declares. This is the closest to our model, but it covers settings and commands only.
- **Rust** has `rustc --explain E0308` and a lint list generated from declarations. Typed error codes with explanations, in the tool, offline.

**Where we'd be better:** one typed source for the app, the website and agents. Every finding links to the rule it checks, the rule's published source and the entries that fix it. Keys show the person's own bindings. Examples are checked against the live schemas. Prose is flagged when the code under it changes. And agents get the same help, as typed facts with a size budget, through the same front door as everything else.

## 8. Risks

- **Writing load.** 256 finding causes and the core guides are a lot of text. Agents draft from code and profile sources, and people sample. Bad prose is worse than none, so a missing explanation is allowed; a missing cause or fix is not.
- **Churn from typed codes.** The finding enum touches every rule owner. It goes in one slice, with no wire change.
- **Stamp fatigue.** If stamping gets routine, people will stamp without rereading. Digests cover only the facts a topic depends on, not whole modules.
- **Version drift on the website.** Mitigated by building per release, never from `main`.
- **Bundle size** from screenshots. Keep in-app images few; the website can carry more.
- **Search quality** without embeddings. Exact-term search plus glossary synonyms first; measure on real questions before adding more.

## 9. Founder decisions

1. **Finding prose moves out of `rules.rs` into library help, and codes become a typed enum (`declare_findings!`).** Recommend yes, in two slices: the enum first, then the prose move. This follows "no hard-coded data".
2. **URI scheme.** `sparksmith://help/{id}` or `help://{id}`. Recommend `sparksmith://help/…`, one scheme with the catalog.
3. **A `help` surface module (`help.search`, `help.topic`) as well as resources.** Recommend yes: resources alone don't reach clients that ignore resources, or the built-in assistant.
4. **MCP guides are help topics** (`describe guide:<topic>` reads `guide/<topic>`). Recommend yes, and drop the separate skills plan in `mcp-surface.md` §3.7.
5. **Glossary.** Move the terms into `library/help` and generate `GLOSSARY.md` from it, so agents keep the path they read. Recommend yes, keeping developer-only terms flagged and hidden in the app.
6. **Website.** Built by an xtask in this repo, served by infra, one folder per release. Recommend yes. Each publication needs your go.
7. **Screenshots.** A few in the app, more on the website, regenerated from the VM before each release. Recommend this split.
8. **i18n.** Ready in structure now (ids, locale folders, digests), English only until 1.0. Recommend yes.
9. **Who writes.** Agents draft causes and guides from code and sources, and you sample about 10 %. Recommend yes.
10. **Duplicates in `docs/`.** Delete the hand-kept tables the generator replaces (the default keymap first). Recommend yes, per "no superseded text".

## 10. Slices (agent hours)

| Slice | Content | Hours |
|---|---|---|
| H0 contract | Entry, block and reference types in the core; id grammar; `library/help` layout; loader in `sparksmith-io` | 3–4 |
| H1 generated index | xtask `help --update/--check`: entries, settings, profile rules, generators, glossary; registry commands through an app test; the gate in `ci` | 6–8 |
| H2 typed findings | `declare_findings!`, the enum, fix lines read from help, prose moved | 6–8 |
| H3 agent view | `help` module and lock entry, both verbs, resources and completion, `guide:` served, assistant prompts trimmed to ids | 5–6 |
| H4 help panel | Sheet, search, F1 and "?" context, effective keys, AccessKit, the shortcuts sheet as a help view | 8–10 |
| H5 content | Finding causes (256, drafted in parallel), five core guides, panel pages, stamps | 8–12 (2–3 agents) |
| H6 website | `help --site`, versioned static output; deploy only after go | 4–5 |
| H7 screenshots | VM scenario, captures, alt text | 3–4 |
| H8 i18n scaffold | Locale folders, digests, fallback | 2–3 |

**Order.** H0 first. Then H1 with H2, H3 with H4, and H5 with H6, at most five agents at a time. H7 and H8 at the end. About 45–60 agent hours in total, roughly 15–20 hours of wall time when run in parallel.
