//! The host's commands have independent successful reply types. A new
//! command must declare its output here before it can pass the catalog gate.
use super::{Builder, array, assistant, generators, object, required};
use crate::session::{
    online::{OnlineAccount, SignOut as SignOutReport},
    sources::SourceInfo,
};
use serde_json::{Value, json};
use sparksmith_core::{Class, Outcome};
use sparksmith_core::{catalog, ids, interface, library, updates};
use sparksmith_solidcraft::models::part_file::{PartParameter, PartReport};

pub(super) fn command(cx: &mut Builder, key: super::super::RequestId) -> Value {
    use super::super::RequestId::*;
    let text = cx.typed::<String>();
    let texts = cx.typed::<Vec<String>>();
    let id = cx.typed::<u64>();
    let count = cx.typed::<usize>();
    let flag = cx.typed::<bool>();
    let sources = |cx: &mut Builder| cx.typed::<Vec<SourceInfo>>();
    match key {
        Apply | ImportFrom | ImportFromProject | SetMarking => cx.edit(),
        ApplyAll => {
            let mut r = cx.document();
            required(&mut r, "outcomes", cx.typed::<Vec<Outcome>>());
            r
        }
        Query => super::queries::all(cx),
        Schema => discovery(cx),
        Inspect => {
            let state = cx.document_or_summary();
            let pointer = object(
                &[("revision", id), ("at", text), ("value", Value::Bool(true))],
                &[],
            );
            json!({"oneOf":[state,pointer]})
        }
        Undo | Redo => {
            let mut r = cx.document();
            required(&mut r, "step", cx.typed::<Option<sparksmith_core::Step>>());
            r
        }
        New | Open => cx.document_or_summary(),
        Save => object(&[("revision", id), ("saved_path", text)], &[]),
        PreviewUpdate => cx.typed::<updates::UpdatePlan>(),
        ApplyUpdate => cx.document(),
        PreviewReplace => {
            let terminal = object(&[("id", id.clone()), ("name", text.clone())], &[]);
            let terminals = array(terminal);
            let suggested = object(
                &[
                    ("from", id),
                    ("name", text.clone()),
                    ("to", cx.typed::<Option<u64>>()),
                ],
                &[],
            );
            object(
                &[
                    ("plan", cx.typed::<updates::UpdatePlan>()),
                    (
                        "terminals",
                        object(
                            &[("current", terminals.clone()), ("target", terminals)],
                            &[],
                        ),
                    ),
                    ("suggested_terminals", array(suggested)),
                    ("suggestion_note", text),
                ],
                &[],
            )
        }
        ReplaceComponent => {
            let mut r = cx.edit();
            required(&mut r, "copper", cx.typed::<Vec<updates::CopperIdentity>>());
            required(&mut r, "warnings", cx.typed::<Vec<updates::PlanLine>>());
            required(&mut r, "summary", text);
            r
        }
        ExportNative => object(
            &[
                ("revision", id),
                ("exported_path", text),
                (
                    "tier",
                    cx.typed::<sparksmith_core::model::interface::Tier>(),
                ),
                ("bodies", count),
                ("ms", cx.typed::<sparksmith_solidcraft::export::Timings>()),
            ],
            &[("check", export_check(cx))],
        ),
        ExportFabrication => {
            let mut r = cx.typed::<sparksmith_io::fabrication::ExportReport>();
            required(&mut r, "revision", id);
            r
        }
        Sources | SourceRemove | SourceEnable | Rescan => object(&[("sources", sources(cx))], &[]),
        SourceAdd | SourceCreate => {
            let key = if key == SourceAdd { "added" } else { "created" };
            object(
                &[(key, cx.typed::<SourceInfo>()), ("sources", sources(cx))],
                &[],
            )
        }
        Account => {
            let mut r = cx.typed::<OnlineAccount>();
            required(&mut r, "registry", cx.typed::<Option<String>>());
            r["properties"]["note"] = text;
            r
        }
        SignOut => {
            let mut r = cx.typed::<SignOutReport>();
            required(&mut r, "registry", text);
            required(&mut r, "signed_in", json!({"const":false}));
            r
        }
        Favourite => object(&[("favourites", cx.typed::<Vec<catalog::EntryKey>>())], &[]),
        Bundle => cx.typed::<library::Bundle>(),
        PlaceFrom => {
            let mut r = cx.edit();
            r["properties"]["warnings"] = texts;
            r
        }
        Publish => {
            let mut r = cx.edit();
            required(&mut r, "root", cx.typed::<library::SourceRevision>());
            required(
                &mut r,
                "written",
                object(
                    &[
                        ("records", texts.clone()),
                        ("unchanged", texts),
                        ("assets", cx.typed::<Vec<ids::ContentHash>>()),
                    ],
                    &[],
                ),
            );
            r
        }
        ImportSvg => {
            let mut r = cx.edit();
            required(
                &mut r,
                "import",
                object(&[("size", cx.typed::<[f64; 2]>()), ("notes", texts)], &[]),
            );
            r
        }
        OpenPreview => cx.typed::<catalog::ItemReport>(),
        PinoutSchema => object(
            &[
                ("format", id),
                ("schema", schema_object()),
                ("rule_api", text),
            ],
            &[],
        ),
        ReviewCheck => {
            let draft = object(
                &[
                    ("id", cx.typed::<ids::DefinitionId>()),
                    ("kind", text.clone()),
                    ("name", text.clone()),
                    ("blocking", texts.clone()),
                    ("warnings", texts.clone()),
                ],
                &[],
            );
            object(
                &[
                    ("accepted", flag),
                    ("problems", texts),
                    ("markdown", text.clone()),
                    ("next", text),
                ],
                &[
                    (
                        "pins",
                        object(&[("unreviewed", count.clone()), ("uncertain", count)], &[]),
                    ),
                    ("drafts", array(draft)),
                    ("uncited", cx.typed::<Option<String>>()),
                ],
            )
        }
        FirmwarePreview => cx.typed::<sparksmith_firmware::Emitted>(),
        FirmwareExport => object(
            &[
                ("path", text.clone()),
                ("file_name", text),
                ("bytes", count),
            ],
            &[],
        ),
        SchematicPdf => object(&[("revision", id), ("path", text), ("pages", count)], &[]),
        InspectModelFile => cx.typed::<sparksmith_solidcraft::models::inspect::ModelInspection>(),
        MarkingSurfaces => {
            let cylinder = object(
                &[
                    ("radius_mm", cx.typed::<f64>()),
                    ("length_mm", cx.typed::<f64>()),
                    ("circumference_mm", cx.typed::<f64>()),
                    ("u_along_axis", flag),
                ],
                &[],
            );
            let row = object(
                &[
                    ("on", cx.typed::<library::marking::SurfaceRef>()),
                    ("description", text),
                    ("map", json!({"enum":["plane","cylinder"]})),
                    (
                        "size",
                        json!({"anyOf":[{"type":"null"},cx.typed::<[f64;2]>(),cylinder]}),
                    ),
                ],
                &[],
            );
            object(
                &[
                    ("package", cx.typed::<ids::DefinitionId>()),
                    ("surfaces", array(row)),
                ],
                &[],
            )
        }
        CheckPartFile => object(
            &[
                ("file", text),
                ("report", cx.typed::<PartReport>()),
                ("materials", texts),
                (
                    "cache",
                    object(&[("hits", count.clone()), ("evaluations", count)], &[]),
                ),
            ],
            &[],
        ),
        UsePartFile | SetPartValues | PlacePartFile | SetPartFileValues => {
            let mut r = cx.edit();
            required(&mut r, "part", cx.typed::<PartReport>());
            r
        }
        PartFiles => {
            let place = object(
                &[
                    ("library", cx.typed::<ids::LibraryId>()),
                    ("lineage", cx.typed::<ids::SourceLineageId>()),
                    ("revision", id),
                ],
                &[],
            );
            let builtin = object(
                &[
                    ("name", text.clone()),
                    ("title", text.clone()),
                    ("summary", text.clone()),
                    ("prefix", text.clone()),
                    (
                        "kind",
                        json!({"enum":["component","pads","mechanical","package"]}),
                    ),
                    ("parameters", cx.typed::<Vec<PartParameter>>()),
                    ("licence", text.clone()),
                ],
                &[("place", place)],
            );
            let project = object(
                &[
                    ("model", cx.typed::<ids::ModelRef>()),
                    ("name", text.clone()),
                    ("file", text.clone()),
                    ("licence", text),
                ],
                &[],
            );
            object(
                &[("builtin", array(builtin)), ("project", array(project))],
                &[],
            )
        }
        InterfacePreview => cx.typed::<interface::PublishPreview>(),
        InterfacePublish => {
            let mut r = cx.edit();
            required(
                &mut r,
                "published",
                object(
                    &[
                        ("path", text.clone()),
                        ("sidecar", text),
                        ("revision", id),
                        ("hash", cx.typed::<ids::ContentHash>()),
                        (
                            "tier",
                            cx.typed::<sparksmith_core::model::interface::Tier>(),
                        ),
                        ("datums", count),
                    ],
                    &[("check", export_check(cx))],
                ),
            );
            r
        }
        AssemblyPreview => cx.typed::<interface::assembly_link::AssemblyPreview>(),
        AssemblyFollow | AssemblyUpdate => {
            let mut r = cx.edit();
            required(
                &mut r,
                "assembly",
                cx.typed::<Option<sparksmith_core::model::assembly_link::AssemblyLink>>(),
            );
            r
        }
        DocumentText => assistant::document_text(cx),
        DocumentPage => assistant::document_page(cx),
        PinoutEdit => assistant::pinout_edit(cx),
        WorkingPreview => assistant::working_preview(cx),
        WorkingApply => assistant::working_apply(cx),
        Workflows => assistant::workflows(cx),
        StepResult => assistant::step_result(cx),
        Offer => assistant::offer(cx),
        AskPerson => assistant::ask_person(cx),
        ViewContext | ViewOpen => assistant::view_context(cx),
        ViewSelect => assistant::view_select(cx),
        TurnBegin | TurnEnd => assistant::turn_begin(cx),
        Generators => generators::generators(cx),
        BodyFit => generators::body_fit(cx),
        MakeBody => generators::make_body(cx),
        GeneratePart => generators::generate_part(cx),
    }
}

