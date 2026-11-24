---
{
  "id": "finding/board.via_fill_told",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_fill_told": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "Filled and open vias share one hole size.",
  "cause": "The fabricator is told which vias to fill by their hole size, so it cannot tell these filled vias from the open ones.",
  "fix": "Give the filled vias a drill of their own, or fill every via of that size",
  "fixed_by": [
    "entry/layout.via_set"
  ],
  "related": [
    "guide/fab-profiles"
  ]
}
---
