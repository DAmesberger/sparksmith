---
{
  "id": "guide/kicad-import",
  "kind": "guide",
  "title": "Open a KiCad project",
  "summary": "Import a board and its schematic as a new project and read the report.",
  "since": "0.1.0",
  "covers": {
    "command/file.import-kicad": "0"
  },
  "keywords": [
    "kicad",
    "import",
    "convert",
    "kicad_pcb",
    "kicad_pro",
    "migrate",
    "other program"
  ],
  "related": [
    "guide/fab-profiles",
    "guide/issues"
  ]
}
---

The import makes a new Sparksmith project. Your KiCad files are not changed.

- Choose [[command/file.import-kicad]] in the File menu and pick the .kicad_pro or .kicad_pcb file.
- Wait for “Importing KiCad project…”. Stop ends it with nothing imported.
- Read the KiCad import report: Exact, Approximated, Imported and listed, and Not imported. It opens again with [[command/view.import-report]].
- The board gets the fab profile a new board takes. Rules that are looser in KiCad than the profile are not taken; the report names them. Choose your fabricator's profile as in [[guide/fab-profiles]].
- Open Issues and go through what it lists, then save the project.
