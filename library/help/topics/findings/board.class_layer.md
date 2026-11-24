---
{
  "id": "finding/board.class_layer",
  "kind": "finding",
  "since": "0.1.0",
  "covers": {
    "finding/board.class_layer": "0000000000000000000000000000000000000000000000000000000000000000"
  },
  "summary": "Check the layers a net class names against the stackup.",
  "cause": "A net class limits its nets to copper layers, and a stackup edit moved or removed one of them. The class still names the layer it named.",
  "fix": "Set the layers of the net class again, or confirm them",
  "fixed_by": [
    "entry/net_class.edit",
    "entry/layout.edit_stackup"
  ],
  "explanation": "A stackup edit never changes the layers of a net class. Choose the layers the class should use now. An agent confirms the list as it is by sending the class with moved_layers empty.",
  "related": [
    "finding/board.via_class"
  ]
}
---
