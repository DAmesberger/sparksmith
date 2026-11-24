---
{
  "id": "finding/fill.clearance",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/fill.clearance": "8a596ec15f66eaf4b2e7e2689917c9bf826016a908b6738a0720398a45e6b35d"
  },
  "cause": "The stored fill copper leaves too little space around another feature on its layer.",
  "fix": "Refill the fill",
  "fixed_by": [
    "entry/fill.refill"
  ],
  "explanation": "The fill and its neighbouring object select the clearance row. Both their net classes and fill-specific clearances can affect the gap. Copper of another net that touches the fill is reported as fill copper on another net."
}
---
