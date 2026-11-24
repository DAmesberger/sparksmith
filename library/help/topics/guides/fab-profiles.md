---
{
  "covers": {
    "entry/rules.effective": "2c692f52f8c0092b",
    "entry/rules.fab_profiles": "331da7beea28b429",
    "entry/rules.preview_fab_profile": "f1a7c7f6aebffc96",
    "term/fab-profile": "1c5968c3afa1e6a0a74a6a0a5c0844cfab0006c8b84292181a425a7f43d24e0c"
  },
  "id": "guide/fab-profiles",
  "keywords": [
    "design rules",
    "manufacturer",
    "clearance",
    "limits",
    "stackup",
    "profile options"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-output",
    "guide/routing",
    "panel/settings"
  ],
  "since": "0.1.0",
  "summary": "Match the board to a fabrication process, then trace each design rule to its source.",
  "title": "Choose a fabrication profile"
}
---

A new board starts with the Standard profile, or with the default you chose in [[panel/settings]], so you can route and fill at once. Standard holds the strictest standard limits of several fabricators, for boards of up to six layers with 1 oz outer copper. Your fabricator can ask for more or allow more, so choose its own profile before you order.

A [[term/fab-profile]] records the manufacturer's process, layer count, rule values and their sources. Choose a profile for the board's layer count and check the manufacturing options, including copper weight, before relying on its numbers.

- Open the board's Rules controls. Inspect the profile, its options and the effective rules. A source row identifies where a limit came from. An author’s choice sets a working default for this profile.
- Preview a profile change. Read the impact on existing overrides and net classes before applying it. A board can need a different profile or a stackup change if its layer count, finish or copper weight falls outside that process.
- Follow a rule link from a finding to inspect the underlying value, source document, row and date. Compare it with the current requirements for your order.
- A profile also says whether the fabricator fills and caps vias, and in which hole sizes. What it does not make cannot be waived. What it offers without stating can be waived, with the fabricator's answer as the reason.
- Set design-specific restrictions with overrides or net classes. Review findings after changing them. Export against the profile you selected for production.

[[entry/rules.fab_profiles]] lists available profiles. [[entry/rules.preview_fab_profile]] previews a change. [[entry/rules.effective]] reads the rules currently in effect on the board.

```example
{"entry":"entry/rules.effective","params":{}}
```
