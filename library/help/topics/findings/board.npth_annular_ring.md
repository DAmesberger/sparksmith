---
{
  "id": "finding/board.npth_annular_ring",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.npth_annular_ring": "5c28f764dc0798c0b5e4f521e630ab9b22c114918ca91ff9257720cb227e59fc"
  },
  "cause": "The copper land around an unplated hole is too narrow for the selected fabrication limit.",
  "fix": "Enlarge the pad or shrink its hole",
  "rule_key": "npth_annular_ring_min",
  "title": "Copper ring around an unplated hole too narrow",
  "keywords": [
    "NPTH",
    "annular ring",
    "non-plated through-hole"
  ],
  "explanation": "This check applies where the footprint puts a copper land around an unplated hole. Measure to the nearest land edge. A bare mechanical hole with no land has no copper ring to measure.",
  "related": [
    "finding/board.drill_size",
    "guide/fab-profiles"
  ]
}
---
