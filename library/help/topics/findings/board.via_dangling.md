---
{
  "id": "finding/board.via_dangling",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_dangling": "8345568d5cc6ce8a1d16b06ec34123b237e54e4b5d3aad08fef49504b721d56e"
  },
  "cause": "The via does not join the intended copper path across layers.",
  "fix": "Route copper to the via or delete it",
  "fixed_by": [
    "entry/layout.route",
    "entry/layout.via_delete"
  ]
}
---
