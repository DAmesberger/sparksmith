---
{
  "id": "finding/assembly.extent_cycle",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/assembly.extent_cycle": "29ef0a034b863eef040cb02c717e0b5c4e02a557f5e0cdc5c029293f048678c2"
  },
  "cause": "The linked geometry derives its extent from the board while the board follows that geometry.",
  "fix": "Check the extents settle; stop following the geometry to break the loop",
  "fixed_by": [
    "entry/assembly.unbind"
  ]
}
---
