---
{
  "id": "finding/instance.terminal_unaccounted",
  "kind": "finding",
  "since": "1.0",
  "covers": {
    "finding/instance.terminal_unaccounted": "0f959e9da3a1586e9412c9d03b537a974d06fe02a8eb49e85016978434a6120e"
  },
  "cause": "The terminal has neither a connection nor a deliberate no-connect decision.",
  "fix": "Connect the terminal or mark it no-connect",
  "fixed_by": [
    "entry/instance.set_no_connect",
    "entry/schematic.connect"
  ]
}
---
