---
{
  "id": "panel/files",
  "kind": "panel",
  "title": "Project files",
  "summary": "Open, save and recover the design together with its exact component revisions.",
  "since": "0.1.0",
  "covers": {
    "entry/project.open": "7317cdb697af20e5",
    "entry/project.save": "d6b27c72606417bf"
  },
  "keywords": [
    "open project",
    "save as",
    "recent files",
    "recovery",
    "offline"
  ],
  "related": [
    "guide/first-board",
    "guide/libraries",
    "panel/messages"
  ]
}
---

Use [[command/file.new]] to begin a design or [[command/file.open]] to open a project. Use {key:file.save} to write the current project. Save as lets you choose a new path. Recent projects provides a quick way back to a file on this computer.

The project contains its library snapshot. You can reopen the saved design offline, including its placed revisions. A shared library update enters through a reviewed update. Recovery snapshots can help restore work after an interrupted editing session.

For automation, [[entry/project.open]] reads a file and [[entry/project.save]] writes one. Saving is a file operation. A live session may ask the person to approve it.
