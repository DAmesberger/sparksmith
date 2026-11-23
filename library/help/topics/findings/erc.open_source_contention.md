---
{
  "id": "finding/erc.open_source_contention",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/erc.open_source_contention": "03763e1883c7819531d26cb689f670cca1f541591f19ef44032a78afa08e51c7"
  },
  "cause": "An open-source terminal can drive high while another output pulls the shared net low.",
  "fix": "Separate the incompatible outputs or change their active functions",
  "fixed_by": [
    "entry/instance.select_functions",
    "entry/schematic.disconnect"
  ]
}
---
