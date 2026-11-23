---
{
  "id": "finding/erc.power_output_conflict",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/erc.power_output_conflict": "7c7070e2b18fe6761bbb722447af8b3e13d2d99a11c266c5db07ac18cf4077c8"
  },
  "cause": "Two power outputs drive the same net.",
  "fix": "Separate the supplies, or waive with the reason if they share the net",
  "fixed_by": [
    "entry/schematic.disconnect",
    "entry/instance.select_functions"
  ]
}
---
