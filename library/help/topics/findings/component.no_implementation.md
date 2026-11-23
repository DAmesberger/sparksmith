---
{
  "id": "finding/component.no_implementation",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/component.no_implementation": "2a2b61099c0bb77150536a3fe81ac1ceb316af919ca854d916ebe0d27c70bd62"
  },
  "cause": "The component has no physical binding, so its instances can appear only in the schematic.",
  "fix": "Add a binding and footprint when the component needs board placement",
  "fixed_by": [
    "entry/library.edit"
  ]
}
---
