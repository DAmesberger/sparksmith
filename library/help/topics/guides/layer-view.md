---
{
  "covers": {},
  "id": "guide/layer-view",
  "keywords": [
    "layer visibility",
    "ratsnest",
    "x-ray"
  ],
  "kind": "guide",
  "related": [
    "guide/moving-around",
    "guide/view-3d",
    "guide/routing"
  ],
  "since": "0.1.0",
  "summary": "Choose which layers Layout and 3D show, hide the airwires, and look from the other side.",
  "title": "Show and hide layers"
}
---

What is shown never changes the board. Layers you hide in Layout cannot be picked there.

- In Layout, press Layers and view at the bottom of the canvas. Untick a layer, the vias or the silkscreen to hide it.
- Focus in the same list says what happens to the copper layers besides the active one: All layers, Dim others or Active only. {key:pcb.layer-focus} steps through them.
- Show airwires hides or shows the thin lines of the connections still to route. The connections stay unrouted and Issues still lists them.
- View from bottom shows Layout mirrored, as you would see it from the back. {key:pcb.from-bottom} does the same.
- In 3D, the Layers list hides bodies. X-ray leaves the copper layers and vias, and Show all brings the rest back. {key:view.bottom} looks at the board from below.
