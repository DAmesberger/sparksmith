# Parts contract

`crates/parts-contract` (package `sparksmith-parts-contract`) is the wire contract between Sparksmith and an online parts library: what the client (`sparksmith_assist::registry`, the mirror in `sparksmith_io::registry_mirror`) and a server both build against, so each route, document, limit and hash is written once. The client and the test registry in `crates/assist/src/registry_tests.rs` use it; neither defines a wire shape of its own.

It is a leaf (`cargo xtask layers`): no workspace crate, no I/O, no network, no clock, no async, and it builds for wasm (`cargo xtask wasm`). It depends on `serde`, `serde_json` and `sha2` (`schemars` behind the `schema` feature, which the core turns on). The core takes its plain shared types from it (library, lineage and revision ids, `SourceRevision`, `ContentHash`, `Hidden`, the hashes and base64 of assets, the bundle and search bounds), so a server links the editor only if it searches or checks parts itself (`catalog::Index`, `validate_bundle`).

## What it fixes

| Module | Contents |
| --- | --- |
| `version` | `MAJOR` 1 (on the wire: the `/v1` path and each catalog document's `format`), `MINOR` (this crate's, never on the wire), and the compatibility rule |
| `route` | `Route`: every route's method, path and query, sign-in (`Auth`), reply and request bounds, `Caching`; `Route::parse` for a server, `Route::target` for the client |
| `documents` | the index, shards, search replies and bundle replies, generic over the core's `SourceCatalog`, `Hit` and `Bundle`; review words |
| `account` | device sign-in requests and replies, polling, the account, bearer headers |
| `status` | what each status means, the error body and its codes, `Retry-After` |
| `limits` | every bound, as data |
| `hash` | how shard, asset and record hashes are computed; base64 of assets |
| `vectors`, `conformance` | committed example documents and a server's conformance test (feature `test-fixtures`) |

| Route | Sign-in | Reply bound | Caching |
| --- | --- | --- | --- |
| `GET /v1/index` | never | 4 MiB | revalidate |
| `GET /v1/shards/{sha256}` | never | 64 MiB each, 96 MiB together | immutable |
| `GET /v1/search?q=&limit=&offset=` | never | 4 MiB | revalidate |
| `GET /v1/bundles/{lineage}/{revision}` | when signed in | 160 MiB | immutable, private |
| `POST /v1/device/code`, `POST /v1/device/token` | never | 64 KiB | never kept |
| `GET /v1/account`, `DELETE /v1/auth/token` | always | 64 KiB | never kept |

The client sends no conditional request and reads no caching header; the caching column says what a server may let be kept (`Caching::header`).

## Compatibility

Within a major a new minor may add an optional field, query parameter, route, error code or review word, and raise a bound a reader accepts. A reader ignores what it does not know; an unknown code is read by its status, an unknown review word as not reviewed. Removing or renaming anything, another type or meaning, a newly required field, another hash or a smaller bound need a new major, served beside the old one. A client refuses another `format`, naming it. Records inside a bundle are read strictly in both directions, since a dropped field would change the content hash.

A shard entry's `glb` is sent by the server; its meaning is not stated, and the client does not read it.

## Hashes

A shard is named by the SHA-256 of the bytes served, an asset by the SHA-256 of its decoded bytes (`data` is standard padded base64, and only the canonical text is accepted), a record by the SHA-256 of its canonical JSON: what `serde_json` writes for the core's `RevisionRecord` (compact, fields in declared order, `float_roundtrip`, without `preserve_order` or `arbitrary_precision`). A bundle is accepted only when it is the asked root and every record hashes to the content hash its shard lists for that revision.

## Example documents and the conformance test

`crates/parts-contract/vectors/` holds an index, a shard, a search reply, a bundle (with one asset), each account document, a rate-limit refusal and `cases.json` (the shard's, records' and assets' hashes, the request lines of every route and some that are none). The core writes them: `cargo test -p sparksmith-assist the_contract_vectors` holds them to it, and `SPARKSMITH_BLESS=1` rewrites them after a deliberate change. The contract's own tests read them without the core, as a server's would.

`conformance::run(transport, setup)` asks a server, through the caller's transport, what a v1 client depends on: the index and its shards, search pages and refusals, an unknown query parameter, bundles against their shard, the account and its refusals, sign-in codes and polls, and revocation. Each check passes, fails or is skipped for want of setup. The test registry passes every check (`the_fixture_registry_passes_the_contracts_conformance_test`).

## Taking it in a server

A server outside this repository adds it as a git dependency at a reviewed revision:

```toml
sparksmith-parts-contract = { git = "https://github.com/DAmesberger/sparksmith", rev = "<commit>" }
```

with `features = ["test-fixtures"]` under `[dev-dependencies]` for the vectors and the conformance test. Publishing it to crates.io is a later choice.
