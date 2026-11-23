---
{
  "id": "finding/binding.mapping_retained",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/binding.mapping_retained": "565bcebbf5236228a40a97a4dfadb6b982b315f83073a1ffc06c19ff00f26965"
  },
  "cause": "Changing the package kept mappings by position. Their terminal identities may have changed.",
  "fix": "Compare each retained mapping with the new package pinout",
  "fixed_by": [
    "entry/library.assign_terminals"
  ]
}
---
