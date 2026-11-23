---
{
  "id": "panel/assistant",
  "kind": "panel",
  "title": "Assistant",
  "summary": "Work on the open design through chat and review the changes it proposes.",
  "since": "0.1.0",
  "covers": {
    "entry/assist.workflows": "d2a6eaec77fd86c3"
  },
  "keywords": [
    "chat",
    "model",
    "AI settings",
    "workflow",
    "datasheet"
  ],
  "related": [
    "workflow/datasheet-to-component",
    "workflow/datasheet-to-body",
    "panel/settings"
  ]
}
---

Open the Assistant with {key:assistant.toggle}. Choose the model connection in Settings › AI. Describe the task and attach source material where the design depends on it. A question card lets you answer the parts that need your decision.

Review a proposed plan before asking the assistant to build. Check the schematic before continuing to the board, then review placement, routing and outstanding issues. Stop interrupts a running reply. Inspect the design and its undo history for completed changes.

When a task matches a refined workflow, the assistant can offer it in chat. [[workflow/datasheet-to-component]] produces drafts and a review report for a new component. [[workflow/datasheet-to-body]] fits a body to a placed part. You decide whether to start the offered process and whether to publish a reviewed revision. [[entry/assist.workflows]] lists available workflows.
