---
{
  "id": "guide/via-types",
  "kind": "guide",
  "title": "Choose how vias are made",
  "summary": "Open, tented, or filled and capped: what each is for and what your fabricator makes.",
  "since": "0.1.0",
  "covers": {
    "entry/layout.add_via": "0"
  },
  "keywords": [
    "via",
    "filled",
    "capped",
    "via in pad",
    "tented",
    "open via",
    "annular ring",
    "drill size",
    "plugged"
  ],
  "related": [
    "guide/routing",
    "guide/fab-profiles",
    "finding/board.via_in_pad_open"
  ]
}
---

A via is a plated hole that joins layers. All vias go through the whole board today.

- Before routing, set Via fill on the Route card. For a via that exists, select it and set Fill in Properties.
- Open is the ordinary via. Tented covers it with solder mask.
- Filled and capped closes the hole and plates over it, so a via can stand in a pad that is soldered.
- The list offers what the board's fab profile makes. “(not stated)” means the fabricator does not say: ask before ordering. When only open vias are made, the card says so.
- The drill and the pad come from the net class or the rules. Change them on the Route card while routing.

“Via ring too thin” and “too small a drill” compare the via with the fab profile. Use a larger pad or drill, or another profile. Filling can cost extra: Production files lists what to write on the order.
