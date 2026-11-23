---
{
  "id": "panel/component",
  "kind": "panel",
  "title": "Component editor",
  "summary": "Author a component draft and check its symbol, footprint, mapping and body before publication.",
  "since": "0.1.0",
  "covers": {
    "entry/component.check_drafts": "5e6b8a03d7e6acfa"
  },
  "keywords": [
    "component authoring",
    "terminals",
    "pins",
    "binding",
    "draft",
    "publish"
  ],
  "related": [
    "guide/libraries",
    "panel/libraries",
    "workflow/datasheet-to-component"
  ]
}
---

Use Details to edit the component name, reference prefix and fields. In Symbol, choose the unit and draw the pins at the connection ends where schematic wires should meet them. A pin represents an electrical terminal.

In Footprint, define the pads, holes and outline for the package. In Mapping, check where each package terminal lands and how it relates to the component terminal. The 3D view lets you inspect the body and its placement against the footprint.

Run [[entry/component.check_drafts]] and locate any blocking findings. Review the exact revision before publishing it for placement or reuse. A changed revision can be applied to placed instances through a preview.
