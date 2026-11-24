---
{
  "cause": "A net has a via its class does not allow, more vias than its class allows one net, or a track on a layer outside the class's list. Routing refuses these; they come from an import or a later change of the class.",
  "covers": {
    "finding/board.via_class": "f28aa9b723a1fca91c1693603edb0472d0d99fc4c292c81a905ee3d6b2857fef"
  },
  "fix": "Remove the via, move the track to an allowed layer, or change the net class",
  "fixed_by": [
    "entry/layout.via_delete",
    "entry/layout.track_set",
    "entry/net_class.edit"
  ],
  "id": "finding/board.via_class",
  "kind": "finding",
  "related": [
    "finding/board.track_width"
  ],
  "since": "0.1.0",
  "summary": "Check vias and tracks against what the net class allows."
}
---
