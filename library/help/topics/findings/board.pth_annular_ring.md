---
{
  "id": "finding/board.pth_annular_ring",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.pth_annular_ring": "c1cd12f4e046a15dfd9f73ef54e7b30c46ca899344324f3da301c39bdd8de620"
  },
  "cause": "The copper left around the plated hole is narrower than the required annular ring.",
  "fix": "Enlarge the pad or shrink its hole",
  "rule_key": "pth_annular_ring_min",
  "title": "Plated through-hole annular ring too narrow",
  "keywords": [
    "PTH",
    "plated hole"
  ],
  "explanation": "Measure the narrowest strip of copper between the plated hole or slot and the land edge. For a slotted pad, inspect the ends as well as the sides. The finding names the affected pad or plated mounting hole.",
  "related": [
    "finding/board.drill_size",
    "guide/fab-profiles"
  ]
}
---
