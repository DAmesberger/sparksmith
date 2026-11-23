---
{
  "id": "finding/pinout.uncertain",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/pinout.uncertain": "42492a8a0957dc555cdb2e9ce41810b22d94e971a6f2399f6c72066221a6cae0"
  },
  "cause": "An imported pin still carries an uncertainty flag from the source table.",
  "fix": "Check the pin against the datasheet and clear the flag once verified",
  "fixed_by": [
    "entry/library.pinout_apply"
  ]
}
---
