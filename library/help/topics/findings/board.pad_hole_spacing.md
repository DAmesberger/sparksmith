---
{
  "id": "finding/board.pad_hole_spacing",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.pad_hole_spacing": "8e6c8fe43570a8ee8c42d121d1bd8edd6dfe02f5f1e19ba9119488aeca2e2b2e"
  },
  "cause": "A pad hole lies too close to another drill. The check measures the gap between their edges.",
  "fix": "Move the part or the other hole",
  "fixed_by": [
    "entry/layout.move_parts"
  ],
  "explanation": "The hole types named in this occurrence select the spacing row. Pad drills are compared with other pad drills, vias and mounting holes, including a second hole within the same pad."
}
---
