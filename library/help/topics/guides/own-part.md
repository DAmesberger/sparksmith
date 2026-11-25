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
- For a part no generator makes, choose Blank component. Type a Designator prefix such as R in Properties. Add its terminals with Terminal on the Pins tab, then draw on the Symbol and Footprint tabs. The order does not matter: Terminal adds a pad to the footprint sketch, unless pads you drew still wait for a terminal; the Mapping tab joins those. Or choose Paste table… on the Pins tab and paste the datasheet's pin table: it adds the terminals with a pin each on the symbol, and numbers the package from the Pin column, so a package From generator on the Footprint tab lands every terminal by its number.
- Give the body its size on the 3D tab, 0.01 to 1000 mm on each side. Publish needs it.
- From a datasheet, choose Create from datasheet…. It needs the assistant: see [[guide/ai-setup]].
- Check the pin numbers against the datasheet on the Mapping tab, and the part on the 3D tab.
- Fix what Issues lists, then choose Publish. A draft cannot be placed.
- Go to Schematic, choose Place and search for its name.

Publishing again makes a new revision. Placed parts keep theirs until you update them.
