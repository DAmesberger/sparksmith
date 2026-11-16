---
name: sparksmith-datasheet-part
description: Create a Sparksmith component (pins and functions, symbol, package, footprint, binding, 3D model) from a part's manufacturer datasheets through the Sparksmith MCP server, with every value cited, for a person to review and publish.
---

# Create a component from datasheets

The method lives with its workflow, as data in the shipped library: [`library/assist/workflows/datasheet-to-component/method.md`](../../library/assist/workflows/datasheet-to-component/method.md). Read it and follow it.

It is one text for every agent: the Sparksmith MCP server serves it as its `datasheet_to_component` prompt, and the app's assistant runs the same workflow (`workflow.json` beside it), giving each judgement step its section of the method. Change the method there, never here; a test holds the workflow's steps and the method's procedure in the same order.
