---
{
  "id": "finding/board.shape_outline",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.shape_outline": "fa251dd933a627bae78ff5f4c7d4d39741ea2c6b1183f14118f1427da6253d67"
  },
  "cause": "The selected sketch curves cannot form a usable closed board perimeter or cutout.",
  "fix": "Close the loop and check the curves at the reported break",
  "fixed_by": [
    "entry/board.edit"
  ],
  "rule_key": "board_outline_fit",
  "explanation": "Read the sketch detail first. Missing or disconnected edges need a geometry edit. If a free curve requires too many arcs for the selected outline fit, inspect the board outline fit rule."
}
---
