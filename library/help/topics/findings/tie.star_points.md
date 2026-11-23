---
{
  "id": "finding/tie.star_points",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.star_points": "e3fcd4d5b0ec17e56dfdb45d84f6873d479bb2e733d42f7419e683e28601b415"
  },
  "cause": "A star-ground tie's nets meet somewhere else as well, so the star has more than one point.",
  "fix": "Remove the other join so the star-ground nets meet only at the tie",
  "fixed_by": [
    "entry/tie.set_purpose",
    "entry/layout.track_drag"
  ]
}
---
