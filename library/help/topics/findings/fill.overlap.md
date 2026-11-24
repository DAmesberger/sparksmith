---
{
  "cause": "The stored copper of a fill touches copper, or the plated hole, of another net: the board would be made with the nets joined.",
  "covers": {
    "finding/fill.overlap": "baddc3dcb88edb3006016bb3e279772f72e148c1756abffd488b0f5ac2d57340"
  },
  "explanation": "A refill cuts the fill back to its clearances. A gap that is only narrower than its rule is reported as a fill clearance finding and can be waived. Contact cannot, and accepting a gap does not accept contact that comes later.",
  "fix": "Refill the fill, or move the copper apart",
  "fixed_by": [
    "entry/fill.refill"
  ],
  "id": "finding/fill.overlap",
  "kind": "finding",
  "since": "0.1.0",
  "title": "Fill copper on another net"
}
---
