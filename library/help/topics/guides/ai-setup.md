---
{
  "id": "guide/ai-setup",
  "kind": "guide",
  "title": "Set up the assistant",
  "summary": "Connect a model for the chat, or let an assistant you have work on the project.",
  "since": "0.1.0",
  "covers": {
    "command/app.connect-assistants": "0",
    "command/assistant.toggle": "0",
    "entry/assist.workflows": "0"
  },
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
  "related": [
    "panel/assistant",
    "panel/settings"
  ]
}
---

There are two ways: the chat inside Sparksmith, and an assistant program of your own.

- For the chat, open Settings with {key:app.settings}, choose AI, and pick your provider under Connection.
- Paste your key into API key and type the Model as your provider lists it. The key is kept in the system keychain.
- Press Test connection: “Connected: … answered.” means it works. Press Save, then open the chat with {key:assistant.toggle}.
- For a model on your own computer, pick a local choice, check its Server address and type the Model. It needs no key.

For an assistant of your own, choose [[command/app.connect-assistants]]. Switch on “Let AI assistants edit this project”, choose what runs without asking, and press Copy command for Claude Code, or Copy for another program's MCP settings. It listens on this computer only.
