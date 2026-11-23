//! The six tools of `docs/mcp-surface.md` §3.1 and the extension surface's
//! verbs: `find` and `describe` over the catalog, and `query`, `apply`,
//! `act` and `show`, one verb per permission class, so each tool's
//! annotations are exact. The catalog's entries are data behind them
//! ([`super::catalog`]); a verb reaches an entry only through its own kind,
//! and every call becomes one [`Call`] of the front door that runs on the
//! desk ([`super::desk`]).
//!
//! Input schemas are generated from the argument types here. `apply`'s `op`
//! is a plain string (the founder's answer to Q3): the catalog resolves it,
//! and an unknown name is refused with the nearest names.
//! Failures of the call itself (bad arguments, a name of another kind, a
//! batch that cannot be one) are tool results with `isError`, like a
//! refusal of the gate, so the model reads their code.
use super::catalog::{self, Unreachable};
use super::desk::{self, Calls};
use super::fields::{self, Sent};
use super::lenient;
use super::outcome;
use crate::front::Call;
use crate::session::discovery::{self, Call as Typed};
use crate::session::metadata::{self, EntryMeta, Kind};
use rmcp::ErrorData;
use rmcp::model::{CallToolResult, JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};
use sparksmith_core::{Class, MAX_BATCH};
use std::sync::{Arc, OnceLock};

/// The longest `find` text, in characters.
pub const MAX_TEXT: usize = 200;
/// The longest catalog name or definition a call may give, in bytes.
pub const MAX_NAME: usize = 128;
/// `find`'s hits by default, and at most.
pub const FIND_LIMIT: usize = 10;
pub const MAX_FIND_LIMIT: usize = 50;
/// The most bytes `tools/list`'s tools may take: six verbs, held near the
/// extension design's 3.5 KB target.
pub const TOOLS_BUDGET: usize = 3_584;

pub const FIND: &str = "find";
pub const DESCRIBE: &str = "describe";
pub const QUERY: &str = "query";
pub const APPLY: &str = "apply";
pub const ACT: &str = "act";
pub const SHOW: &str = "show";

/// The requests that open a project of their own rather than act on one.
const OPENS: &[&str] = &["project.new", "project.open"];

/// `find` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindArgs {
    /// What it should do, in plain words.
    pub text: String,
    #[serde(default)]
    pub kind: Option<Kind>,
    /// 1 to 50; 10 by default.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `describe` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeArgs {
    /// A catalog name.
    pub name: String,
    /// A key from more_defs or reply.more_defs, including its prefix.
    #[serde(default)]
    pub def: Option<String>,
    #[serde(default)]
    pub detail: crate::session::SchemaDetail,
}

/// `query` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QueryArgs {
    /// The handle from the last reply.
    #[serde(default)]
    pub project: Option<String>,
    pub name: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// `act` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunArgs {
    /// Target handle if several are open.
    #[serde(default)]
    pub project: Option<String>,
    pub name: String,
    #[serde(default)]
    pub params: Map<String, Value>,
    /// For an action that changes the document.
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

impl From<QueryArgs> for RunArgs {
    fn from(args: QueryArgs) -> Self {
        Self {
            project: args.project,
            name: args.name,
            params: args.params,
            expected_revision: None,
        }
    }
}

/// `apply` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplyArgs {
    /// Target handle if several are open.
    #[serde(default)]
    pub project: Option<String>,
    pub expected_revision: u64,
    /// 1 to 256, in order.
    pub operations: Vec<Step>,
    /// Report the change; keep nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// One step of `apply`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// operation name; find it with `find`, get its schema with `describe`
    pub op: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// `show` arguments.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShowArgs {
    pub name: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// The six tools, in their fixed order.
pub fn tools() -> Vec<Tool> {
    static TOOLS: OnceLock<Vec<Tool>> = OnceLock::new();
    TOOLS.get_or_init(build).clone()
}

