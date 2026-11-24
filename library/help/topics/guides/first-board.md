---
{
  "covers": {
    "entry/component.place": "c33c11f7baa52915",
    "entry/layout.place": "29400de56d6ae471",
    "entry/layout.sync": "8c0ca73827fa1a28",
    "entry/library.search": "b8b0cb4373806d60",
    "entry/project.issues": "8179456145cb1eba",
    "entry/schematic.connect": "b7e09bfd74ef037e",
    "entry/schematic.pins": "8649c33cede45de9"
  },
  "id": "guide/first-board",
  "keywords": [
    "new project",
    "first PCB",
    "schematic to board",
    "build order",
    "wire two parts",
    "getting started",
    "beginner",
    "tutorial"
  ],
  "kind": "guide",
  "related": [
    "guide/libraries",
    "guide/wiring",
    "guide/update-pcb",
    "guide/routing",
    "guide/fab-profiles",
    "guide/fab-output",
    "panel/schematic",
    "panel/layout"
  ],
  "since": "0.1.0",
  "summary": "Start here: place two parts, wire them, and follow the connection onto the board.",
  "title": "Make your first board"
}
---

Start a project with [[command/file.new]]. Its board starts with the Standard fab profile, or the default from your settings, so you can route at once.

- In Schematic, choose Place and type a package and a type, such as resistor 0603. A new project has no parts of its own: a generator makes the one you choose. Press Enter, then click the sheet. Place two parts. [[guide/libraries]] says more.
- Choose Wire, click a pin of one part, then a pin of the other. Select the wire to see its net and members in Properties. [[guide/wiring]] says more.
- Go to PCB and choose Update PCB. Read the review and press Apply update. Drag the parts into the board outline. A thin line, the airwire, shows the connection. See [[guide/update-pcb]].
- Choose Route, click the first pad, then the matching pad. Look at the board in the 3D tab, and read Issues. [[guide/routing]] and [[guide/issues]] help.
- Save the project. [[guide/fab-output]] makes the files to order it.

