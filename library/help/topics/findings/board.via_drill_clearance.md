---
{
  "id": "finding/board.via_drill_clearance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_drill_clearance": "510de9af5a87f471a5c54056cf82dbe689f753ebc44929a24321dd9679fb4758"
  },
  "cause": "The edge of the via's drill is too close to the named copper feature.",
  "fix": "Move the via",
  "fixed_by": [
    "entry/layout.via_move",
    "entry/layout.track_drag"
  ],
  "explanation": "The via hole and the kind of nearby copper select the clearance row. A clearance set by the copper net’s class can increase the required distance."
}
---
