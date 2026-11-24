---
{
  "covers": {
    "entry/board.edit": "61a6f30290e0b1f2",
    "entry/board.shape": "2c692f52f8c0092b"
  },
  "id": "guide/parameters",
  "keywords": [
    "parameter",
    "variable",
    "parametric",
    "expression",
    "formula",
    "width",
    "holes follow",
    "move with board"
  ],
  "kind": "guide",
  "related": [
    "guide/board-shape",
    "guide/dimensions"
  ],
  "since": "0.1.0",
  "summary": "Name a value, use it in other dimensions, and change the board in one place.",
  "title": "Use parameters for the board"
}
---

A parameter is a named value. Dimensions that use it follow when you change it.

- In [[guide/board-shape]], choose Dimension and pick an edge.
- In Value, type a name and a value, such as width = 80 mm. The parameter appears under Board parameters.
- Use the name in another dimension: width / 2, or width - 5 mm. Dimension a hole from the edges this way and it moves when the board grows.
- Change the value under Board parameters and press Enter. The outline and everything that uses it follow.
- “Add a board parameter” makes one without a dimension.

If a value is refused, nothing changes. Choose a value that keeps the shape as drawn, or change the dimension that holds it. A parameter cannot be removed while it is read.
