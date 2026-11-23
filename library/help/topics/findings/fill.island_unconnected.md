---
{
  "id": "finding/fill.island_unconnected",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/fill.island_unconnected": "5019b82b564e915697c178d31c54314f822370bc06e49544f84253192c6e707d"
  },
  "cause": "A piece of fill copper has no conductive path to another object on its assigned net.",
  "fix": "Remove unconnected islands or connect them",
  "fixed_by": [
    "entry/fill.edit",
    "entry/layout.route"
  ],
  "explanation": "Inspect the fill’s island removal mode and its net. A kept island can remain in the drawing when it touches no other copper on that net."
}
---
