---
{
  "id": "guide/routing",
  "kind": "guide",
  "title": "Route a connection",
  "summary": "Use the airwires and the Route preview to join two pads on the same net.",
  "since": "0.1.0",
  "covers": {
    "entry/layout.route_preview": "6ced55688f581884",
    "entry/layout.route_step": "6ced55688f581884"
  },
  "keywords": [
    "route tracks",
    "via",
    "shove",
    "walkaround",
    "copper",
    "airwires"
  ],
  "related": [
    "panel/layout",
    "panel/issues",
    "guide/fab-profiles"
  ]
}
---

In Layout, use the airwire to find the two ends of an unfinished connection. Choose Route and click the first pad. The Route card shows the layer and width. Move toward the target and watch the preview before committing each corner.

- Click a point to place a corner. Finish on a pad or track on the same net. Use {key:route.back} to undo the last routing step, including copper that moved with it.
- Change width with {key:route.width-next}. Use {key:route.flip} to switch the elbow and {key:route.posture} to change the corner style.
- Move the pointer to the intended via position and use {key:route.via-next} to continue on the next layer. A through via joins the selected net across the copper layers.
- Change routing mode with {key:route.router-mode-next}. Shove can move eligible tracks and vias. Walkaround seeks a path around obstacles. In Ignore mode the pointer follows the requested path and the engine still checks the proposed copper.
- If the head stops short, look at the blocking copper, keepout or edge. Try another elbow, layer or path. Finish the route, then inspect [[panel/issues]] for remaining connections and rule findings.

For automation, [[entry/layout.route_preview]] returns the proposed tracks, via, moved copper and any stopping reason. [[entry/layout.route_step]] commits a step. Read its endpoint and whether it reached the target before planning the next step.
