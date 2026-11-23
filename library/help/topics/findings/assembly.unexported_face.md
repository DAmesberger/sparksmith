---
{
  "id": "finding/assembly.unexported_face",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/assembly.unexported_face": "578b772835f28e3fc6846176789612f5650c12c6fe5a0f34e230b17629259f23"
  },
  "cause": "A linked feature uses a board face that the published interface does not expose.",
  "fix": "Export the face, then pick it again in SolidCraft",
  "fixed_by": [
    "entry/interface.export"
  ]
}
---
