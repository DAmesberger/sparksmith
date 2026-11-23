---
{
  "id": "finding/board.via_annular_ring",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_annular_ring": "c5769bdb455b04399b68cfcc5718f45c345dc70b667c7ca96aa298be3865f5ec"
  },
  "cause": "The copper ring between the via drill and the outside of its pad is narrower than the profile allows.",
  "fix": "Enlarge the via pad or shrink its drill",
  "fixed_by": [
    "entry/layout.add_via",
    "entry/layout.via_delete"
  ],
  "rule_key": "via_annular_ring_min",
  "explanation": "For a round via, subtract the drill diameter from the pad diameter and divide by two. A larger pad leaves more copper around the drill. If you reduce the drill diameter, also check the minimum drill size and board thickness-to-drill ratio.",
  "related": [
    "finding/board.via_drill_size",
    "finding/board.via_aspect_ratio",
    "guide/fab-profiles"
  ]
}
---
