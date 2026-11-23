---
{
  "id": "panel/issues",
  "kind": "panel",
  "title": "Issues",
  "summary": "Locate a design finding, see why it was reported, and decide what to change.",
  "since": "0.1.0",
  "covers": {
    "entry/project.issues": "8179456145cb1eba"
  },
  "keywords": [
    "error warning",
    "ERC",
    "DRC",
    "problems",
    "locate",
    "fix",
    "scope"
  ],
  "related": [
    "panel/properties",
    "panel/messages",
    "guide/fab-output"
  ]
}
---

Open Issues with {key:view.issues} or select the Issues bar below Properties. Choose the workspace, all areas, or the current selection. Use the severity filter to narrow the list. A collapsed group still counts toward the total.

Select a row to locate the affected objects. Expand it to see the objects and suggested fix. Open its help for the cause and any applicable rule source. If the row offers Fix, it opens the corresponding command or its input dialog.

For a list across workspaces, use [[entry/project.issues]]. Copy report in the card gives the raw findings for review.

```example
{"entry":"entry/project.issues","params":{"scope":{"kind":"all"}}}
```
