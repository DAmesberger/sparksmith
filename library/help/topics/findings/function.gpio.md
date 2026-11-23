---
{
  "id": "finding/function.gpio",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/function.gpio": "29bb8be1bbfccb2b87596c6e634950d15c9ad2f5edd2c8ce893317c91f10d443"
  },
  "cause": "A GPIO port and index are invalid or the same GPIO identity appears on several terminals.",
  "fix": "Check the datasheet and give each GPIO terminal its own port and index",
  "fixed_by": [
    "entry/library.edit"
  ]
}
---
