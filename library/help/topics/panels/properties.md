---
{
  "id": "panel/properties",
  "kind": "panel",
  "title": "Properties",
  "summary": "Inspect the selected object or change a field in the subject card.",
  "since": "0.1.0",
  "covers": {
    "entry/view.select": "74fda399933448fc"
  },
  "keywords": [
    "inspector",
    "selection",
    "field error",
    "properties card"
  ],
  "related": [
    "panel/issues",
    "panel/browser",
    "panel/layout"
  ]
}
---

Select a symbol, placement, pad or track. The card header identifies what is selected. Select empty canvas to return to the sheet, board or current component. PCB Layout and 3D share the board selection.

Commit an edited field with Enter or by leaving it. For a coordinate, the paired values commit together. Invalid input stays marked in the field until you correct or revert it. Review dialogs have their own Apply action.

While a drawing or routing tool is armed, the tool card occupies this space. Return to Select to inspect the object again. The Issues bar remains available. [[entry/view.select]] lets automation select objects by identity or designator in the live window.
