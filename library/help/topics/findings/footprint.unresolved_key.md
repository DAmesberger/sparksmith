---
{
  "id": "finding/footprint.unresolved_key",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/footprint.unresolved_key": "7ae4271abb50d4f3d6e50afc2f2e2be5f414ac6f732b870630ea0690ef593865"
  },
  "cause": "New array pads have no recorded terminal decision.",
  "fix": "Map the new array pads before publishing",
  "fixed_by": [
    "entry/library.assign_terminals"
  ]
}
---
