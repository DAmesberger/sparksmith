# Ties: deliberate joins between nets

The founder approved the 22 November 2026 design and clarified independent tie groups on 23 November. The original baseline was `main` at c0a125f8. Hours are estimates.

Behaviour comes from this project's documents, general electronics practice, public standards and other tools' public documentation. No other tool's code was read.

## What a tie is

A **tie** is a typed electrical element that joins two or more nets on purpose while they stay distinct nets. AGND and GND keep their own names, classes, airwires and lengths. The netlist, ERC, the schematic, the board checks and the outputs all know that they meet, where, and why.

One concept covers three **forms**:

| Form | Electrically | What is built | BOM and placement |
| --- | --- | --- | --- |
| **Copper tie** | always closed | a copper bridge in the footprint | nothing to buy or place |
| **Solder jumper** | open or closed, typed default, switchable per variant | pads, plus a solder bridge or a cuttable copper trace | nothing to buy or place; paste when closed |
| **0 Ω link** | closed when fitted, open when not fitted | a bought part on its own lands | a BOM line and a placement row |

One component can contain several independent ties. Each has its own ends, bridge copper, instance purpose, anchors, checks and query row. Its group identity survives renames and reordering. Commands take an optional group, required when their target would be ambiguous. Assembly and population belong to the physical component.

Every tie has a **purpose**: star ground, Kelvin sense, shield bond, supply join, or none. The purpose decides which checks apply and how strict they are.

The name is "tie", not "link", because "linked design" already means something else in Sparksmith. "Net short" is avoided too: a short is a fault.

## Domain model

### On the component (the library type)

A tie is an ordinary component with terminals, a symbol, a package, a footprint and a binding. Pin mapping stays explicit. Two optional fields are added to `ComponentDefinition`:

- `ties: Vec<TieSpec>`: each group has a stable component-local TieId, `ends`, the terminals it joins (2 to 8, each electrical and passive, each its own net), and `form`. The form is `copper`, `solder_jumper { default: open | closed, bridge: solder | trace }` or `zero_ohm`.
- `assembly: bought | copper_only | solder_only`: what the factory does with it, stated by type. This replaces the inference in `library::mechanical::copper_only` (below).

`library.publish` checks the pieces against each other. A copper tie's footprint must carry a bridge that joins pads of every end and must have no body (`PackageModel::NoBody`). A 0 Ω link must have a body and pad grouping like any two-terminal chip. A solder jumper with `trace` must carry the trace as a bridge. `assembly` must agree with the form: a copper tie is `copper_only`, a solder jumper `solder_only`, a 0 Ω link `bought`.

### On the footprint

`FootprintDefinition` gets `bridges: Vec<Bridge>`. A bridge is a copper shape on one side, in the footprint frame, together with the pads it joins. Footprints carry no copper apart from pads today, so this is new and is kept narrow: only a tie's footprint may have bridges. No new `PadKind` is needed. The tie's pads are ordinary SMD or plated pads, and each lands one end's terminal.

### On the instance (the use)

`ComponentInstance` gets `ties: BTreeMap<TieId, TieUse>`:

- `purpose`: star ground, Kelvin sense, shield bond, supply join or none. It lives on the instance because one 0603 tie footprint serves many purposes.
- `anchors`, used for Kelvin sense only. For each sense end, the pad of *another placed part* where the sense net must meet the anchored net, for example `{ R7, pad 1 }`. A Kelvin tie has no lands of its own: its package has no pads, and its ends sit on the anchor part's lands. It still has a symbol and a designator, and the sense nets are drawn in the schematic.
- The **current requirement** is the existing standard field *Current*, an instance requirement like any other. A 0 Ω link compares it with its Part's *Rated current* through `instances::rating_findings`. Copper ties and trace jumpers compare it with the bridge's capacity (see Checks).

The **electrical state** of a solder jumper or 0 Ω link is the instance's **population**, so there is one switch and it already works per variant (`variant.set_population`). For a jumper, the UI shows populated as *Closed* and not populated as *Open*. When a jumper is placed, its default population comes from the type's `default`. A copper tie cannot be "not populated", and that request is refused with the reason. The copper never changes between variants, as [assembly variants](../assembly-variants.md) require: a trace jumper's trace is always fabricated, and *Open* in a variant becomes a "cut JP3" step on the assembly drawing.

