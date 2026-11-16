# Part search for placing, package picker for making

Design for founder approval, 16 November 2026, written against `main` at 95675ef3. Documents only: nothing was built. Hours are agent hours and are estimates.

## Founder decisions this design follows (16 November 2026)

- A **generator makes a package**: the footprint, the 3D model and the lead layout, from dimensions.
- A **part** is a package plus its pinout and functions, symbol, values and MPN.
- **Placing searches parts.** Results come from the project, library folders, the shipped library and the user's own parts. Generic passives and headers count as parts because their parameters are the whole part. A bare package is never placeable on a schematic. (A demo placed a part named just "SOT-23-5".)
- **Generators belong to part making**: the part editor's package step and the datasheet workflow. The generator-chip gallery (one chip per generator) "does not scale" and is no longer an entry point for placing.

Run 11 (`docs/e2e-run-11.md`, not yet on `main`) adds three concrete problems this design must fix:

1. **Naming a placed IC took the long way.** The palette's one-keystroke path names the part after its package, with no name field. A real name meant Component › New, then the generator form's Name, then Create, Publish and Place. Pin names were typed one row at a time.
2. **Passive values needed manual setup.** A value could be set only after adding the Resistance or Capacitance field to the component and updating the placed parts.
3. **The BOM was wrong.** The Package column read "TLV75733PDBVR package" instead of SOT-23-5. The Value column was empty for the ICs, the LED and the header.

## What exists today (and what changes)

| Today | Where | After this design |
|---|---|---|
| The Place palette lists project components, then "Generic parts" (on-demand generator parts), "From generator" presets of every generator, built-in parts and library-folder rows. Enter on a preset makes a component *named after the package* and arms placement | `apps/sparksmith/src/place_palette.rs` (3,109 lines), `catalog::generic::generators` | One ranked list of **parts only**. Package presets leave the palette. When nothing matches, the last rows offer **Make it…** |
| The generator gallery is a sheet: chips for the 9 generators, then presets, then the one form. It opens from New component, the start page, the palette, the Footprint tab's From generator and Change package… | `generator_gallery.rs` (964), `generator_form.rs` (462) | Replaced by the **package picker**: search, categories and filters, then the same `generator_form`. It is reached only from part-making entry points |
| Component-scope generation of an IC package gives terminals "1…n" and names the component, package and footprint after the preset (or after the Name the person typed) | `generators/*/src/lib.rs` (`Component { name, terminals }`), `library::generated::expand` | Component scope is only for **declared complete parts** (passives, LEDs, diodes, headers) and for part-making with a name. The package half is always named after the package |
| `PartOffer.instance_fields` declares `value` (chip) and `colour` (chip LED) or nothing at all, and GP1 selects no field definitions | `generator-plugins.md` "Limits of GP1" | Generic parts carry their typed value field (Resistance, Capacitance, Inductance) from the start. The palette's parsed value goes onto the instance |
| `generic-families.json` holds the search grammar for generic parts | `library/catalog/` | It stays, and gains the category and the complete-part mark |
| The catalog's `Source` is `Project` or `Library(id)`. Filters are `pkg:`, `pins:`, `pitch:`, `body:`, `has:` and field predicates. Ranking is match tier, then used, favourite and recent | `crates/editor-core/src/catalog{,/query}.rs` | `Source` gains `Shipped`, `Generic` and later `Online`. The query gains `cat:` and `style:` |

## 1. Part search for placing

### 1.1 Entry points (one search, `library.search`)

