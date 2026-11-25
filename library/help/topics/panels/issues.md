---
{
  "covers": {
    "entry/project.issues": "c8b7fbd7ea9a1af3"
  },
  "id": "panel/issues",
  "keywords": [
    "error warning",
    "ERC",
    "DRC",
    "problems",
    "locate",
    "fix",
    "scope"
  ],
  "kind": "panel",
  "related": [
    "panel/properties",
    "panel/messages",
    "guide/fab-output"
  ],
  "since": "0.1.0",
  "summary": "Locate a design finding, see why it was reported, and decide what to change.",
  "title": "Issues"
}
---

Open Issues with {key:view.issues} or select the Issues bar below Properties. Choose the workspace, all areas, or the current selection. Use the severity filter to narrow the list. A collapsed group still counts toward the total.

Select a row to locate the affected objects. Expand it to see the objects and suggested fix. Open its help for the cause and any applicable rule source. If the row offers Fix, it opens the corresponding command or its input dialog.

For a list across workspaces, use [[entry/project.issues]]. Copy report in the card gives the raw findings for review.

```example
{"entry":"entry/project.issues","params":{"scope":{"kind":"all"}}}
```
