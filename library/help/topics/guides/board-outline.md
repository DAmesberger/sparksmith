---
{
  "covers": {
    "entry/board.edit": "61a6f30290e0b1f2"
  },
  "id": "guide/board-outline",
  "keywords": [
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
    "guide/first-board",
    "panel/layout",
    "finding/board.shape_open",
    "finding/board.param_idle"
  ],
  "since": "0.1.0",
  "summary": "Draw the outline as a sketch, constrain it, and drive its size from board parameters.",
  "title": "Constrain a board outline"
}
---

Open Board shape from Layout and draw the outline as one closed loop. The Sketch row of the card says how much can still move: a fully defined sketch changes only when you change a dimension or a parameter.

- Click existing points as you draw, so that curves share their ends. To join two points later, choose Select and drop one on the other, or use Coincident ({key:shape.bs-coincident}).
- Set directions first. Horizontal ({key:shape.bs-horizontal}) and Vertical ({key:shape.bs-vertical}) take a line or two points. Parallel ({key:shape.bs-parallel}) and Perpendicular ({key:shape.bs-perpendicular}) take two lines. Tangent ({key:shape.bs-tangent}) makes an arc, spline or conic leave a line smoothly. Symmetric ({key:shape.bs-symmetric}) takes two points and the line between them.
- Make equal what should stay equal with Equal ({key:shape.bs-equal}), then dimension it once with {key:shape.bs-dimension}. Type a parameter, or name = expression, as the value to change it later from the Board card.
- Point at a curve or point, or select it, to see its constraints. {key:shape.bs-constraints} shows them all. Click a constraint's glyph and press {key:shape.delete} to delete it. If parts are dimensioned to what it held, the card says how many.
- A constraint the sketch sets already is refused, and the message names what sets it. Delete that one first if the new one says better what you mean.
- Name the edges parts are placed from ({key:shape.bs-name-edge}) and choose Done. An outline that is still open is reported as [[finding/board.shape_open]].

Automation sends the same constraints with [[entry/board.edit]], under the same names.
