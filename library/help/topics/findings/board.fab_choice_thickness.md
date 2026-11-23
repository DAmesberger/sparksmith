---
{
  "id": "finding/board.fab_choice_thickness",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.fab_choice_thickness": "40487792407c80922393d1f87dbdcd951fa73da0659689c2167c1b9b1770d246"
  },
  "cause": "The board stack is too thick for a selected manufacturing option.",
  "fix": "Choose another option in Board › Rules, or a thinner stackup",
  "fixed_by": [
    "entry/rules.set_fab_options",
    "entry/layout.edit_stackup"
  ],
  "explanation": "The maximum thickness comes from the selected option choice. Compare that choice with the total thickness in the stackup."
}
---