fn build() -> Vec<Tool> {
    let read = metadata::annotations(Class::Read, false);
    vec![
        tool(
            FIND,
            "Find a catalog entry",
            "Searches the catalog by what an entry does. A hit's kind names its tool: query, operation (apply), show or action (act).",
            schema::<FindArgs>(),
            read,
        ),
        tool(
            DESCRIBE,
            "Describe a catalog entry",
            "Input schema and call. Use full for the reply schema.",
            schema::<DescribeArgs>(),
            read,
        ),
        tool(
            QUERY,
            "Read a project",
            "Runs a read: project.inspect, project.issues, library.search, project.bom and the rest.",
            schema::<QueryArgs>(),
            read,
        ),
        tool(
            APPLY,
            "Edit a project",
            "Applies operations as one undo step, all or nothing: a failure or a stale expected_revision changes nothing. Undo, redo, updates and folder imports go alone.",
            apply_schema(),
            metadata::annotations(Class::Edit, false),
        ),
        tool(
            ACT,
            "Files, exports and publishing",
            "Runs an action (files, exports, publishing); a person approves it. project.new and project.open {path} return a new project's handle, valid until the server exits.",
            schema::<RunArgs>(),
            metadata::annotations(Class::Ask, true),
        ),
        tool(
            SHOW,
            "Steer the window",
            "Changes what the window shows (view.select, view.open); never the document, no undo step. Needs the window (mcp --live).",
            schema::<ShowArgs>(),
            metadata::annotations(Class::Show, false),
        ),
    ]
}

fn tool(
    name: &'static str,
    title: &str,
    description: &'static str,
    schema: JsonObject,
    hints: Option<metadata::Annotations>,
) -> Tool {
    let mut tool = Tool::new(name, description, Arc::new(schema));
    tool.title = Some(title.to_owned());
    tool.annotations = hints.map(|hints| {
        let mut annotations = ToolAnnotations::new()
            .read_only(hints.read_only)
            .open_world(hints.open_world);
        // False is the protocol's default.
        if hints.idempotent {
            annotations = annotations.idempotent(true);
        }
        if let Some(destructive) = hints.destructive {
            annotations = annotations.destructive(destructive);
        }
        annotations
    });
    tool
}

/// The input schema of `T`, without what costs tokens and tells a model
/// nothing: the generator's root keys, `default` and `format`, `null` in an
/// optional field's type, and references to a definition, which is inlined.
fn schema<T: JsonSchema>() -> JsonObject {
    let mut value = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);
    let defs = value
        .as_object_mut()
        .and_then(|object| object.remove("$defs"))
        .unwrap_or(Value::Null);
    compact(&mut value, &defs, 0);
    if let Some(object) = value.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
        object.remove("description");
    }
    match value {
        Value::Object(object) => object,
        _ => JsonObject::new(),
    }
}

/// How deep [`compact`] inlines; the argument types nest two levels.
const MAX_DEPTH: usize = 8;

fn compact(value: &mut Value, defs: &Value, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Value::Array(items) = value {
        for item in items {
            compact(item, defs, depth.saturating_add(1));
        }
        return;
    }
    let Value::Object(object) = value else {
        return;
    };
    for key in ["default", "format", "minimum"] {
        object.remove(key);
    }
    // Any property is allowed unless a schema says otherwise.
    if object.get("additionalProperties") == Some(&Value::Bool(true)) {
        object.remove("additionalProperties");
    }
    // An optional field: what it is when given.
    if let Some(Value::Array(options)) = object.get("anyOf") {
        let given: Vec<&Value> = options.iter().filter(|o| o["type"] != "null").collect();
        if let [only] = given.as_slice() {
            let only = (*only).clone();
            object.remove("anyOf");
            if let Value::Object(inner) = only {
                for (key, inner) in inner {
                    object.entry(key).or_insert(inner);
                }
            }
        }
    }
    if let Some(Value::Array(types)) = object.get("type") {
        let given: Vec<Value> = types.iter().filter(|t| *t != "null").cloned().collect();
        if let [only] = given.as_slice() {
            object.insert("type".into(), only.clone());
        }
    }
    if let Some(name) = object.remove("$ref").and_then(|r| {
        r.as_str()
            .and_then(|r| r.strip_prefix("#/$defs/"))
            .map(str::to_owned)
    }) && let Some(Value::Object(def)) = defs.get(&name)
    {
        for (key, inner) in def {
            if key != "description" {
                object.entry(key.clone()).or_insert(inner.clone());
            }
        }
    }
    for inner in object.values_mut() {
        compact(inner, defs, depth.saturating_add(1));
    }
}

