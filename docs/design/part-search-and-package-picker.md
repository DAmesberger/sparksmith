# Part search for placing, package picker for making

Design for founder approval, 16 November 2026, written against `main` at 01235593 with the founder's answers of 16 November 2026. Documents only: nothing was built. Hours are agent hours and are estimates.

## Founder decisions this design follows (16 November 2026)

- A **generator makes a package**: the footprint, the 3D model and the lead layout, from dimensions. A generator **may also declare** that some of its outputs are whole parts (§3.2).
- A **part** is a package plus its pins and functions, symbol, explicit pin mapping, values and MPN (§0).
- **Placing searches parts.** Results come from the project, library folders, the shipped library, the user's own parts and, later, the online registry. A package with electrical pads is never placed on its own. (A demo placed a part named just "SOT-23-5".)
- **Generators belong to part making**: the part editor's package step, the New part dialog and the datasheet workflow. The generator-chip gallery (one chip per generator) "does not scale" and is no longer an entry point for placing.
- **Founder answers** (16 November 2026):

| # | Question | Answer | Where |
|---|---|---|---|
| 1 | Facets in the generator's description | Yes, next to the presets | §3.2 |
| 2 | Fixed top-level categories | No: top-level categories can be created, and there is an **Other**. Research whether the six miss anything | §1.3, §3.1, [research](#appendix-category-research) |
| 3 | Nomenclature | Package and part defined explicitly, used consistently. A package whose pads are all non-electrical can be placed as a pin-less mechanical part | §0 |
| 4 | One package shared by parts | Yes | §3.4 |
| 5 | BOM Value | Yes, but **smart and user-definable**: a template with `{field}` placeholders, per part and per category, as data, with sensible defaults | §3.5 |
| 6 | Bare packages in old projects | A package with electrical pads can never be placed. **No migration**: nobody uses the software yet; examples are regenerated | §4 |
| 7 | Category as a standard field | Yes | §3.3 |
| 8 | Which generic parts are complete | **No hard-coded list.** A generator declares, as data, which outputs are parts (pins with an explicit pin mapping) | §3.2 |
| 9 | One ranked list | Yes; groups only when browsing | §1.2 |
| 10 | Chat entry | The chat **can do everything**: plan, make parts, place, route, check and export ("Claude Code for PCBs"), with the same verbs as every caller, under the person's permissions and review gates. No narrow `view.arm_place` | §5 |
| 11 | Online sources | Yes. Define the registry format; it will be hosted at `parts.sparkforge.io` | §6 (sketch; follow-up design) |

Run 11 (`docs/e2e-run-11.md`, not yet on `main`) adds three concrete problems this design must fix:

1. **Naming a placed IC took the long way.** The palette's one-keystroke path names the part after its package, with no name field. A real name meant Component › New, then the generator form's Name, then Create, Publish and Place. Pin names were typed one row at a time.
2. **Passive values needed manual setup.** A value could be set only after adding the Resistance or Capacitance field to the component and updating the placed parts.
3. **The BOM was wrong.** The Package column read "TLV75733PDBVR package" instead of SOT-23-5. The Value column was empty for the ICs, the LED and the header.

## 0. Words this design uses

Used in this sense throughout this document, the UI text and the chat's method text. They agree with [GLOSSARY.md](../GLOSSARY.md) (Package, Footprint, Mechanical part, Board-only part), which K0 extends with **Part** in this sense.

| Word | Means | Does not mean |
|---|---|---|
| **Package** | The physical body: its footprint (pads and lands, courtyard, silkscreen), its 3D model and its lead or terminal geometry. It has terminals numbered by position, and **no electrical meaning**: no pin names, no functions | A part; anything with a pin name |
| **Part** (component) | A package plus a symbol, pins and their functions, the **explicit pin mapping** (which pin lands on which pad), fields and values, and an MPN where one is chosen. Only parts are placed | A package with a name |
| **Electrical pad** | An SMD or plated-through pad that lands a terminal (the footprint rule `footprint.terminal_required`) | Mechanical pads, non-plated holes, fiducials |
| **Mechanical part** | A part whose package has **no electrical pad** (a mounting hole, a fiducial, a standoff, a heatsink). It has no pins and is placed like any part, usually board-only | A package with electrical pads whose pins were left out |
| **Generic part** | A part a generator declares whole from its parameters (a chip resistor, a pin header): its pins and mapping come from the generator. Its value is per instance | Any generator preset |
| **Make** | Create a part (from a datasheet, a package, a generator's part output, or by hand) | Place |
| **Place** | Put an existing part on a sheet or the board | Make |

The rule that follows: **a package with any electrical pad is never placed.** It becomes placeable only as part of a part, whose pins map onto its pads explicitly. A package with no electrical pad can be made into a pin-less mechanical part (§2.3) and placed.

## What exists today (and what changes)

| Today | Where | After this design |
|---|---|---|
| The Place palette lists project components, then "Generic parts" (on-demand generator parts), "From generator" presets of every generator, built-in parts and library-folder rows. Enter on a preset makes a component *named after the package* and arms placement | `apps/sparksmith/src/place_palette.rs` (3,109 lines), `catalog::generic::generators` | One ranked list of **parts only**. Package presets leave the palette. When nothing matches, the last rows offer **Make it…** |
| The generator gallery is a sheet: chips for the 9 generators, then presets, then the one form. It opens from New component, the start page, the palette, the Footprint tab's From generator and Change package… | `generator_gallery.rs` (964), `generator_form.rs` (462) | Replaced by the **package picker**: search, categories and filters, then the same `generator_form`. It is reached only from part-making entry points |
| Component-scope generation of an IC package gives terminals "1…n" and names the component, package and footprint after the preset (or after the Name the person typed) | `generators/*/src/lib.rs` (`Component { name, terminals }`), `library::generated::expand` | Component scope is accepted **only for outputs the generator declares as parts** (§3.2). Any other package is made in package scope and becomes a part through **New part** (§2.3). The package half is always keyed and named as the package |
| `PartOffer.instance_fields` declares `value` (chip) and `colour` (chip LED) or nothing at all, and GP1 selects no field definitions | `generator-plugins.md` "Limits of GP1" | A declared part output names its category and value field. The palette's parsed value goes onto the instance |
| `generic-families.json` holds the search grammar for generic parts | `library/catalog/` | It stays, and gains the category. A family must point at a declared part output |
| The catalog's `Source` is `Project` or `Library(id)`. Filters are `pkg:`, `pins:`, `pitch:`, `body:`, `has:` and field predicates. Ranking is match tier, then used, favourite and recent | `crates/editor-core/src/catalog{,/query}.rs` | `Source` gains `Shipped`, `Generic` and later `Online`. The query gains `cat:` and `style:` |
| No category data | — | Categories are **data** from shipped, folder, plugin and user sources, merged by id, top level included, with a fixed fallback **Other** (§3.1) |

## 1. Part search for placing

### 1.1 Entry points (one search, `library.search`)

| Entry | How it opens | What Enter does |
|---|---|---|
| **Place palette** (Schematic: `P`, the toolbar, the command palette's "Place…") | A sheet over the canvas with the cursor in the field. The last query and category are kept per project | Places the part. A generic part not yet held is made and placed (one undo step with the first placement, as today) |
| **PCB** (`P` in the board editor) | The same sheet, with `cat:mechanical` and `cat:board` parts first | A mechanical or board-only part goes on the board. A part with pins is refused in place: "Place it in the schematic; the board follows with Update PCB" |
| **Chat** | The model calls `query library.search` like any caller (§5) | The model places with `apply component.place` or makes the part first (§5.2). No special entry |
| **Assign mode** (Choose part dialog, Resolve parts) | Unchanged: `kind:part` with the instances' requirements | `instance.assign_part` |

### 1.2 Sources and ranking

The sources, in **precedence order**:

1. **Project.** Every placeable component lineage the project holds, including generic parts it has already made.
2. **Library folders** (S3, as built): read-only or writable folders listed in the user's settings.
3. **Shipped library.** `library/parts/` components such as the GCT USB4105 and the mechanical parts. Today these are the palette's "Built-in parts" group.
4. **Generic parts on demand.** A text that the grammar (`generic-families.json`) or a declared part output reads as a whole part, such as "10k 0603", "led 0805 red" or "header 2x5 2mm". Such a part is made when chosen. Only outputs a generator declares as parts are listed (§3.2).
5. **Online registry** (§6). It is ranked after every local source.

Ranking keeps the built rule (parts-library "Ranking") and adds source as the last key before the final ties:

1. **Match tier**: exact, prefix, word, substring, field or filter. A generic part whose whole grammar matches ranks **Exact**. "10k 0603" is that part, not a fuzzy hit.
2. **Boosts**: used in this project, then favourite, then recent.
3. **Source precedence** (above). A generic part the project already holds is listed once, as a project row.
4. **Final ties**: folded name, then lineage id.

The list is **one ranked list**, with each row's source shown as a chip, not a group heading (founder Q9). When the field is **empty**, the sheet browses instead: Recent, then Favourites, then the selected category's rows.

**Never listed:** packages (a generator preset that is not a declared part output, or a package lineage on its own) and unpublished drafts (they count in "N hidden (draft)", as today).

### 1.3 Categories by function, and filters

Categories are **data** (§3.1), never code. The shipped set, from the [research](#appendix-category-research), has fifteen top-level categories with first subcategories. Folders, plugins and the person may add categories **at every level, top level included**; a part with no category or an unknown one is listed under **Other**.

| Category (id) | Subcategories (first data set) | Default value template |
|---|---|---|
| Passives (`passives`) | Resistors, capacitors, potentiometers and trimmers, resistor and capacitor arrays (thermistors are under Sensors, varistors under Protection) | Resistors `{Resistance}`; capacitors `{Capacitance} {Rated voltage}` |
| Magnetics (`magnetics`) | Inductors, ferrite beads, common-mode chokes, transformers | `{Inductance}` |
| Frequency control (`frequency`) | Crystals, oscillators, resonators | `{Frequency}` |
| Protection (`protection`) | Fuses and PTC fuses, TVS and ESD diodes, varistors, gas discharge tubes | `{MPN}` |
| Discretes (`discretes`) | Diodes and rectifiers, Zener diodes, bipolar transistors, MOSFETs, thyristors | `{MPN}` |
| Optoelectronics (`opto`) | LEDs, displays, optocouplers, photodiodes and phototransistors, IR | LEDs `{Colour}` |
| ICs (`ics`) | Power management, interface, logic, MCUs and processors, memory, analogue, clock and timing, audio | `{MPN}` |
| Sensors (`sensors`) | Temperature, motion and position, pressure, current, light, humidity, magnetic, microphones | `{MPN}` |
| RF and wireless (`rf`) | Antennas, RF modules, RF ICs, baluns and filters, shields | `{MPN}` |
| Power (`power`) | Board-mount converters and modules, batteries, battery holders and contacts | `{MPN}` |
| Electromechanical (`electromech`) | Switches and buttons, relays, buzzers and speakers, motors, fans | `{MPN}` |
| Connectors and cables (`connectors`) | Headers and sockets, USB, board-to-board, wire-to-board, terminal blocks, card sockets, RF connectors, cables and jumpers | Headers `{Name}` |
| Mechanical and hardware (`mechanical`) | Fasteners, standoffs and spacers, heatsinks and thermal pads, enclosure hardware | `{Name}` |
| Board features (`board`) | Test points, fiducials, mounting holes, net ties and solder jumpers, logos and marks | `{Name}` |
| Other (`other`) | — (the fallback; always present) | `{MPN}`, `{Name}` |

Every template list ends in the shared fallback `{MPN}`, then `{Name}` (§3.5).

**Modules** are no longer a top-level category: "module" is a **package style** (`style:module`), so a radio module is under RF and wireless, a power module under Power and a sensor board under Sensors, each findable with `style:module`. This follows the function-first rule of the distributors' trees and of IEC 81346-2 (see the appendix).

**Filters**, all of them typed terms of the one query language:

| Filter | Term | Source of the values |
|---|---|---|
| Category | `cat:passives`, `cat:passives/resistors`, `cat:other` | The part's Category field (§3.3). For generic outputs, the generator's declaration |
| Package style | `style:sot`, `style:qfn`, `style:module` | The package's `style` facet (§3.2) |
| Pins | `pins:5`, `pins>=8` | `PackageShape` (built) |
| Pitch | `pitch:0.5mm` | `PackageShape` (built) |
| Body | `body<=3x3` | `PackageShape` (built) |
| Values | `res:10k`, `V>=16`, `tol<=1%` | Typed fields (built). The value field the category declares appears as its own facet |
| Source | `src:project`, `src:library`, `src:shipped`, `src:generic` (later `src:online`) | Entry source (`src:` exists; new values) |
| Held | `is:held`, `is:new` | Whether the project holds it |
| Completeness | `has:3d`, `missing:mpn` | Built |

The left rail (or, at small sizes, the chip row) shows a category's facets with their counts (`Index::facets`, built). Clicking a facet value adds its term to the field, so the field always shows the whole query.

### 1.4 Result rows

Each row has one line of identity and one line of facts. Long text is shortened by `kit::text` with hover. Rows are drawn with the `kit/list` Ledger rows.

```
TLV75733PDBVR       LDO 1 A 3.3 V                     ICs/Power   SOT-23-5  5 pins  2.9×1.6
  Project · r2 · used 1          sym · fp · 3D · MPN                       in project
Chip resistor 0603  10 kΩ                             Passives    0603      2 pins  1.6×0.8
  Generic · chip v2               value from your text                     new: made on place
USB-C receptacle    USB 2.0, 16 contacts (GCT USB4105) Connectors  —        16+4     8.9×7.3
  Shipped                         sym · fp · 3D · MPN                       new
```

- **Where it comes from**: Project, `Library · <folder>`, Shipped, Generic · `<generator> v<N>` or (later) `Online · <registry>`.
- **Held or new**: "in project", "in project · update available (r3)" (the S3 update badge, built), or "new" (copied or made on place).
- **Regeneration status** of generic parts, from `library::generated::standing` (G15): "made on place", "in project", "older generator: made again as r2 on place" (Enter opens the form review, as today) or "made by a newer generator: can't use" (disabled, with the reason on hover).
- **Completeness**: `sym · fp · 3D · MPN`. Missing items are dimmed.
- **Value**: the part's shown value from its template (§3.5). For a generic part, the value parsed from the text ("10 kΩ"); it is written to the instance's typed field on placement.

A preview pane (on the right at 1920 wide, toggled with Space at 1024) shows the symbol, the footprint, a 3D thumbnail, fields and provenance. All of it comes from `library.inspect`, as built.

### 1.5 When nothing matches: "Make it…"

**Make rows** always end the list. When the list is empty they are the only rows, and Enter takes the first one:

| Row | Shown when | What it does |
|---|---|---|
| **Make "TLV75733PDBVR" from its datasheet…** | Always | Opens the chat with the datasheet workflow's card (`assist.offer`), with `part` set to the text. The run's result card has **Place**, which arms placement in the window (the person's click, not a catalog entry) next to Open in editor |
| **Make "TLV75733PDBVR" on a package…** | Always | Opens the **New part** dialog (§2.3) with Name set to the text |
| **Make a resistor 0603 with 10 kΩ** | The text reads as a generic part (already listed as a row above) | Not repeated |

This fixes run 11 finding 1: the name is given where the part is made, and the part is placed directly. No path places a package.

### 1.6 Generic parts are declared by their generators

- One component lineage per generic **kind and size**, for example "Chip resistor 0603". The **value is per instance**, as a typed requirement. This is parts-library D5's "design by requirements", and Resolve parts then picks the MPN.
- **Which outputs are parts is the generator's declaration, never the app's** (founder Q8). An output is a part when the generator declares that it emits pins with an explicit pin mapping (§3.2): a chip resistor's pins 1 and 2 land on its two pads, so it is a part. The host checks the declaration on every generation; the app holds no list of part kinds.
- Every other output is a package. Making a part on it needs the person's (or the datasheet's) pins: §2.3.

### 1.7 How folders and online sources slot in

- **Library folders** are built (S3): their `SourceCatalog` entries already pass through the same `Index`. This design adds their Category field and facets to `SourceCatalog` entries (fields on component revisions, so no format change), and lets a folder contribute categories (§3.1).
- **Online registry** (§6): it implements the same `SourceCatalog` shape, fetched and cached per user (S5 rules: never authoritative, never written into a project except by an import). Its rows read "Online · parts.sparkforge.io". Enter shows the **import preview**, then imports and places. Nothing in §1.2 to §1.5 changes; the registry is one more source with the lowest precedence.

## 2. Package picker for making

### 2.1 Where it appears

| Entry | Scope | Result |
|---|---|---|
| Part editor › **New part** dialog (§2.3) › Package "Choose…" | A package for the new part | The chosen package (made in package scope when it is new); the part is made by `library.new_part` |
| Part editor › Package step / 3D tab › **Change package…**, and the Footprint tab › **From generator** | The working set's package | `library.package_from_generator` (built) |
| **Datasheet workflow**, `pick` step | The model's choice | The `choice` card shows the model's generator, preset and values with up to 2 alternatives. **Choose another package…** opens the picker. Its answer replaces `/steps/plan` and the run goes on (no new control kind) |
| Component start page | — | "New from a generator" becomes **New part…** (the dialog) |

It is **not** reachable from the Place palette, the PCB editor or the Open page.

### 2.2 Search, categories and the single form

- **Rows** are packages, not generators. Each preset is one row ("SOT-23-5", "SOIC-8 3.9 mm", "QFN-16 3×3 0.5 EP 1.7"). Each generator also gets one "Custom <style>…" row that starts from its defaults. The rows come from `library.generators` (the shelf), packages from library folders and packages the project already holds, with their source chip, so "a SOT-23-5 we already have" ranks first.
- **Categories** are package styles, from data (§3.1): Chip, Molded, SOT/SC, SOIC/SOP/TSSOP, QFN/DFN, QFP, BGA/LGA, Through-hole (axial, radial, DIP, TO), Headers, Modules, Mechanical. Styles merge from the same sources as part categories and also end in **Other**.
- **Filters**: `pins`, `pitch`, `body`, `style`, `lands:cited|estimated` (the preset's `basis`), `pads:mechanical` (packages without electrical pads) and source. Aliases ("SOT-753", "SC-74A") and JEDEC names are searched as text.
- **The form** is the existing `generator_form` (built from the description, with groups, units, rules, refusals on their fields and G15 standing). From the New part dialog it has **Name** and **Pins** as its first section (§2.3). From Change package… it has neither.
- **Scale**: there are no chips per generator. The list is virtualised (`kit/list`) and bounded per query (ROWS 200, then "Show all"). Categories and facets come from data, so 50 generators or a plugin folder add rows and facet values without any UI change.

### 2.3 The New part dialog (making with a name)

A docked `kit::dialog::DialogPanel` in the right column (sidebar IA: a command with inputs). It sends one new session command, **`library.new_part`** (Edit, library module):

```
library.new_part { name, manufacturer?, category, package, pins?, fields?, place? , expected_revision }
  package: { held: <package lineage> } | { generate: {generator, preset?, params} }   // package scope
  pins:    a pinout document (the built `library.pinout_preview` format)
```

It makes the package when it is new (package scope, keyed as a package, §3.4), drafts the component, symbol and binding with the pins and their explicit pad mapping (through the built pinout import), sets Category and fields, creates the `Part` record when a manufacturer is given, and publishes, as one undo step. Refusals change nothing and name the field:

- **Every electrical pad must be landed by a pin, and every pin must have a name** ("NC" is a name). "Pad 4 of SOT-23-5 has no pin: name it, or NC" (`part.pad_unmapped`), "Pin 3 has no name" (`part.pin_unnamed`).
- **No pins** is accepted only when the package has **no electrical pad**: the result is a mechanical part (§0). Otherwise: "SOT-23-5 has 5 electrical pads: a part needs their pins. To place a hole or fiducial, choose a package with mechanical pads only" (`part.pins_required`).

| Field | Notes |
|---|---|
| Name | Required, at most 64 bytes, set from the palette text. Becomes the component's name, and the MPN when Manufacturer is filled |
| Manufacturer | Optional. With Name it creates the `Part` record |
| Category | Choice from the merged category data, guessed from the package style (SOT → Discretes or ICs by pin count; mechanical-only pads → Board features). **New category…** at the end of the list (§3.1) |
| Package | "SOT-23-5 · 5 pins · 2.9 × 1.6 mm" and **Choose…**, which opens the picker |
| Pins | A multiline paste: "1 IN, 2 GND, 3 EN, 4 NC, 5 OUT", or two columns pasted from a datasheet table, parsed by the built pinout import with the preview's row errors on the field. Hidden for a package without electrical pads. "5 of 5 pads mapped" below |
| Value | The category's value field, when it declares one |

**Create and place** (primary) sends `library.new_part` with `place: true`, which arms placement in the window after the publish. **Create and open** makes drafts only and opens the part editor; the drafts publish under the same rules. Run 11's five-step path becomes one dialog, with the pins from one paste.

## 3. Data

### 3.1 Categories and styles: data, merged from every source

**Format.** `categories.json` format 1, the same file shape wherever it comes from. There is no list in code, so `no-builtin-data` holds (files are read at runtime, never compiled in).

```json
{"format": 1,
 "categories": [
   {"id": "passives", "title": "Passives",
    "children": [{"id": "resistors", "title": "Resistors", "value_field": "Resistance",
                  "value_template": ["{Resistance}"]},
                 {"id": "capacitors", "title": "Capacitors", "value_field": "Capacitance",
                  "value_template": ["{Capacitance} {Rated voltage}", "{Capacitance}"]}]},
   {"id": "audio", "title": "Audio", "same_as": ["sound"]}],
 "styles": [
   {"id": "sot", "title": "SOT / SC", "aliases": ["sot-23", "sc-70", "sc-74"]},
   {"id": "module", "title": "Modules", "aliases": ["castellated"]}]}
```

**Sources**, merged in this order (highest precedence first):

| Source | Where | Can add |
|---|---|---|
| The person | `<config>/library/categories.json` | Any category, top level included |
| Library folders | The folder manifest's optional `categories` (a writable folder can be edited from the app) | Any category, top level included |
| Plugins | A generator's description (`PartOutput.category` may name a new path; its title comes from `categories` in the description) | Any category, top level included |
| Shipped | `library/catalog/categories.json` | The shipped set (§1.3) and **Other** |

**Merging rules** (one pure function in the core, `catalog::categories::merge`, with its tests):

- A category is identified by its **path of ids** (`passives/resistors`). Ids are lower-case ASCII slugs, at most 32 bytes, at most 3 levels deep. Equal paths from different sources **are the same category**: their children are united. That is what keeps `cat:` comparable across sources.
- **Titles, order, value field and value template** come from the highest-precedence source that sets them. A lower source's different value is kept out and listed in Settings › Library › Categories as "overridden by <source>".
- **`same_as`** maps another id onto this one ("sound" → "audio"), so two folders that named the same idea differently merge without either being edited. A part whose Category names the alias is listed under the target.
- **Refused, naming the source and the ids**: a duplicate id among siblings within one file, a cycle through `same_as`, an id that is `same_as` two targets, a `value_field` or template placeholder that names no standard field, and any redefinition of `other`. A refused file contributes nothing; the others still load.
- **Other** (`other`) is shipped, always present, and cannot be removed, renamed or given children. Every part with no Category, or with a Category no loaded source defines, is listed there; its unknown id stays visible ("Other · audio-transducers"), so nothing is dropped and the field is never rewritten.

**Creating a category.** New catalog entries `library.categories` (Read: the merged tree, with each node's sources) and `library.category_define {scope: user | folder <id>, parent?, id, title, value_field?, value_template?, same_as?}` (Edit, settings-class for `user`; a folder must be writable). The New part dialog's **New category…**, Settings › Library › Categories and the chat all call it. It writes the person's or the folder's file, never the shipped one and never the project.

### 3.2 What a generator declares (in its description, beside its presets)

Optional fields of the one plugin ABI (an added optional field is a minor change, founder 16 November 2026):

```rust
pub struct PartOffer {          // existing: families, scopes, grammar, instance_fields
    pub style: Option<String>,             // "sot": the styles id of its packages
    pub parts: Vec<PartOutput>,            // which outputs are whole parts (placeable)
    pub categories: Vec<CategoryDef>,      // categories it introduces (same shape as categories.json)
}
pub struct PartOutput {
    pub when: Params,                      // e.g. {"part": "resistor"}; empty = every preset
    pub category: String,                  // "passives/resistors"
    pub value_field: Option<String>,       // standard field the instance carries
    pub name: String,                      // "Chip resistor {size}"
}
pub struct Preset {             // existing: key, name, aliases, description, params, citation, basis
    pub style: Option<String>,             // overrides the offer's style (a molded MELF)
    pub tags: Vec<String>,                 // "jedec mo-178", "automotive"
}
```

- **A `PartOutput` is a promise the host checks.** Component scope is accepted only for parameters a `parts` entry matches. For such a generation the host requires, beside today's checks, that **every electrical pad lands a terminal, every terminal has a named symbol pin, and the binding maps them explicitly** (`generator.invalid_output`, naming the pad or pin). A mechanical output (a mounting hole) declares a part and emits no terminals; the host then requires that no pad is electrical.
- **Component scope for anything else is refused**: "A SOT-23-5 package is not a part: make the package and give it pins (library.new_part), or ask the generator's author to declare a part output" (`generator.not_a_part`). The datasheet workflow and the New part dialog use package scope plus `library.new_part`.
- **Placing** lists only generic outputs matched by a `parts` entry. **Making** (the picker) lists every preset.
- `generic-families.json` families gain `"category"`. A family whose generator declares no matching `parts` entry fails the shipped-data check.
- **Why the description and not `generator.json`** (founder Q1): presets, families and grammar already live there, and the module is embedded in projects (G3). A project opened elsewhere then searches the same way, and a third-party plugin brings its own facets and categories. `generator.json` keeps only files, hashes and provenance. The shelf caches the read descriptions, so search never runs a module.
- **Versions.** Adding the declarations changes no geometry, but a module now refuses component scope for package outputs it used to accept, and the shipped chip, molded and header outputs gain checked pin mappings. The nine generators therefore raise their `version` (minor) once in K0. Regeneration of parts held by projects is moot (§4).

### 3.3 How library parts carry the same facets

| Facet | Library part (component revision) | Generic output |
|---|---|---|
| Category | Standard field **Category** (a category path, from the merged data) on the component. Part of the revision, so it travels with copies and folders | `PartOutput.category` |
| Style | The package's `style`: set by its generator, or chosen when a package is drawn by hand | `PartOffer.style` / `Preset.style` |
| Pins, pitch, body | `PackageShape`, derived (built) | Preset params via the same shape function on the generation's leads |
| Value | Typed fields (built), shown through the value template (§3.5) | `PartOutput.value_field` plus the parsed text |
| Aliases | Component `description`, and the package name and aliases | `Preset.aliases` |

The Category and Value template standard fields are **data**: records added to `library/standard-fields/standard-fields.json`. Adding a standard field to a component uses the existing field machinery, so the file format does not change.

### 3.4 Names out of a generation, and one package shared by parts (run 11 findings 1 and 3)

- **Package and footprint names come from the package**, never from the part: "SOT-23-5", "SOT-23-5 footprint".
- **Parts made on the same package share one package lineage** (founder Q4). The package is always made in package scope and keyed as a package, so a second part on SOT-23-5 finds the held one (G15: the same module and parameters return what the project holds). The BOM's Package column reads "SOT-23-5" for every part on it, and Change package… on one part offers "used by 2 parts".

### 3.5 The shown value: templates as data (founder Q5)

The value a person sees on the schematic, in the BOM's Value column, in Properties and in search rows comes from a **value template**: a list of alternatives, each a text with `{Field}` placeholders. **The first alternative whose placeholders are all filled wins.**

| Where the template comes from | Precedence |
|---|---|
| The part's own **Value template** field (a standard field, text), set in the part editor or by `library.edit` | 1 |
| Its category's `value_template`, from the merged category data (nearest category first, then its parents) | 2 |
| The shared fallback `["{MPN}", "{Name}"]`, in the shipped `other` category | 3 |

- **Placeholders** name standard fields (`{Resistance}`, `{Rated voltage}`, `{Colour}`, `{Frequency}`, `{Tolerance}`), the part record (`{MPN}`, `{Manufacturer}`) and the part (`{Name}`). Values are looked up on the instance first (its requirements and set values), then the component, then the chosen Part. Each value is formatted by its typed field ("10 kΩ", "100 nF", "16 V"), so a template never formats a number.
- **Checked on load**: a placeholder naming no field is refused, naming it and the file. A template is at most 128 bytes and 4 alternatives.
- **Defaults** (shipped data): resistors `{Resistance}`, capacitors `{Capacitance} {Rated voltage}` then `{Capacitance}`, inductors and ferrites `{Inductance}`, crystals `{Frequency}`, LEDs `{Colour}`, headers and mechanical parts `{Name}`, everything else `{MPN}` then `{Name}`. The person changes any of them per category (Settings › Library › Categories, through `library.category_define`) or per part.
- **The value to-do.** A part whose category declares a `value_field` and an instance where it is empty is a to-do: "R3 has no resistance". The template then falls through to `{Name}`, so the sheet never shows a blank.
- **BOM columns.** Value is the shown value; Package is the shared package lineage's name. Run 11's BOM then reads "10 kΩ", "100 nF 16 V", "TLV75733PDBVR", "Red" and "Pin header 1×4 2.54 mm".

## 4. No migration: the rule holds by construction

Founder Q6: nobody uses the software yet, so there is **no migration** for projects that hold package-only components, and none is planned. Until the first release the file format is version 1 with no backward compatibility: a stored-type change edits version 1 (no `FILE_VERSION` raise, no named feature, no converter step) and the examples are regenerated. Nothing in this design changes a stored type: Category and Value template are standard-field records, and the rest is commands and shipped data.

- **The rule holds by construction.** Component scope accepts only declared part outputs, whose mappings the host checks (§3.2). `library.new_part` refuses unmapped electrical pads (§2.3). The publish check gains the same rule for hand-drawn parts (`component.pad_unmapped`), so no path can produce a placeable component that leaves an electrical pad without a pin.
- **Examples are regenerated** with the new generators and `library.new_part` (`examples/esp32-s31-devboard`, `examples/tlv75733-*` and the parity records), and their golden files are blessed again.
- **Removing the gallery and changing the palette:**
  - `generator_gallery.rs` is deleted. Its form host moves into `package_picker.rs`, and `generator_form.rs` is kept. The Footprint tab's `footprint_template_picker.rs` opens the picker.
  - In the palette, the "From generator" group, `generator_gallery::preset_row` and `KEYS_TEMPLATE` are removed. "Generic parts" and "Built-in parts" become source chips. The make rows are added.
  - On the start page, "New from a generator" becomes **New part…**, and the start page's preset rows are removed.
  - The tests that drive the gallery from the palette (`generator_tests.rs`, `placing_a_resistor_a_qfn_and_a_header`) change: the QFN is made through the New part dialog with a name and pins, then placed.
- **MCP**: `library.generators`, `library.generate_part` and `library.package_from_generator` stay; `library.new_part`, `library.categories` and `library.category_define` are added. `cargo xtask surface` needs the library module's version raised.

## 5. The chat drives everything (founder Q10)

### 5.1 The principle

The chat is "Claude Code for PCBs": asked to build a board, it plans, searches and makes parts, places, wires, lays out, routes, checks and exports. It does so **with the same catalog entries and the same six verbs as every other caller** (`find`, `describe`, `query`, `apply`, `act`, `show`), through `Surface::call` as `LocalAgent` (endpoint) or `RemoteAgent` (ACP agent), as in the [assistant panel design](../assistant-panel.md#5-how-it-sits-on-the-surface). There is **no chat-only entry and no narrower path**, and no placement entry of its own: the chat places with `component.place`. Anything the palette, the dialogs or the CLI can do, the chat can do, because they all call the same entries.

What bounds it is the person's, never the design's:

- **Permissions** (Settings › AI › Permissions): document edits are allowed inside a reviewable turn (approved 5 November); files, network (the registry), publishing to folders and exports are Ask-class and show an Allow/Deny card; keys, endpoints and grants are Never-class.
- **Review gates**: every preview is a dry run with an impact card bound to its revision; a workflow's `confirm` gates; each message is one labelled undo step with **Undo turn**.
- **Limits** per turn (requests, tokens, time, edits, calls) from Settings › AI.

### 5.2 Part search and part making, end to end in the chat

| Step | What the model calls | What the person sees |
|---|---|---|
| Understand the request | `query view.context`; for a board, its own plan (a list in the reply) | The plan, as text |
| Find each part | `query library.search {text, cat?, filters}`, the same ranking as the palette (§1.2) | A tool row per search; when the choice is the person's (two LDOs that fit), a `choice` card with the top hits in the palette's row layout |
| A generic part | `apply component.place {part: <generic offer>, value, at}`: made and placed in one step, as the palette does | The placed part; the turn summary |
| A held or library part | `apply component.place {part, at}` (or `library.place_from` for a folder part, which copies it in) | Same |
| A registry part (later) | `query library.search … src:online`, then `act library.import_preview`, then `apply library.import` | The import preview card (licence, provenance, closure); network and import are Ask-class |
| No part found, datasheet available | `show assist.offer {workflow: "datasheet-to-component", inputs: {part}}`: the workflow runs in the chat with its own controls (pick, pinout impact card, result) | The workflow card, then its controls |
| No part found, pins known | `apply library.new_part {name, category, package: {generate: …}, pins}`; it chose the package from `query library.generators` | The new part's tool row; refusals name the unmapped pad |
| No category fits | `apply library.category_define {scope: user, …}` (settings-class: an Ask card) | Allow/Deny card |
| Place, wire and lay out | `component.place`, `schematic.connect`, the board and routing entries; geometry through the [compact encoding](../llm-geometry.md) | Each message one undo step |
| Check and export | `checks.*`, then `fab.*` (Ask-class: files) | The checks' findings; an Allow/Deny card before files are written |

- **Making stays reviewable.** Parts the chat makes are published only through the same publish rules as the dialog (§2.3); the datasheet workflow keeps its `reach` (it cannot publish, as approved), and its result card shows **Publish** and **Place** as the person's actions. Outside a workflow, `library.new_part` publishes as part of the turn and is undone with it.
- **The palette's make rows open the chat** with the workflow card; the chat's result card's **Place** arms placement in the window. Both are the person's clicks, not catalog entries.
- **A board-sized request spans several turns.** One message is one turn; a build runs as the model's plan of turns (parts, schematic, placement, routing, checks, export), each its own undo step within the per-turn limits. See open question 5.

### 5.3 What changes in the assistant plan

- The assistant design's slice list is unchanged; this design adds no entry to it. `assist.offer` and the `choice` card are its C2. The datasheet workflow's data changes (K7 below): `make` becomes package scope plus `library.new_part`, `pick` gains **Choose another package…**, and the workflow's version is raised to 1.1.
- `method.md` gains a short section "Finding and making parts" (search first, then generic, then datasheet, then `new_part`; never place a package), served to external MCP clients as well.

## 6. Online registry: `parts.sparkforge.io` (sketch; follow-up design)

This section fixes the shape so §1 does not change when the registry arrives. A full design (wire format, publisher accounts, moderation, upload) is a **follow-up document**, slice R0.

### 6.1 What an entry is

An **entry** is one immutable, versioned unit: `publisher/name@version` (version `major.minor.patch`; a published version never changes). Its **manifest** (canonical JSON, the generator ABI's encoding):

| Field | Contents |
|---|---|
| `kind` | `part`, `package` or `generator` |
| `id`, `version`, `title`, `summary` | Identity and the search row's text |
| `category`, `style`, `shape` | The facets of §1.3 (category path, package style, `PackageShape`), so the index is searchable without the blobs |
| `fields` | Typed values (Resistance, Rated voltage…), `MPN`, `Manufacturer`, value template if any |
| `content` | For `part`/`package`: the revision closure as a `library.bundle` (component, symbol, binding, package, footprint, 3D model), each file by SHA-256. For `generator`: the generator bundle (module, part files, `generator.json`) by bundle hash |
| `recipe` | For a generated part or package: `{generator: id@version, bundle hash, scope, params}`, so a client can regenerate and compare result hashes |
| `provenance` | Sources (datasheet URLs with page cites, manufacturer CAD files used like datasheets), the workflow id and version that made it, review report digest, author |
| `licence` | SPDX expression for the data and for each model file, and a redistribution flag per file (a manufacturer STEP may be linkable but not redistributable) |
| `depends` | Other entries by `id@version` and digest (a part on a registry package; a generated part on its generator) |
| `level` | Completeness (symbol, footprint, 3D, MPN, reviewed) |
| `digest`, `signatures` | SHA-256 of the canonical manifest; the publisher's signature over it; the registry's countersignature |

### 6.2 Index and search API (shape)

- `GET /v1/index` → the current index snapshot id and its shard list; shards are content-addressed, so a client fetches only what changed.
- `GET /v1/search?q=&cat=&style=&pins=&pitch=&body=&field…&page=` → rows in the `SourceCatalog` entry shape (identity, facets, level, digest), plus facet counts. The query terms are the same as §1.3's, so the client sends its own query unchanged.
- `GET /v1/entries/{publisher}/{name}/{version}` → the manifest. `GET /v1/blobs/{sha256}` → content, immutable and cacheable.
- Read-only and anonymous. Publishing to the registry is outside this design.

### 6.3 Trust

- **Digests everywhere**: every blob and manifest is checked by SHA-256 before use; a mismatch is refused, naming the file.
- **Signatures**: publishers sign manifests (Ed25519); the registry countersigns. The client holds the registry's key per registry in its settings (Never-class: no agent may change it). An unsigned or unknown-key entry is refused.
- **Generators** from the registry go through the existing plugin approval by bundle hash before any module runs.

### 6.4 Offline cache

- `<cache>/registry/<host>/` holds index shards and blobs by digest. Search works offline on the last snapshot and says its date ("index of 14 November").
- The cache is never authoritative and never part of a project: placing **imports** an exact snapshot into the project (as `library.place_from` does today), so the project opens identically without the network.

### 6.5 Import preview

Before the first place of a registry part: the closure (what is new, what the project already holds by digest), licence and redistribution per file, provenance and signatures, the generator approval if a module is new, and the recipe check (regenerated and equal, or "not verified: generator not approved"). Apply imports as one undo step and places. Network fetches are Ask-class for agents.

## 7. Wireframes

### 7.1 Place palette, 1920 × 1080 (sheet 1200 × 720, centred on the canvas)

```
┌ Place ───────────────────────────────────────────────────────────────────────────────────────────────┐
│ [ tlv757                                          ] [cat: ICs ×] [pins:5 ×]          Esc close      │
├──────────────────┬──────────────────────────────────────────────────────────┬───────────────────────┤
│ CATEGORIES       │ 3 parts                                       sort: best  │  TLV75733PDBVR        │
│ ▸ Passives   212 │ ▌TLV75733PDBVR   LDO 1 A 3.3 V      ICs/Power  SOT-23-5   │  ┌──────┐  ┌────────┐ │
│ ▸ Magnetics   18 │ ▌ Project · r2 · used 1   sym·fp·3D·MPN     in project    │  │ IN OUT│  │ ▫ ▫ ▫  │ │
│ ▸ Discretes   41 │  TLV75718PDBVR   LDO 1 A 1.8 V      ICs/Power  SOT-23-5   │  │GND EN │  │ ▫   ▫  │ │
│ ▾ ICs         17 │   Library · team-parts · r1  sym·fp·3D·MPN  new           │  └──────┘  └────────┘ │
│     Power      9 │  TLV757P (adj)   LDO 1 A adj        ICs/Power  SOT-23-5   │   [ 3D thumbnail   ]  │
│ ▸ Sensors      5 │   Shipped               sym·fp·3D·—      new              │  Vout 3.3 V  Iout 1 A │
│ ▸ Connectors  12 │ ─────────────────────────────────────────────────────────  │  Package SOT-23-5     │
│ ▸ …   ▸ Other  2 │  + Make "tlv757" from its datasheet…          (chat)      │  TI · TLV75733PDBVR   │
│ FILTERS          │  + Make "tlv757" on a package…                            │  Source: project r2   │
│ Style  SOT 3     │                                                           │                       │
│ Pins   5 ▣  8 □  │                                                           │                       │
│ Pitch  0.95 mm   │                                                           │                       │
│ Source ▣Proj ▣Lib│                                                           │                       │
│ ▣Shipped ▣Generic│                                                           │                       │
├──────────────────┴──────────────────────────────────────────────────────────┴───────────────────────┤
│ ↑↓ move   Enter place   Shift+Enter place & keep open   Space preview   Cmd+D favourite   Esc close │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

The rail lists the merged categories in data order; empty ones are hidden, Other is last.

### 7.2 Place palette, 1024 × 640 (sheet fills the canvas area, about 760 × 520)

```
┌ Place ──────────────────────────────────────────────────────────────┐
│ [ 10k 0603                                        ]        Esc      │
│ (All) (Passives) (Magnetics) (Discretes) (ICs) (Sensors) (Conn) ›   │
│ res: 10 kΩ ×   pkg: 0603 ×                         Filters ▾        │
├─────────────────────────────────────────────────────────────────────┤
│ ▌Chip resistor 0603   10 kΩ          Passives  0603  1.6×0.8        │
│ ▌ Project · used 4 · value from your text           in project      │
│  Chip resistor 0603 1%  10 kΩ         Passives  0603                │
│   Library · team-parts · Yageo RC0603FR-0710KL      new             │
│ ─────────────────────────────────────────────────────────────────── │
│  + Make "10k 0603" from its datasheet…                              │
├─────────────────────────────────────────────────────────────────────┤
│ ↑↓  Enter place  Space preview  Esc                                 │
└─────────────────────────────────────────────────────────────────────┘
```

Space replaces the list with the preview, and Space again goes back. The category chips scroll sideways (›), and Filters ▾ opens the facet panel over the list.

### 7.3 Package picker, 1920 × 1080 (sheet 1400 × 800, from New part › Package › Choose…)

```
┌ Choose a package for "TLV75733PDBVR" ─────────────────────────────────────────────────────────────────┐
│ [ sot-23-5                                ] [pins:5 ×]                                    Esc back   │
├────────────────┬──────────────────────────────────────┬───────────────────────────────────────────────┤
│ STYLES         │ 4 packages                           │ SOT-23-5 · Tabbed small-outline v2            │
│ Chip       14  │ ▌SOT-23-5   2.9×1.6  0.95  JEDEC     │ ┌ Part ───────────────────────────────────┐   │
│ Molded      9  │ ▌ Shipped generator · cited lands    │ │ Name  [TLV75733PDBVR            ]       │   │
│ ▾ SOT / SC  4  │  SOT-23-5 (in project, used by 1)    │ │ Pins  [1 IN, 2 GND, 3 EN, 4 NC, 5 OUT]  │   │
│ SOIC/TSSOP 21  │   Project · package r1               │ └─────────────────────────────────────────┘   │
│ QFN / DFN  38  │  SC-74A (SOT-753)  2.9×1.6           │ ┌ Body ─┐ ┌ Leads ┐ ┌ Lands ┐  preset ▾     │
│ QFP         8  │   Shipped generator · estimated      │ │ L 2.90│ │ n 3+2 │ │ TI ▾  │               │
│ BGA / LGA  11  │  Custom SOT…                         │ └───────┘ └───────┘ └───────┘               │
│ Through-h. 26  │   tabbed v2 · start from defaults    │  [footprint preview]   [3D preview]           │
│ Headers    40  │                                      │  Lands are cited: TI SBVS... p.32             │
│ Modules     3  │                                      │  5 of 5 pads mapped                           │
│ Mechanical  6  │                                      │                                               │
│ FILTERS pins 5 │                                      │                       [Cancel] [Use package]  │
├────────────────┴──────────────────────────────────────┴───────────────────────────────────────────────┤
│ ↑↓ move   Enter use package   Tab to the form   Esc back                                             │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 7.4 Package picker, 1024 × 640 (two steps in one sheet)

```
┌ Choose a package ─────────────────────────────┐   ┌ ‹ SOT-23-5 · tabbed v2 ───────────────────────┐
│ [ sot-23-5                ]            Esc    │   │ Name [TLV75733PDBVR       ]                    │
│ (All)(Chip)(SOT)(SOIC)(QFN)(QFP)(BGA)(THT) ›  │   │ Pins [1 IN, 2 GND, 3 EN, 4 NC, 5 OUT ]         │
├───────────────────────────────────────────────┤   │ ▾ Body   L 2.90  W 1.60  H 1.10 mm            │
│ ▌SOT-23-5     2.9×1.6  0.95   cited     ›     │   │ ▸ Leads  ▸ Lands (TI)                          │
│  SOT-23-5     in project (used by 1)    ›     │   │ [footprint]          [3D]                      │
│  SC-74A       2.9×1.6  0.95   estimated ›     │ → │ Lands are cited: TI SBVS… p.32                 │
│  Custom SOT…                            ›     │   │                                                │
├───────────────────────────────────────────────┤   ├────────────────────────────────────────────────┤
│ ↑↓  Enter open form  Esc back                 │   │ Esc list            [Cancel] [Use package]     │
└───────────────────────────────────────────────┘   └────────────────────────────────────────────────┘
```

### 7.5 New part dialog, docked (right column, both sizes)

```
┌ New part ─────────────────────────┐
│ Name      [TLV75733PDBVR        ] │
│ Maker     [Texas Instruments    ] │
│ Category  [ICs / Power        ▾ ] │
│ Package   SOT-23-5 · 5 pins       │
│           2.9 × 1.6 mm [Choose…]  │
│ Pins      ┌─────────────────────┐ │
│           │1 IN  2 GND  3 EN    │ │
│           │4 NC  5 OUT          │ │
│           └─────────────────────┘ │
│           5 of 5 pads mapped      │
├───────────────────────────────────┤
│ [Cancel] [Create and open] [Create│
│                       and place]  │
└───────────────────────────────────┘
```

With a mechanical-only package (a mounting hole), Pins is replaced by "No electrical pads: a mechanical part, no pins" and Category defaults to Board features.

## 8. Slices

At most five agents at once. Every slice ships with its tests. UI evidence is gathered in the VM only, never on the host desktop.

| # | Slice | Delivers | Hours | Needs |
|---|---|---|---|---|
| **K0** | Contract | `categories.json` format 1 (with `same_as`, `value_template`, styles, `other`) and its load check; the shipped category and style data (§1.3); ABI optional fields `PartOffer.style`, `parts`, `categories`, `Preset.style`, `tags` (ABI minor); the Category and Value template standard-field records; `generic-families.json` `category`; GLOSSARY **Part** entry; the nine generators' declarations, versions raised, modules rebuilt | 7–10 | — |
| **K1** | Core search and categories | `Source::{Shipped, Generic}`; `cat:`, `style:`, `src:`, `is:` terms; source precedence in ranking; generic outputs only from `parts`; `catalog::categories::merge` over shipped, plugin, folder and user sources with its refusals; Other for unknown ids; category facets; `library.search` reports `make` offers; catalog module version bump | 9–13 | K0 |
| **K2** | Generation rules, values, BOM | Host check of a `PartOutput` (pads, pins, binding) and `generator.not_a_part`; package half keyed and named as a package, shared across parts; `value_field` on generic parts with the parsed value on the instance; value-template rendering (schematic value text, Properties, BOM Value, search rows) with its load check; the value to-do; BOM Package column | 9–13 | K0 |
| **K3** | Place palette | The list on `kit/list`, category rail and chips from merged data, facet panel, rows, preview pane and Space, make rows (chat offer, New part). "From generator" removed. PCB `P` with mechanical and board-only first | 10–14 | K1 |
| **K4** | Package picker | `package_picker.rs` replaces `generator_gallery.rs`: styles, filters, rows from presets, customs and held packages, the one form with the Name/Pins section. Entry from Change package…, From generator, start page. Gallery deleted | 8–12 | K0 |
| **K5** | New part | `library.new_part` (package scope, pins through the pinout import, explicit mapping, mechanical parts without pins, one undo step, refusals) and the publish check `component.pad_unmapped`; the docked dialog with Create and place / Create and open; library module version bump | 8–11 | K2, K4 |
| **K6** | Categories as the person's data | `library.categories`, `library.category_define` (user file, writable folder), folder manifest `categories`, New category… in the dialog, Settings › Library › Categories (tree, sources, overridden values, per-category value template) | 5–7 | K1 |
| **K7** | Chat end to end | The datasheet workflow 1.1 as data (package scope plus `library.new_part`, Choose another package… in `pick`, Publish and Place on the result card); `method.md` "Finding and making parts"; a recorded chat run (replayed in tests) that searches, places generic passives, makes an LDO through `new_part` and places it, with no paid calls | 5–8 | K3, K5, assistant W2 |
| **K8** | Examples regenerated | `examples/` boards and parts made again with the new generators and `new_part`; golden and parity records blessed; package-only components gone from every example | 2–4 | K2, K5 |
| **K9** | Evidence | VM run at 1920 × 1080 and 1024 × 640: run 11's board made with names, pins and values, the BOM checked, a user top-level category created and used, a mounting hole placed as a mechanical part | 3–4 | K3–K8 |
| **R0** | Registry design (documents) | The follow-up design of §6: wire format, signing and key rotation, publisher accounts, upload and moderation, rate limits, cache eviction, import preview UI. Documents only | 4–6 | — |

**Order** (≤5 parallel):

| Round | Agents |
|---|---|
| 1 | K0 (alone, the contract), R0 (documents, independent) |
| 2 | K1, K2, K4 |
| 3 | K3, K5, K6 |
| 4 | K8, K7 (once assistant W2 is merged) |
| 5 | K9 |

**Total** about **66–96 agent hours** for K0–K9, plus **4–6 h** for the registry design (R0); building the registry itself is estimated in R0. The critical path is K0 → K2 → K5 → K7 → K9, about 32–46 h, with K7 waiting on the assistant plan's W2. Run 11's three findings are closed after round 3, with K2 and K5 doing the work.

## 9. Open questions, each with a recommendation

1. **Does a project carry the category definitions its parts use?** A project from someone with their own top-level "Audio" opens elsewhere with "Other · audio". *Recommendation: no, not now.* The Category field keeps the path id, which is never rewritten; Other shows it readably. Carrying definitions would add project data (an edit of format version 1) and a second copy of each definition, for a cosmetic gain. Revisit when the registry ships categories.
2. **Category ids from different sources: merge by id, or namespace them?** *Recommendation: merge by plain id* (equal paths are one category), with `same_as` for differing names and titles from the highest-precedence source. Namespaced ids (`team:audio`) would keep `cat:` filters from meaning the same thing across folders, which is why categories exist.
3. **Can a part be created with pins named only by number?** *Recommendation: no.* Every electrical pad needs a named pin (NC counts) before publishing or placing. Create and open still allows unfinished drafts. A part with pins "1…5" is what the founder ruled out.
4. **Should publishing a part the chat made ask the person?** *Recommendation: inside a turn, no*: `library.new_part` publishes as part of the turn, which is one undo step, and the datasheet workflow keeps its approved rule (no publish inside the workflow; Publish on the result card). Publishing to a library folder stays Ask-class.
5. **Board-sized requests and per-turn limits.** "Build me a USB-C LDO board" exceeds 60 edits in one turn. *Recommendation:* the model works as a plan of turns (one per stage), each its own undo step within the limits, and continues on its own within a total-run budget the person sets in Settings › AI (default: ask before each stage). A "build a board" workflow as data comes later, as its own decision.
6. **Should category data carry a default designator prefix** (R, C, L, Y, F, D, Q, U, J, SW, K, H, TP, FID)? *Recommendation: yes, as an optional field in `categories.json`*, used only as the New part dialog's default. Today the prefix is typed by hand or guessed, which is a hidden rule.
7. **Modules: top-level category or package style?** *Recommendation: style* (`style:module`), with the part under its function (RF, Power, Sensors). Distributors file modules by function; a function-first tree avoids a part belonging in two places.
8. **Registry parts made by a generator: ship the result, the recipe, or both?** *Recommendation: both.* The client accepts the result by digest and signature, and, when the generator is approved, regenerates from the recipe and compares result hashes, showing "verified by generator" in the import preview.
9. **Registry hosting and identity.** `parts.sparkforge.io` serves read-only, anonymous search and blobs. *Recommendation:* publisher accounts, upload and moderation are designed in R0 and need the founder's decision before any service is built or published.

## Appendix: category research

The founder asked whether six top-level categories (passives, discretes, ICs, connectors, modules, mechanical) miss anything. Compared with three widely used distributor trees and the function-based classification standards, on 16 November 2026. Only the top-level groupings were compared; no category data was copied, and the shipped titles and subcategories above are this project's own wording.

| Family | Digi-Key top level | Mouser top level | LCSC catalog page | Standards | Earlier six | Recommended |
|---|---|---|---|---|---|---|
| Resistors, capacitors | Resistors; Capacitors; Potentiometers | Passive Components | Resistors; Passives | IEEE 315 R, C | Passives | Passives |
| Inductors, ferrites, transformers | Inductors, Coils, Chokes; Transformers; Magnetics | Passive Components | Transformers | IEEE 315 L, T, FB | Passives (inductors only) | **Magnetics** |
| Crystals, oscillators | Crystals, Oscillators, Resonators | Passive Components | Crystals, Oscillators, Resonators | IEEE 315 Y, G | Passives | **Frequency control** |
| Fuses, TVS, varistors | Circuit Protection | Circuit Protection | Circuit Protection | IEC 81346-2 "protecting" class; IEEE 315 F | Split (passives, discretes) | **Protection** |
| Diodes, transistors | Discrete Semiconductor Products | Semiconductors | Discrete Semiconductors | IEEE 315 D, Q | Discretes | Discretes |
| LEDs, displays, optocouplers | Optoelectronics | Optoelectronics | Optoelectronics | — | Discretes (LEDs only) | **Optoelectronics** |
| ICs | Integrated Circuits (ICs) | Semiconductors | Integrated Circuits (ICs) | IEEE 315 U | ICs | ICs |
| Sensors | Sensors, Transducers | Sensors | Sensors, Transducers | IEC 81346-2 "sensing" class | ICs/Sensors | **Sensors** |
| Antennas, RF | RF and Wireless | (Semiconductors, Passives) | RF and Wireless | IEEE 315 E (antenna) | ICs/RF, Modules/Radio | **RF and wireless** |
| Batteries, board power | Battery Products; Power Supplies – Board Mount | Power | Battery Products; Power Supplies | IEEE 315 BT | Modules/Power | **Power** |
| Switches, relays, audio, motors | Switches; Relays; Audio Products; Motors…; Fans… | Electromechanical | Switches; Relays; Audio; Motors | IEEE 315 S, K, LS; IEC 81346-2 "switching" class | missing | **Electromechanical** |
| Connectors, cables | Connectors, Interconnects; Cables, Wires; Cable Assemblies | Connectors; Wire and Cable | Connectors; Cables | IEEE 315 J, P, W; IEC 81346-2 "connecting" class | Connectors (no cables) | **Connectors and cables** |
| Heatsinks, fasteners, enclosures | Fans, Thermal Management; Hardware, Fasteners; Boxes, Enclosures | Thermal Management; Enclosures | Hardware, Fasteners; Thermal | IEEE 315 H (hardware) | Mechanical | Mechanical and hardware |
| Test points, fiducials, mounting holes | Test and Measurement (test points) | — | — | Common designators TP, FID, H, MH | Mechanical | **Board features** |
| Modules | Spread by function (RF transceiver modules under RF; power modules under power supplies) | Spread by function | Spread by function | IEEE 315 A (assemblies) | Modules | Package style `module` |
| Tools, test equipment, office, industrial, kits | Many | Many | Many | — | — | Not board parts; left out. Any source may add them; Other catches the rest |

**Result.** The six missed **electromechanical** parts (switches, relays, buzzers, motors) entirely, and folded six families that every distributor keeps at the top level into other headings: **magnetics, frequency control, circuit protection, optoelectronics, sensors and RF**. Power and batteries, and cables, also had no home. Distributors and IEC 81346-2 classify by function; the shipped set therefore files by function, keeps "module" as a package style, and adds a **Board features** category for the pin-less and one-pin board items (fiducials, mounting holes, test points, net ties). The recommended shipped top-level set is the fifteen of §1.3: **Passives, Magnetics, Frequency control, Protection, Discretes, Optoelectronics, ICs, Sensors, RF and wireless, Power, Electromechanical, Connectors and cables, Mechanical and hardware, Board features, Other.**

Sources (top-level names only, read 16 November 2026): [Digi-Key product index](https://www.digikey.com/en/products); [LCSC products](https://www.lcsc.com/products) (its catalog page lists a largely Digi-Key-aligned tree); [Mouser products](https://www.mouser.com/products) (Passive Components, Semiconductors, Optoelectronics, Electromechanical, Circuit Protection, Sensors, Connectors, Wire and Cable, Thermal Management, Power); [reference designator class letters](https://en.wikipedia.org/wiki/Reference_designator) (IEEE 315, ASME Y14.44); [IEC 81346-2:2019](https://webstore.iec.ch/en/publication/29181) (classification of objects by inherent function).
