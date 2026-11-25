---
{
  "covers": {
    "entry/schematic.connect": "b7e09bfd74ef037e",
    "entry/schematic.pins": "8649c33cede45de9"
  },
  "id": "guide/wiring",
  "keywords": [
    "wire",
    "connect pins",
    "net name",
    "junction",
    "crossing",
    "power symbol",
    "ground",
    "GND",
    "label",
    "no connect",
    "unconnected"
  ],
  "kind": "guide",
  "related": [
    "guide/first-board",
    "guide/update-pcb",
    "panel/schematic"
  ],
  "since": "0.1.0",
  "summary": "Connect pins with wires, name nets, and add power and ground.",
  "title": "Wire the schematic"
}
---

A wire joins pins into a net. Every pin on a net is connected.

- In Schematic, choose Wire.
- Click a pin, click where the wire should turn, then click the other pin. A pin, a label or another wire ends the wire. {key:wire.end} ends it in the open.
- Check the result: select the wire and read its members in Properties. A pin that lies under the wire joins it too, so keep wires off pins you do not mean.
- Wires that only cross are not connected. To join them, choose Junction and click the crossing.
- For power and ground, choose Power, pick the name under Common, such as GND or +3V3, and click a pin or wire. Ports of one name are one net. A typed name draws as the ports of that name already do, else as a supply bar; choose Style to draw it otherwise. Changing the Style of one port changes every port of its name.
- To name a net, choose Net label and click the wire. Labels of one name join on this sheet, Global labels on every sheet.
- Mark a pin you leave open with No connect: choose it, then click the pin. Click a symbol to mark all its open pins, and click a marked pin to clear it. Then Issues does not ask about them.
