# Packages

`cargo xtask package [--out DIR]` makes the download for the system it runs on (default folder: `package` in Cargo's target folder). Nothing is published by it.

| System | Archive | Starts with |
| --- | --- | --- |
| Linux x86_64 | `.tar.xz` | `bin/sparksmith-desktop`, or `sh install.sh` for a menu entry and the file type |
| Windows x86_64 | `.zip` | `sparksmith-desktop.exe`; `file-type.ps1` registers the file type for this user |
| macOS arm64 | `.zip` | `Sparksmith.app`, which owns the file type |

Each holds the app, the command line, `library/` as committed (with the help search's model), a README, and the licences: `LICENSE-*`, `NOTICE`, `ATTRIBUTION.md`, `licenses/` and `THIRD-PARTY-NOTICES.txt`, which lists every crate linked into the programs and the shipped generator modules with its licence, source and texts (`xtask/src/package/notices.rs`). The licence gates (`licences`, `brand-crates`) run first.

- **Why an xtask.** It needs no tool beyond Cargo and what each system has (`tar`, `ditto`, `codesign`), so there is nothing to install, license or pin. A tar.xz rather than an AppImage: that needs a downloaded runtime and FUSE on the person's machine.
- **Build.** Profile `dist`: release, checked arithmetic, thin LTO, stripped. Paths of the building machine are remapped, so panic messages do not name them. Windows links the C runtime in. Linux programs name the loader every distribution has, and the README names the glibc they ask for, read from them.
- **Version.** `sparksmith_core::product::version()`: the version and the commit, which the xtask hands to the build (`SPARKSMITH_COMMIT`). Settings › About, the macOS About, `--version` of both programs, and the README show it. A tree with changes is marked `-dirty`.
- **Signing.** macOS: an ad-hoc signature, which arm64 needs to start at all; not notarised, so the first start has to be allowed by hand. Windows: not signed, so SmartScreen asks.
- **Updates.** None built in: a newer package replaces this one. Settings, recovery files and the cache are outside it ([development](development.md#build-and-run)). Uninstalling is deleting it (Linux: `install.sh --uninstall`; Windows: `file-type.ps1 -Remove` first).
- **Trying one.** `sh packaging/smoke.sh ARCHIVE NEW_DIR` unpacks it outside the repository, checks both versions, writes the ESP32 example's production files and compares them with the committed ones, and starts the app on that example. `.github/workflows/packages.yml`, started by hand, builds and tries all three and keeps them as artifacts of the run.
