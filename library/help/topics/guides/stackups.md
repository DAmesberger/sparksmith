---
{
  "covers": {},
  "id": "guide/stackups",
  "keywords": [
    "stackup",
    "layers",
    "prepreg",
    "core",
    "impedance",
    "dielectric",
    "fabricator"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-profiles",
    "guide/routing",
    "finding/board.class_layer"
  ],
  "since": "0.1.0",
  "summary": "Take a fabricator's published layer stackup, with its sources and what it changes.",
  "title": "Choose a fabricator's stackup"
}
---

A stackup is the board's foils and the dielectrics between them. Impedance depends on it, so take one your fabricator publishes before you route pairs.

- Open Stackup… on the Board card and press Choose…. The list shows the stackups for the board's copper layers, the nearest thickness and the maker of its fab profile. The Standard profile names no maker, so every maker's are listed, each with its name.
- Select one to see its stack to scale, its thicknesses and Dk, where each value was read, and where the fabricator's own sources disagree. Both values are shown then.
- Read what it changes and what the fab profile says of it. Copper layers keep their place. Copper on a layer the stackup lacks needs a place before the change can be applied. A net class that names a layer keeps its list, and [[finding/board.class_layer]] tells you.
- Use this stackup puts it in the dialog. OK applies it as one step, and Undo takes it back.

Some stackups put plies of several Dk between two foils. The layer then has no Dk until you enter one, or choose the average of its plies by thickness, which is a computed value and not the fabricator's. Some fabricators list a prepreg before and after pressing; the board takes the pressed one.

You can still edit every layer by hand. Ask the fabricator for its current table before you order.
