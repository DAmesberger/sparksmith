---
{
  "id": "panel/model-3d",
  "kind": "panel",
  "title": "PCB 3D view",
  "summary": "Inspect component bodies, copper and board placement from different angles.",
  "since": "0.1.0",
  "covers": {
    "entry/project.export_native": "ee897275b361b97f",
    "entry/view.open": "f138f09bd022744a"
  },
  "keywords": [
    "3D model",
    "rotate board",
    "bottom view",
    "camera",
    "selection"
  ],
  "related": [
    "panel/layout",
    "panel/component",
    "guide/fab-output"
  ]
}
---

Switch from Layout to PCB 3D to see the same design and selection. Orbit the board to inspect the top, bottom and edge. Select a part to inspect its placement in Properties, then return to Layout to adjust it.

Look for a misplaced or flipped body, incorrect height, or a hole that meets the wrong pad. A component with an approximate envelope still needs its actual body or a drawing check before you rely on its mechanical clearance.

[[entry/view.open]] opens the requested workspace in the live application. [[entry/project.export_native]] writes a separate native board model for mechanical work. Use [[guide/fab-output]] for manufacturing output.
