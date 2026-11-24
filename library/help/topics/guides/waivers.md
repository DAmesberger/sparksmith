---
{
  "id": "guide/waivers",
  "kind": "guide",
  "title": "Accept a finding with a waiver",
  "summary": "Mark a finding as reviewed and accepted, and see why a waiver ends.",
  "since": "0.1.0",
  "covers": {
    "entry/fabrication.waive": "0"
  },
  "keywords": [
    "waive",
    "waiver",
    "ignore error",
    "accept warning",
    "exception",
    "suppress",
    "restore"
  ],
  "related": [
    "guide/issues",
    "guide/fab-output",
    "guide/export-disabled"
  ]
}
---

A waiver says that you looked at one finding and accept it. It does not change the board. Fix a finding when you can; waive it when it is right for this build.

- Open Issues with {key:view.issues}, or Production files. Hover the finding and press Waive….
- Write the reason in the field, then confirm. The next reviewer reads it.
- The finding moves to the Waived group. Restore puts it back.
- A waiver accepts the value the finding has now. If the value gets worse, the waiver ends and the finding says what was accepted and what it is now.
- Some findings have no Waive…: a short, a limit of the fab profile itself, a fill that is not filled, an outline that is not closed, a part that is not placed. Their files would not be your design. Fix those, or switch to a fab profile that allows the limit.

Settings › Production can require a reason for waiving errors.
