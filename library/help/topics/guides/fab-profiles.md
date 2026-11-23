---
{
  "id": "guide/fab-profiles",
  "kind": "guide",
  "title": "Choose a fabrication profile",
  "summary": "Match the board to a fabrication process, then trace each design rule to its source.",
  "since": "0.1.0",
  "covers": {
    "entry/rules.effective": "2c692f52f8c0092b",
    "entry/rules.fab_profiles": "331da7beea28b429",
    "entry/rules.preview_fab_profile": "7f6df267290929e6",
    "term/fab-profile": "74fba9de1b184729b6de8167c3801028d50fe7643a3f0aec3dd371936efc11dc"
  },
  "keywords": [
    "design rules",
    "manufacturer",
    "clearance",
    "limits",
    "stackup",
    "profile options"
  ],
  "related": [
    "guide/fab-output",
    "guide/routing",
    "panel/settings"
  ]
}
---

A [[term/fab-profile]] records the manufacturer's process, layer count, rule values and their sources. Choose a profile for the board's layer count and check the manufacturing options, including copper weight, before relying on its numbers.

- Open the board's Rules controls. Inspect the profile, its options and the effective rules. A source row identifies where a limit came from. An author’s choice sets a working default for this profile.
- Preview a profile change. Read the impact on existing overrides and net classes before applying it. A board can need a different profile or a stackup change if its layer count, finish or copper weight falls outside that process.
- Follow a rule link from a finding to inspect the underlying value, source document, row and date. Compare it with the current requirements for your order.
- Set design-specific restrictions with overrides or net classes. Review findings after changing them. Export against the profile you selected for production.

[[entry/rules.fab_profiles]] lists available profiles. [[entry/rules.preview_fab_profile]] previews a change. [[entry/rules.effective]] reads the rules currently in effect on the board.

```example
{"entry":"entry/rules.effective","params":{}}
```
