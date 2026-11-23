---
{
  "id": "finding/package.terminals_unmatched",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/package.terminals_unmatched": "aa6516d1179c00cb5fdb208901512a71abdbde6f01b6a4cb0bd5483f405d804b"
  },
  "cause": "A package change removed or added terminal numbers that could not be matched exactly.",
  "fix": "Review unmatched terminals and explicitly map corresponding contacts",
  "fixed_by": [
    "entry/library.assign_terminals"
  ]
}
---
