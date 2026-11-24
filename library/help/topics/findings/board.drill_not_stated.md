---
{
  "id": "finding/board.drill_not_stated",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.drill_not_stated": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "The fabricator offers filled vias without saying what it makes.",
  "cause": "The fabricator's pages name the fill but give no usable limit, or limits that contradict each other.",
  "fix": "Ask the fabricator, then waive with its answer as the reason, or choose a profile that states it",
  "fixed_by": [
    "entry/fabrication.waive",
    "entry/layout.via_set",
    "entry/rules.set_fab_profile"
  ],
  "explanation": "Sparksmith does not guess a limit the fabricator has not published. A waiver of this finding always needs a reason. It holds only for the profile and the via sizes it was given for: under another profile, or with other sizes, it asks again.",
  "related": [
    "finding/board.drill_not_made",
    "guide/fab-profiles"
  ]
}
---
