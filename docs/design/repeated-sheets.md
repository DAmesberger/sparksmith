# Reusing a child sheet for several channels

**Proposed for founder decision.** A designer should be able to draw an ADC front end once, use it twice, and place and route the two channels independently. Today a sheet symbol can reference a child once. Copying its contents creates separate components and drawings that the designer must maintain individually. [Current hierarchy and copy behavior](../schematic-capture.md#sheets-and-hierarchy)

## Definition and occurrence

Keep one sheet definition for the component choices and values, symbol-unit positions, wires, junctions, labels, ports and nested sheet symbols. Each traversal from a top-level sheet to that definition is a **sheet occurrence**. Give it a stable path consisting of the top-level sheet ID followed by the IDs of the sheet symbols traversed. If both front ends contain the same nested filter symbol, its full path still distinguishes the two filters. A channel name such as “Front end A” is a display name. Renaming it, reordering pages or moving its box preserves the path. Distinguish top-level sheets from unused definitions. Removing the last use of a definition leaves it available to place again and contributes zero parts to the board.

Each component in the definition has a stable local ID. Persist one binding from (occurrence path, local component ID) to the project’s physical component instance ID. All units of that component in the definition resolve to the same physical instance within the path. Keep the component revision, terminal mapping and shared field values in the definition. Keep the designator, PCB placement and assembly-variant choices on the physical instance. A footprint or board selection can then cross-probe to the specific channel, and an edit to a shared value reaches both channels in one transaction. For the first implementation, refuse a repeat when a multi-unit component spans its boundary. Identify the units the designer would need to bring inside the definition.

## Connectivity and board updates

Instantiate wire ends and local labels within their occurrence. Match a parent sheet pin to the hierarchical port of that name **in that child occurrence**. Project-wide global labels and power ports keep their existing scope. Resolve each component terminal to the existing physical endpoint (instance ID, terminal ID), then give those endpoints to the existing net reconciliation and board-update machinery. Identical local labels on the two channels can display as `Front end A/FB` and `Front end B/FB`. The label text and path display names can change without changing endpoint identity. [Current net naming and routed-net protection](../schematic-capture.md)

Preview a shared-definition edit across every affected path, naming the designators, pad mappings and routed nets it affects. Removing one sheet symbol previews the affected placements and copper on that path and preserves its siblings. Undo restores the same identities. Creating a new symbol allocates a new occurrence. Reject definition cycles before committing. Traversal of nested repeats must check arithmetic and handle expansion as cancellable work before an edit is committed. Existing single-use designs should retain their physical instance and net IDs when represented by this model.

## Acceptance case

1. Draw one `Front end` with a 10 kΩ feedback resistor, a local `FB`, a hierarchical `OUT`, and global `GND`. Place two sheet symbols referencing it, A and B. Wire their output pins on the parent to `OUT_A` and `OUT_B`.
2. Annotate the two resistors as `R1` and `R101`. Place and route each at a different board position. Verify distinct output and feedback nets, a common ground, and a BOM quantity of two for the resistor with both designators. Cross-probing either placement opens its own occurrence.
3. Change the shared resistor value to 22 kΩ once. Both resistors adopt it. Save and reopen after renaming A and reordering the pages. Both designators, placements, net identities and routes survive. Mark `R101` unpopulated in one assembly variant. Its fitted BOM contains one resistor and its PCB footprint remains.
4. Remove B through impact review. Only B’s physical instances and connections are proposed for removal. Undo restores B with its original IDs and routes. Placing another nested repeat must keep its internal nets separate by the whole path.
