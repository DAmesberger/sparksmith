---
{
  "id": "finding/board.hole_clearance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.hole_clearance": "4b25b505b81458ea5cfb715428c9f1fbd9502d80563311d8ca447a1476d426b2"
  },
  "cause": "The distance from a mounting hole to nearby copper is smaller than the clearance for those features.",
  "fix": "Move the copper away from the hole",
  "fixed_by": [
    "entry/board.edit",
    "entry/layout.track_drag"
  ],
  "explanation": "The mounting hole’s plating and the nearby copper type select the clearance row. The check also considers the copper net’s class clearance."
}
---
