---
{
  "id": "finding/board.via_in_pad_open",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_in_pad_open": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "Check for open via holes under solder paste.",
  "cause": "The via's hole is open and lies in a paste opening, so solder can run into it during reflow.",
  "fix": "Fill and cap the via, or move it out of the pad",
  "fixed_by": [
    "entry/layout.via_set",
    "entry/layout.via_delete"
  ],
  "related": [
    "finding/board.drill_not_made"
  ]
}
---
