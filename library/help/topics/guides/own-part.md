---
{
  "covers": {
    "command/component.new": "5f89f451eaa1ae331b3120561b47efa34f6121ab1120365f15efe8c017f39328",
    "command/component.publish": "6ff201d05a1da5f6362c23f7789a78fa51fa54c56ae1f1100def8c277fc244b4"
  },
  "id": "guide/own-part",
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
  "kind": "guide",
  "related": [
    "guide/libraries",
    "panel/component",
    "workflow/datasheet-to-component"
  ],
  "since": "0.1.0",
  "summary": "Use a generator, start from nothing, or draft from a datasheet, then publish.",
  "title": "Make your own component"
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
