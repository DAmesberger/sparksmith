---
{
  "id": "finding/board.pad_hole_clearance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.pad_hole_clearance": "744f5ff32bd7ee6ad3a6615d098035a8ab56002c039b690225b15a3f94dbb29a"
  },
  "cause": "A pad's drilled hole comes too close to the named copper feature.",
  "fix": "Move the copper away from the hole",
  "fixed_by": [
    "entry/layout.move_parts",
    "entry/layout.track_drag"
  ],
  "explanation": "The pad’s plating determines the hole type. That type and the neighbouring copper select the clearance row. The check skips copper electrically joined to the pad."
}
---
