---
{
  "covers": {},
  "id": "guide/track-width",
  "keywords": [
    "track width",
    "trace width",
    "width of a track",
    "thicker track",
    "wider track",
    "thinner track",
    "minimum track width",
    "default track width",
    "power net width"
  ],
  "kind": "guide",
  "related": [
    "guide/routing",
    "guide/fab-profiles",
    "finding/board.track_width",
    "term/net-class"
  ],
  "since": "0.1.0",
  "summary": "Set how wide a track is: for the next step, for a drawn track, for a net class or for the board.",
  "title": "Set the width of a track"
}
---

While you route, the Route card's Width field holds the width of the next step. For a track already drawn, select it and change Width on its card. That changes the one track, not the rest of the net.

- A new route starts at the width of the net's class. Without a class, or with no width in it, it starts at the board's default track width. A class with an impedance target sets the width for each layer instead.
- A width you type stays for that net. Starting another net, or changing the class, sets it again.
- {key:route.width-next} steps to the next width and {key:route.width-previous} back, through the default and the widths of all net classes, none below the net's minimum.
- For every track of some nets, use a net class: press Edit… at Net classes on the Board card, and set Width for new routes and Min width as the limit. Assign a net on its card with Class, or by a name pattern. Tracks already drawn keep their width.
- For the whole board, press Rules… on the Board card. Under Copper, Min track width and Default track width show the profile's value. A value you type is a project override, and Reset takes it back.

A track narrower than its minimum is refused, with where the minimum comes from, as in “Track on GND needs at least 0.2 mm (net class Power: min track width)”. The source is the fab profile's name, project override or the net class. A class's Min width counts only where it is larger than the board's.

If the rules change after a track is drawn, the track stays and the check reports [[finding/board.track_width]]: “A track on GND: width 0.15 mm is under the minimum; needs 0.2 mm (net class Power: min track width)”. Widen the track, or change the rule that asks for it.
