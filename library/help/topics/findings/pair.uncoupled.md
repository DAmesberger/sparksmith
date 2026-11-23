---
{
  "id": "finding/pair.uncoupled",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/pair.uncoupled": "d720fe27497b288e88ebee5c45d0a4f45277df2a6703b05871a1a9e9e9b40342"
  },
  "cause": "The portion routed without a coupled partner exceeds the pair's maximum uncoupled length.",
  "fix": "Route the halves together with pair routing",
  "fixed_by": [
    "entry/layout.route_pair"
  ],
  "explanation": "The positive net’s class sets a maximum uncoupled length. Compare that limit with the longer of the two uncoupled lengths in the routing report."
}
---
