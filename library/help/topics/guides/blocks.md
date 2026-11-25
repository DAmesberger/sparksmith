---
{
  "covers": {
    "entry/layout.block_add": "ded7a9d87b31f7d7",
    "entry/layout.block_delete": "244adef7c3868093",
    "entry/layout.block_lock": "07f4ac02b6cea050",
    "entry/layout.block_members": "ff0c6f8c164453e6",
    "entry/layout.block_move": "d5173502033f7b9b",
    "entry/layout.block_rename": "19c486c52d52efb8"
  },
  "id": "guide/blocks",
  "keywords": [
    "block",
    "layout block",
    "group parts",
    "module",
    "move parts together",
    "keep parts together",
    "ungroup",
    "lock a block",
    "flip a block",
    "rotate a block",
    "level of detail",
    "border"
  ],
  "kind": "guide",
  "related": [
    "panel/layout",
    "guide/update-pcb",
    "guide/dimensions",
    "term/layout-block"
  ],
  "since": "0.1.0",
  "summary": "Make a block of parts that belong together, so they move, turn, flip and lock as one.",
  "title": "Move parts together as a block"
}
---

A [[term/layout-block]] is a set of parts that belong together, such as a converter with its inductor and capacitors.

- In Layout, select the parts and press {key:pcb.make-block}, or choose Make block on the Selection card. The block is selected; type its name on its card.
- A dash-dot border shows the block. It runs round the courtyards of its parts and follows them. Other parts and traces may lie in it.
- Drag the border to move the whole block. Drag one part to move it alone.
- Rotate 90° ({key:pcb.rotate}) and Flip on the card turn the block or mirror it onto the other side. Traces attached to its parts come along on a move or a turn. What is left unrouted is named, and so is a part that now lies beyond the board. A flip leaves the traces on their layers.
- Locked keeps the block and every part of it where they are. A locked part, or one its dimensions hold, keeps the whole block from moving, and the message names it. Dimensions come first: a block adds none, and a change of the board still moves the parts they hold.
- To take a part out, press × on its row of the card. To add parts, select them with the block and choose Add to block. A part is in one block at most.
- Delete ({key:pcb.delete}) deletes the block. Its parts and their copper stay.

A part that is not on the board yet can be in a block; it joins the border once it is placed. Copied symbols make new parts, which are in no block.

Zoomed out, a block is drawn as its border and name, nearer as its parts, and nearer still with their pads and copper. Set Blocks in the Layers and view menu, or press {key:pcb.block-detail}, to keep one of these. This changes only what is drawn, and a click takes what is drawn. While a tool such as Route is on, everything is drawn, and so is what is selected.

The Browser lists the blocks under PCB. Blocks in the selection filter says whether a click takes them.
