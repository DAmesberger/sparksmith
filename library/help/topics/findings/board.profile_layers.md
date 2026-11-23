---
{
  "id": "finding/board.profile_layers",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.profile_layers": "2fcf1f1f2bd9104f376bd62c16ea3497bcafed0656dda9b02ec3fd1630ae856a"
  },
  "cause": "The stackup's copper layer count falls outside the boards covered by this profile.",
  "fix": "Choose a fab profile for the board's layer count (Board › Rules)",
  "fixed_by": [
    "entry/rules.set_fab_profile",
    "entry/layout.edit_stackup"
  ]
}
---
