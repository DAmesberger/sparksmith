---
{
  "id": "finding/footprint.override_orphaned",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/footprint.override_orphaned": "03320890e452369e8832ab9af8d8d5d59b04447e1d28a545335f8bfcc8b427c2"
  },
  "cause": "An array override names a pad the array does not generate or duplicates another override.",
  "fix": "Remove the stale override or assign it to a unique generated pad",
  "fixed_by": [
    "entry/library.edit_footprint"
  ]
}
---
