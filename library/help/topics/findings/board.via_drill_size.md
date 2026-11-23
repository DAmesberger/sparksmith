---
{
  "id": "finding/board.via_drill_size",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_drill_size": "830fcf16fe35036261951098f79551d66fd20bf3623c98b9311284b284ffebaa"
  },
  "cause": "The via drill is smaller than the effective drill minimum.",
  "fix": "Enlarge the via drill",
  "fixed_by": [
    "entry/layout.add_via",
    "entry/layout.via_delete"
  ],
  "rule_key": "drill_min",
  "title": "Via drill below the minimum diameter"
}
---
