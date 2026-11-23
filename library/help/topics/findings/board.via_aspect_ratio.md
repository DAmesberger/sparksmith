---
{
  "id": "finding/board.via_aspect_ratio",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_aspect_ratio": "d7cb85b8961f95e486b26e039535f1b780c5e320c9a6b03be80d554bd79a7fe6"
  },
  "cause": "The board thickness divided by the via drill diameter exceeds the profile's plating limit.",
  "fix": "Enlarge the via drill",
  "fixed_by": [
    "entry/layout.add_via",
    "entry/layout.edit_stackup"
  ],
  "rule_key": "aspect_ratio_max"
}
---
