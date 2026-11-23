# The shipped library

Sparksmith holds no rules, profiles or library data in its code. Everything that ships with the program is data in one folder, `library/` in the source tree, loaded at run time through `sparksmith_io::shipped::ShippedLibrary`.

## Finding it

The folder is the first of these that holds `sparksmith-shipped.json`:

1. `SPARKSMITH_LIBRARY_DIR`. If it is set but names another folder, Sparksmith refuses rather than falling back.
2. The macOS app bundle: `Contents/Resources/library`.
3. `<prefix>/share/sparksmith/library`, next to `<prefix>/bin`.
4. `library` next to the executable.
5. A `library` folder in up to eight of the executable's parent folders. Builds and tests inside the source tree find the repository's `library/` this way.

If none is found, the message lists every folder searched and names the variable.

## Layout

| Shelf | Folder | Holds |
|---|---|---|
| Profiles | `profiles/` | Fab profiles, one JSON each, with sources |
| Stackups | `stackups/` | Layer stackups and their materials |
| Materials | `materials/` | Body, board, finish and mask materials |
| Standards | `standards/` | Published standards and conventions: preferred number series, marking codes, schematic symbol drawings (`symbols/`), and differential pair names and sourced PCB impedance targets (`differential-pairs.json`) |
| Standard fields | `standard-fields/` | Standard quantity fields, as a bundle `library.import` takes, and what each is for |
| Catalog | `catalog/` | The part catalog's search vocabulary: the generic-part families, their patterns and defaults; the part categories by function and the designator prefixes that file a part under them (`categories.json`) |
| Assist | `assist/` | Assistant run profiles and prompts; workflows, `workflows/<id>/` ([workflow format](workflow-format.md)) |
| Parts | `parts/` | Part files (`.solidcraft`), each with a sidecar; a shipped component (the USB-C receptacle) also with its revision records, `NAME.component.json` ([shipped part files](../library/parts/README.md)) |
| Templates | `templates/` | Package templates and starter rows |
| Generators | `generators/` | Generator bundles, `<name>/`: `generator.json`, the built module `<name>.wasm` and the part files it names, `parts/*.solidcraft` ([generator plugins](generator-plugins.md)) |
| Defaults | `defaults/` | What a new project starts with: its fab profile, stackup and board size (`new-project.json`); the values Settings start with until the user sets their own, such as the impedance tolerance (`settings.json`) |
| Artwork | `artwork/` | Optional, and absent from this repository: marking artwork that artwork templates name by id (`<id>.svg`, ids `a-z`, `0-9`, `-`), such as makers' marks and certification marks, installed with the parts library that holds them ([artwork templates](generator-plugins.md)). Read at startup; a file that is missing or unreadable is left out quietly and its marking is drawn without it, the one exception to the rule below. |

## Rules

- **The core stays file-free.** `sparksmith-core` never reads the library. `sparksmith-io` loads the data and hands typed values to the core, and the core states what it needs as types or parameters.
- **No compiled-in copy.** `include_str!` and `include_bytes!` of library data are not allowed outside tests; `cargo xtask no-builtin-data` enforces it, with an allowlist for the program's own assets and the sites still being moved. Test fixtures are original and invented.
- **Users override, never edit.** A per-user file with the same id (for example in the config folder's `fab-profiles/`) wins over the shipped entry, and the shipped files stay read-only.
- **No product or vendor names in code or tool text.** Names such as a fab house or a part number live only in the data that cites them.
- **A missing or broken file is an error naming the file.** It never falls back silently to a value written in code.
- **Packaging.** The macOS bundle copies `library/` into `Contents/Resources/library`.
- **Generator modules are committed builds.** `cargo xtask generators build --ship` writes each bundle `library/generators/<name>/`, and `cargo xtask generators check` fails when a clean build of the source differs from the shipped module (whose SHA-256, recorded in the manifest, is the module's one hash) or a bundle differs from what its crate builds. Every platform ships the same bytes, so a part's key does not depend on where the program was built.
