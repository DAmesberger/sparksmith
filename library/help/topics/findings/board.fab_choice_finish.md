---
{
  "id": "finding/board.fab_choice_finish",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.fab_choice_finish": "38ffa25c340bb3837f863086fb36721c75c41a94081f67637ce4a2bf3df8b6e4"
  },
  "cause": "A selected manufacturing option requires a different board finish.",
  "fix": "Choose a compatible finish or change the manufacturing option",
  "fixed_by": [
    "entry/rules.set_fab_options",
    "entry/layout.set_fabrication"
  ],
  "explanation": "The requirement comes from the option choice named in the finding. Inspect that choice in the active fabrication profile for its permitted pad finishes."
}
---
