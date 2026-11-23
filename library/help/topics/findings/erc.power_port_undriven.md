---
{
  "id": "finding/erc.power_port_undriven",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/erc.power_port_undriven": "19b4f50ca02089bf2a1a7eab5dff765ee6c16f8e003890031df2a6f8b391f4de"
  },
  "cause": "The net has a power port and power inputs but no declared source.",
  "fix": "Connect a supply to the net",
  "fixed_by": [
    "entry/schematic.connect"
  ]
}
---
