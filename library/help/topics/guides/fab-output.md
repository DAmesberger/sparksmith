---
{
  "id": "guide/fab-output",
  "kind": "guide",
  "title": "Prepare production files",
  "summary": "Check the board under the intended fabrication profile and export its manufacturing and assembly files.",
  "since": "0.1.0",
  "covers": {
    "entry/fabrication.preflight": "a3ec66cfe9c6655c",
    "entry/project.export_fabrication": "f0b41d7ede081315"
  },
  "keywords": [
    "manufacturing",
    "Gerber",
    "Excellon",
    "BOM",
    "pick and place",
    "export ZIP",
    "preflight"
  ],
  "related": [
    "guide/fab-profiles",
    "panel/issues",
    "panel/model-3d"
  ]
}
---

In PCB, open Production files from Output or the command palette. Choose the profile for the manufacturer and board stackup, the output groups, and a new ZIP or a new or empty folder. Check which assembly variant is active before exporting.

- Read the production check grouped by finding. Unplaced parts, open connections and stale fills can block production. Locate a finding, correct the board, refill where needed, and check again.
- A waiver records a reviewed exception. Waive only a finding you have evaluated. Add a reason so the next reviewer can see why the exception is acceptable. You can restore a waived finding.
- Review the placement origin and bottom-side convention with the assembler. The Gerber layers, job file and drills describe board manufacture. The assembly groups include the placement file, BOMs and drawings.
- Export after the blocking findings are resolved. The result lists the actual files written. Review those files with the fabricator and assembler before ordering.

[[entry/fabrication.preflight]] reads the check without writing files. Its optional profile selects the rules for the check. [[entry/project.export_fabrication]] requires the output destination and uses the chosen profile for the production snapshot. Project overrides, net classes and fitted variant carry into that snapshot.

```example
{"entry":"entry/fabrication.preflight","params":{}}
```
