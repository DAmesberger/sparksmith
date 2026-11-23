---
{
  "id": "finding/fill.empty",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/fill.empty": "e47d6f2779e6421bc549d8352e52b7366f6a28d59b3b1ee38b696abcb69454cc"
  },
  "cause": "After clearances, priorities and island removal were applied, the fill contained no copper.",
  "fix": "Enlarge the fill outline or lower its priority",
  "fixed_by": [
    "entry/fill.edit",
    "entry/fill.refill"
  ],
  "explanation": "Check the outline and the clearances to its neighbours. The fill’s minimum width, island removal settings and the priority of overlapping fills can each remove the last patch of copper."
}
---
