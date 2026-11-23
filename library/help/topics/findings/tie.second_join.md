---
{
  "id": "finding/tie.second_join",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.second_join": "c4d6e80499a7698a3565d4c94789adcd65729082f19d931c6e2ab0fda2d6fd0c"
  },
  "cause": "The nets a tie joins also meet through other copper outside the tie.",
  "fix": "Remove the extra copper join so the nets meet only at the tie",
  "fixed_by": [
    "entry/layout.track_drag",
    "entry/fill.edit"
  ]
}
---
