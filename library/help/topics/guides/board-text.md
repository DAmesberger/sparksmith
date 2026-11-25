---
{
  "covers": {
    "entry/layout.text_add": "7c971b45d206bbe6",
    "entry/layout.text_delete": "244adef7c3868093",
    "entry/layout.text_edit": "8412c745919504d0"
  },
  "id": "guide/board-text",
  "keywords": [
    "text",
    "board text",
    "silkscreen",
    "silk",
    "legend",
    "label",
    "revision",
    "board name",
    "write on the board"
  ],
  "kind": "guide",
  "related": [
    "guide/fab-output",
    "guide/issues"
  ],
  "since": "0.1.0",
  "summary": "Write a board name, a revision or a note on the top or bottom silkscreen.",
  "title": "Put text on the silkscreen"
}
---

Text on the board's silkscreen that belongs to no part, such as the board's name and revision, is a board text.

- In PCB Layout, choose Text. On the card, type the text, set its height and the side (top or bottom silkscreen), then click where its centre goes. Each click places one.
- Click a text to select it. Its card changes the words, the height, the turn, the side and the position; Delete removes it.
- Text on the bottom reads mirrored from the top, as it reads correctly when the board is turned over.
- It prints like a part's text: in the legend Gerber files, in the 3D view and on the fabrication drawing. Text shorter than the fab profile's minimum silkscreen text height, or past the board's edge, is listed in Issues.
