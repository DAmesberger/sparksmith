---
{
  "id": "finding/tie.kelvin_off_anchor",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.kelvin_off_anchor": "8df17ff6134ccb7c3b2c0da8dd10ef3921a52565e447da915fddeb7853ef9400"
  },
  "cause": "A Kelvin tie's sense copper meets the other net away from its anchor land, or is not joined there.",
  "fix": "Route the sense connection from the anchor land",
  "fixed_by": [
    "entry/tie.set_anchor",
    "entry/layout.track_drag"
  ]
}
---
