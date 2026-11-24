---
{
  "covers": {
    "entry/layout.dimension_add": "3e44b6de33f1ea93"
  },
  "id": "guide/dimensions",
  "keywords": [
    "dimension",
    "distance",
    "measure",
    "keep from edge",
    "hold part",
    "held by its dimensions",
    "align connector"
  ],
  "kind": "guide",
  "related": [
    "guide/parameters",
    "guide/update-pcb"
  ],
  "since": "0.1.0",
  "summary": "Hold a part at a distance from an edge, a hole or another part.",
  "title": "Dimension a part"
}
---

A dimension on the board holds a part at a distance. It is kept when the board changes.

- In PCB Layout or 3D, choose Dimension.
- Click a part's face, edge, hole or pad, then what to measure to: a board edge, a hole or another part.
- The value field shows the distance now. Type the one you want, such as 2 mm, or width / 2 with a board parameter, and press Enter. On a board with a sketched shape, gap = 2 mm makes the parameter gap. Esc starts again.
- Click a drawn dimension to select it. Double-click it to change the value. Delete removes it.

A part that does not move and says it is held by its dimensions: edit or delete one of them. Free part in Properties does that when there is only one.

To read a distance only, look at the value field and press Esc.
