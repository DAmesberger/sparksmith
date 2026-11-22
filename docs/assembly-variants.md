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

Pin names are never compared, and electrical compatibility is not inferred: the Part's binding is the explicit pin mapping. A substitute that fails any rule is refused with the reason.

Fitting is physical fit only. Two parts with VCC and GND swapped, or a part rated lower than the one it replaces, fit the same lands with the same pad grouping. So a substitute is fitted as soon as it fits, but its purchase is resolved only when a person has reviewed it (`variant.review_substitute`, `model::variants::SubstituteReview`, bound to the substitute Part it reviewed):

- **Electrical equivalence:** the person confirms that on every pad the substitute does the job of the part it replaces. Until then Issues reports `variant.substitute_unreviewed` and its BOM line is unresolved.
- **Requirements:** the instance's requirements are compared with the substitute's rated specs exactly as with its own Part (`instances::rating_findings`). Each one it does not rate, meet or compare with is `variant.substitute_unmet`, unless the review records it as a deliberate exception with a reason (a 0 Ω link where a resistance was required).

Fitting another Part voids the review. A library update can leave a substitute that no longer fits. It is then not fitted: the default part is, and Issues reports it as an error (`variant.substitute_unfit`) until it is chosen again or removed.

A substitute is placed with its own package body and its own markings. Its BOM line is the Part (manufacturer, MPN, rated values), and its value text is the Part's rated values through its component's value template, else its MPN.

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
| `variant.review_substitute { instance, variant, equivalent, exceptions? }` | A person's review of the substitute a variant fits: electrically equivalent or not, and the requirements it is accepted not to meet, each `{field, reason}`. Not equivalent with no exceptions clears it. An Ask entry: it is a person's sign-off. |
| `variant.list` | Query: the variants, the active one, and each bought part's default and per-variant state. |
| `variant.substitutes { instance }` | Query: the project library's Parts that fit the instance's footprint. |

Overrides of a removed part are dropped with it. A copied part (`schematic.paste`) is the same part (`ComponentInstance::duplicate`): it keeps the default population, so a part left off by default is pasted left off; variant overrides belong to their variants and are not copied. The variants' invariants (bounded count and names, unique names, overrides of existing parts, substitutes in the library, the active variant listed) are checked on every edit and on open.

## Outputs and checks

- **BOM** (`project.bom`): each instance is listed with the Part the active variant fits. A substitute's line is resolved only when its review confirms equivalence and leaves no requirement unmet. Parts it does not fit group in lines of their own with `fitted: false`; they are not counted as unresolved and `bom.csv` leaves them out. `bom-parts.csv` lists every part with its Fitted column.
- **Pick-and-place:** a part that is not fitted has no row; a substitute is written with its package and value.
- **3D:** a part that is not fitted places no body, in the live view and the exact export alike (both read `placed_parts`); a substitute places its own body, marked from its own data through the same `instance.marking` path as any fitted part: its Part's top mark, else its component's marking, filled with its part number, manufacturer and rated values (a resistor's code, say) on its own package's surface. The instance's own replacing marking describes the default part, so a substitute does not take it; hiding the marking still applies.
- **Checks:** body checks (`board.body_off_pads`) skip bodies that are not fitted and check a substitute's body against its pads. Electrical, footprint, courtyard and copper checks are unchanged. A part the active variant does not fit is not reported as having no Part chosen.

## Import

The KiCad importer maps KiCad's do-not-populate attribute (the footprint's `dnp` flag, the symbol's `(dnp yes)`), and a value that is or carries a not-fitted marker ("DNP", "DNF", "NF", "NC (not fitted)" and similar), to `not_populated` in the default. A real value is kept; a value that is only the marker becomes empty. The import report names each part. KiCad's exclude-from-BOM attribute (`exclude_from_bom`, `(in_bom no)`) is not a not-fitted state: it marks test points, mechanical items and logos, so such a part stays populated, and on a footprint without a 3D model the footprint flag still means nothing is assembled there (no body).

## Out of scope

- Variants of the schematic (a part present in one variant only): every variant has the same parts.
- Per-variant field values (a different resistance on the same Part): fit a different Part.
- A substitute from outside the project library: add the Part to the project first.
