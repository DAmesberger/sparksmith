---
{
  "id": "finding/tie.current_exceeded",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.current_exceeded": "78f6ca91e4a33fdf3da54faee0d6f69f35c3be510dad8fa0960e0b219c332e94"
  },
  "cause": "The current the tie must carry is above what its bridge is rated for by the supplied table.",
  "fix": "Widen the bridge, allow more temperature rise or lower the current",
  "fixed_by": [
    "entry/rules.set_tie_current",
    "entry/library.edit_footprint"
  ]
}
---
