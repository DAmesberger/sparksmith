---
{
  "id": "finding/instance.required_terminal_no_connect",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/instance.required_terminal_no_connect": "049b5507fbe6b062700bea57204693c67f9593ba8e8d7e170634f4646804802e"
  },
  "cause": "A power input is marked no-connect although its active function requires a supply.",
  "fix": "Connect the power input",
  "fixed_by": [
    "entry/instance.set_no_connect",
    "entry/schematic.connect"
  ]
}
---