### `copper_only` is replaced

Today `copper_only` *infers* "nothing to buy or place" from "no terminals and every package has no body". A tie has terminals, so the inference cannot hold for it, and the founder wants this decided by type. The recommendation (decision 9) is to replace the inference with the `assembly` field. `in_bom` and `in_pick_and_place` read the field. The fiducial and logo generators set it to `copper_only`, and the examples are regenerated. Mechanical parts keep their own `bom` and `pick_and_place` flags, because a hand-fitted screw can be bought without being placed.

## Fit with the copper graph

The copper graph's rule stays as it is: **different nets never join**, and islands never merge across nets ([copper graph](../copper-graph.md#one-joining-rule)). A tie does not change that. Net A is routed to the tie's pad 1, and net B to pad 2. Each net is complete on its own, and airwires stay per net.

The tie adds one new fact to the graph:

- **Bridge elements.** A bridge is a graph element that carries the tie's identity and a *set* of nets, not one net. It is not a member of any island. The graph records which island of each end net it touches (`CopperGraph::ties()`). The tie's join point is that record. It lives in `layout/connectivity` (a new `ties.rs`). Anchor lands of Kelvin ties are recorded the same way.
- **Queries, not new deciders.** Every tie check below asks the graph. Clearance only asks "is this overlap a declared contact of tie T", which is a lookup by identity, not a contact test, so `cargo xtask connectivity` passes without an exception.
- **The board plan** draws bridges as copper bodies with the tie's `ObjectRef`, layer and both nets. Layout, the live 3D mesh and the exact export all read them from the plan (`cargo xtask copper-source`, `board-parity`).
- **Fills** treat a bridge as foreign copper for every net. A fill joins a tie only through the tie's pads. A fill touching the bridge would be a second join.

## File format

Version 1 is edited in place. All fields are optional and left out when empty, so revisions without a tie keep their content hashes:

- `ComponentDefinition.ties`, `ComponentDefinition.assembly` (default `bought`);
- `FootprintDefinition.bridges`;
- `ComponentInstance.ties`.

Fiducial and logo parts change hash once, when they gain `assembly: copper_only`. The examples and the golden file are regenerated. The jumper state is the existing population, so no variant field is added.

## Commands and module versions

All entries go through `Surface::call`. Each edit is one undo step and is refused whole.

| Entry | Module | Does |
| --- | --- | --- |
| `library.generate` with the `tie` generator | generators | makes a tie part (below) |
| `tie.set_purpose { instance, purpose }` | schematic | sets the purpose |
| `tie.set_anchor { instance, end, anchor? }` | layout | anchors a Kelvin sense end on another part's pad, or clears it |
| `tie.list` | project | query: every tie with its form, purpose, ends, state in the active variant, join point and current margin |
| current requirement | existing field command | unchanged |
| jumper state | `variant.set_population` | unchanged, with jumper wording |

The new entries and optional fields raise the affected module minor versions from the implementation baseline. The surface lock records the resulting versions, including the rules module for the supplied current table. `cargo xtask surface --update` records them.

## Checks and their sources

| Code | Severity | What it finds |
| --- | --- | --- |
| `tie.same_net` | error (ERC) | two ends of a tie are on the same net, so the tie does nothing |
| `tie.second_join` | error | copper of two tied nets meets somewhere other than the tie: a fill, a track or a via. The short that clearance finds is renamed to this code, naming the tie: "AGND and GND also meet at 12.4, 8.0 mm through the GND fill; star ground NT1 must be their only join." |
| `tie.loop` | error for star ground, warning otherwise (decision 6) | two closed ties (any form, in the active variant) give two paths between the same nets |
| `tie.star_points` | error | the nets of a star ground meet at more than one point |
| `tie.bridge_contact` | error | a bridge touches copper other than its own pads |
| `tie.kelvin_off_anchor` | error | the sense net touches the anchored net's copper anywhere but the anchor land, including a track of the anchored net that runs onto the pad |
| `tie.current_exceeded` | error | the bridge's capacity, from a sourced rule, is below the current requirement |
| `tie.current_unchecked` | info | a current requirement is set, but the board has no current rule |
| `tie.other_bridge` | info (decision 7) | another part has terminals on both nets of a star ground (a ferrite or a resistor) |

