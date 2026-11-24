---
{
  "covers": {
    "command/app.connect-assistants": "1116eb4f7144b081a521cfa9b34abcfadeff09bae2d12297d5e1ebc629241f3a",
    "command/assistant.toggle": "0957b49d17077d65b96bfe0a54e7b1bc061c35de7fab2fde1d38359517dc87f6",
    "entry/assist.workflows": "d2a6eaec77fd86c3"
  },
  "id": "guide/ai-setup",
  "keywords": [
    "AI",
    "assistant",
    "api key",
    "claude",
    "openai",
    "local model",
    "chat",
    "MCP",
    "codex",
    "connect"
  ],
  "kind": "guide",
  "related": [
    "panel/assistant",
    "panel/settings"
  ],
  "since": "0.1.0",
  "summary": "Connect a model for the chat, or let Claude or another assistant work on the project.",
  "title": "Set up the assistant"
}
---

There are two ways: the chat inside Sparksmith, and an assistant program of your own.

- For the chat, open Settings with {key:app.settings}, choose AI, and pick your provider under Connection.
- Paste your key into API key and type the Model as your provider lists it. The key is kept in the system keychain.
- Press Test connection: “Connected: … answered.” means it works. Press Save, then open the chat with {key:assistant.toggle}.
- For a model on your own computer, pick a local choice, check its Server address and type the Model. It needs no key.

For an assistant of your own, choose [[command/app.connect-assistants]]. Switch on “Let AI assistants edit this project”, choose what runs without asking, and press Copy command for Claude Code, or Copy for another program's MCP settings. It listens on this computer only.