/// `apply`'s schema. A step's `op` is a plain string, not an enum of the
/// operation names, which would cost most of the tools' budget and grow with
/// every operation; [`apply`] resolves it against the catalog instead.
fn apply_schema() -> JsonObject {
    let mut schema = schema::<ApplyArgs>();
    let step = schema
        .get_mut("properties")
        .and_then(|p| p.get_mut("operations"))
        .and_then(|o| o.get_mut("items"))
        .and_then(Value::as_object_mut);
    if let Some(step) = step {
        step.remove("description");
    }
    schema
}

/// Runs the tool `name` with `arguments` on the desk. `Err` only for an
/// unknown tool or a stopped session thread.
pub async fn call(
    handle: &desk::Handle,
    name: &str,
    arguments: Option<JsonObject>,
) -> Result<CallToolResult, ErrorData> {
    let arguments = Value::Object(arguments.unwrap_or_default());
    if !NAMES.contains(&name) {
        return Err(ErrorData::invalid_params(
            format!(
                "unknown tool {name}: the tools are find, describe, query, apply, act and show"
            ),
            None,
        ));
    }
    // `find` reads only the catalog: it never waits for the desk.
    let reply = if name == FIND {
        find_tool(arguments, &|_| true)
    } else {
        let name = name.to_owned();
        handle
            .run(move |d| dispatch(d, &name, arguments).unwrap_or(Value::Null))
            .await?
    };
    Ok(outcome(&reply))
}

/// The six verbs' names, in their fixed order.
pub const NAMES: [&str; 6] = [FIND, DESCRIBE, QUERY, APPLY, ACT, SHOW];

/// Runs the verb `name` with `arguments` where `desk` runs calls, and
/// answers in the transport envelope; `None` for a name that is no verb.
/// MCP runs it on its desk; the built-in agent on its own session. The
/// arguments are read leniently first ([`lenient`]): arrays and objects
/// sent as JSON text, and integers sent as digits, are read as meant.
pub fn dispatch(desk: &mut dyn Calls, name: &str, arguments: Value) -> Option<Value> {
    Some(match name {
        FIND => find_tool(arguments, &|entry| desk.reaches(entry)),
        DESCRIBE => match parse::<DescribeArgs>(name, arguments) {
            Ok(args) => describe(desk, args),
            Err(failure) => failure,
        },
        QUERY => match parse::<QueryArgs>(name, arguments) {
            Ok(args) => run(desk, Kind::Query, args.into()),
            Err(failure) => failure,
        },
        APPLY => match parse::<ApplyArgs>(name, arguments) {
            Ok(args) => apply(desk, args),
            Err(failure) => failure,
        },
        ACT => match parse::<RunArgs>(name, arguments) {
            Ok(args) => run(desk, Kind::Action, args),
            Err(failure) => failure,
        },
        SHOW => match parse::<ShowArgs>(name, arguments) {
            Ok(args) => show(desk, &args),
            Err(failure) => failure,
        },
        _ => return None,
    })
}

/// Select the default contract view for an agent or a traced MCP session.
pub fn set_default_detail(verb: &str, args: &mut JsonObject, detail: crate::session::SchemaDetail) {
    if detail == crate::session::SchemaDetail::Input {
        return;
    }
    let fields = match verb {
        DESCRIBE => Some(args),
        QUERY if args.get("name").and_then(Value::as_str) == Some("project.schema") => args
            .entry("params")
            .or_insert_with(|| Value::Object(Default::default()))
            .as_object_mut(),
        _ => None,
    };
    if let Some(fields) = fields {
        fields.entry("detail").or_insert_with(|| json!(detail));
    }
}

fn find_tool(arguments: Value, reaches: &dyn Fn(&EntryMeta) -> bool) -> Value {
    match parse::<FindArgs>(FIND, arguments) {
        Ok(args) => find(args, reaches),
        Err(failure) => failure,
    }
}