ERC follows closed ties. A power input on AGND is driven when the tie joins AGND to a driven GND. An open jumper passes nothing, so the netlist and ERC follow the active variant. Netlist outputs keep the nets distinct and list the tie with its form, purpose and state.

**Current, by IPC-2152.** Sparksmith ships no guessed values ([no invented limits](../board-rules-and-shape.md#invariants)). The capacity is the bridge's narrowest width times the stackup's copper thickness, read against an optional current table supplied by the person designing the board or their library. Sparksmith ships no IPC-2152 curve data. That file cites the edition and figure, as fab profiles cite their sources. The allowed temperature rise is a required project setting, because it is the designer's choice. Without both, the check does not run and says so (`tie.current_unchecked`). IPC-2152 is a paid standard, so where its data comes from is decision 8. A short neck carries more current than the long conductors the standard describes, so the check errs on the safe side, and the finding says so.

## UI, briefly

- **Schematic.** A tie draws as its symbol with a small purpose mark (a star, Kelvin arrows, a shield). Each end shows its net name. A jumper is drawn open or closed for the active variant. The property card shows the form, purpose, ends, state, current requirement, capacity and margin, and lists the tie's findings.
- **Layout.** A bridge draws in copper with a two-net hatch. Selecting a tie highlights both nets and marks the join point. Highlighting a net shows its ties as gates. A Kelvin anchor is set by clicking the pad, and the sense track's contact is marked. Picking returns the tie, never one of its nets.
- **3D.** A copper tie is plain board copper from the board plan. A closed solder jumper shows a solder bridge body, and an open one shows bare pads. A 0 Ω link shows its chip body when fitted.

## The generator family

A new `tie` generator module, version 1, sits in the **Board features** category. It makes the whole part: component with `tie` and `assembly`, symbol, package, footprint with bridges, binding and model.

- **Copper tie**: 2 to 8 ends. Pad size, bridge width and length are parameters. Bridge width has no default that pretends to be a rule: the gap comes from the board's fab profile (`project.*` context parameters, G7). No body (`Model::NoBody`).
- **Solder jumper**: two-pad and three-pad (1-2 or 2-3) layouts, default open or closed, solder or trace bridge. Presets come only from published sources such as fabricators' guidelines, cited in `presets.json`. The closed solder bridge is a part file in the generator's bundle, placed when the jumper is populated.
- **0 Ω link**: the existing `chip` generator's resistor presets with `tie.form = zero_ohm`. No new geometry.
- **Kelvin tie**: no lands. Its ends are anchored when placed.

## Other tools, from their public documentation

- **KiCad** marks a footprint as a net tie with pad groups (`net_tie_pad_groups`). Its DRC then allows those pads' nets to touch inside the footprint. The schematic has net-tie symbols. There is no purpose, no current check and no single-point rule beyond the ordinary short check. Solder jumpers are ordinary footprints, with their state carried by DNP or the value text.
- **Altium** has a component type for net ties, with and without a BOM line, and its DRC allows the short within such a component. Jumpers are ordinary parts that variants fit or leave off.
- **Allegro** documents a net-short property that allows named nets to short at given objects.

Where Sparksmith is better: the purpose is typed and drives the checks. Loops and star points are checked across all three forms, including variant-switched jumpers. Kelvin sensing is a checked contact on another part's pad, not a footprint hack laid over the resistor. Current is checked against a cited source. Jumper state is typed and per variant, and the netlist and ERC follow it. BOM and placement follow the type, and copper, 2D, 3D and export come from one board plan. These statements about other tools must be checked against their current public docs before anything is published.

## KiCad import

The mapping uses KiCad's public file-format documentation only. The parser already reads the flag (`crates/kicad/src/board.rs`).

- A footprint with `net_tie_pad_groups` (or the older `net_tie` attribute) becomes a **copper tie**. Each quoted group becomes one independent tie within the same component. Its distinct terminals become the ends. A copper drawing belongs only to the group owning all of the pads it contacts. Copper crossing groups requires review. The purpose is left as none, and a finding asks the person to set it. The report entry G5 changes from "tie missing" to "imported as a copper tie".
- KiCad has no typed solder jumper. A two-pad part without a body becomes an ordinary part, and a copper-bridged net-tie footprint stays a copper tie. Where the designator starts with `JP`, the report *suggests* converting it to a solder jumper. It never converts by name. A jumper that is DNP in the source is imported as *Open*.
- Footprint pad groups that only say pads are joined inside the part, such as a switch's paired legs, become one terminal with several pads, as today. They are not ties. If KiCad's documentation calls these "jumper" groups, the importer still does not treat them as solder jumpers.

## Risks

1. **One net per copper body** is assumed in picking, highlighting, the board plan's identity, Gerber X2 net attributes (`%TO.N`) and future netlist outputs. A bridge carries two nets. Each of these places needs a test, and the Gerber attribute's multi-net form must be checked against the published X2 specification.
2. **The clearance exemption must stay narrow**: only a bridge against its own pads, and a sense net against its anchor land. A wider exemption would hide real shorts. Tests must show that every other overlap is still reported.
3. **IPC-2152 data licence.** Its charts may not be redistributable (decision 8).
4. **Replacing `copper_only`** touches the fiducial generator, BOM, placement output and the examples.
5. **Solder paste per variant** would be the first fabrication film to follow the variant (decision 5).
6. **Import guesses.** A wrong form is worse than none, so the importer only suggests.

## Founder decisions

1. **Name.** "Tie" for the concept; copper tie, solder jumper and 0 Ω link for the forms. *Recommend: yes.*
2. **Purpose on the instance, not the component.** *Recommend: instance.* One footprint serves many uses.
3. **Jumper state is the population.** One switch that already works per variant, shown as Closed or Open. *Recommend: yes*, instead of a second state field.
4. **Trace jumpers.** The trace is always fabricated. *Open* in a variant becomes a "cut" step on the assembly drawing, and the netlist and ERC treat it as open. *Recommend: yes.*
5. **Paste for solder jumpers** follows the active variant, like the BOM. *Recommend: yes.* The alternative is to keep one stencil and list jumper closing as a hand step.
6. **Loops.** Error for star grounds, warning for other purposes. *Recommend: yes.*
7. **Other parts across star nets** (a ferrite or a resistor) get an info finding that names them, with no threshold. *Recommend: yes.*
8. **Source of the IPC-2152 data.** *Recommend:* a rules data file entered from a purchased copy, cited by edition and figure, with temperature rise a required project setting. No check until both exist, and no IPC-2221 formula as a quiet fallback.
9. **Replace the `copper_only` inference with the typed `assembly` field.** *Recommend: replace it fully.* Fiducials and logos set it, and the examples are regenerated.
10. **Kelvin.** An anchored tie component with no lands. *Recommend: yes.* Four-terminal sense resistors keep working as ordinary parts.
11. **Ties with more than two ends**, up to 8: a star point where N nets meet at one place is one tie. *Recommend: yes.*
12. **Import never types a tie from a name or designator**, and only suggests. *Recommend: yes.*

## Slice plan

| Slice | Content | Depends on | Agent hours |
| --- | --- | --- | --- |
| T0 contract | types, file-format fields, publish validation, glossary entry, `assembly` replacing `copper_only`, regenerated examples | decisions 1–3, 9 | 3–4 |
| T1 copper | bridge elements and `ties()` in `layout/connectivity`, board-plan bodies (2D, 3D, export), clearance exemption, fills | T0 | 6–8 |
| T2 checks | same net, second join, loop, star points, bridge contact, ERC drive through ties, netlist | T1 | 4–6 |
| T3 Kelvin | anchors, `tie.set_anchor`, off-anchor check | T1 | 4–5 |
| T4 current | rules data file, temperature-rise setting, capacity check, 0 Ω rating through requirements | T0, decision 8 | 3–4 |
| T5 variants and outputs | jumper wording, BOM and placement by type, paste per variant, cut steps, solder bridge body | T0 | 3–4 |
| T6 generator | `tie` module, presets with sources, solder-bridge part file, chip `zero_ohm` | T0 | 5–6 |
| T7 commands and UI | `tie.*` entries, surface lock, schematic marks and card, layout highlight and picking, 3D | T2, T3 | 6–8 |
| T8 KiCad import | net-tie mapping, bridges, suggestions, report | T1 | 3–4 |

Total: 37–49 agent hours. After T0, T1, T4, T5 and T6 can run in parallel (four agents). T2, T3 and T8 follow T1, and T7 comes last.
