---
{
  "id": "finding/board.drill_not_made",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.drill_not_made": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "Check filled vias against what the fabricator makes.",
  "cause": "The fab profile does not make this fill, or not in a hole of this size.",
  "fix": "Leave the via open, change its drill, or choose a fab profile that makes it",
  "fixed_by": [
    "entry/layout.via_set",
    "entry/rules.set_fab_profile"
  ],
  "explanation": "The profile states which fills the fabricator makes of a through via and the hole sizes it fills. This finding cannot be waived: no reason makes the fabricator able to build it.",
  "related": [
    "finding/board.drill_not_stated",
    "guide/fab-profiles"
  ]
}
---
