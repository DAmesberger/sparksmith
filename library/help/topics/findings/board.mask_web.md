---
{
  "id": "finding/board.mask_web",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.mask_web": "82a3da4c8d8d0f481a016825b5e3ab347f9cfaee6220b25bfd1f81f4f08b94e6"
  },
  "cause": "Two solder mask openings leave a strip of mask narrower than the fabrication limit.",
  "fix": "Move the parts apart or shrink the mask expansion",
  "fixed_by": [
    "entry/layout.move_parts",
    "entry/rules.set_override"
  ],
  "rule_key": "mask_web_min"
}
---