| Entry | How it opens | What Enter does |
|---|---|---|
| **Place palette** (Schematic: `P`, the toolbar, the command palette's "Place…") | A sheet over the canvas with the cursor in the field. The last query and category are kept per project | Places the part. A generic part not yet held is made and placed (one undo step with the first placement, as today) |
| **PCB** (`P` in the board editor) | The same sheet, with `cat:mechanical` and board-only parts first | A mechanical part goes on the board. A schematic part is refused in place: "Place it in the schematic; the board follows with Update PCB" |
| **Chat** | The model calls `query library.search` through the verbs. The chat shows the hits as a `choice` card: the top 5 rows, the same row layout as below | Choosing a hit arms placement on the canvas through a new show entry, `view.arm_place {part}`. This is the palette's Enter, so the result is identical. When there are no hits the model offers the datasheet workflow (`assist.offer`) with `part` filled from the text |
| **Assign mode** (Choose part dialog, Resolve parts) | Unchanged: `kind:part` with the instances' requirements | `instance.assign_part` |

### 1.2 Sources and ranking

The sources, in **precedence order**:

1. **Project.** Every placeable component lineage the project holds, including generic parts it has already made.
2. **Library folders** (S3, as built): read-only or writable folders listed in the user's settings.
3. **Shipped library.** `library/parts/` components such as the GCT USB4105 and the mechanical parts. Today these are the palette's "Built-in parts" group.
4. **Generic parts on demand.** A text that the grammar (`generic-families.json`) or a complete-part preset reads as a whole part, such as "10k 0603", "led 0805 red" or "header 2x5 2mm". Such a part is made when chosen. Only offers that a generator declares complete are listed (§3.2).
5. **Online registry** (later, §1.7). It is ranked after every local source.

Ranking keeps the built rule (parts-library "Ranking") and adds source as the last key before the final ties:

1. **Match tier**: exact, prefix, word, substring, field or filter. A generic part whose whole grammar matches ranks **Exact**. "10k 0603" is that part, not a fuzzy hit.
2. **Boosts**: used in this project, then favourite, then recent.
3. **Source precedence** (above). A generic part the project already holds is listed once, as a project row.
4. **Final ties**: folded name, then lineage id.

The list is **one ranked list**, with each row's source shown as a chip, not a group heading. Groups by source no longer scale once folders and the registry arrive, and they push the exact hit below weaker rows. When the field is **empty**, the sheet browses instead: Recent, then Favourites, then the selected category's rows.

**Never listed:** package presets (that is, a generator preset that is not a declared complete part), unpublished drafts (they count in "N hidden (draft)", as today) and components flagged as bare packages (§4). A bare package still in the project shows in the hidden count: "1 hidden (package only)".

### 1.3 Categories by function, and filters

There are six top-level categories, each with subcategories. All of them are **data** (§3.1), not code:

| Category | Subcategories (first data set) |
|---|---|
| Passives | Resistors, capacitors, inductors and ferrites, crystals and resonators, fuses |
| Discretes | Diodes, LEDs, transistors, MOSFETs, TVS and ESD |
| ICs | Power, interface, sensors, logic, MCUs, memory, analogue, RF |
| Connectors | Headers, USB, board-to-board, terminal blocks, cards |
| Modules | Radio, power, sensor |
| Mechanical | Fasteners, standoffs, heatsinks, test points, fiducials |

**Filters**, all of them typed terms of the one query language:

| Filter | Term | Source of the values |
|---|---|---|
| Category | `cat:passives`, `cat:passives/resistors` | The part's Category field (§3.3). For generic offers, the generator's declaration |
| Package style | `style:sot`, `style:qfn` | The package's `style` facet (§3.2) |
| Pins | `pins:5`, `pins>=8` | `PackageShape` (built) |
| Pitch | `pitch:0.5mm` | `PackageShape` (built) |
| Body | `body<=3x3` | `PackageShape` (built) |
| Values | `res:10k`, `V>=16`, `tol<=1%` | Typed fields (built). The value field the category declares appears as its own facet |
| Source | `src:project`, `src:library`, `src:shipped`, `src:generic` | Entry source (`src:` exists; new values) |
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

- **Where it comes from**: Project, `Library · <folder>`, Shipped, Generic · `<generator> v<N>` or (later) Online.
- **Held or new**: "in project", "in project · update available (r3)" (the S3 update badge, built), or "new" (copied or made on place).
- **Regeneration status** of generic parts, from `library::generated::standing` (G15): "made on place", "in project", "older generator: made again as r2 on place" (Enter opens the form review, as today) or "made by a newer generator: can't use" (disabled, with the reason on hover).
- **Completeness**: `sym · fp · 3D · MPN`. Missing items are dimmed.
- **Value** for a generic part: the value parsed from the text ("10 kΩ"). It is written to the instance's typed field on placement (§3.4).

A preview pane (on the right at 1920 wide, toggled with Space at 1024) shows the symbol, the footprint, a 3D thumbnail, fields and provenance. All of it comes from `library.inspect`, as built.

### 1.5 When nothing matches: "Make it…"

**Make rows** always end the list. When the list is empty they are the only rows, and Enter takes the first one:

| Row | Shown when | What it does |
|---|---|---|
| **Make "TLV75733PDBVR" from its datasheet…** | Always | Opens the chat with the datasheet workflow's card (`assist.offer`), with `part` set to the text and the workspace given as context. The run's result card has **Place** (`view.arm_place`) next to Open in editor |
| **Make "TLV75733PDBVR" on a package…** | Always | Opens the **New part** dialog (§2.3) with Name set to the text |
| **Make a resistor 0603 with 10 kΩ** | The text reads as a generic part (already listed as a row above) | Not repeated |

This fixes run 11 finding 1: the name is given where the part is made, and the part is placed directly. Neither path places a bare package.

### 1.6 Generic passives and headers are complete parts

- One component lineage per generic **kind and size**, for example "Chip resistor 0603". The **value is per instance**, as a typed requirement. This is parts-library D5's "design by requirements", and Resolve parts then picks the MPN.
- The generator declares which presets and families are complete parts (§3.2): chip R, C, L and ferrite, chip LED, molded diode and LED, pin header and THT axial or radial R and C. The declaration names the category, the symbol kind and the typed value field.
- **Not complete**, so only offered for making: tabbed, gull-wing and no-lead packages (SOT, SOIC, QFN, …), grid, module and DIP. A transistor in SOT-23 is not generic either, because its pin order depends on the MPN.

### 1.7 How folders and online sources slot in

- **Library folders** are built (S3): their `SourceCatalog` entries already pass through the same `Index`. This design adds their Category field and facets to `SourceCatalog` entries, which needs no format change because these are fields on component revisions.
- **Online registry** (V1 Q3 and Q4, later): it implements the same `SourceCatalog` shape, fetched and cached per user (S5 rules: never authoritative, never written into a project). Its rows read "Online · <registry>". Enter shows an **import preview** (the closure, licence and provenance) and then places, as `library.place_from` does today. Ratings and part levels (symbol + footprint + 3D minimum) become extra facets. Nothing in §1.2 to §1.5 changes. The registry is one more source with the lowest precedence.

## 2. Package picker for making

### 2.1 Where it appears

| Entry | Scope | Result |
|---|---|---|
| Part editor › **New part** dialog (§2.3) › Package "Choose…" | Component draft with a name | `library.generate_part` (scope `component`, `name` given), then the drafts open |
| Part editor › Package step / 3D tab › **Change package…**, and the Footprint tab › **From generator** | The working set's package | `library.package_from_generator` (built) |
| **Datasheet workflow**, `pick` step | The model's choice | The `choice` card shows the model's generator, preset and values with up to 2 alternatives. **Choose another package…** opens the picker. Its answer replaces `/steps/plan` and the run goes on (no new control kind) |
| Component start page | — | "New from a generator" becomes **New part…** (the dialog) |

It is **not** reachable from the Place palette, the PCB editor or the Open page.

### 2.2 Search, categories and the single form

- **Rows** are packages, not generators. Each preset is one row ("SOT-23-5", "SOIC-8 3.9 mm", "QFN-16 3×3 0.5 EP 1.7"). Each generator also gets one "Custom <style>…" row that starts from its defaults. The rows come from `library.generators` (the shelf). Later, packages from library folders and packages the project already holds join them, with their source chip, so "a SOT-23-5 we already have" ranks first.
- **Categories** are package styles, from data (§3.1): Chip, Molded, SOT/SC, SOIC/SOP/TSSOP, QFN/DFN, QFP, BGA/LGA, Through-hole (axial, radial, DIP, TO), Headers, Modules.
- **Filters**: `pins`, `pitch`, `body`, `style`, `lands:cited|estimated` (the preset's `basis`) and source. Aliases ("SOT-753", "SC-74A") and JEDEC names are searched as text.
- **The form** is the existing `generator_form` (built from the description, with groups, units, rules, refusals on their fields and G15 standing). In component scope the form has **Name (required)** and **Pins** as its first section (§2.3). In package scope it has neither.
- **Scale**: there are no chips per generator. The list is virtualised (`kit/list`) and bounded per query (ROWS 200, then "Show all"). Categories and facets come from data, so 50 generators or a plugin folder add rows and facet values without any UI change.

### 2.3 The New part dialog (making with a name)

This is a docked `kit::dialog::DialogPanel` in the right column (sidebar IA: a command with inputs). Its fields:

| Field | Notes |
|---|---|
| Name | Required, at most 64 bytes, set from the palette text. Becomes the component's name, and the MPN when it parses as one and Manufacturer is filled |
| Manufacturer | Optional. With Name it creates the `Part` record (one cascade with the publish) |
| Category | Choice from the category data, guessed from the package style (for example SOT → ICs). Can be changed |
| Package | Shows "SOT-23-5 · 5 pins · 2.9 × 1.6 mm" and **Choose…**, which opens the picker |
| Pins | A multiline paste: "1 IN, 2 GND, 3 EN, 4 NC, 5 OUT", or two columns pasted from a datasheet table. It is parsed by the built pinout import (`library.pinout_preview`), with the preview's row errors on the field. Left empty, pins are named by number and the part shows a to-do |
| Value | Shown when the category declares a value field |

**Create and place** (primary) sends `library.generate_part` (component scope, the name), then `library.pinout_apply`, then `library.publish`, as one undo step, and arms placement. **Create and open** opens the part editor instead. This is run 11's five-step path reduced to one dialog. The pins come from one paste.

## 3. Data

### 3.1 Category and style vocabularies: `library/catalog/categories.json`

These are shipped data, read through `ShippedLibrary` like `generic-families.json`. There is no list in code, so `no-builtin-data` holds (the file is read at runtime, never compiled in).

```json
{"format": 1,
 "categories": [
   {"id": "passives", "title": "Passives",
    "children": [{"id": "resistors", "title": "Resistors", "value_field": "Resistance"},
                 {"id": "capacitors", "title": "Capacitors", "value_field": "Capacitance"}]},
   {"id": "ics", "title": "ICs", "children": [{"id": "power", "title": "Power"}]}],
 "styles": [
   {"id": "sot", "title": "SOT / SC", "aliases": ["sot-23", "sc-70", "sc-74"]},
   {"id": "qfn", "title": "QFN / DFN", "aliases": ["mlf", "son", "vqfn"]}]}
```

The load check refuses a duplicate id, a cycle or a `value_field` that names no standard field, and names each one. Library folders may add **subcategories** under an existing top-level id, through their manifest's optional `categories`. The six top-level categories stay shipped data, so search stays comparable across sources (parts-library D4 asked for a few original starter shelves at most, and this is that list).

### 3.2 What a generator declares (in its description, beside its presets)

These are optional fields of the one plugin ABI (an added optional field is a minor change, founder 16 November 2026):

```rust
pub struct PartOffer {          // existing: families, scopes, grammar, instance_fields
    pub style: Option<String>,             // "sot": the styles id of its packages
    pub complete: Vec<CompletePart>,       // which outputs are whole parts (placeable)
}
pub struct CompletePart {
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

- **Placing** lists only the generic offers matched by a `complete` entry. **Making** (the picker) lists every preset.
- `generic-families.json` families gain `"category"`. A family whose generator declares no matching `complete` entry fails the shipped-data check.
- **Why the description and not `generator.json`**: presets, families and grammar already live there, and the module is embedded in projects (G3). A project opened elsewhere then searches the same way, and a third-party plugin brings its own facets. `generator.json` keeps only files, hashes and provenance. The shelf caches the read descriptions, so search never runs a module. See open question 1.
- Facet changes alter only the description, not what a module makes, so no generator `version` is raised (G14). The modules are rebuilt and their hashes change. Under G15, the same version built to other bytes "changes nothing".

### 3.3 How library parts carry the same facets

| Facet | Library part (component revision) | Generic offer |
|---|---|---|
| Category | Standard field **Category** (choice, `categories.json` ids) on the component. It is part of the revision, so it travels with copies and folders | `CompletePart.category` |
| Style | The package's `style`: set by its generator, or chosen when a package is drawn by hand | `PartOffer.style` / `Preset.style` |
| Pins, pitch, body | `PackageShape`, derived (built) | Preset params via the same shape function on the generation's leads |
| Value | Typed fields (built) | `CompletePart.value_field` plus the parsed text |
| Aliases | Component `description`, and the package name and aliases | `Preset.aliases` |

The Category standard field is **data**: one record added to `library/standard-fields/standard-fields.json`. Adding a standard field to a component uses the existing field machinery, so the file format does not change. Existing parts without the field are listed under "Uncategorised", which counts as a facet value and is filterable with `cat:none`.

### 3.4 Names and values out of a generation (run 11 findings 1 and 3)

- **Package and footprint names come from the package**, never from the component: "SOT-23-5", "SOT-23-5 footprint". Two parts made on the same package share one package lineage (the package half is keyed in package scope). The BOM's Package column then reads "SOT-23-5" for every part on it. See open question 4.
- **The value field.** A complete part's component carries `CompletePart.value_field` as a field (Resistance for a resistor). The palette's parsed value ("10k") is the placed instance's value. An instance with an empty value field is a to-do: "R3 has no resistance".
- **BOM columns.** Value is the instance's value field, else the Part's MPN, else the component name. With that rule, ICs, the header and modules read "TLV75733PDBVR", "Pin header 1×4 2.54 mm" and so on. LEDs read their colour field when they have no value. Package is the package lineage's name. See open question 5.

## 4. Migration

### 4.1 Projects that hold bare-package parts

- **Definition (core, derived, never stored).** A component is a **bare package** when its `GeneratedPart` record is component scope and no `complete` entry of its generator matches its params. A component the person has drafted and published since then (pins named, renamed) has no record (G2), so it is the person's own part and is never flagged.
- **Presentation**: "**SOT-23-5 · no part chosen**", wherever the component's name appears (the schematic value text, Properties header, BOM Value, palette hidden count). The designator is unchanged. The text is muted, in the style of the pending state. Nothing is renamed in the file.
- **Issues**: one to-do per component, with its instances: "U1, U4 are a SOT-23-5 with no part: choose or make its part". In the production check it is a warning ("Package without a part 2"), never an error, so existing boards still export.
- **Resolve parts** gets a group "Package only", above "No Part chosen", with two actions:
  - **Name this part…** opens the New part dialog on a draft of the *same lineage* (`library.draft_from`). Saving publishes the next revision, then the built update preview (`project.preview_update`) moves the instances. Nets stay, because terminals keep their numbers. No new engine command is needed.
  - **Use another part…** opens part search pre-filtered with `pins:5 style:sot`. Choosing a part sends a new update change, `Change::ComponentReplace {instances, target, terminals}`, which moves instances to another lineage. The terminal map is explicit and listed in the preview (by package terminal number, never by name). It is bound to the revision and is one undo step.
- **No file format change**: the flag is derived, and the new Change is a command, not stored. `cargo xtask surface` needs the updates module's version raised.

### 4.2 Removing the gallery and changing the palette

- `generator_gallery.rs` is deleted. Its form host moves into `package_picker.rs`, and `generator_form.rs` is kept. The Footprint tab's `footprint_template_picker.rs` opens the picker.
- In the palette, the "From generator" group, `generator_gallery::preset_row` and `KEYS_TEMPLATE` are removed. "Generic parts" and "Built-in parts" become source chips. The make rows are added.
- On the start page, "New from a generator" becomes **New part…**, and the start page's preset rows are removed. A search there lists parts, like the palette, plus the make rows.
- The tests that drive the gallery from the palette (`generator_tests.rs`, `placing_a_resistor_a_qfn_and_a_header`) change: the QFN is made through the New part dialog with a name and then placed.
- MCP is unchanged: `library.generators` and `library.generate_part` stay. A component-scope request **without** `name` for a non-complete offer is refused: "A SOT-23-5 package is not a part: give it a name, or make the package (scope package)". The datasheet workflow always passes `name` already.

## 5. Wireframes

### 5.1 Place palette, 1920 × 1080 (sheet 1200 × 720, centred on the canvas)

```
┌ Place ───────────────────────────────────────────────────────────────────────────────────────────────┐
│ [ tlv757                                          ] [cat: ICs ×] [pins:5 ×]          Esc close      │
├──────────────────┬──────────────────────────────────────────────────────────┬───────────────────────┤
│ CATEGORIES       │ 3 parts                                       sort: best  │  TLV75733PDBVR        │
│ ▸ Passives   212 │ ▌TLV75733PDBVR   LDO 1 A 3.3 V      ICs/Power  SOT-23-5   │  ┌──────┐  ┌────────┐ │
│ ▸ Discretes   41 │ ▌ Project · r2 · used 1   sym·fp·3D·MPN     in project    │  │ IN OUT│  │ ▫ ▫ ▫  │ │
│ ▾ ICs         17 │  TLV75718PDBVR   LDO 1 A 1.8 V      ICs/Power  SOT-23-5   │  │GND EN │  │ ▫   ▫  │ │
│     Power      9 │   Library · team-parts · r1  sym·fp·3D·MPN  new           │  └──────┘  └────────┘ │
│     Sensors    5 │  TLV757P (adj)   LDO 1 A adj        ICs/Power  SOT-23-5   │   [ 3D thumbnail   ]  │
│ ▸ Connectors  12 │   Shipped               sym·fp·3D·—      new              │  Vout 3.3 V  Iout 1 A │
│ ▸ Modules      3 │ ─────────────────────────────────────────────────────────  │  Package SOT-23-5     │
│ ▸ Mechanical   6 │  + Make "tlv757" from its datasheet…          (chat)      │  TI · TLV75733PDBVR   │
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

### 5.2 Place palette, 1024 × 640 (sheet fills the canvas area, about 760 × 520)

```
┌ Place ──────────────────────────────────────────────────────────────┐
│ [ 10k 0603                                        ]        Esc      │
│ (All) (Passives) (Discretes) (ICs) (Connectors) (Modules) (Mech) ›  │
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

### 5.3 Package picker, 1920 × 1080 (sheet 1400 × 800, from New part › Package › Choose…)

```
┌ Choose a package for "TLV75733PDBVR" ─────────────────────────────────────────────────────────────────┐
│ [ sot-23-5                                ] [pins:5 ×]                                    Esc back   │
├────────────────┬──────────────────────────────────────┬───────────────────────────────────────────────┤
│ STYLES         │ 4 packages                           │ SOT-23-5 · Tabbed small-outline v2            │
│ Chip       14  │ ▌SOT-23-5   2.9×1.6  0.95  JEDEC     │ ┌ Part ───────────────────────────────────┐   │
│ Molded      9  │ ▌ Shipped generator · cited lands    │ │ Name  [TLV75733PDBVR            ]       │   │
│ ▾ SOT / SC  4  │  SOT-23-5 (in project, U1's)         │ │ Pins  [1 IN, 2 GND, 3 EN, 4 NC, 5 OUT]  │   │
│ SOIC/TSSOP 21  │   Project · package r1               │ └─────────────────────────────────────────┘   │
│ QFN / DFN  38  │  SC-74A (SOT-753)  2.9×1.6           │ ┌ Body ─┐ ┌ Leads ┐ ┌ Lands ┐  preset ▾     │
│ QFP         8  │   Shipped generator · estimated      │ │ L 2.90│ │ n 3+2 │ │ TI ▾  │               │
│ BGA / LGA  11  │  Custom SOT…                         │ └───────┘ └───────┘ └───────┘               │
│ Through-h. 26  │   tabbed v2 · start from defaults    │  [footprint preview]   [3D preview]           │
│ Headers    40  │                                      │  Lands are cited: TI SBVS... p.32             │
│ Modules     3  │                                      │                                               │
│ FILTERS pins 5 │                                      │                       [Cancel] [Use package]  │
├────────────────┴──────────────────────────────────────┴───────────────────────────────────────────────┤
│ ↑↓ move   Enter use package   Tab to the form   Esc back                                             │
└──────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 5.4 Package picker, 1024 × 640 (two steps in one sheet)

```
┌ Choose a package ─────────────────────────────┐   ┌ ‹ SOT-23-5 · tabbed v2 ───────────────────────┐
│ [ sot-23-5                ]            Esc    │   │ Name [TLV75733PDBVR       ]                    │
│ (All)(Chip)(SOT)(SOIC)(QFN)(QFP)(BGA)(THT) ›  │   │ Pins [1 IN, 2 GND, 3 EN, 4 NC, 5 OUT ]         │
├───────────────────────────────────────────────┤   │ ▾ Body   L 2.90  W 1.60  H 1.10 mm            │
│ ▌SOT-23-5     2.9×1.6  0.95   cited     ›     │   │ ▸ Leads  ▸ Lands (TI)                          │
│  SOT-23-5     in project (U1's)         ›     │   │ [footprint]          [3D]                      │
│  SC-74A       2.9×1.6  0.95   estimated ›     │ → │ Lands are cited: TI SBVS… p.32                 │
│  Custom SOT…                            ›     │   │                                                │
├───────────────────────────────────────────────┤   ├────────────────────────────────────────────────┤
│ ↑↓  Enter open form  Esc back                 │   │ Esc list            [Cancel] [Use package]     │
└───────────────────────────────────────────────┘   └────────────────────────────────────────────────┘
```

### 5.5 New part dialog, docked (right column, both sizes)

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
│           5 of 5 pins named       │
├───────────────────────────────────┤
│ [Cancel] [Create and open] [Create│
│                       and place]  │
└───────────────────────────────────┘
```

## 6. Slices

At most five agents at once. Every slice ships with its tests. UI evidence is gathered in the VM only, never on the host desktop.

| # | Slice | Delivers | Hours | Needs |
|---|---|---|---|---|
| **K0** | Contract | `categories.json` format 1 with its load check. ABI optional fields `PartOffer.style`, `complete`, `Preset.style`, `tags` (ABI minor). The Category standard field record. `generic-families.json` `category`. The nine generators' declarations filled and modules rebuilt (golden generation hashes unchanged) | 6–9 | — |
| **K1** | Core search | `Source::{Shipped, Generic}`. `cat:`, `style:`, `src:` and `is:` terms. Source precedence in ranking. Generic offers only from `complete`. Category facets. Bare-package derivation and its hidden count. `library.search` report gains `make` offers. Catalog module version bump | 8–12 | K0 |
| **K2** | Names, values, BOM | The package half of a component-scope generation keyed and named as a package. `value_field` on complete parts, with the parsed value set on the instance. The value to-do. BOM Value/Package fallbacks. Refusal of a nameless component-scope request for a non-complete offer | 8–12 | K0 |
| **K3** | Place palette | The list on `kit/list`, category rail and chips, facet panel, rows, preview pane and Space, make rows (chat offer, New part). "From generator" removed. PCB `P` with mechanical first | 10–14 | K1 |
| **K4** | Package picker | `package_picker.rs` replaces `generator_gallery.rs`: styles, filters, rows from presets, customs and held packages, the one form with the Name/Pins section in component scope. Entry from Change package…, From generator, start page. Gallery deleted | 8–12 | K0 |
| **K5** | New part dialog | Docked dialog: name, maker, category, package (picker), pins paste through `library.pinout_preview`, value. Create and place / Create and open as one undo step | 6–9 | K2, K4 |
| **K6** | Migration | Bare-package presentation "· no part chosen", Issues to-do, production warning, Resolve parts "Package only" group, **Name this part…** (draft, publish, update preview) | 5–8 | K1, K5 |
| **K7** | Replace component | `Change::ComponentReplace` with an explicit terminal map in the update preview. **Use another part…** in Resolve parts and in the instance context menu. Updates module version bump | 8–12 | K1 |
| **K8** | Chat and workflow hooks | `view.arm_place`. The search `choice` card. The datasheet workflow's `pick` step "Choose another package…" and `Place` on its result card. Needs assistant C1/C2 | 4–6 | K3, K4, assistant C2 |
| **K9** | Evidence | VM run at 1920 × 1080 and 1024 × 640: run 11's board made with names, values and the BOM checked, and an old project's bare SOT-23-5 migrated both ways | 3–4 | K3–K7 |

**Order** (≤5 parallel):

| Round | Agents |
|---|---|
| 1 | K0 (alone, the contract) |
| 2 | K1, K2, K4 |
| 3 | K3, K5, K7 |
| 4 | K6, K8 (once assistant C2 is merged) |
| 5 | K9 |

**Total** about **66–98 agent hours**. The critical path is K0 → K2 → K5 → K6 → K9, about 28–42 h. Run 11's three findings are closed after round 3, with K2 and K5 doing the work.

## 7. Open questions, each with a recommendation

1. **Where do the facets live: the generator's description or `generator.json`?** *Recommendation: the description* (§3.2). It is one source with the presets and grammar, and it travels inside projects with the module. `generator.json` stays files and hashes. The cost is a module rebuild for a facet edit.
2. **Is the top-level category list fixed?** *Recommendation: six shipped top-level categories* (data, not code). Folders and the registry may add subcategories only, so filters mean the same across sources.
3. **Should component scope stay for packages that are not complete parts?** *Recommendation: yes, but only with a name*, for part making (the New part dialog and the datasheet workflow). A nameless request is refused. Placing never lists these packages.
4. **Should two parts on one package share the package lineage?** *Recommendation: yes.* Key the package half in package scope, so the BOM and Change package read "SOT-23-5". The risk is the key-to-id map (G4) for parts made before the change. Those keep their own package until they are made again. The fallback is renaming only, about 2 h less, but the duplicate packages stay.
5. **What does the BOM Value show for parts without a value field?** *Recommendation:* the typed value field, else the MPN, else the component name. An LED shows its colour when it has no value.
6. **How strict is the rule for bare packages in existing projects?** *Recommendation:* a to-do in the schematic and a warning in the production check, never an error. New placements are impossible, so the remaining cases are migrations, and their boards still export.
7. **Should a library part's category be a field or a shelf?** *Recommendation: a standard field on the component revision*, because it is intrinsic and travels with copies. Shelves and tags (D13) stay for people's own organisation.
8. **Which generic parts count as complete?** *Recommendation:* chip R, C, L and ferrite, LEDs, two-terminal diodes, pin headers and THT axial or radial R and C. Transistors, regulators and other ICs need an MPN's pinout, so they are not complete.
9. **Should the list be grouped by source or ranked as one list?** *Recommendation: one ranked list with source chips.* Groups are used only when browsing with an empty field (Recent, Favourites, a category).
10. **The chat entry needs a new show entry (`view.arm_place`).** *Recommendation: yes.* It is the palette's Enter exposed as a `show` entry. It is window-only (`host.unavailable` headless), the same as `view.select`.
11. **Online sources.** *Recommendation:* defer until the registry exists. Design it now as one more `SourceCatalog`, ranked last, with an import preview before the first place, so no part of §1 changes when it arrives.
