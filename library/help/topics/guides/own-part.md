---
{
  "id": "guide/own-part",
  "kind": "guide",
  "title": "Make your own component",
  "summary": "Use a generator, start from nothing, or draft from a datasheet, then publish.",
  "since": "0.1.0",
  "covers": {
    "command/component.new": "0",
    "command/component.publish": "0"
  },
  "keywords": [
    "new component",
    "own part",
    "custom part",
    "footprint",
    "symbol",
    "draw footprint",
    "datasheet",
    "publish",
    "blank component"
  ],
  "related": [
    "guide/libraries",
    "panel/component",
    "workflow/datasheet-to-component"
  ]
}
---

A component is a symbol, a footprint and a map between them.

- Go to Component. On the start page choose New from generator… for a usual package: search, for example qfn 16, check the values and press Create component.
- For a part no generator makes, choose Blank component. Add its pins with Terminal, then draw on the Symbol and Footprint tabs.
- From a datasheet, choose Create from datasheet…. It needs the assistant: see [[guide/ai-setup]].
- Check the pin numbers against the datasheet on the Mapping tab, and the part on the 3D tab.
- Fix what Issues lists, then choose Publish. A draft cannot be placed.
- Go to Schematic, choose Place and search for its name.

Publishing again makes a new revision. Placed parts keep theirs until you update them.
