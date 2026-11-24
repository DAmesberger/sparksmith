---
{
  "cause": "Copper of two different nets touches: the board would be made with the nets joined.",
  "covers": {
    "finding/board.overlap": "6e70efa7cfda3c766999b77e3c7bda90119dc955ab2e1ea755ebd42754453992"
  },
  "explanation": "Pads, vias and tracks are compared on every layer they share. A gap that is only narrower than its rule is reported as a clearance finding and can be waived. Contact cannot, and accepting a gap does not accept contact that comes later.",
  "fix": "Move the copper apart",
  "fixed_by": [
    "entry/layout.track_drag",
    "entry/layout.move_parts"
  ],
  "id": "finding/board.overlap",
  "kind": "finding",
  "since": "0.1.0",
  "title": "Copper of different nets touches"
}
---
