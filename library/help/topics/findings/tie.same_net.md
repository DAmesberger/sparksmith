---
{
  "id": "finding/tie.same_net",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.same_net": "0a6136d4357146e9ae7cbfc5390ef136d44290ec916335d469c99e1e1e1c1bb1"
  },
  "cause": "Two ends of a tie are on the same net, so the tie joins nothing.",
  "fix": "Connect each tie end to a different net",
  "fixed_by": [
    "entry/schematic.disconnect",
    "entry/schematic.connect"
  ]
}
---
