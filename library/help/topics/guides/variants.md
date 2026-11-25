---
{
  "covers": {},
  "id": "guide/variants",
  "keywords": [
    "assembly variant",
    "not populated",
    "do not populate",
    "DNP"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-output",
    "command/pcb.variants"
  ],
  "since": "0.1.0",
  "summary": "Build one board in several ways: leave parts off, or fit other parts.",
  "title": "Make assembly variants"
}
---

A variant says which parts are fitted, and which part is bought for a footprint. The copper is the same in every variant.

- In PCB, press Edit… at Variants on the Board card, then Add variant, and give it a name.
- Choose it in the list. The table shows every part. Untick its cell in the In column to leave the part off.
- Choose the Active variant. The 3D view, the BOM and the assembly files follow it. Default is the board without a variant.
- A part's card has the same ticks under Assembly, for the default and for the active variant.
- Fit another part instead of the default one from the Part column. It has to fit the same lands. Issues asks for a review before its BOM line counts as resolved.

One export writes one variant: choose the Active one first, then make the files. See [[guide/fab-output]].
