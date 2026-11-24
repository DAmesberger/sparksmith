---
{
  "covers": {
    "entry/project.issues": "8179456145cb1eba"
  },
  "id": "guide/issues",
  "keywords": [
    "warnings on the board",
    "errors on the board",
    "error",
    "warning",
    "problem",
    "list of problems",
    "what is wrong",
    "DRC",
    "ERC",
    "check"
  ],
  "kind": "guide",
  "related": [
    "panel/issues",
    "guide/waivers",
    "guide/export-disabled"
  ],
  "since": "0.1.0",
  "summary": "Read a finding in Issues: what it is, where it is and what to do.",
  "title": "Why is this error there"
}
---

Issues lists everything the checks found in the schematic, the board and the parts.

- Open Issues with {key:view.issues}, or click the Issues bar at the bottom right.
- Choose the workspace, Whole project or Selection, and a kind such as Errors.
- Click a row. Sparksmith shows the objects it names.
- Open the row with the arrow at its side. It gives the cause and the fix.
- Some rows offer a button that opens the right command, such as Update PCB….
- When the fix is done, the row goes away. If you accept the finding instead, see [[guide/waivers]].

“To do” rows are unfinished work, such as a part that is not placed. Errors stop the production files.
