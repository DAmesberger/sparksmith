---
{
  "id": "finding/board.via_class",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.via_class": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "Check vias and tracks against what the net class allows.",
  "cause": "A net has a via its class does not allow, more vias than its class allows one net, or a track on a layer outside the class's list. Routing refuses these; they come from an import or a later change of the class.",
  "fix": "Remove the via, move the track to an allowed layer, or change the net class",
  "fixed_by": [
    "entry/layout.via_delete",
    "entry/layout.track_set",
    "entry/net_class.edit"
  ],
  "related": [
    "finding/board.track_width"
  ]
}
---
