---
{
  "covers": {
    "entry/fabrication.preflight": "24b87f4a5d0237d7",
    "entry/project.export_fabrication": "f0b41d7ede081315"
  },
  "id": "guide/fab-output",
  "keywords": [
    "manufacturing",
    "Gerber",
    "Excellon",
    "BOM",
    "pick and place",
    "export ZIP",
    "preflight"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-profiles",
    "panel/issues",
    "panel/model-3d"
  ],
  "since": "0.1.0",
  "summary": "Check the board under the intended fabrication profile and export its manufacturing and assembly files.",
  "title": "Prepare production files"
}
---

In PCB, open Production files from Output or the command palette. Choose the profile for the manufacturer and board stackup, the output groups, and a new ZIP or a new or empty folder. A check under Standard, the profile a new board starts with, says to choose your fabricator's before ordering: do that in the board's Rules. Check which assembly variant is active before exporting.

- Read the production check grouped by finding. Unplaced parts, open connections and stale fills can block production. Locate a finding, correct the board, refill where needed, and check again.
- A waiver records a reviewed exception. Waive only a finding you have evaluated. Add a reason so the next reviewer can see why the exception is acceptable. A waiver accepts the value the finding has: it holds while that stays the same or gets better, and ends when the value or its limit gets worse, saying what was accepted and what it is now. A waiver saved without its value counts for nothing until you waive again. You can restore a waived finding. A finding that breaks the fab profile's own limit cannot be waived: change the design, or switch to a profile that allows it ([[entry/fabrication.profile_suggestions]] lists them). Neither can a short (copper of two nets that touches) nor a finding whose files would not be the design: fills that are stale or never filled (refill them), an outline that is open, invalid or does not solve (finish it in Board shape), a part without a footprint (choose one) or one that does not resolve.
- Review the placement origin and bottom-side convention with the assembler. The Gerber layers, job file and drills describe board manufacture. The assembly groups include the placement file, BOMs and drawings.
- When a via is filled, or a published charge falls on the board, the set has order notes: what to choose and write on the order. Copy them into the order form.
- Export after the blocking findings are resolved. Before anything is released, the files are read back and compared with the design. A difference refuses the whole set, names the file, the place and both values, and writes nothing; it cannot be waived, so report it with the project. Stop ends an export with nothing written. The result lists the actual files written. Review those files with the fabricator and assembler before ordering.

[[entry/fabrication.preflight]] reads the check without writing files. Its optional profile selects the rules for the check. [[entry/project.export_fabrication]] requires the output destination and uses the chosen profile for the production snapshot. Project overrides, net classes and fitted variant carry into that snapshot.

```example
{"entry":"entry/fabrication.preflight","params":{}}
```
