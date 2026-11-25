---
{
  "id": "finding/board.unrouted",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.unrouted": "7eda3e28e211a54926e226bd4d100635f5708c7825ee3b834a943609c3ef6908"
  },
  "cause": "The named pads are the ends of the connections still to route: each joins an island of its net's copper to another, as the airwires in Layout show. Pads already joined, through a pour for example, are not named.",
  "fix": "Route the connection",
  "fixed_by": [
    "entry/layout.route"
  ]
}
---
