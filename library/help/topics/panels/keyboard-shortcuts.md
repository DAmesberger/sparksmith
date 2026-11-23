---
{
  "id": "panel/keyboard-shortcuts",
  "kind": "panel",
  "title": "Keyboard shortcuts",
  "summary": "Look up the keys that apply to a workspace or an active tool.",
  "since": "0.1.0",
  "covers": {
    "term/shortcut-context": "09da476e1aae2da6f2ea43e61054f42a62624cecc2efe6acdb6f0bae8e3982da"
  },
  "keywords": [
    "rebind",
    "shortcut sheet",
    "keymap",
    "gesture",
    "commands"
  ],
  "related": [
    "panel/settings",
    "panel/palette",
    "guide/routing"
  ]
}
---

Open the shortcut list with {key:app.shortcuts}. Search for a command by name. The same key can perform different actions in different workspaces, so read the [[term/shortcut-context]] beside the result.

To change a binding, open Settings and find the command in Keyboard. Record a key and resolve any conflict the dialog reports. Reset restores that command’s default. Returning to the shortcut list shows the effective binding, including for actions available only while a tool is active.

For example, start Route and use {key:route.width-next} to cycle its width. The shortcut shown here changes when you rebind that action.
