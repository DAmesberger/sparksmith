---
{
  "covers": {
    "command/file.import-kicad": "4ca41ceec9a75e287333df2929cf08d58a30570bb03e50782bb2f6857915c314"
  },
  "id": "guide/kicad-import",
  "keywords": [
    "kicad",
    "import",
    "convert",
    "kicad_pcb",
    "kicad_pro",
    "migrate",
    "other program"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-profiles",
    "guide/issues"
  ],
  "since": "0.1.0",
  "summary": "Import a board and its schematic as a new project and read the report.",
  "title": "Open a KiCad project"
}
---

The import makes a new Sparksmith project. Your KiCad files are not changed.

- Choose [[command/file.import-kicad]] in the File menu and pick the .kicad_pro or .kicad_pcb file.
- Wait for “Importing KiCad project…”. Stop ends it with nothing imported.
- Read the KiCad import report: Exact, Approximated, Imported and listed, and Not imported. It opens again with [[command/view.import-report]].
- The board gets the fab profile a new board takes. Rules that are looser in KiCad than the profile are not taken; the report names them. Choose your fabricator's profile as in [[guide/fab-profiles]].
- All copper comes in. A track or via outside the board outline stays where it is, and Issues lists it.
- A board whose KiCad file has no outline, or one that does not close, shows a provisional rectangle round everything. Draw or close the outline in Edit shape: until then the board cannot be exported, and that error cannot be waived.
- Open Issues and go through what it lists, then save the project.
