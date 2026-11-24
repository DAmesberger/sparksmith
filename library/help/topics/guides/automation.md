---
{
  "id": "guide/automation",
  "kind": "guide",
  "title": "Build a board through the tools",
  "summary": "For assistants and scripts: the calls that take a circuit from parts to a routed board.",
  "since": "0.1.0",
  "covers": {
    "entry/component.place": "0",
    "entry/layout.place": "0",
    "entry/layout.sync": "0",
    "entry/library.search": "0",
    "entry/project.issues": "0",
    "entry/schematic.connect": "0",
    "entry/schematic.pins": "0"
  },
  "keywords": ["MCP", "agent", "script", "batch", "build order"],
  "related": ["guide/first-board", "guide/ai-setup", "panel/assistant"]
}
---

Use [[entry/library.search]] to find the part, then [[entry/component.place]] with its physical choice and properties. Leave out position to let Sparksmith place the symbol in the next free space. Search again after making a generic component. Its value belongs on each instance.

For a group of placed parts, read [[entry/schematic.pins]] with their designators. The rows give the instance, terminal, pad numbers, pin name and connected net. Identify the connections from the intended circuit. Batch the terminal-to-terminal calls to [[entry/schematic.connect]] at the revision you read. To extend a supply net, connect to a pin already on that net.

```example
{"entry":"entry/schematic.pins","params":{"designators":["J1","R1"]}}
```

After the schematic review, [[entry/layout.sync]] carries the placed parts and their nets to the board. Place the parts using [[entry/layout.place]], then route the required nets. A fill needs a net, layer and outline. Use [[entry/project.issues]] after each stage and resolve any finding that prevents the next step.
