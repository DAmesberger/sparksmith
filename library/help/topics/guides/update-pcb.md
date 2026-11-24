---
{
  "covers": {
    "entry/layout.place": "29400de56d6ae471",
    "entry/layout.sync": "8c0ca73827fa1a28"
  },
  "id": "guide/update-pcb",
  "keywords": [
    "update pcb",
    "part missing on board",
    "not placed",
    "place parts",
    "move part",
    "rotate part",
    "bottom side",
    "flip",
    "schematic to board"
  ],
  "kind": "guide",
  "related": [
    "guide/wiring",
    "guide/routing",
    "guide/board-shape",
    "panel/layout"
  ],
  "since": "0.1.0",
  "summary": "Update PCB places new parts and nets on the board; then move, turn and flip them.",
  "title": "Bring parts to the board"
}
---

Parts and nets come to the board from the schematic. A part you added is not on the board until you update it.

- In PCB, choose Update PCB. It is off when the board already matches the schematic.
- Read the review: what is placed, what changes, and the new issues.
- Press Apply update. New parts stand beside the board.
- Drag a part into the outline, or select it and type its Position in Properties.
- A part that moves takes its tracks along. The message says how many connections were re-routed, and which were left unrouted.
- Turn it with {key:pcb.rotate} or Rotate 90°. Flip moves it to the other side.
- The thin lines are airwires: connections still to route. [[guide/routing]] is next.

If a part is missing, look at the review again: “has no footprint” means the component needs a footprint first. If a part does not move, Properties says whether it is locked or held by its dimensions.
