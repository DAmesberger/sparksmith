---
{
  "id": "finding/impedance.mismatch",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/impedance.mismatch": "64cf3a41ef7ad16666c928d3e79be2e10fb28552c5086468b0b9d94e4784d3be"
  },
  "cause": "The routed impedance is outside the allowed band, or the layer has no reference plane for the estimate.",
  "fix": "Route at the width the Route card shows for the layer",
  "fixed_by": [
    "entry/layout.track_set",
    "entry/net_class.edit"
  ],
  "checks": "setting/impedance_tolerance_pct",
  "explanation": "For a numeric result, compare the named track or coupled run with its net class target. The class tolerance takes precedence over Settings. If the message names a missing reference plane, inspect the stackup for that layer."
}
---
