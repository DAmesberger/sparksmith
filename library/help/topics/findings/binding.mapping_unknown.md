---
{
  "id": "finding/binding.mapping_unknown",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/binding.mapping_unknown": "b2f268bbd140f116c765eb8534fb631f9de049cb96d62be2d62ea2f92c79b553"
  },
  "cause": "A mapping refers to a terminal absent from the selected package.",
  "fix": "Replace the missing terminal in the mapping",
  "fixed_by": [
    "entry/library.assign_terminals"
  ],
  "title": "Pin mapping names an unknown package terminal"
}
---
