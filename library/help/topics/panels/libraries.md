---
{
  "id": "panel/libraries",
  "kind": "panel",
  "title": "Library sources",
  "summary": "Manage the folders that supply reusable components to the project.",
  "since": "0.1.0",
  "covers": {
    "entry/library.promote_preview": "9bf3709b8989b66e",
    "entry/library.search": "b8b0cb4373806d60",
    "entry/library.sources": "2c692f52f8c0092b"
  },
  "keywords": [
    "source folders",
    "rescan",
    "writable",
    "library catalog"
  ],
  "related": [
    "guide/libraries",
    "panel/browser",
    "panel/component"
  ]
}
---

Choose [[command/library.sources]] to see the project and the configured folders. Add folder registers a library folder on this computer. A disabled or unreadable source stays listed with a reason. Check its path or enable it, then refresh the catalog.

A source marked writable can accept a promoted revision. Review the promotion preview and its dependencies before saving to the shared library. Existing placements keep their exact project revisions when a folder changes.

Use [[entry/library.sources]] to list configured sources. [[entry/library.promote_preview]] shows the proposed promotion. To find a component for the current design, use Place or [[entry/library.search]].
