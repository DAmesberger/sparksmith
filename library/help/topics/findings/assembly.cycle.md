---
{
  "id": "finding/assembly.cycle",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/assembly.cycle": "2671f5898d41278fed0c943b4ae0347a046643094300ce9a59a39bd902b5b11a"
  },
  "cause": "A board position follows geometry that the linked design positions from that same board.",
  "fix": "Stop following the linked design geometry, or rebuild it without the datum",
  "fixed_by": [
    "entry/assembly.unbind"
  ]
}
---
