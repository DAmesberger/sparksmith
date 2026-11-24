---
{
  "id": "guide/first-board",
  "kind": "guide",
  "title": "Make your first board",
  "summary": "Place two parts, wire their pins, and follow that connection onto the board.",
  "since": "0.1.0",
  "covers": {
    "entry/component.place": "c33c11f7baa52915",
    "entry/layout.place": "29400de56d6ae471",
    "entry/layout.sync": "8c0ca73827fa1a28",
    "entry/library.search": "b8b0cb4373806d60",
    "entry/project.issues": "8179456145cb1eba",
    "entry/schematic.connect": "b7e09bfd74ef037e",
    "entry/schematic.pins": "8649c33cede45de9"
  },
  "keywords": [
    "new project",
    "first PCB",
    "schematic to board",
    "build order",
    "wire two parts"
  ],
  "related": [
    "guide/libraries",
    "guide/routing",
    "guide/fab-profiles",
    "guide/fab-output",
    "panel/schematic",
    "panel/layout"
  ]
}
---

Start a project with [[command/file.new]]. Its board starts with the Standard fab profile, or the default from your settings, so you can route at once. [[guide/fab-profiles]] explains how to choose your fabricator's own before you order. Save the project with [[command/file.save]] so you can return to it after each stage.

- In Schematic, choose Place and search for a component. For a simple exercise use two headers, or a header and a resistor. Choose a result with a footprint and place an instance of each on the sheet. For a resistor enter the required value on the instance.
- Use Wire to connect one pin of each component. Select the wire to inspect the net and its members. Open [[panel/issues]] and check the schematic before placing the parts on the board.
- Open PCB and choose Update PCB. Review the instances and connectivity in the preview, then apply the update. Set the board outline and place the footprints inside it. An airwire shows the connection you made on the sheet.
- Start Route at the first pad and finish on the matching pad. Check the connection in Layout and inspect the board in 3D. Save your work. Use [[guide/fab-output]] when you are ready to check a production set.

For automation, use [[entry/library.search]] to find the part, then [[entry/component.place]] with its physical choice and properties. Leave out position to let Sparksmith place the symbol in the next free space. Search again after making a generic component. Its value belongs on each instance.

For a group of placed parts, read [[entry/schematic.pins]] with their designators. The rows give the instance, terminal, pad numbers, pin name and connected net. Identify the connections from the intended circuit. Batch the terminal-to-terminal calls to [[entry/schematic.connect]] at the revision you read. To extend a supply net, connect to a pin already on that net.

```example
{"entry":"entry/schematic.pins","params":{"designators":["J1","R1"]}}
```

After the schematic review, [[entry/layout.sync]] carries the placed parts and their nets to the board. Place the parts using [[entry/layout.place]], then route the required nets. A fill needs a net, layer and outline. Use [[entry/project.issues]] after each stage and resolve any finding that prevents the next step.