/// Each returned schema is itself a JSON Schema document (including
/// boolean schemas), at whatever depth the described type requires.
fn schema_object() -> Value {
    json!({"type":["object","boolean"]})
}

fn export_check(cx: &mut Builder) -> Value {
    let count = cx.typed::<usize>();
    let distance = cx.typed::<f64>();
    object(
        &[
            ("planned", count.clone()),
            ("built", count.clone()),
            ("max_deviation_mm", distance.clone()),
            ("max_area_mm2", distance),
            ("differs", cx.typed::<bool>()),
            ("difference_count", count),
            (
                "differences",
                cx.typed::<Vec<sparksmith_solidcraft::project::plan::Difference>>(),
            ),
            ("lines", cx.typed::<Vec<String>>()),
        ],
        &[],
    )
}

fn discovery(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    let class = cx.typed::<Class>();
    let defs = json!({"type":"object","additionalProperties":schema_object()});
    let opt_defs = [("$defs", defs), ("more_defs", cx.typed::<Vec<String>>())];
    let matched = object(
        &[
            ("kind", json!({"enum":["command","query","operation"]})),
            ("class", class.clone()),
            ("name", text.clone()),
            ("schema", schema_object()),
            ("reply", schema_object()),
        ],
        &[
            ("editor", text.clone()),
            ("send", json!({"type":"object"})),
            ("examples", array(Value::Bool(true))),
            ("asks_with_path", cx.typed::<bool>()),
            opt_defs[0].clone(),
            opt_defs[1].clone(),
        ],
    );
    let row = object(
        &[("name", text.clone()), ("summary", text.clone())],
        &[
            ("class", class.clone()),
            ("asks_with_path", cx.typed::<bool>()),
        ],
    );
    let rows = array(row);
    let index = object(
        &[
            (
                "classes",
                object(&[("operations", class.clone()), ("queries", class)], &[]),
            ),
            (
                "operations",
                json!({"type":"object","additionalProperties":rows}),
            ),
            ("queries", rows.clone()),
            ("commands", rows),
        ],
        &[],
    );
    let matches = object(&[("name", text.clone()), ("matches", array(matched))], &[]);
    let def = object(
        &[
            ("name", text.clone()),
            ("def", text),
            ("schema", schema_object()),
        ],
        &opt_defs,
    );
    json!({"oneOf":[index,matches,def]})
}
