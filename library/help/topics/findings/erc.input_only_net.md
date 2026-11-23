---
{
  "id": "finding/erc.input_only_net",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/erc.input_only_net": "598fee051d8310073b554841c91bc2510c757a6bf38b6da4873e87a67f363709"
  },
  "cause": "All checked terminals on the net are inputs, leaving no terminal to drive the signal.",
  "fix": "Connect the intended driver or correct its pin function",
  "fixed_by": [
    "entry/schematic.connect"
  ]
}
---
