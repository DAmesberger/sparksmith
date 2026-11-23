---
{
  "id": "finding/erc.open_drain_contention",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/erc.open_drain_contention": "f908c51e4b8b8ae4794f62131664aafcf78ab000732a57f4b119fb9686f364a0"
  },
  "cause": "A push-pull output can drive high while an open-drain terminal pulls the shared net low.",
  "fix": "Separate the outputs or configure compatible open-drain functions",
  "fixed_by": [
    "entry/instance.select_functions",
    "entry/schematic.disconnect"
  ]
}
---
