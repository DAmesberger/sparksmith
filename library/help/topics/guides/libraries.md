---
{
  "covers": {
    "entry/instance.set_properties": "7c8e1a4e6a53618d",
    "entry/library.search": "b8b0cb4373806d60",
    "entry/project.preview_update": "504d73c473ec9a87",
    "term/component-definition": "594eab48265807fa15f099e00c5985ec68a0abf0a3855318bc81f041adbb346a"
  },
  "id": "guide/libraries",
  "keywords": [
    "library folders",
    "find parts",
    "generic resistor",
    "publish",
    "update revisions"
  ],
  "kind": "guide",
  "related": [
    "panel/libraries",
    "panel/component",
    "workflow/datasheet-to-component"
  ],
  "since": "0.1.0",
  "summary": "Choose a component and its physical implementation, and keep the design tied to the revision you reviewed.",
  "title": "Find and reuse components"
}
---

A [[term/component-definition]] describes electrical terminals. A symbol draws them, a footprint places the pads, and a binding maps the two sets explicitly. Choose the implementation that matches the package you will fit.

- In Schematic, choose Place. Type a package and a type, then a value, for example “resistor 0603 10k”. A value alone finds nothing. Parts marked “not yet created” are made by a generator: press Enter to make and place one. Read the symbol, package and footprint beside the list. A component that is still a draft needs to be published before placement.
- For an existing generic component, use the physical choice and field values in the search result. A single component can serve many values. To change a placed resistor's value, edit its instance property.
- For a part that is not there, make your own: see [[guide/own-part]]. For a specific IC, use its manufacturer datasheet with [[workflow/datasheet-to-component]]. Verify the pin numbers, footprint and explicit mapping before publishing.
- To use a folder of parts, choose [[command/library.sources]] and add it. [[panel/libraries]] says what it must hold. When you place a published revision, the project keeps the definitions and assets needed to reopen it offline.
- When a newer revision appears, read the affected instances, terminals and copper in the update preview. Apply the update only after the changes match the intended design.

Automation returns generic hits alongside other matches in [[entry/library.search]]. A hit with a component carries its physical choice and instance properties. A hit with a make operation gives the operation to apply first. Use [[entry/instance.set_properties]] for a placed instance's fields. Use [[entry/project.preview_update]] to inspect a proposed revision change.

```example
{"entry":"entry/library.search","params":{"query":"resistor 0603 10k"}}
```
