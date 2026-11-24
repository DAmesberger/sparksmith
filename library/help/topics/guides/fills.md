---
{
  "covers": {
    "entry/fill.add": "dc27fe15e5c1dc6d",
    "entry/fill.refill": "3995183b00d92f49",
    "entry/keepout.add": "ecdadc61edde2063"
  },
  "id": "guide/fills",
  "keywords": [
    "fill",
    "pour",
    "copper pour",
    "ground plane",
    "plane",
    "refill",
    "stale",
    "out of date",
    "thermal relief",
    "zone",
    "antenna",
    "keepout"
  ],
  "kind": "guide",
  "related": [
    "guide/routing",
    "guide/issues"
  ],
  "since": "0.1.0",
  "summary": "Draw a ground fill, refill it, and see why copper stays away from a pad.",
  "title": "Fill an area with copper"
}
---

A fill is an area of copper on one layer, usually connected to ground.

- In PCB Layout, choose Fill. Pick the net and the layer on the card.
- Click the corners and close the area on its first point, or press Finish.
- To cover the whole board, select the fill and set Region to Board outline.
- Copper is computed in the background. Refill computes every fill again now.
- The Fill card shows its state in the Copper row. “Out of date · Refill updates it” means the board changed since: that is a stale fill. Production needs fills that are up to date.

If the fill does not touch a pad: the pad may be on another net, or the fill's layer may not reach it. Under Connections, the card says how pads join: Solid, Thermal relief (thin spokes) or Not connected. A clearance keeps copper of other nets away. Copper islands that reach no pad of the net are removed, as Remove islands says.

An area where no copper may be, such as under an antenna, is a keepout. A part's own keepout comes with its footprint.
