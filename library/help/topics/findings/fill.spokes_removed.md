---
{
  "id": "finding/fill.spokes_removed",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/fill.spokes_removed": "45932c1da4afb6128189bf7b4da29668655f2c45c7ecce7352a76c7142a827b2"
  },
  "cause": "The refill omitted thermal spokes that would have ended in a gap instead of reaching copper on the same net.",
  "fix": "Inspect the thermal connection and adjust the fill if a spoke is needed",
  "fixed_by": [
    "entry/fill.edit",
    "entry/fill.refill"
  ],
  "explanation": "Inspect the thermal gap, spoke width, count and angle at the indicated pad or via. The fill can override the profile’s thermal settings. A spoke survives only if it reaches copper of the same net."
}
---
