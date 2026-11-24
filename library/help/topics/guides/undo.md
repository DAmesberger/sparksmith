---
{
  "covers": {
    "command/edit.redo": "8da1bba70e36c1e0c7c5d6fb1ebe95c449109f4f7d2709140007e4a6eecc18f7",
    "command/edit.undo": "b8ef91e96442d51f5060981093178915e0fbb041ee6bf0964f3dced3005af77d"
  },
  "id": "guide/undo",
  "keywords": [
    "undo",
    "redo",
    "go back",
    "mistake",
    "revert",
    "step back"
  ],
  "kind": "guide",
  "related": [
    "guide/save-recover"
  ],
  "since": "0.1.0",
  "summary": "Take back one step or many, and do them again.",
  "title": "Undo a mistake"
}
---

- Press {key:edit.undo} to take back the last change. Press it again to go further back.
- Press {key:edit.redo} to do it again.
- While drawing a track, {key:route.back} removes the last piece.
- Esc ends what you have begun. A track keeps the pieces already placed; undo takes them back.

There is no single step that undoes everything. To go back to the saved file, open it again with [[command/file.open]] and choose not to save the changes.
