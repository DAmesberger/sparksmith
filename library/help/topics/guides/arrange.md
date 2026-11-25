---
{
  "covers": {},
  "id": "guide/arrange",
  "keywords": [
    "align",
    "distribute",
    "even spacing"
  ],
  "kind": "guide",
  "related": [
    "guide/update-pcb",
    "guide/dimensions",
    "guide/keyboard"
  ],
  "since": "0.1.0",
  "summary": "Put several parts on one line, or space them evenly.",
  "title": "Align and space parts"
}
---

Align puts the same edge or the centres of the selected parts on one line. Distribute spaces them evenly.

- In the Layout view, select the parts: click the first, then hold Shift and click the others.
- Press {key:app.palette}, type align or distribute, and choose one, such as Align left edges or Distribute horizontally. The same entries are in the Edit menu on macOS.
- Align needs two parts or more, Distribute three or more.
- Tracks on the parts go with them. Undo takes it back as one step.

A part that would have to move but is locked, or is held by its dimensions, refuses the whole step and is named. Unlock it, or change its dimension first: [[guide/dimensions]].