fn parse<T: DeserializeOwned>(verb: &str, mut arguments: Value) -> Result<T, Value> {
    if let Some(schema) = tools()
        .iter()
        .find(|tool| tool.name == verb)
        .map(|tool| Value::Object((*tool.input_schema).clone()))
        && let Err(why) = lenient::read(&mut arguments, &schema, &Value::Null)
    {
        return Err(desk::failure("arguments.invalid", why));
    }
    serde_json::from_value(arguments).map_err(|error| {
        desk::failure(
            "arguments.invalid",
            format!("{error}: the tool's input schema lists its arguments"),
        )
    })
}

fn too_long(what: &str, text: &str, max: usize) -> Option<Value> {
    (text.len() > max).then(|| {
        desk::failure(
            "arguments.invalid",
            format!("{what} is {} bytes; at most {max}", text.len()),
        )
    })
}

/// `find`: the reachable entries whose name, title or summary match the
/// most words of `text`, best first and then in catalog order.
fn find(args: FindArgs, reaches: &dyn Fn(&EntryMeta) -> bool) -> Value {
    if args.text.chars().count() > MAX_TEXT {
        return desk::failure(
            "arguments.invalid",
            format!("text is longer than {MAX_TEXT} characters"),
        );
    }
    let limit = args.limit.unwrap_or(FIND_LIMIT);
    if !(1..=MAX_FIND_LIMIT).contains(&limit) {
        return desk::failure(
            "arguments.invalid",
            format!("limit is {limit}; give 1 to {MAX_FIND_LIMIT}"),
        );
    }
    let wanted = words(&args.text);
    let mut scored: Vec<(usize, &EntryMeta)> = catalog::reachable()
        .filter(|e| reaches(e))
        .filter(|e| args.kind.is_none_or(|kind| e.kind == Some(kind)))
        .filter_map(|e| {
            let score = score(&wanted, &args.text, e);
            (score > 0).then_some((score, e))
        })
        .collect();
    // Stable: equal scores keep catalog order.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let more = scored.len().saturating_sub(limit);
    let hits = scored
        .into_iter()
        .take(limit)
        .map(|(_, entry)| entry.line_card())
        .collect::<Result<Vec<_>, _>>();
    let hits = match hits {
        Ok(hits) => hits,
        Err(reason) => return desk::failure("catalog.line_too_long", reason),
    };
    let mut result = json!({"hits": hits});
    if more > 0 {
        result["more"] = json!(more);
    }
    if hits_empty(&result) {
        result["hint"] = json!("nothing matched: try other words, or fewer");
    }
    json!({"ok": true, "result": result})
}

fn hits_empty(result: &Value) -> bool {
    result["hits"].as_array().is_none_or(Vec::is_empty)
}

/// Lower-case words of letters and digits.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether two words are the same but for an ending ("via", "vias").
fn alike(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 3 && (a.starts_with(b) || b.starts_with(a)))
}

fn score(wanted: &[String], text: &str, entry: &EntryMeta) -> usize {
    if entry.name == text.trim() {
        return 1000;
    }
    let name = words(&entry.name);
    let title = words(entry.title);
    let summary = words(&entry.summary);
    wanted
        .iter()
        .map(|w| {
            let has = |list: &[String]| list.iter().any(|x| alike(x, w));
            usize::from(has(&name)) * 3 + usize::from(has(&title)) * 2 + usize::from(has(&summary))
        })
        .sum()
}

