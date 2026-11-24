---
{
  "covers": {
    "entry/project.open": "7317cdb697af20e5",
    "entry/project.save": "d6b27c72606417bf"
  },
  "id": "guide/save-recover",
  "keywords": [
    "save",
    "open",
    "recent",
    "crash",
    "recover",
    "lost work",
    "unsaved",
    "backup",
    "restore"
  ],
  "kind": "guide",
  "related": [
    "panel/files",
    "guide/undo"
  ],
  "since": "0.1.0",
  "summary": "Save the project, reopen it, and restore a session after a crash.",
  "title": "Save and recover your work"
}
---

A project is one .sparksmith file. It holds the parts it uses, so it opens without a network.

- Save with {key:file.save}. The first time, choose a name and a folder.
- Save a copy with [[command/file.save-as]].
- Open a project with {key:file.open}, or a recent one from the File menu.
- If you leave with unsaved changes, Sparksmith asks first.

After a crash, start Sparksmith again. It asks “Recover an earlier session?”. Restore opens an unsaved copy, so save it. Keep for later asks again next time. Discard deletes the session. The original file is unchanged.
