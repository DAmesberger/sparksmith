---
{
  "id": "finding/board.shape_outline",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.shape_outline": "fa251dd933a627bae78ff5f4c7d4d39741ea2c6b1183f14118f1427da6253d67"
  },
  "cause": "The selected curves do not produce a closed loop within the board's outline fit tolerance.",
  "fix": "Close the loop and check the curves at the reported break",
  "fixed_by": [
    "entry/board.edit"
  ],
  "rule_key": "board_outline_fit"
}
---
