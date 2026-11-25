---
{
  "covers": {},
  "id": "guide/sheets",
  "keywords": [
    "page order",
    "child sheet",
    "sheet symbol",
    "hierarchy"
  ],
  "kind": "guide",
  "related": [
    "guide/wiring",
    "guide/schematic-pdf",
    "panel/schematic"
  ],
  "since": "0.1.0",
  "summary": "Add pages to the schematic, order them, and draw a sheet inside a sheet.",
  "title": "Work with several sheets"
}
---

A schematic can have several sheets. They print in page order.

- In Schematic, choose Add sheet in the Sheets group, or Add sheet under Pages on the sheet's card. Every new sheet is a page.
- Move up and Move down under Pages change the order. The sheets are listed in the Browser under Sheets.
- To split a circuit, choose Sheet and draw a rectangle: it stands for a new child sheet. Sheet pins join it to its parent.
- Net labels join only on their own sheet. Global labels and power ports join on every sheet: [[guide/wiring]].
- Annotate numbers the designators that have no number yet (R?, R) across the sheets. A designator that ends in a number stays as it is, whatever its letters, so a J1 you named is kept. With Renumber all, the card lists the designators it would rename, and they change only when you press Rename.
