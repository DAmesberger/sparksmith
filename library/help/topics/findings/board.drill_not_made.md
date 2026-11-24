---
{
  "cause": "The fab profile does not make this fill, or not in a hole of this size.",
  "covers": {
    "finding/board.drill_not_made": "94f8a4355ff45130682ddfd9e3fb8a884b2ca8d0bd2e59b94eef874944403c54"
  },
  "explanation": "The profile states which fills the fabricator makes of a through via and the hole sizes it fills. This finding cannot be waived: no reason makes the fabricator able to build it.",
  "fix": "Leave the via open, change its drill, or choose a fab profile that makes it",
  "fixed_by": [
    "entry/layout.via_set",
    "entry/rules.set_fab_profile"
  ],
  "id": "finding/board.drill_not_made",
  "kind": "finding",
  "related": [
    "finding/board.drill_not_stated",
    "guide/fab-profiles"
  ],
  "since": "0.1.0",
  "summary": "Check filled vias against what the fabricator makes."
}
---
