# Agent bench

The developer bench runs the ordinary chat loop and six verbs on original generic projects. Each arm gets a fresh conversation, session and private output directory. Choose a directory that does not exist. Its parent must exist.

```sh
cargo xtask agent-bench scripted board-build target/build-a input
cargo xtask agent-bench replay board-build target/build-b target/build-a/trace.jsonl input
cargo xtask agent-bench scripted two-header target/change-a full
```

Each run writes `trace.jsonl` and `revised.sparksmith`. The final JSON line reports the reopened project's verification result, document revision, saved file digest and measurements. A verifier failure also makes the command exit unsuccessfully. In the two-header task the model must save its changes. In the board-build task the bench materializes the documented generic starting recipe as `start.sparksmith` in the new run folder. It saves the board again after the turn for inspection.

The trace records neutral model replies and verb calls in order. Request bytes count the UTF-8 tool definitions, system text and serialized messages sent to the model, the same convention used by the board-chat regression. In-process call bytes count the request and response envelopes. Model-visible reply bytes count the actual tool-result text in the chat history. A malformed call refused in the chat has zero transport bytes. A retry repeats a previously refused call with the same arguments.

Provider usage includes cached and uncached input and output. A null usage field means provider counts were unavailable. Estimated prompt totals come from chat's request events. Estimated output counts include reasoning, answer text and tool arguments. Offline replay reads the same format as the product recorder, including its earlier untagged neutral replies. A recording must match the task version and fixture digest.

CI verifies the scripted tasks and their byte and token ceilings. For the engineering change, the save action exposes the output path once in a tool result and once in the following request. Its gate accounts for the path's encoded length at each layer. The trace always contains the measured totals.


## Record initial ceilings

Run `cargo xtask agent-bench measure NEW_DIR` in the integration checkout. This runs the scripted scenarios against the input view and writes each verified run's proposed `ceilings.json` beside its trace and saved file. Inspect the traces and copy the proposals into the corresponding `tests/agent-tasks/NAME/ceilings.json` files in the commit recording those measurements. A missing ceiling fails the CI gate. Measurement never edits tracked ceilings.


## Compare a DRC repair

Run `cargo xtask agent-bench compare-drc NEW_DIR` to compare two fresh scripted repairs of the same generic board. The `delta` arm saves after its edit reports one dangling via resolved. The `query` arm adds an all-scope issues read before saving. Both arms reopen their files, identify the original via and its net, and check that the saved document differs only by removal of that via. The comparison requires matching file digests and exactly one extra query and model request in the control. Read each arm's final trace row for its measured bytes and tokens.
