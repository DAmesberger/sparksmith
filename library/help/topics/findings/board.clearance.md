---
{
  "id": "finding/board.clearance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.clearance": "8ddcb8515c4498fb641623c2572519eb69e24770ec8401619068dfd90f45df7c"
  },
  "cause": "The gap between the two named copper features falls below their effective clearance.",
  "fix": "Move the parts apart or reroute the copper",
  "fixed_by": [
    "entry/layout.track_drag",
    "entry/layout.move_parts"
  ],
  "title": "Copper features too close together",
  "explanation": "The two copper object types select a clearance row. Net classes can require more separation, including an isolation distance between pads on the same footprint. The finding names the source of the limit that applied."
}
---
