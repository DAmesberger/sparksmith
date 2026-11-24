---
{
  "cause": "The fabricator is told which vias to fill by their hole size, so it cannot tell these filled vias from the open ones.",
  "covers": {
    "finding/board.via_fill_told": "6edbf33d09d0b97f83114d351caf0842b71062cf40ffc99d5dc7a315fbb4f201"
  },
  "fix": "Give the filled vias a drill of their own, or fill every via of that size",
  "fixed_by": [
    "entry/layout.via_set"
  ],
  "id": "finding/board.via_fill_told",
  "kind": "finding",
  "related": [
    "guide/fab-profiles"
  ],
  "since": "0.1.0",
  "summary": "Filled and open vias share one hole size."
}
---
