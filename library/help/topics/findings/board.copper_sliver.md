---
{
  "id": "finding/board.copper_sliver",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.copper_sliver": "b7d1ea8ceb86e4cafdf7e9733289e0d70086d866ccdf21eb507cb07ab69f5028"
  },
  "cause": "A narrow gap between copper features, or a narrow hatch stroke, falls below the etching rule named in the finding.",
  "fix": "Move the copper apart or together, or widen the hatch",
  "fixed_by": [
    "entry/layout.track_drag",
    "entry/fill.edit"
  ],
  "explanation": "For a bare gap in a fill, compare its width with the minimum sliver width. For thin hatch lines, compare the hatch stroke with that fill’s minimum width, which defaults to the profile’s minimum fill width."
}
---
