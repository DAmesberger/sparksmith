---
{
  "covers": {
    "entry/project.open": "7317cdb697af20e5",
    "entry/project.save": "d60ee39b134c3acd"
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

“The project was not saved” means that another program has the file open, or the file is write-protected. The file is as it was.

- Another program has it: the message says where your version is kept. Close that program and press Try again, or press Save As….
- It is write-protected: press Save As…, or make the file writable and save again. Replace puts your version in its place; the file stays write-protected.

When you log out or the computer ends Sparksmith, unsaved work is kept for recovery. After that or after a crash, start Sparksmith again. It asks “Recover an earlier session?”. Restore opens an unsaved copy, so save it. Keep for later asks again next time. Discard deletes the session. The original file is unchanged.
