---
{
  "id": "guide/board-shape",
  "kind": "guide",
  "title": "Change the board's size and shape",
  "summary": "Set the size, round corners, add slots, holes and cutouts.",
  "since": "0.1.0",
  "covers": {
    "entry/board.edit": "0",
    "entry/board.start_sketch": "0"
  },
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
    "dimension"
  ],
  "related": [
    "guide/parameters",
    "guide/dimensions",
    "guide/update-pcb"
  ]
}
---

The board's outline is a sketch: lines and arcs held by dimensions.

- In PCB, choose Shape. Or click an empty place on the board and press Edit shape… on the Board card.
- To change the size, choose Dimension, pick an edge, and type the length in Value. If the edge has a parameter, change its value under Board parameters.
- Draw with Line, Arc, Circle or Rectangle. Fillet rounds the corner you click.
- The outline must be one closed loop. Open ends are marked and the card says what is wrong.
- Slot: click both ends. Mounting hole: click its place. Cutout: two corners, or centre and radius.
- Press Done.

A refused value changes nothing; the reason stands on the card. Undo takes back each step. To move holes and parts with the edge, see [[guide/parameters]] and [[guide/dimensions]].