/// `describe`: `project.schema` of one reachable entry, through the gate,
/// with the call that sends it on this surface.
fn describe(desk: &mut dyn Calls, args: DescribeArgs) -> Value {
    if let Some(failure) = too_long("name", &args.name, MAX_NAME) {
        return failure;
    }
    if let Some(failure) = args
        .def
        .as_deref()
        .and_then(|d| too_long("def", d, MAX_NAME))
    {
        return failure;
    }
    if let Some(topic) = args.name.strip_prefix("guide:") {
        return desk.catalog(envelope(
            "help.topic",
            json!({"id":format!("guide/{topic}")}),
            None,
        ));
    }
    let Some(entry) = catalog::entry(&args.name) else {
        return generator(desk, &args.name)
            .unwrap_or_else(|| Unreachable::unknown(&args.name, None).reply());
    };
    let mut reply = desk.catalog(envelope(
        "project.schema",
        json!({"name": args.name, "def": args.def, "detail": args.detail}),
        None,
    ));
    if reply["ok"] != true || args.def.is_some() {
        return reply;
    }
    let Some(found) = reply["result"]["matches"]
        .as_array()
        .and_then(|m| m.iter().find(|m| m["name"] == entry.name.as_str()))
        .cloned()
    else {
        return Unreachable::unknown(&entry.name, None).reply();
    };
    let mut described = found;
    if args.detail == crate::session::SchemaDetail::Line {
        described["kind"] = json!(entry.kind);
        reply["result"] = described;
        return reply;
    }
    if let Some(object) = described.as_object_mut() {
        if args.detail == crate::session::SchemaDetail::Input {
            object.remove("reply");
        } else {
            object.remove("asks_with_path");
        }
        object.remove("send");
        object.remove("kind");
        object.insert("kind".into(), json!(entry.kind));
        object.insert("title".into(), json!(entry.title));
        object.insert("summary".into(), json!(entry.summary));
        // The call with the entry's first example as its params, else a
        // placeholder: the examples are exactly what `params` takes.
        let params = object
            .get("examples")
            .and_then(|e| e.get(0))
            .cloned()
            .unwrap_or_else(|| json!("<params>"));
        let (submitted, _) = apply_target(entry, Value::Null);
        object.insert("call_reply_entry".into(), json!(submitted));
        object.insert("call".into(), example(entry, params));
        if entry.person.is_some() {
            object.insert("person".into(), json!(entry.person));
        }
    }
    if args.detail == crate::session::SchemaDetail::Input {
        discovery::fit_input_page(&mut described);
    }
    reply["result"] = described;
    reply
}

/// `describe` of a shipped generator's name (`gull-wing`): its
/// `library.generators {generator}` entry (parameters and rules in full,
/// presets by key), with the call that makes a part with it. `None` when
/// no shipped generator has that name.
fn generator(desk: &mut dyn Calls, name: &str) -> Option<Value> {
    let mut reply = desk.catalog(envelope(
        "library.generators",
        json!({"generator": name}),
        None,
    ));
    if reply["ok"] != true {
        return None;
    }
    let found = reply["result"]["shipped"].get(0)?.clone();
    reply["result"] = json!({
        "generator": found,
        "call": {"tool": APPLY, "arguments": {
            "project": "<project>", "expected_revision": "<revision>",
            "operations": [{"op": "library.generate_part", "params": {
                "generator": name, "scope": "component", "preset": "<preset key, optional>",
                "params": "<params, optional>", "name": "<part name>"}}]}},
    });
    Some(reply)
}

/// How this surface sends `entry` with `params`.
fn example(entry: &EntryMeta, params: Value) -> Value {
    let name = entry.name.as_str();
    match entry.kind {
        Some(Kind::Operation) => json!({"tool": APPLY, "arguments": {
            "project": "<project>", "expected_revision": "<revision>",
            "operations": [{"op": name, "params": params}]}}),
        Some(Kind::Action) if OPENS.contains(&name) => {
            json!({"tool": ACT, "arguments": {"name": name, "params": params}})
        }
        Some(Kind::Action) => json!({"tool": ACT, "arguments": {
            "project": "<project>", "name": name, "params": params}}),
        Some(Kind::Show) => {
            json!({"tool": SHOW, "arguments": {"name": name, "params": params}})
        }
        _ => json!({"tool": QUERY, "arguments": {
            "project": "<project>", "name": name, "params": params}}),
    }
}

/// A call of the front door: `entry` with `params`, at `expected`.
fn envelope(entry: &str, params: Value, expected: Option<u64>) -> Call {
    Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision: expected,
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    }
}

