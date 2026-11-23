---
{
  "id": "finding/erc.driver_conflict",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/erc.driver_conflict": "e47727755e23707f2611526a044af2bd7432653c4aa4c2bf1404b751145e061c"
  },
  "cause": "Two push-pull outputs on the same net can drive opposite levels.",
  "fix": "Separate the outputs or select compatible functions",
  "fixed_by": [
    "entry/instance.select_functions",
    "entry/schematic.disconnect"
  ]
}
---
