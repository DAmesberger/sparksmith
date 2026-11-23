---
{
  "id": "finding/tie.unowned_bridge",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/tie.unowned_bridge": "3d869db766bf61a3a09ad52b01020bd0977a4331995e16c5896aba5863f0e59d"
  },
  "cause": "A bridge does not contact pads belonging to one declared tie group.",
  "fix": "Declare the tie group whose pads the bridge joins, or remove the bridge",
  "fixed_by": [
    "entry/library.edit_footprint"
  ]
}
---
