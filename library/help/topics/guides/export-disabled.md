---
{
  "id": "guide/export-disabled",
  "kind": "guide",
  "title": "Why Export is off",
  "summary": "Production files: read the reason beside Export and clear it.",
  "since": "0.1.0",
  "covers": {
    "entry/fabrication.preflight": "0"
  },
  "keywords": [
    "cannot export",
    "can't export",
    "export grey",
    "export disabled",
    "export greyed out",
    "export button",
    "blocked export"
  ],
  "related": [
    "guide/fab-output",
    "guide/waivers",
    "guide/issues",
    "guide/fab-profiles"
  ]
}
---

In Production files, hover Export: it says why it is off. When findings are the reason, the line beside Export says so too.

- “Choose where to save the files”: under Output, press Choose… beside Save to and pick a new or empty folder, or a new ZIP file.
- “Save or name the project first: the files take its name”: save the project with [[command/file.save]].
- “Fix or waive the errors first”: open the groups under Production check. Click a finding to see it on the board and fix it there, or hover it and press Waive… if you accept it. [[guide/waivers]] explains when that is right.
- “Fix the error first; it cannot be waived”: this kind of finding has no Waive…. Read its help for the fix. Unfilled copper needs Refill, an unplaced part needs a place, an open connection needs a track.
- “Fix the design or switch to a fab profile that allows it first”: the board asks more than the profile allows. Change the design, or press Switch to … if the dialog offers a profile that fits.
- “Choose a fab profile first”: the board has no rules yet. Choose a profile as in [[guide/fab-profiles]].
- “Checking the design…” and “Writing the production files…” end by themselves.
