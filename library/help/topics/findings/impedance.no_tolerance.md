---
{
  "id": "finding/impedance.no_tolerance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/impedance.no_tolerance": "11e0e44eccece67bdb1f82f324ed476149a5a168f0442c7df6246d1cd3a01027"
  },
  "cause": "The net has an impedance target but lacks a tolerance to compare against.",
  "fix": "Set Impedance tolerance in Settings or on the net class",
  "fixed_by": [
    "entry/net_class.edit"
  ],
  "checks": "setting/impedance_tolerance_pct",
  "explanation": "The net class can supply its own percentage. The checker uses the Settings value when the class leaves its tolerance unset."
}
---
