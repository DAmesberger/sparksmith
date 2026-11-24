---
{
  "cause": "A net class limits its nets to copper layers, and a stackup edit moved or removed one of them. The class still names the layer it named.",
  "covers": {
    "finding/board.class_layer": "3049380aa0200d1770dc787e62c33c2231df99b418ba6f1f609de4d93a1f95d2"
  },
  "explanation": "A stackup edit never changes the layers of a net class. Choose the layers the class should use now. An agent confirms the list as it is by sending the class with moved_layers empty.",
  "fix": "Set the layers of the net class again, or confirm them",
  "fixed_by": [
    "entry/net_class.edit",
    "entry/layout.edit_stackup"
  ],
  "id": "finding/board.class_layer",
  "kind": "finding",
  "related": [
    "finding/board.via_class"
  ],
  "since": "0.1.0",
  "summary": "Check the layers a net class names against the stackup."
}
---
