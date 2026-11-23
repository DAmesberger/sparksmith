---
{
  "id": "finding/erc.undriven_power_input",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/erc.undriven_power_input": "3ee8e8da3e6faec12e35e619b4d8ffe8b6e7371fe48c5c22e4f871d555e1d052"
  },
  "cause": "A power input is on a net without a power-output driver.",
  "fix": "Connect the supply and verify its pin is declared as a power output",
  "fixed_by": [
    "entry/schematic.connect"
  ]
}
---
