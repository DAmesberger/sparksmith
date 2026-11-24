---
{
  "covers": {
    "entry/layout.route_preview": "892f04d3b3e05b4d"
  },
  "id": "panel/layout",
  "keywords": [
    "board editor",
    "airwires",
    "placement",
    "tracks",
    "copper fill",
    "vias"
  ],
  "kind": "panel",
  "related": [
    "guide/board-shape",
    "guide/first-board",
    "guide/routing",
    "panel/model-3d"
  ],
  "since": "0.1.0",
  "summary": "Place footprints and build connected copper on the board’s layers.",
  "title": "PCB layout"
}
---

After the schematic is ready, choose Update PCB and review what will be placed or changed. Position the footprints inside the board outline. Select a placement to inspect its side, rotation and pad connections.

Choose Route to start from a pad or a track. The active layer controls where the next track or fill is created. Airwires indicate physical connections that remain to be made. Follow [[guide/routing]] for corners, widths and layer changes.

Set the net and layer before drawing a fill outline. Refill after edits that change its copper. Check [[panel/issues]], then switch to PCB 3D to inspect the same board. [[entry/layout.route_preview]] shows a proposed route step without committing it.
