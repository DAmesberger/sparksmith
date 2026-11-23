---
{
  "covers": {
    "entry/instance.set_properties": "7c8e1a4e6a53618d",
    "entry/library.search": "b8b0cb4373806d60",
    "entry/project.preview_update": "ddee0d5306fed713",
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

- Open Place in Schematic. Search by part type, package and value, for example “resistor 0603 10k”. Review the symbol, package and available footprint in the result. A component that is still a draft needs to be published before placement.
- For an existing generic component, use the physical choice and field values in the search result. A single component can serve many values. To change a placed resistor's value, edit its instance property.
- If a generic result offers to make the part, use that choice once and search again. For a specific IC absent from the catalog, use its manufacturer datasheet with [[workflow/datasheet-to-component]]. Verify the pin numbers, footprint and explicit mapping before publishing.
- Add shared folders in [[panel/libraries]]. When you place a published revision, the project keeps the definitions and assets needed to reopen it offline.
- When a newer revision appears, read the affected instances, terminals and copper in the update preview. Apply the update only after the changes match the intended design.

Automation returns generic hits alongside other matches in [[entry/library.search]]. A hit with a component carries its physical choice and instance properties. A hit with a make operation gives the operation to apply first. Use [[entry/instance.set_properties]] for a placed instance's fields. Use [[entry/project.preview_update]] to inspect a proposed revision change.

```example
{"entry":"entry/library.search","params":{"query":"resistor 0603 10k"}}
```
