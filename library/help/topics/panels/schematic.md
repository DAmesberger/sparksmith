---
{
  "id": "panel/schematic",
  "kind": "panel",
  "title": "Schematic editor",
  "summary": "Place symbols and connect their electrical terminals on a sheet.",
  "since": "0.1.0",
  "covers": {
    "entry/schematic.net_places": "1cae6f693afb7b2c",
    "entry/schematic.pins": "8649c33cede45de9"
  },
  "keywords": [
    "sheet",
    "wire",
    "junction",
    "net label",
    "pin",
    "place symbol"
  ],
  "related": [
    "guide/first-board",
    "guide/libraries",
    "panel/issues"
  ]
}
---

Choose Place to search for a published component and choose its physical implementation. Place its symbol on the sheet. Select it to edit the designator, instance properties and other available decisions.

Use Wire to connect pin ends. A junction makes a deliberate branch. A net label identifies a named connection, and a power port identifies a supply net. Select a wire to inspect the net membership. Use a no-connect marker for a pin you intentionally leave unconnected.

Before continuing to PCB, check that the intended terminals share each net. [[entry/schematic.pins]] lists each requested part’s terminals and nets. [[entry/schematic.net_places]] locates occurrences of a net across the design. Use [[panel/issues]] to review electrical findings.
