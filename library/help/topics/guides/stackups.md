---
{
  "covers": {},
  "id": "guide/stackups",
  "keywords": [
    "stackup",
    "layers",
    "number of layers",
    "layer count",
    "copper layers",
    "2 layer board",
    "4 layer board",
    "6 layer board",
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
    "finding/board.class_layer",
    "finding/board.profile_layers"
  ],
  "since": "0.1.0",
  "summary": "Set the number of copper layers, or take a fabricator's published stackup with its sources.",
  "title": "Choose a fabricator's stackup"
}
---

A stackup is the board's foils and the dielectrics between them. Impedance depends on it, so take one your fabricator publishes before you route pairs.

The number of copper layers is part of the stackup. A new board has 2. Copper on the Board card shows how many the board has.

- To change the number, open Stackup… on the Board card and take a Preset, such as 4 layers, or press Add pair for two more inner layers. Select an inner layer and press Remove to take one away. OK applies it as one step.
- Copper on a layer you remove needs a place: choose a layer to move it to in the dialog. Left at Not moved, the change is refused.
- The fab profile states the numbers of layers it is for. Standard is for 1, 2, 4 or 6. Another number is reported: [[finding/board.profile_layers]]. Choose a profile for the number you want, see [[guide/fab-profiles]].

To take a fabricator's published stackup:

- Open Stackup… on the Board card and press Choose…. The list shows the stackups for the board's copper layers (Layers shows another number), the nearest thickness and the maker of its fab profile. The Standard profile names no maker, so every maker's are listed, each with its name.
- Select one to see its stack to scale, its thicknesses and Dk, where each value was read, and where the fabricator's own sources disagree. Both values are shown then.
- Read what it changes and what the fab profile says of it. Copper layers keep their place. Copper on a layer the stackup lacks needs a place before the change can be applied. A net class that names a layer keeps its list, and [[finding/board.class_layer]] tells you.
- Use this stackup puts it in the dialog. OK applies it as one step, and Undo takes it back.
- The board keeps the stackup's name, and the export writes it in the order notes, the job file and the fabrication drawing. Changing a layer by hand drops the name, because the board is no longer that stackup. Undo brings it back.

Some stackups put sheets of several Dk between two foils. The layer then takes their average by thickness. It is a calculated value and not the fabricator's: the dialog says so and shows the sheets, and you can enter another. Some fabricators list a prepreg before and after pressing; the board takes the pressed one.

You can still edit every layer by hand. Ask the fabricator for its current table before you order.
