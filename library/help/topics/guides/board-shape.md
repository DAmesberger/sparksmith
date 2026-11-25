---
{
  "covers": {
    "entry/board.edit": "61a6f30290e0b1f2",
    "entry/board.start_sketch": "011f97f53f8e6cc2"
  },
  "id": "guide/board-shape",
  "keywords": [
    "board size",
    "edge of the board",
    "outline",
    "edge",
    "resize",
    "round board",
    "fillet",
    "slot",
    "cutout",
    "mounting hole",
    "width",
    "height",
    "dimension",
    "constrain a board outline",
    "board shape",
    "constraints",
    "fully defined",
    "parallel",
    "tangent",
    "parametric outline"
  ],
  "kind": "guide",
  "related": [
    "guide/parameters",
    "guide/dimensions",
    "guide/update-pcb",
    "guide/first-board",
    "panel/layout",
    "finding/board.shape_open",
    "finding/board.param_idle"
  ],
  "since": "0.1.0",
  "summary": "Set the size, round corners, add slots and holes, and constrain the outline so it changes only where you say.",
  "title": "Change the board's size and shape"
}
---

The board's outline is a sketch: lines and arcs that dimensions and constraints hold.

- In PCB, choose Shape. Or click an empty place on the board and press Edit shape… on the Board card.
- To change the size, choose Dimension ({key:shape.bs-dimension}), pick an edge, and type the length in Value. If the edge has a dimension already, click it and change its Value. If it uses a parameter, change that under Board parameters.
- Draw with Line, Arc, Circle or Rectangle. Click existing points as you draw, so that curves share their ends. Fillet ({key:shape.bs-fillet}) rounds the corner where two lines meet.
- Slot ({key:shape.bs-slot}): click both ends. Mounting hole ({key:shape.bs-hole}): click its place. Cutout ({key:shape.bs-cutout}): two corners, or centre and radius. Their widths and drills are on the card.
- The outline must be one closed loop. To join two ends, choose Select and drop one point on the other: a ring shows where it joins. An outline that is still open is marked, and Issues says [[finding/board.shape_open]].
- Name the edges that parts are placed from with Name edge ({key:shape.bs-name-edge}).
- Press Done.

A refused value changes nothing; the reason stands under its row or on the card. Undo takes back each step.

A constraint is a rule between points and curves, such as two lines that stay parallel. The Sketch row of the card says how much can still move: a fully defined sketch changes only when you change a dimension or a parameter. Choose a constraint from the Constrain group, or press its key, then click what it is about.

| Constraint | Key | Click |
| --- | --- | --- |
| Coincident | {key:shape.bs-coincident} | two points, or a point and a curve |
| Horizontal | {key:shape.bs-horizontal} | a line, or two points |
| Vertical | {key:shape.bs-vertical} | a line, or two points |
| Parallel | {key:shape.bs-parallel} | two lines |
| Perpendicular | {key:shape.bs-perpendicular} | two lines |
| Equal | {key:shape.bs-equal} | two lines, or two circles or arcs |
| Tangent | {key:shape.bs-tangent} | a circle or arc and a line or another curve |
| Midpoint | {key:shape.bs-midpoint} | a point, then a line |
| On curve | {key:shape.bs-on-curve} | a point, then a curve |
| Concentric | {key:shape.bs-concentric} | two circles or arcs |
| Symmetric | {key:shape.bs-symmetric} | two points, then the line of symmetry |
| Fix | {key:shape.bs-fix} | a point |

- Set directions first, then make equal what should stay equal, and dimension it once. A name or an expression as the value is a parameter: see [[guide/parameters]].
- Point at a curve or a point, or select it, to see its constraints. Constraints ({key:shape.bs-constraints}) shows them all.
- Click a constraint's mark and press {key:shape.delete} to delete it. If parts are dimensioned to what it held, the card says how many.
- A constraint or dimension that the sketch sets already is refused, and the message names what sets it, as in “Horizontal already sets this; it would hold nothing more”. Edit or delete that one if the new one says better what you mean.

To place parts from the edges, see [[guide/dimensions]]. Automation sends the same constraints with [[entry/board.edit]], under the same names.
