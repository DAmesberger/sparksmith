# Part files

A **part file** is an ordinary parametric Solidcraft design (`.solidcraft`). A package uses one as its 3D body by naming it and giving values for some of its parameters. Sparksmith builds the body through Solidcraft's engine and reads the marking surfaces, datums and pin 1 from construction geometry that follows the naming conventions below. The footprint and terminals stay the package's own (package form, datasheet assistant, generators). Geometry varies per package; instances vary by render-time markings only.

Generators ship their part files in their bundles ([generator plugins](generator-plugins.md)). Mechanical parts placed directly on the board are built on part files ([mechanical parts](mechanical-parts.md)). The shipped part files are in [`library/parts/`](../library/parts/README.md).

## The package's reference

- **Asset.** The part file is a Model library definition (`library/model3d.rs`) of format `solidcraft`: an exact revision whose asset pins the file's SHA-256 and whose provenance records author and licence. Snapshots, bundles, promotion and update plans carry it like any Model. A Solidcraft-format model may map no bodies (`library/model3d.rs:570`): its looks come from appearance names.
- **Package model.** `PackageModel::PartFile(Box<PartFileModel>)` (`crates/editor-core/src/library/package_model.rs:201`): `model` (the exact Model revision), `values` (by the part's own parameter name) and `derived` (what the part evaluated to when it was assigned).
- **Values** are literal integers, never expressions, so a value cannot name another parameter and hashes the same on every platform. JSON: `{"nm": 3100000}` (a length), `{"mdeg": 45000}` (an angle), `{"count": 25}`, `{"milli": 500}` (a plain number in thousandths). The commands also take `{"mm": 3.1}`, `{"deg": 45}` and `{"number": 0.5}` and round them once (`crates/automation/src/session/part_files.rs:50`). A name must be an existing user parameter of the file (not a feature input or dimension), and the unit must match the parameter's (`mm`, `deg`, none). At most 256 values; names are 1 to 64 bytes of ASCII letters, digits and `_`, not starting with a digit; each value is within ±10000 in its own unit (`package_model.rs:35-44,210`).
- **Derived data** are the union bounds, the body count, the marking surfaces, the datums the file declares and the lead bodies with their package terminals (`PartDerived`, `package_model.rs:155`). The core lays out markings from `derived.surfaces` (`library.marking_surfaces`, `surface::package_anchors`), so it never evaluates Solidcraft. Every board build evaluates the part again and refuses with `part_file.stale`, naming the package, when the body count differs or the bounds, surfaces or datums differ by more than 0.01 mm (`part_file::matches`, `crates/solidcraft-integration/src/models/part_file.rs:234`; `RECHECK_TOLERANCE_MM`, `models/imported.rs:53`; `project.rs:400`).
- **Identity of a build.** `PartKey = sha256(asset hash || JSON of the values)` (`part_file.rs:399`). Packages with the same key share one native definition named `pkgp<first 12 hex of the key>` (`part_file.rs:201`), its parameters prefixed `<stem>_`, its bodies `<stem> <name>`.

## Conventions and the check

All conventions are over generic Solidcraft features; nothing Sparksmith-specific is stored in Solidcraft.

| Convention | Rule |
|---|---|
| Single component | The features live in the root component: no subcomponents, occurrences or joints (`part_file.rs:569`). |
| Frame | The design origin is the footprint origin and z = 0 the seating plane. The lowest geometry lies at z = 0 within 0.01 mm; a body of a lead material may reach below it (a through-hole post). The origin should lie inside the xy bounds (advisory). |
| Parameters | Plain user parameter names. Any user parameter may be given a value; the rest are internal. |
| Materials | Each body's Solidcraft appearance is named after a Sparksmith material: its `MaterialId` in kebab case (`nickel-silver`, `gold`, `epoxy-mould`, ...; `sparksmith part check --materials` lists them). Case and `_`, `-` or space are not told apart (`part_file::material_of`, `part_file.rs:305`). A body with any other appearance, or none, is a blocking finding. |
| Marking planes | A construction plane named `marking-<name>` is a marking surface: its origin is the area's centre, its x axis the reading direction (u), its normal points out of the part. Its size is the length parameters `marking_<name>_u` and `marking_<name>_v` (`-` in `<name>` becomes `_`). The anchor is `<name>` with `-` turned into `_`. `marking-top` comes first and is the default area (`part_file.rs:824-829`). |
| Marking cylinders | A construction axis named `marking-<name>` is a cylindrical surface: radius `marking_<name>_r`, length `marking_<name>_l` from `marking_<name>_from` (0 when absent) past the axis origin. Angle 0 faces +x (+y for an axis along x); u runs round an upright axis and along any other (`marking_cylinder`, `part_file.rs:1236`). At most 16 surfaces. |
| Pin 1 | A construction point named `pin1` is reported by the check. Other named points are listed. |
| Package datums | A construction plane `opening` or `opening-<suffix>` declares the datum `opening` or `opening_<suffix>` ([board interface](architecture/board-interface.md#how-a-connectors-opening-is-defined)): a rectangle centred on the plane's origin, `<datum>_u` along its x axis and `<datum>_v` across, corners rounded by `<datum>_r` when given; with `<datum>_depth` a volume swept along the normal, which points out of the part along the mating plug's way in; `<datum>_clearance` records a recommended clearance. A construction axis `axis` or `axis-<suffix>` declares an axis datum, its diameter `<datum>_d` when given. Every declared datum is marked external. Setting the package's model copies the datums into the package and keeps a datum of the same name that a person authored or edited (`package_model.rs:600-610`, `package.rs:105`). |
| Roles | A body's role follows its material (`part_file::role_of`, `part_file.rs:345`): a lead finish (`gold`, `tin-matte`, `tin-bright`, `solder-sac`, `nickel`, `copper-alloy`, `phosphor-bronze`, `brass`) is a termination, drawn as a lead; clear and diffuse silicone is a lens; a resistor overcoat is other; marking inks and laser-marked epoxy or metal are markings; anything else is the body. Display only. |
| Lead bodies | The file names no terminal. The host binds each termination body to the package terminal whose lead contact lies under it (within 10 um in x and y) and stores the binding in `derived.leads` (`part_file::lead_bodies`, `part_file.rs:991`). |

**The check** evaluates the file at given values and reports findings. These block assignment and build unless marked advisory:

- `part_file.read`: unreadable, empty, larger than 64 MiB or an unsupported Solidcraft format (a refusal);
- `part_file.value`: a value names an unknown or non-user parameter, or its unit does not match (a refusal);
- `part_file.structure`: subcomponents, occurrences or joints;
- `part_file.feature`: a feature fails at the values (the first 16 are named);
- `part_file.empty`, `part_file.bodies`: no bodies, or more than 1024;
- `part_file.seating`: the lowest geometry is not at z = 0;
- `part_file.origin` (advisory): the origin is outside the xy bounds;
- `part_file.material`: a body's appearance names no Sparksmith material;
- `part_file.marking`: a marking plane without both size parameters or an axis without its radius and length, a size that is not positive, two surfaces with one name, or more than 16;
- `part_file.no_marking` (advisory): no marking surface, so markings use the envelope's top;
- `part_file.datum`: a declaration that is not a valid datum, two datums of one name, or more than 32;
- `part_file.import` (advisory): the file contains an import feature, so its source licence must be recorded on the Model;
- `part_file.slow` (advisory): the cold evaluation took more than 100 ms (a target).

The core adds `part_file.model` (a model that is missing or not of format `solidcraft`) and `part_file.values` (malformed values or derived data); the board build adds `part_file.stale` (above) and `part_file.insert` (a name already taken in the board design).

## Build and cache (`sparksmith_solidcraft::models::part_file`)

- **`evaluate(bytes, values)`** reads the bytes with Solidcraft's reader, applies the values as literal parameter expressions, evaluates once and reads the bodies, appearances, construction geometry and parameters. It returns an `EvaluatedPart` (key, parameters, bodies, surfaces, pin 1, datums, bounds, findings, evaluation time); `derived(part)` gives the stored form.
- **Cache.** A process-wide LRU of 256 evaluated parts keyed by `PartKey` (`CACHE_ENTRIES`, `part_file.rs:80`). Equal packages, the Component 3D preview, the check and every board build share it.
- **`insert`** adds the part's history to the board design as component `<stem>`: every root feature is cloned with complete name maps (feature ids, feature and body names `<stem> <name>`, every parameter `<stem>_<name>`), user parameters are copied as expressions and the design is evaluated once (`insert_design`, `part_file.rs:1078`). It refuses a name collision or a feature that fails. It runs inside the board build, which is discarded whole on any failure, so a refusal leaves no partial board. Looks are assigned from the material table, so the saved `.solidcraft` board stays parametric and opens in plain Solidcraft. It uses only the Solidcraft engine's public API.
- **Board.** `project::build` defines each distinct `PartKey` once (`define_part`, `project.rs:386`) and places occurrences as for generated models.

## Commands

| Kind | ID | What it does |
|---|---|---|
| Core | `library.edit_package` with `SetModel(PackageModel::PartFile(..))` | A package draft uses a part file: shape-checked (`part_file.values`), one undo step. |
| Core | `library.import_model` with `assign.part` (`values`, `derived`) | The Model draft and the package's part-file model in one step (`model3d.rs:714`). |
| Automation | `library.use_part_file` | `{ package, path or builtin, values?, name?, licence?, redistribution?, expected_revision }`. Reads the file (bounded) and checks it, refusing blocking findings; applies `library.import_model` with `assign.part`, or `SetModel` when the file is already a Model in the project (`part_files.rs:395`). A file given by `path` needs its licence. |
| Automation | `library.set_part_values` | `{ package, values, expected_revision }`: evaluates at the new values and applies `SetModel`. |
| Automation query | `library.check_part_file` | `{ path or builtin, values? }`: parameters, bodies, materials, surfaces, pin 1, bounds, findings, timing. |
| Automation query | `library.part_files` | The shipped part files and the project's part-file Models. |
| CLI | `sparksmith part check FILE or --builtin NAME [--set NAME=VALUE[mm or deg]]... [--json] [--materials]` | The report; exit status 1 on a blocking finding. `part list` lists the shipped part files (`apps/sparksmith-cli/src/part.rs:9`). |
| MCP and assistant | `library_check_part_file`, `library_use_part_file`, `library_set_part_values`, `library_part_files` | The automation requests as tools. |

The desktop's part-file chooser and the Model section's **Part file...** use the same operation builders (`use_part_file_operation`, `set_part_values_operation`); see [mechanical parts](mechanical-parts.md#desktop).

The shipped part files are original work (MIT OR Apache-2.0), authored through Solidcraft's commands by `cargo test -p sparksmith-solidcraft --test author_parts -- --ignored` (`crates/solidcraft-integration/tests/author_parts.rs`).
