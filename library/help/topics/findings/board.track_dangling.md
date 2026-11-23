---
{
  "id": "finding/board.track_dangling",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/board.track_dangling": "d9771d5b106009fc291e2411615f7938a65352ae0536afa5ab286524fe95f2fd"
  },
  "cause": "A track branch ends without reaching another connected feature on its net.",
  "fix": "Route the end to copper of its net or delete the track",
  "fixed_by": [
    "entry/layout.route",
    "entry/layout.track_delete"
  ]
}
---
