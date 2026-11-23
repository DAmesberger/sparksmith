---
{
  "id": "finding/resource.role_capacity",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/resource.role_capacity": "f595c6af5bb9a10fdc39dccd8f4323f087f841ac640a2e6b6d8a59f0685519ab"
  },
  "cause": "A role has zero capacity, so no function could ever use it.",
  "fix": "Set the role capacity to the supported number of simultaneous users",
  "fixed_by": [
    "entry/library.edit"
  ]
}
---
