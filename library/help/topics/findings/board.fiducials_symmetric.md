---
{
  "id": "finding/board.fiducials_symmetric",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.fiducials_symmetric": "fd5814d2c64c1c6b3cfe139890a470de3a584f0f2f9cd524ba9f55a1bdea5435"
  },
  "cause": "Turning the board through half a turn leaves the marks in the same positions, so the pattern cannot distinguish that orientation.",
  "fix": "Move one fiducial so the set is not symmetric",
  "fixed_by": [
    "entry/layout.move_parts"
  ],
  "rule_key": "fiducials_asymmetric"
}
---
