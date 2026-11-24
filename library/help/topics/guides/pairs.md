---
{
  "covers": {
    "entry/layout.route_pair": "40b1d78f24bcca75"
  },
  "id": "guide/pairs",
  "keywords": [
    "differential pair",
    "diff pair",
    "USB",
    "HDMI",
    "D+ D-",
    "impedance",
    "length match",
    "skew",
    "tune",
    "same length"
  ],
  "kind": "guide",
  "related": [
    "guide/routing",
    "guide/fab-profiles"
  ],
  "since": "0.1.0",
  "summary": "Make a pair, route both nets together and match lengths.",
  "title": "Route a differential pair"
}
---

A differential pair is two nets that run together, such as the data lines of USB.

- On the PCB's Board card, open High speed with Edit…. (The command is [[command/pcb.high-speed]].)
- Under Suggested pairs, press Add pair only, or Add with target to give it an impedance too.
- Press Done, choose Route and click a pad of one of the two nets. Both tracks start. The Route card shows the pair; {key:route.pair} switches pair routing off and on.
- Click the corners, then finish on the pads. {key:route.via-next} takes both nets to the next layer.
- To make lengths equal, choose Tune, point at a track and click when the preview shows the meander you want. Tune is off until the board has a track.
- The Lengths part of High speed lists each net's length and the skew.
