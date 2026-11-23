---
{
  "id": "finding/tie.loop",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.loop": "3ec2bf0da92099f8bc4478e368bb3bd0bfb828e7cad229bec726b21c6e7c3389"
  },
  "cause": "Closed ties form a loop, so the nets they join meet by more than one path.",
  "fix": "Open or remove one of the ties in the loop",
  "fixed_by": [
    "entry/schematic.disconnect"
  ]
}
---
