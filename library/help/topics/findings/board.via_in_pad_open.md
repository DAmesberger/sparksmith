---
{
  "cause": "The via's hole is open and lies in a paste opening, so solder can run into it during reflow.",
  "covers": {
    "finding/board.via_in_pad_open": "aa0224fc32d63af58ec125fd949af0f9dca6a57e557c8caeb2ddcfa4a5c03d6f"
  },
  "fix": "Fill and cap the via, or move it out of the pad",
  "fixed_by": [
    "entry/layout.via_set",
    "entry/layout.via_delete"
  ],
  "id": "finding/board.via_in_pad_open",
  "kind": "finding",
  "related": [
    "finding/board.drill_not_made"
  ],
  "since": "0.1.0",
  "summary": "Check for open via holes under solder paste."
}
---
