---
{
  "id": "panel/settings",
  "kind": "panel",
  "title": "Settings",
  "summary": "Set your personal keyboard, library, display and checking preferences.",
  "since": "0.1.0",
  "covers": {
    "entry/rules.effective": "2c692f52f8c0092b"
  },
  "keywords": [
    "preferences",
    "keyboard",
    "online registry",
    "default fab profile",
    "impedance tolerance"
  ],
  "related": [
    "panel/keyboard-shortcuts",
    "guide/fab-profiles",
    "panel/libraries"
  ]
}
---

Open Settings with {key:app.settings}. The Keyboard section records command bindings and controls the key-reveal display. Lists changes density and motion. Browser controls how components are grouped.

Rules lets you choose a fab profile for new boards (the shipped default, Standard, until you choose one) and the fallback tolerance for a net class’s impedance target. Open [[setting/impedance_tolerance_pct]] to inspect the shipped tolerance and its source. A class can set its own tolerance. See [[entry/rules.effective]] for the rules in the open design.

Use Libraries for online registry preferences and Library sources to add or enable local folders. AI holds the model connection settings for [[panel/assistant]].