/// `query` and `act`: one entry of `kind` as one call.
fn run(desk: &mut dyn Calls, kind: Kind, args: RunArgs) -> Value {
    if let Some(failure) = too_long("name", &args.name, MAX_NAME) {
        return failure;
    }
    let entry = match catalog::resolve(kind, &args.name) {
        Ok(entry) => entry,
        Err(unreachable) => return unreachable.reply(),
    };
    let mut params = Value::Object(args.params);
    if let Err(why) = lenient::read_params(&entry.name, &mut params) {
        return desk::failure("arguments.invalid", why);
    }
    let how = if kind == Kind::Query {
        Sent::Query
    } else {
        Sent::Act
    };
    if let Err(why) = fields::check(&entry.name, &params, how) {
        return desk::failure("arguments.invalid", why);
    }
    let mut sent = envelope(&entry.name, params, args.expected_revision);
    if entry.module == "help" {
        if args.project.is_none() {
            return desk.catalog(sent);
        }
        sent.project.clone_from(&args.project);
    }
    if OPENS.contains(&entry.name.as_str()) {
        if args.project.is_some() {
            return desk::failure(
                "project.opens_new",
                format!(
                    "{} opens another project and returns its handle: leave project out",
                    entry.name
                ),
            );
        }
        return desk.open(sent);
    }
    desk.call(args.project.as_deref(), sent)
}

const APPLY_BATCH: &str = "project.apply_all";

/// The entry the desk actually submits. Each core operation joins the
/// batch, even when it is the only step. Other entries run directly.
fn apply_target(entry: &EntryMeta, params: Value) -> (&str, Option<Value>) {
    match discovery::call(&entry.name, params) {
        Some(Typed::Operation(operation)) => (APPLY_BATCH, Some(operation)),
        _ => (&entry.name, None),
    }
}

/// `apply`: the steps as one call, `project.apply_all` for document
/// operations, or the one command-level edit (undo, an update) alone.
fn apply(desk: &mut dyn Calls, args: ApplyArgs) -> Value {
    let count = args.operations.len();
    if count == 0 || count > MAX_BATCH {
        return desk::failure(
            "arguments.invalid",
            format!("operations has {count} steps; give 1 to {MAX_BATCH}"),
        );
    }
    let mut operations = Vec::with_capacity(count);
    let mut alone = None;
    let mut submitted = APPLY_BATCH;
    for (n, step) in (1..).zip(args.operations) {
        if let Some(failure) = too_long("op", &step.op, MAX_NAME) {
            return failure;
        }
        let entry = match catalog::resolve(Kind::Operation, &step.op) {
            Ok(entry) => entry,
            Err(unreachable) => return unreachable.reply(),
        };
        let mut params = Value::Object(step.params);
        if let Err(why) = lenient::read_params(&entry.name, &mut params) {
            return desk::failure("arguments.invalid", why);
        }
        if let Err(why) = fields::check(&entry.name, &params, Sent::Apply) {
            let which = if count > 1 {
                format!(" (operation {n} of {count})")
            } else {
                String::new()
            };
            return desk::failure("arguments.invalid", format!("{why}{which}"));
        }
        let (target, operation) = apply_target(entry, params.clone());
        submitted = target;
        match operation {
            Some(operation) => operations.push(operation),
            None if count == 1 => alone = Some(params),
            _ => {
                return desk::failure(
                    "apply.alone",
                    format!(
                        "{} is not a document operation, so it cannot share a batch: send it as the only step",
                        entry.name
                    ),
                );
            }
        }
    }
    let params = alone.unwrap_or_else(|| json!({"operations":operations}));
    let mut sent = envelope(submitted, params, Some(args.expected_revision));
    sent.dry_run = args.dry_run;
    desk.call(args.project.as_deref(), sent)
}

/// `show`: one entry of the window's (`view.select`, `view.open`) as one
/// call. It names no revision. Headless there is no window, so it answers
/// `host.unavailable`; live, the window runs it.
fn show(desk: &mut dyn Calls, args: &ShowArgs) -> Value {
    if let Some(failure) = too_long("name", &args.name, MAX_NAME) {
        return failure;
    }
    let entry = match catalog::resolve(Kind::Show, &args.name) {
        Ok(entry) => entry,
        Err(unreachable) => return unreachable.reply(),
    };
    let mut params = Value::Object(args.params.clone());
    if let Err(why) = lenient::read_params(&entry.name, &mut params) {
        return desk::failure("arguments.invalid", why);
    }
    if let Err(why) = fields::check(&entry.name, &params, Sent::Show) {
        return desk::failure("arguments.invalid", why);
    }
    let sent = envelope(&entry.name, params, None);
    if !desk.is_live() {
        return crate::front::envelope(Err(crate::view::unavailable(&sent.entry)));
    }
    desk.call(None, sent)
}
