# Assembly variants

A design is often built more than one way: a part fitted on one build and left off another, or a different part on the same footprint (a 0 Ω link instead of a resistor, a second-source regulator). Sparksmith keeps this as typed state, never as a value text such as "DNP".

## Model

- **Population** (`model::Population`): `populated` or `not_populated` (do not populate). Every component instance stores its **default population** (`ComponentInstance::population`): the board as it is built when no variant is chosen. A value never says whether a part is fitted.
- **Assembly variant** (`model::AssemblyVariant`): a named, ordered entry in `Project::variants`, with an id from the project's allocator. Per instance it may override the population and the **fitted Part**, the manufacturer part bought for that footprint. An override field left out follows the default, so changing the default changes every variant that does not say otherwise.
- **Active variant** (`AssemblyVariants::active`): a project setting saved with the design. `None` is the default. The 3D view and the exact export, `project.bom`, the pick-and-place and BOM files, the assembly drawing and the body checks follow it.

The copper never changes between variants. Footprints, pads, nets, tracks and fills are the same on every build; only bodies, BOM lines and pick-and-place rows differ.

## What a variant fits

`instances::variants::fitted(project, instance)` is the one answer every consumer reads: the population, the Part, the component revision and the physical implementation (binding, footprint and package) fitted under the active variant. Nothing else decides which body is placed or which line is bought.

A **substitute** (`variant.set_part`) is a Part of the project library. Its component and binding may differ from the instance's, but it must fit the footprint (`substitute_fit`):

1. the Part's binding is one its component offers;
2. one of the binding's footprints is the instance's footprint revision, or has the same land pattern: the same pads (kind, number, position, rotation, side, shape, hole, mask, paste and pad vias, in any stored order) and the same datum;
3. its pads group into terminals exactly as the instance's do: pads that share a terminal share one in the substitute, and no pad that carries a terminal becomes free or the reverse. So no net is joined, split or dropped.

Pin names are never compared, and electrical compatibility is not inferred: the Part's binding is the explicit pin mapping. A substitute that fails any rule is refused with the reason. A library update can leave a substitute that no longer fits. It is then not fitted: the default part is, and Issues reports it as an error (`variant.substitute_unfit`) until it is chosen again or removed.

A substitute is placed with its own package body. Its BOM line is the Part (manufacturer, MPN, rated values), and its value text is the Part's rated values through its component's value template, else its MPN.

## Commands

All of them are document edits in the `project` module: one undo step each, refused whole, through `Surface::call` like every other entry. A command that restates the current state takes no undo step.

| Entry | Does |
| --- | --- |
| `variant.add { name, copy_of? }` | Adds a variant, empty or a copy of another (duplicate). Names are unique, case-insensitively. |
| `variant.rename { variant, name }` | Renames. |
| `variant.remove { variant }` | Deletes it and its overrides; the default becomes active if it was. |
| `variant.set_active { variant? }` | Chooses the active variant; null is the default. |
| `variant.set_population { instances, variant?, population? }` | Sets the default population (no `variant`) or a variant's. In a variant, null follows the default again. |
| `variant.set_part { instance, variant, part? }` | Fits a substitute Part in a variant; null fits the default part. |
| `variant.list` | Query: the variants, the active one, and each bought part's default and per-variant state. |
| `variant.substitutes { instance }` | Query: the project library's Parts that fit the instance's footprint. |

Overrides of a removed part are dropped with it. The variants' invariants (bounded count and names, unique names, overrides of existing parts, substitutes in the library, the active variant listed) are checked on every edit and on open.

## Outputs and checks

- **BOM** (`project.bom`): each instance is listed with the Part the active variant fits. Parts it does not fit group in lines of their own with `fitted: false`; they are not counted as unresolved and `bom.csv` leaves them out. `bom-parts.csv` lists every part with its Fitted column.
- **Pick-and-place:** a part that is not fitted has no row; a substitute is written with its package and value.
- **3D:** a part that is not fitted places no body, in the live view and the exact export alike (both read `placed_parts`); a substitute places its own body.
- **Checks:** body checks (`board.body_off_pads`) skip bodies that are not fitted and check a substitute's body against its pads. Electrical, footprint, courtyard and copper checks are unchanged. A part the active variant does not fit is not reported as having no Part chosen.

## Import

The KiCad importer maps KiCad's do-not-populate attribute (the footprint's `dnp` flag, the symbol's `(dnp yes)`), and a value that is or carries a not-fitted marker ("DNP", "DNF", "NF", "NC (not fitted)" and similar), to `not_populated` in the default. A real value is kept; a value that is only the marker becomes empty. The import report names each part. KiCad's exclude-from-BOM attribute (`exclude_from_bom`, `(in_bom no)`) is not a not-fitted state: it marks test points, mechanical items and logos, so such a part stays populated, and on a footprint without a 3D model the footprint flag still means nothing is assembled there (no body).

## Out of scope

- Variants of the schematic (a part present in one variant only): every variant has the same parts.
- Per-variant field values (a different resistance on the same Part): fit a different Part.
- A substitute from outside the project library: add the Part to the project first.
