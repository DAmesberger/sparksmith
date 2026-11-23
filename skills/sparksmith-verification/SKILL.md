---
name: sparksmith-verification
description: Add or run Sparksmith tests, benchmarks and CI; verify crash fixes, geometry changes or milestone completion.
---

# Verification and evidence

Run `cargo xtask ci` for the current aggregate gate; see [development](../../docs/development.md) for prerequisites and narrower checks. The wasm gate checks compilation, not browser execution. [Milestone 0](../../docs/milestone-0.md) defines the remaining acceptance evidence; report unimplemented gates explicitly.

Choose evidence by the changed behavior:

| Behavior | Required evidence |
| --- | --- |
| Commands and transactions | Observable result; rejection leaves state unchanged; undo/redo; malformed input |
| Library changes | Mapping/identity invariants; affected routing; offline snapshot reopening |
| Electrical/geometry checks | Hand-derived expected nets, gaps, dimensions and known deliberate violations |
| Native Solidcraft integration | Save/reopen in compatible Solidcraft; change parameters and measure regeneration without Sparksmith |
| UI | Real interaction and inspected screenshots/recording; scenario coverage for the affected behavior |
| Performance | Reproducible workload and machine/settings; latency distributions and invalidation/upload counters |
| Crash/data-loss fix | Reproducer and regression; file/transaction state remains recoverable |

For whole-app UI flows, drive the headless app through its accessibility tree with the `Ax` harness (`apps/sparksmith/src/app/ax_harness.rs`, flows in `ax_tests.rs`): find widgets by role and name, click and type with real events. A widget a test cannot find by name needs an accessible label, not a pixel position.

Run focused checks during development and the implemented aggregate gate before merging or handing off executable changes. Broaden testing when a shared boundary changes. Documentation-only work needs link/skill validation, not a full CAD build. Do not add superficial tests that only mirror implementation text.

Expected values come from analytic reasoning, public specifications or independent permitted references. An implementation's own export/reimport agreement alone does not establish manufacturing correctness. Keep known limitations explicit; never adjust expected values just to match a wrong result.

Place regression tests with their area. Performance reports distinguish targets, historical upstream numbers and measurements of this checkout. Report unavailable platforms, unrun checks and remaining API gaps accurately. A milestone is complete only when its exit evidence exists, not when its command list or UI skeleton exists.

For an external-client arm, start a fresh headless `sparksmith mcp` on the run’s copy of the generic fixture. Pass `--allow-folder RUN/board --permission-mode allow-all --bench-trace RUN/wire.jsonl --bench-detail input` (or `full`) and the copied project path. Each client gets one fresh context and one folder-scoped grant. Charge its requests against the operator’s ledger, or record the client’s own counters when it supplies them. The server trace measures JSON-RPC bytes, visible result bytes and server latency. It cannot see the client’s token usage. Mark missing counters unavailable and verify the saved file independently. The paired batch command runs the two detail settings in alternating order with identical task and grant.

Run the compiled developer bench with `plan OPERATOR.json` before starting a paid batch. Keep the operator configuration outside the product. It identifies the historical prompt revision, the number of paired repeats, each model’s rate source, four per-token prices in nano-USD, model allocation and request allowances. Record the planned arms and combined maximum spend before supplying credentials. Invoke `batch OPERATOR.json BATCH_DIR SPARKSMITH_CLI` with the credential in the bench process environment. Each run owns its fresh project folder, context and trace. Reusing the same batch directory skips existing attempts and preserves every reservation. Read `report OPERATOR.json BATCH_DIR` for individual receipts, held amounts, verification outcomes and paired differences. Counts labelled estimated describe the harness call representation. Provider counters and server wire bytes have separate fields.

Use `external-probe SPARKSMITH_CLI NEW_DIR` to check real stdio traffic and both server views without a model. When driving an independent client, pass its saved project and server tap to `external-report TASK SAVED_PROJECT WIRE.jsonl [USAGE.json]`. The optional usage file supplies the client’s uncached input, cache creation, cache read and output counts as `input_tokens`, `cache_write_tokens`, `cache_read_tokens` and `output_tokens`. Omit the file if the client cannot provide those counters. Use fresh copies and alternate the client order across pairs when comparing independent clients.
