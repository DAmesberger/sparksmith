//! Workflows: the refined processes the chat finds, offers and runs, as
//! versioned data (`docs/assistant-panel.md` §2, `docs/workflow-format.md`).
//!
//! A workflow is a folder, `<id>/`, holding `workflow.json` (what it is for,
//! its inputs, steps, budget, reach and controls) and `method.md` (the
//! judgement guide its model steps read). Shipped workflows are in the
//! library's `assist/workflows/`; a person's own in
//! `<config>/assist/workflows/`, same format. The app holds no workflow
//! logic: one generic runner (`sparksmith_assist::workflow::runner`) and one
//! closed set of controls ([`Control`]) read what the file says.
//!
//! This module is the contract (W0): the format as types, its load check
//! ([`Workflow::parse`], [`Workflow::check`], [`Workflow::check_method`],
//! [`Workflow::availability`]), the controls ([`control`]), the `assist.*`
//! entries' parameters and replies ([`entries`]) and the run's events
//! ([`event`]). A file that fails the check is refused with the reason,
//! naming the field; unknown fields are refused, naming them. It sits in
//! the automation crate, below the runner, because the catalog's
//! `assist.workflows` and `assist.step_result` and the MCP prompts
//! ([`crate::mcp::prompts`]) are made from the same data.
pub mod control;
pub mod entries;
pub mod event;
pub mod load;

pub use control::{Control, ControlCard};
pub use load::{Catalog, HostCatalog, Listed, Loaded, Source};

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use sparksmith_core::Class;
use std::collections::{BTreeMap, BTreeSet};

/// The one workflow format until the first release.
pub const WORKFLOW_FORMAT: u32 = 1;
/// A workflow's description, in its folder.
pub const WORKFLOW_FILE: &str = "workflow.json";
/// The judgement guide of its model steps, in its folder.
pub const METHOD_FILE: &str = "method.md";

const MAX_ID: usize = 64;
const MAX_TITLE: usize = 60;
const MAX_SUMMARY: usize = 200;
const MAX_TEXT: usize = 400;
const MIN_INTENTS: usize = 3;
const MAX_INTENTS: usize = 6;
const MAX_INTENT: usize = 120;
const MAX_SIGNALS: usize = 8;
const MAX_NEEDS: usize = 16;
const MAX_INPUTS: usize = 24;
const MAX_OUTPUTS: usize = 16;
const MAX_FIELDS: usize = 32;
const MAX_OPTIONS: usize = 64;
const MAX_STEPS: usize = 32;
const MAX_ITEMS: usize = 16;
/// The most bytes a `text` input may allow.
pub const MAX_TEXT_INPUT: u64 = 64 * 1024;
/// The most files a `files` input may take.
pub const MAX_FILES: u64 = 16;
/// The most times a `check` sends a step back.
pub const MAX_RETRIES: u8 = 10;
/// How often a `check` sends a step back when `retries` is not given.
pub const DEFAULT_RETRIES: u8 = 3;
/// Budget ceilings: no workflow asks for more than these.
pub const MAX_REQUESTS: u32 = 1_000;
pub const MAX_OUTPUT_TOKENS: u64 = 10_000_000;
pub const MAX_TOTAL_TOKENS: u64 = 100_000_000;
pub const MAX_MINUTES: u32 = 24 * 60;

/// A workflow as stored in `workflow.json`, format 1.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    pub format: u32,
    /// Lowercase `a–z`, `0–9` and `-`; the folder's name.
    pub id: String,
    /// `major.minor`. A recording names it with the files' digest.
    pub version: String,
    /// The card's and the palette's title.
    pub title: String,
    /// The one line the chat's index and `prompts/list` show.
    pub summary: String,
    /// Three to six example requests the model matches against.
    pub intents: Vec<String>,
    /// Structured signals that offer the workflow without a model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offer_when: Vec<Signal>,
    /// The surface modules and versions it uses.
    pub needs: Vec<Need>,
    /// Its ceiling per module: a run's grant is the smaller of the chat's
    /// and this. A module not listed is out of reach. Classes are `read`,
    /// `show`, `edit` and `ask`.
    #[schemars(with = "BTreeMap<String, String>")]
    pub reach: BTreeMap<String, Class>,
    pub budget: Budget,
    /// The inputs a person (or an offer) gives, by name.
    pub inputs: BTreeMap<String, Input>,
    /// The answer shapes of its model steps, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, Output>,
    pub steps: Vec<Step>,
}

/// A signal that offers a workflow's card without a model (§3.3). Never
/// text matching.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Signal {
    /// A file of this media type was dropped or attached.
    Attachment(String),
    /// The person is in this workspace.
    Workspace(String),
    /// The person acts on an Issues row with this rule code.
    Issue(String),
}

/// A surface module and the version a workflow needs: the same major, at
/// least the minor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Need {
    pub module: String,
    pub major: u16,
    #[serde(default)]
    pub minor: u16,
}

/// What a run may use. A run gets the smaller of this and the limits in
/// Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Model requests.
    pub requests: u32,
    pub output_tokens: u64,
    /// Prompt plus output tokens; Settings' limit when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    pub minutes: u32,
}

/// What an input holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InputType {
    /// Text, at most `max` bytes.
    Text,
    /// A length in integer nanometres.
    Length,
    /// An angle in integer millidegrees.
    Angle,
    /// A finite number.
    Number,
    /// A whole number.
    Integer,
    /// One of `options` (or several, with `multiple`).
    Enum,
    /// One of the options a step's result lists (`from`).
    EnumFrom,
    /// Files of the types in `accept`, `min` to `max` of them.
    Files,
    /// An `https://` URL, fetched only through an Ask.
    Url,
    /// Pages such as `"1-5, 31, 32"`.
    PageRanges,
}

impl InputType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Length => "length",
            Self::Angle => "angle",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Enum => "enum",
            Self::EnumFrom => "enum_from",
            Self::Files => "files",
            Self::Url => "url",
            Self::PageRanges => "page_ranges",
        }
    }

    fn numeric(self) -> bool {
        matches!(
            self,
            Self::Length | Self::Angle | Self::Number | Self::Integer
        )
    }

    fn integral(self) -> bool {
        matches!(self, Self::Length | Self::Angle | Self::Integer)
    }
}

/// One input. Which of the optional fields apply depends on `type`; a
/// field that does not apply is refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(rename = "type")]
    pub ty: InputType,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    /// A run may start without it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// The control that shows it; [`Control::for_input`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Control>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Numbers: the least value. Files: the fewest files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Number>,
    /// Numbers: the greatest value. Text: the most bytes. Files: the most
    /// files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Number>,
    /// Text: several lines.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiline: bool,
    /// Files: the media types taken.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accept: Vec<String>,
    /// Enum: the options.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ChoiceOption>,
    /// Enum or enum_from: several may be chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
    /// Enum_from: where the options come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<OptionsFrom>,
    /// Page_ranges: the `files` input whose pages these are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<String>,
}

/// A titled option of an `enum` input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOption {
    pub value: String,
    pub title: String,
}

/// The options of an `enum_from` input: a list in an earlier step's result
/// and, inside each item, the value and the title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionsFrom {
    /// A pointer to the list: `/steps/<id>/…`.
    pub list: String,
    /// A pointer inside one item to its value (`/name`).
    pub value: String,
    /// A pointer inside one item to its title; the value when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The answer shape of a model step: the fields `assist.step_result`
/// checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Output {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    pub fields: BTreeMap<String, OutputField>,
    /// An answer that passes the check, shown with a refusal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<Value>,
}

/// One field of a model step's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputField {
    #[serde(rename = "type")]
    pub ty: ValueType,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// The JSON type of an answer's field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    Text,
    Integer,
    Number,
    Boolean,
    Object,
    List,
    /// Any JSON value; the next tool step checks it.
    Any,
}

impl ValueType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Object => "object",
            Self::List => "list",
            Self::Any => "any",
        }
    }

    fn admits(self, value: &Value) -> bool {
        match self {
            Self::Text => value.is_string(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Object => value.is_object(),
            Self::List => value.is_array(),
            Self::Any => true,
        }
    }
}

/// What a step does (§2.3). Five kinds and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Shows controls for some inputs and waits for the person.
    Input,
    /// One `Surface::call`, no model.
    Tool,
    /// The model works on a goal and answers with `assist.step_result`.
    Model,
    /// A person's gate.
    Review,
    /// The result card.
    Result,
}

impl StepKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Tool => "tool",
            Self::Model => "model",
            Self::Review => "review",
            Self::Result => "result",
        }
    }
}

/// How a review step holds the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Gate {
    /// Waits for Apply or Decline (through `Asker::review`).
    Confirm,
    /// Shows a card with Change and goes on.
    Notify,
}

/// One call a step names: an entry and its parameters, whose strings that
/// start with `/` are pointers (`{"literal": "/…"}` writes such a string
/// as itself).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CallSpec {
    pub call: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

/// A titled value on the result card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultItem {
    pub title: String,
    /// A pointer to the value.
    pub from: String,
}

/// A button on the result card: a `show` entry (Open in editor).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResultAction {
    pub title: String,
    pub call: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// One step. Which optional fields apply depends on `kind`; a field that
/// does not apply is refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Lowercase `a–z`, `0–9`, `-` and `_`; unique in the workflow.
    pub id: String,
    pub kind: StepKind,
    /// Its row in the run's step list.
    pub title: String,
    /// Input: the inputs it asks for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// Tool: the entry it calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call: Option<String>,
    /// Tool: its parameters, with pointers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    /// Tool: a pointer to a list; the call repeats per item (`/item`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub each: Option<String>,
    /// Model: what the step must achieve, in one or two sentences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    /// Model: its guide, `method.md#<heading>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// Model: the name of its answer's shape in `outputs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Model: the most model requests the step may use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requests: Option<u32>,
    /// Tool or model: a later tool step whose refusal sends the run back
    /// here with the refusal's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    /// With `check`: how often ([`DEFAULT_RETRIES`] when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u8>,
    /// Review: confirm or notify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<Gate>,
    /// Review: the control the card uses (`impact` to confirm, an input
    /// control to notify).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<Control>,
    /// Review, notify: a pointer to what the card shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<String>,
    /// Review, notify with `choice`: where the other options come from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<OptionsFrom>,
    /// Review, notify: the earlier model step a change sends the run back
    /// to, with the person's choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    /// Review, confirm: a pointer to the dry-run reply (the plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// Review, confirm: the call Apply makes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<CallSpec>,
    /// Result: the values the card lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ResultItem>,
    /// Result: its buttons.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ResultAction>,
}

/// A refusal of a workflow file: where, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    /// The field, as a JSON pointer into `workflow.json` (`/steps/3/call`).
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.at.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.at, self.message)
        }
    }
}

fn problem(at: impl Into<String>, message: impl Into<String>) -> Problem {
    Problem {
        at: at.into(),
        message: message.into(),
    }
}

/// A pointer a step uses: into the inputs, an earlier step's result, or
/// the item of `each`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pointer<'a> {
    Input { name: &'a str, rest: &'a str },
    Step { id: &'a str, rest: &'a str },
    Item { rest: &'a str },
}

impl<'a> Pointer<'a> {
    /// Read a pointer: `/inputs/<name>…`, `/steps/<id>…` or `/item…`.
    pub fn parse(text: &'a str) -> Result<Self, String> {
        let split = |after: &'a str| -> (&'a str, &'a str) {
            match after.find('/') {
                Some(at) => (&after[..at], &after[at..]),
                None => (after, ""),
            }
        };
        if let Some(after) = text.strip_prefix("/inputs/") {
            let (name, rest) = split(after);
            if !name.is_empty() {
                return Ok(Self::Input { name, rest });
            }
        } else if let Some(after) = text.strip_prefix("/steps/") {
            let (id, rest) = split(after);
            if !id.is_empty() {
                return Ok(Self::Step { id, rest });
            }
        } else if let Some(rest) = text.strip_prefix("/item")
            && (rest.is_empty() || rest.starts_with('/'))
        {
            return Ok(Self::Item { rest });
        }
        Err(format!(
            "{text:?} is not a pointer: use /inputs/<name>, /steps/<id> or /item, each with an optional /path"
        ))
    }
}

impl Workflow {
    /// Read and check `workflow.json`'s text; `origin` names it in errors.
    /// Unknown fields and controls are refused by name.
    pub fn parse(text: &str, origin: &str) -> Result<Self, String> {
        let workflow: Self =
            serde_json::from_str(text).map_err(|e| format!("workflow {origin}: {e}"))?;
        workflow.check().map_err(|problems| {
            let list: Vec<String> = problems.iter().map(Problem::to_string).collect();
            format!("workflow {origin}: {}", list.join("; "))
        })?;
        Ok(workflow)
    }

    /// The version as `(major, minor)`, when it reads.
    pub fn version(&self) -> Option<(u16, u16)> {
        parse_version(&self.version)
    }

    /// Every rule of format 1 that the file alone decides. Every problem
    /// is listed, each naming its field.
    pub fn check(&self) -> Result<(), Vec<Problem>> {
        let mut out = Vec::new();
        self.check_head(&mut out);
        self.check_inputs(&mut out);
        self.check_outputs(&mut out);
        self.check_steps(&mut out);
        if out.is_empty() { Ok(()) } else { Err(out) }
    }

    fn check_head(&self, out: &mut Vec<Problem>) {
        if self.format != WORKFLOW_FORMAT {
            out.push(problem(
                "/format",
                format!(
                    "format {} is not supported (this version reads format {WORKFLOW_FORMAT})",
                    self.format
                ),
            ));
        }
        if !valid_id(&self.id, false) {
            out.push(problem(
                "/id",
                format!("{:?}: use 1–{MAX_ID} characters a–z, 0–9 and '-'", self.id),
            ));
        }
        if self.version().is_none() {
            out.push(problem(
                "/version",
                format!("{:?} is not major.minor (such as \"1.0\")", self.version),
            ));
        }
        text_rule(out, "/title", &self.title, MAX_TITLE);
        text_rule(out, "/summary", &self.summary, MAX_SUMMARY);
        if !(MIN_INTENTS..=MAX_INTENTS).contains(&self.intents.len()) {
            out.push(problem(
                "/intents",
                format!(
                    "{} given; give {MIN_INTENTS} to {MAX_INTENTS} example requests",
                    self.intents.len()
                ),
            ));
        }
        for (n, intent) in self.intents.iter().enumerate() {
            text_rule(out, &format!("/intents/{n}"), intent, MAX_INTENT);
        }
        if self.offer_when.len() > MAX_SIGNALS {
            out.push(problem(
                "/offer_when",
                format!("at most {MAX_SIGNALS} signals"),
            ));
        }
        for (n, signal) in self.offer_when.iter().enumerate() {
            let at = format!("/offer_when/{n}");
            match signal {
                Signal::Attachment(media) if !valid_media_type(media) => out.push(problem(
                    at,
                    format!("{media:?} is not a media type such as application/pdf"),
                )),
                Signal::Workspace(name) | Signal::Issue(name) if !valid_id(name, true) => {
                    out.push(problem(
                        at,
                        format!("{name:?}: use 1–{MAX_ID} characters a–z, 0–9, '.', '-' or '_'"),
                    ))
                }
                _ => {}
            }
        }
        if self.needs.is_empty() || self.needs.len() > MAX_NEEDS {
            out.push(problem("/needs", format!("name 1 to {MAX_NEEDS} modules")));
        }
        let mut seen = BTreeSet::new();
        for (n, need) in self.needs.iter().enumerate() {
            if !valid_id(&need.module, false) {
                out.push(problem(
                    format!("/needs/{n}/module"),
                    format!("{:?} is not a module id", need.module),
                ));
            }
            if !seen.insert(need.module.as_str()) {
                out.push(problem(
                    format!("/needs/{n}/module"),
                    format!("{} is named twice", need.module),
                ));
            }
            if need.major == 0 {
                out.push(problem(format!("/needs/{n}/major"), "modules start at 1.0"));
            }
        }
        for (module, class) in &self.reach {
            let at = format!("/reach/{module}");
            if !valid_id(module, false) {
                out.push(problem(&at, format!("{module:?} is not a module id")));
            }
            if *class == Class::Never {
                out.push(problem(
                    at,
                    "never is out of every agent's reach; leave the module out",
                ));
            }
        }
        let budget = &self.budget;
        if !(1..=MAX_REQUESTS).contains(&budget.requests) {
            out.push(problem(
                "/budget/requests",
                format!("from 1 to {MAX_REQUESTS}"),
            ));
        }
        if !(1..=MAX_OUTPUT_TOKENS).contains(&budget.output_tokens) {
            out.push(problem(
                "/budget/output_tokens",
                format!("from 1 to {MAX_OUTPUT_TOKENS}"),
            ));
        }
        if let Some(total) = budget.total_tokens
            && !(budget.output_tokens..=MAX_TOTAL_TOKENS).contains(&total)
        {
            out.push(problem(
                "/budget/total_tokens",
                format!("from output_tokens to {MAX_TOTAL_TOKENS}"),
            ));
        }
        if !(1..=MAX_MINUTES).contains(&budget.minutes) {
            out.push(problem(
                "/budget/minutes",
                format!("from 1 to {MAX_MINUTES}"),
            ));
        }
    }

    fn check_inputs(&self, out: &mut Vec<Problem>) {
        if self.inputs.len() > MAX_INPUTS {
            out.push(problem("/inputs", format!("at most {MAX_INPUTS} inputs")));
        }
        for (name, input) in &self.inputs {
            let at = format!("/inputs/{name}");
            if !valid_id(name, true) {
                out.push(problem(
                    &at,
                    format!("{name:?}: use 1–{MAX_ID} characters a–z, 0–9, '-' or '_'"),
                ));
            }
            input.check(&at, self, out);
        }
    }

    fn check_outputs(&self, out: &mut Vec<Problem>) {
        if self.outputs.len() > MAX_OUTPUTS {
            out.push(problem(
                "/outputs",
                format!("at most {MAX_OUTPUTS} outputs"),
            ));
        }
        for (name, output) in &self.outputs {
            let at = format!("/outputs/{name}");
            if !valid_id(name, true) {
                out.push(problem(&at, format!("{name:?} is not a name")));
            }
            text_rule_optional(out, &format!("{at}/help"), &output.help, MAX_TEXT);
            if output.fields.is_empty() || output.fields.len() > MAX_FIELDS {
                out.push(problem(
                    format!("{at}/fields"),
                    format!("give 1 to {MAX_FIELDS} fields"),
                ));
            }
            for (field, spec) in &output.fields {
                if !valid_id(field, true) {
                    out.push(problem(
                        format!("{at}/fields/{field}"),
                        format!("{field:?} is not a name"),
                    ));
                }
                text_rule_optional(
                    out,
                    &format!("{at}/fields/{field}/help"),
                    &spec.help,
                    MAX_TEXT,
                );
            }
            if let Some(example) = &output.example
                && let Err(problems) = output.check_value(example)
            {
                for p in problems {
                    out.push(problem(format!("{at}/example{}", p.at), p.message));
                }
            }
            if !self
                .steps
                .iter()
                .any(|s| s.output.as_deref() == Some(name.as_str()))
            {
                out.push(problem(&at, "no model step answers with it"));
            }
        }
    }

    fn check_steps(&self, out: &mut Vec<Problem>) {
        if self.steps.is_empty() || self.steps.len() > MAX_STEPS {
            out.push(problem("/steps", format!("give 1 to {MAX_STEPS} steps")));
            return;
        }
        let mut asked: BTreeMap<&str, usize> = BTreeMap::new();
        for (n, step) in self.steps.iter().enumerate() {
            let at = format!("/steps/{n}");
            if !valid_id(&step.id, true) {
                out.push(problem(
                    format!("{at}/id"),
                    format!(
                        "{:?}: use 1–{MAX_ID} characters a–z, 0–9, '-' or '_'",
                        step.id
                    ),
                ));
            }
            if self.steps[..n].iter().any(|s| s.id == step.id) {
                out.push(problem(
                    format!("{at}/id"),
                    format!("{} is used by an earlier step", step.id),
                ));
            }
            text_rule(out, &format!("{at}/title"), &step.title, MAX_TITLE);
            self.check_fields(n, step, out);
            match step.kind {
                StepKind::Input => {
                    if step.inputs.is_empty() {
                        out.push(problem(
                            format!("{at}/inputs"),
                            "name the inputs it asks for",
                        ));
                    }
                    for (k, name) in step.inputs.iter().enumerate() {
                        if !self.inputs.contains_key(name) {
                            out.push(problem(
                                format!("{at}/inputs/{k}"),
                                format!("no input {name:?}"),
                            ));
                        } else if asked.insert(name, n).is_some() {
                            out.push(problem(
                                format!("{at}/inputs/{k}"),
                                format!("{name} is asked for by an earlier step"),
                            ));
                        }
                    }
                }
                StepKind::Tool => {
                    if let Some(call) = &step.call {
                        entry_rule(out, &format!("{at}/call"), call);
                    } else {
                        out.push(problem(format!("{at}/call"), "a tool step names its entry"));
                    }
                    if let Some(each) = &step.each {
                        self.pointer_rule(out, &format!("{at}/each"), each, n, &asked, false);
                    }
                    if let Some(params) = &step.params {
                        if !params.is_object() {
                            out.push(problem(format!("{at}/params"), "an object"));
                        }
                        self.params_rule(
                            out,
                            &format!("{at}/params"),
                            params,
                            n,
                            &asked,
                            step.each.is_some(),
                        );
                    }
                    self.check_rule(out, &at, n, step);
                }
                StepKind::Model => {
                    match &step.goal {
                        Some(goal) => text_rule(out, &format!("{at}/goal"), goal, MAX_TEXT),
                        None => out.push(problem(
                            format!("{at}/goal"),
                            "a model step states its goal",
                        )),
                    }
                    match &step.method {
                        Some(method) => {
                            if let Err(why) = method_anchor(method) {
                                out.push(problem(format!("{at}/method"), why));
                            }
                        }
                        None => out.push(problem(
                            format!("{at}/method"),
                            format!("a model step names its guide, {METHOD_FILE}#<heading>"),
                        )),
                    }
                    match &step.output {
                        Some(name) if !self.outputs.contains_key(name) => out.push(problem(
                            format!("{at}/output"),
                            format!("no output {name:?} in outputs"),
                        )),
                        Some(_) => {}
                        None => out.push(problem(
                            format!("{at}/output"),
                            "a model step names its answer's shape",
                        )),
                    }
                    if let Some(requests) = step.requests
                        && !(1..=self.budget.requests).contains(&requests)
                    {
                        out.push(problem(
                            format!("{at}/requests"),
                            "from 1 to the workflow's budget",
                        ));
                    }
                    self.check_rule(out, &at, n, step);
                }
                StepKind::Review => self.check_review(out, &at, n, step, &asked),
                StepKind::Result => {
                    if n + 1 != self.steps.len() {
                        out.push(problem(&at, "the result step is the last step"));
                    }
                    if step.items.len() > MAX_ITEMS || step.actions.len() > MAX_ITEMS {
                        out.push(problem(
                            &at,
                            format!("at most {MAX_ITEMS} items and actions"),
                        ));
                    }
                    for (k, item) in step.items.iter().enumerate() {
                        text_rule(
                            out,
                            &format!("{at}/items/{k}/title"),
                            &item.title,
                            MAX_TITLE,
                        );
                        self.pointer_rule(
                            out,
                            &format!("{at}/items/{k}/from"),
                            &item.from,
                            n,
                            &asked,
                            false,
                        );
                    }
                    for (k, action) in step.actions.iter().enumerate() {
                        text_rule(
                            out,
                            &format!("{at}/actions/{k}/title"),
                            &action.title,
                            MAX_TITLE,
                        );
                        entry_rule(out, &format!("{at}/actions/{k}/call"), &action.call);
                        self.params_rule(
                            out,
                            &format!("{at}/actions/{k}/params"),
                            &action.params,
                            n,
                            &asked,
                            false,
                        );
                    }
                }
            }
        }
        if self.steps.last().map(|s| s.kind) != Some(StepKind::Result) {
            out.push(problem("/steps", "end with a result step"));
        }
        for (name, input) in &self.inputs {
            if !asked.contains_key(name.as_str()) {
                out.push(problem(
                    format!("/inputs/{name}"),
                    "no input step asks for it",
                ));
            }
            if let Some(from) = &input.from {
                let first = asked.get(name.as_str()).copied().unwrap_or(0);
                self.pointer_rule(
                    out,
                    &format!("/inputs/{name}/from/list"),
                    &from.list,
                    first,
                    &asked,
                    false,
                );
            }
        }
    }

    /// A field set that its step's kind does not use.
    fn check_fields(&self, n: usize, step: &Step, out: &mut Vec<Problem>) {
        let kind = step.kind;
        let present: [(&str, bool, &[StepKind]); 17] = [
            ("inputs", !step.inputs.is_empty(), &[StepKind::Input]),
            ("call", step.call.is_some(), &[StepKind::Tool]),
            ("params", step.params.is_some(), &[StepKind::Tool]),
            ("each", step.each.is_some(), &[StepKind::Tool]),
            ("goal", step.goal.is_some(), &[StepKind::Model]),
            ("method", step.method.is_some(), &[StepKind::Model]),
            ("output", step.output.is_some(), &[StepKind::Model]),
            ("requests", step.requests.is_some(), &[StepKind::Model]),
            (
                "check",
                step.check.is_some(),
                &[StepKind::Tool, StepKind::Model],
            ),
            (
                "retries",
                step.retries.is_some(),
                &[StepKind::Tool, StepKind::Model],
            ),
            ("gate", step.gate.is_some(), &[StepKind::Review]),
            ("control", step.control.is_some(), &[StepKind::Review]),
            ("show", step.show.is_some(), &[StepKind::Review]),
            ("options", step.options.is_some(), &[StepKind::Review]),
            ("change", step.change.is_some(), &[StepKind::Review]),
            (
                "plan",
                step.plan.is_some() || step.then.is_some(),
                &[StepKind::Review],
            ),
            (
                "items",
                !step.items.is_empty() || !step.actions.is_empty(),
                &[StepKind::Result],
            ),
        ];
        for (field, set, kinds) in present {
            if set && !kinds.contains(&kind) {
                let names: Vec<&str> = kinds.iter().map(|k| k.name()).collect();
                out.push(problem(
                    format!("/steps/{n}/{field}"),
                    format!("is for {} steps, not {}", names.join(" and "), kind.name()),
                ));
            }
        }
    }

    fn check_rule(&self, out: &mut Vec<Problem>, at: &str, n: usize, step: &Step) {
        match &step.check {
            Some(target) => match self.steps.iter().position(|s| &s.id == target) {
                Some(k) if k > n && self.steps[k].kind == StepKind::Tool => {}
                _ => out.push(problem(
                    format!("{at}/check"),
                    format!("{target:?} is not a later tool step"),
                )),
            },
            None if step.retries.is_some() => {
                out.push(problem(format!("{at}/retries"), "retries go with check"));
            }
            None => {}
        }
        if let Some(retries) = step.retries
            && !(1..=MAX_RETRIES).contains(&retries)
        {
            out.push(problem(
                format!("{at}/retries"),
                format!("from 1 to {MAX_RETRIES}"),
            ));
        }
    }

    fn check_review(
        &self,
        out: &mut Vec<Problem>,
        at: &str,
        n: usize,
        step: &Step,
        asked: &BTreeMap<&str, usize>,
    ) {
        let Some(gate) = step.gate else {
            out.push(problem(format!("{at}/gate"), "confirm or notify"));
            return;
        };
        match gate {
            Gate::Confirm => {
                if step.control.is_some_and(|c| c != Control::Impact) {
                    out.push(problem(
                        format!("{at}/control"),
                        "a confirm gate shows impact",
                    ));
                }
                match &step.plan {
                    Some(plan) => {
                        self.pointer_rule(out, &format!("{at}/plan"), plan, n, asked, false)
                    }
                    None => out.push(problem(
                        format!("{at}/plan"),
                        "a confirm gate shows a plan: a pointer to a dry-run reply",
                    )),
                }
                match &step.then {
                    Some(then) => {
                        entry_rule(out, &format!("{at}/then/call"), &then.call);
                        self.params_rule(
                            out,
                            &format!("{at}/then/params"),
                            &then.params,
                            n,
                            asked,
                            false,
                        );
                    }
                    None => out.push(problem(
                        format!("{at}/then"),
                        "a confirm gate names the call Apply makes",
                    )),
                }
                for (field, set) in [
                    ("show", step.show.is_some()),
                    ("options", step.options.is_some()),
                    ("change", step.change.is_some()),
                ] {
                    if set {
                        out.push(problem(format!("{at}/{field}"), "is for a notify gate"));
                    }
                }
            }
            Gate::Notify => {
                match step.control {
                    Some(control) if !control.takes_input() => out.push(problem(
                        format!("{at}/control"),
                        format!(
                            "{} takes no input; a notify gate shows an input control",
                            control.name()
                        ),
                    )),
                    None => out.push(problem(
                        format!("{at}/control"),
                        "name the control the card shows",
                    )),
                    Some(_) => {}
                }
                match &step.show {
                    Some(show) => {
                        self.pointer_rule(out, &format!("{at}/show"), show, n, asked, false)
                    }
                    None => out.push(problem(
                        format!("{at}/show"),
                        "a pointer to what the card shows",
                    )),
                }
                if let Some(options) = &step.options {
                    if step.control != Some(Control::Choice) {
                        out.push(problem(
                            format!("{at}/options"),
                            "options go with the choice control",
                        ));
                    }
                    self.pointer_rule(
                        out,
                        &format!("{at}/options/list"),
                        &options.list,
                        n,
                        asked,
                        false,
                    );
                    item_pointer_rule(out, &format!("{at}/options/value"), &options.value);
                    if let Some(title) = &options.title {
                        item_pointer_rule(out, &format!("{at}/options/title"), title);
                    }
                }
                if let Some(change) = &step.change {
                    match self.steps[..n].iter().find(|s| &s.id == change) {
                        Some(s) if s.kind == StepKind::Model => {}
                        _ => out.push(problem(
                            format!("{at}/change"),
                            format!("{change:?} is not an earlier model step"),
                        )),
                    }
                }
                if step.plan.is_some() || step.then.is_some() {
                    out.push(problem(
                        format!("{at}/plan"),
                        "plan and then are for a confirm gate",
                    ));
                }
            }
        }
    }

    /// Check one pointer used by step `n`: an input already asked for, an
    /// earlier step (into a model step: one of its answer's fields), or
    /// `/item` inside `each`.
    fn pointer_rule(
        &self,
        out: &mut Vec<Problem>,
        at: &str,
        text: &str,
        n: usize,
        asked: &BTreeMap<&str, usize>,
        item: bool,
    ) {
        match Pointer::parse(text) {
            Err(why) => out.push(problem(at, why)),
            Ok(Pointer::Input { name, .. }) => {
                if !self.inputs.contains_key(name) {
                    out.push(problem(at, format!("no input {name:?}")));
                } else if asked.get(name).is_none_or(|k| *k >= n) {
                    out.push(problem(
                        at,
                        format!("input {name} is not asked for before this step"),
                    ));
                }
            }
            Ok(Pointer::Step { id, rest }) => {
                match self.steps[..n.min(self.steps.len())]
                    .iter()
                    .find(|s| s.id == id)
                {
                    None => out.push(problem(at, format!("{id:?} is not an earlier step"))),
                    Some(step) if step.kind == StepKind::Model => {
                        let field = rest.trim_start_matches('/').split('/').next().unwrap_or("");
                        let known = step
                            .output
                            .as_ref()
                            .and_then(|o| self.outputs.get(o))
                            .is_none_or(|o| field.is_empty() || o.fields.contains_key(field));
                        if !known {
                            out.push(problem(
                                at,
                                format!("step {id}'s answer has no field {field:?}"),
                            ));
                        }
                    }
                    Some(step) if matches!(step.kind, StepKind::Input | StepKind::Result) => {
                        out.push(problem(
                            at,
                            format!("step {id} has no result to point into"),
                        ));
                    }
                    Some(_) => {}
                }
            }
            Ok(Pointer::Item { .. }) if !item => {
                out.push(problem(at, "/item is only for a step with each"));
            }
            Ok(Pointer::Item { .. }) => {}
        }
    }

    /// Check every pointer in a call's parameters, at any depth.
    fn params_rule(
        &self,
        out: &mut Vec<Problem>,
        at: &str,
        params: &Value,
        n: usize,
        asked: &BTreeMap<&str, usize>,
        item: bool,
    ) {
        match params {
            Value::String(text) if text.starts_with('/') => {
                self.pointer_rule(out, at, text, n, asked, item);
            }
            Value::Object(map) if is_literal(map) => {}
            Value::Object(map) => {
                for (key, value) in map {
                    self.params_rule(out, &format!("{at}/{key}"), value, n, asked, item);
                }
            }
            Value::Array(list) => {
                for (k, value) in list.iter().enumerate() {
                    self.params_rule(out, &format!("{at}/{k}"), value, n, asked, item);
                }
            }
            _ => {}
        }
    }

    /// Check that every model step's guide is a heading of `method`, the
    /// text of `method.md`.
    pub fn check_method(&self, method: &str) -> Result<(), Vec<Problem>> {
        let anchors = heading_anchors(method);
        let mut out = Vec::new();
        for (n, step) in self.steps.iter().enumerate() {
            if let Some(reference) = &step.method
                && let Ok(anchor) = method_anchor(reference)
                && !anchors.contains(anchor)
            {
                out.push(problem(
                    format!("/steps/{n}/method"),
                    format!("{METHOD_FILE} has no heading #{anchor}"),
                ));
            }
        }
        if out.is_empty() { Ok(()) } else { Err(out) }
    }

    /// The workflow's line in the chat's index: id, summary and intents.
    pub fn index_line(&self) -> String {
        format!(
            "{} v{}: {} Asks like: {}",
            self.id,
            self.version,
            self.summary,
            self.intents.join("; ")
        )
    }

    /// The input `name`.
    pub fn input(&self, name: &str) -> Option<&Input> {
        self.inputs.get(name)
    }

    /// The step `id`.
    pub fn step(&self, id: &str) -> Option<&Step> {
        self.steps.iter().find(|s| s.id == id)
    }
}

/// The chat's workflow index: one line per workflow. A test holds the
/// shipped index under 1 KiB (§3.1).
pub fn index(workflows: &[&Workflow]) -> String {
    workflows
        .iter()
        .map(|w| w.index_line())
        .collect::<Vec<_>>()
        .join("\n")
}

impl Input {
    /// The control that shows this input.
    pub fn control(&self) -> Control {
        self.control.unwrap_or_else(|| Control::for_input(self.ty))
    }

    fn check(&self, at: &str, workflow: &Workflow, out: &mut Vec<Problem>) {
        let ty = self.ty;
        text_rule(out, &format!("{at}/title"), &self.title, MAX_TITLE);
        text_rule_optional(out, &format!("{at}/help"), &self.help, MAX_TEXT);
        if let Some(control) = self.control
            && !control.accepts(ty)
        {
            out.push(problem(
                format!("{at}/control"),
                format!("{} does not show a {} input", control.name(), ty.name()),
            ));
        }
        let only = |field: &str, set: bool, types: &[InputType], out: &mut Vec<Problem>| {
            if set && !types.contains(&ty) {
                let names: Vec<&str> = types.iter().map(|t| t.name()).collect();
                out.push(problem(
                    format!("{at}/{field}"),
                    format!("is for {} inputs, not {}", names.join(", "), ty.name()),
                ));
            }
        };
        use InputType as T;
        only("multiline", self.multiline, &[T::Text], out);
        only("accept", !self.accept.is_empty(), &[T::Files], out);
        only("options", !self.options.is_empty(), &[T::Enum], out);
        only("multiple", self.multiple, &[T::Enum, T::EnumFrom], out);
        only("from", self.from.is_some(), &[T::EnumFrom], out);
        only("of", self.of.is_some(), &[T::PageRanges], out);
        only(
            "min",
            self.min.is_some(),
            &[T::Length, T::Angle, T::Number, T::Integer, T::Files],
            out,
        );
        only(
            "max",
            self.max.is_some(),
            &[
                T::Text,
                T::Length,
                T::Angle,
                T::Number,
                T::Integer,
                T::Files,
            ],
            out,
        );
        for (field, bound) in [("min", &self.min), ("max", &self.max)] {
            let Some(bound) = bound else { continue };
            let fits = if ty.integral() || matches!(ty, T::Text | T::Files) {
                bound.is_i64()
            } else {
                bound.as_f64().is_some_and(f64::is_finite)
            };
            if !fits {
                out.push(problem(
                    format!("{at}/{field}"),
                    format!(
                        "{bound} is not a {} bound",
                        if fits_integer(ty) {
                            "whole-number"
                        } else {
                            "finite"
                        }
                    ),
                ));
            }
        }
        if ty.numeric()
            && let (Some(min), Some(max)) = (number(&self.min), number(&self.max))
            && min > max
        {
            out.push(problem(format!("{at}/min"), "greater than max"));
        }
        match ty {
            T::Text => match self.max.as_ref().and_then(Number::as_u64) {
                Some(max) if (1..=MAX_TEXT_INPUT).contains(&max) => {}
                _ => out.push(problem(
                    format!("{at}/max"),
                    format!("a text input states its most bytes, 1 to {MAX_TEXT_INPUT}"),
                )),
            },
            T::Files => {
                let min = self.min.as_ref().and_then(Number::as_u64).unwrap_or(1);
                let max = self.max.as_ref().and_then(Number::as_u64).unwrap_or(1);
                if min > max || max > MAX_FILES || max == 0 {
                    out.push(problem(
                        format!("{at}/max"),
                        format!("files: min ≤ max, max from 1 to {MAX_FILES}"),
                    ));
                }
                if self.accept.is_empty() {
                    out.push(problem(
                        format!("{at}/accept"),
                        "name the media types taken",
                    ));
                }
                for (k, media) in self.accept.iter().enumerate() {
                    if !valid_media_type(media) {
                        out.push(problem(
                            format!("{at}/accept/{k}"),
                            format!("{media:?} is not a media type such as application/pdf"),
                        ));
                    }
                }
            }
            T::Enum => {
                if self.options.is_empty() || self.options.len() > MAX_OPTIONS {
                    out.push(problem(
                        format!("{at}/options"),
                        format!("give 1 to {MAX_OPTIONS} options"),
                    ));
                }
                for (k, option) in self.options.iter().enumerate() {
                    if option.value.is_empty() || option.value.len() > MAX_ID {
                        out.push(problem(format!("{at}/options/{k}/value"), "1 to 64 bytes"));
                    }
                    if self.options[..k].iter().any(|o| o.value == option.value) {
                        out.push(problem(
                            format!("{at}/options/{k}/value"),
                            format!("{} is listed twice", option.value),
                        ));
                    }
                    text_rule(
                        out,
                        &format!("{at}/options/{k}/title"),
                        &option.title,
                        MAX_TITLE,
                    );
                }
            }
            T::EnumFrom => match &self.from {
                Some(from) => {
                    item_pointer_rule(out, &format!("{at}/from/value"), &from.value);
                    if let Some(title) = &from.title {
                        item_pointer_rule(out, &format!("{at}/from/title"), title);
                    }
                }
                None => out.push(problem(
                    format!("{at}/from"),
                    "name the step result the options come from",
                )),
            },
            T::PageRanges => {
                if let Some(of) = &self.of
                    && workflow.inputs.get(of).is_none_or(|i| i.ty != T::Files)
                {
                    out.push(problem(
                        format!("{at}/of"),
                        format!("{of:?} is not a files input"),
                    ));
                }
            }
            T::Length | T::Angle | T::Number | T::Integer | T::Url => {}
        }
        if let Some(default) = &self.default
            && let Err(why) = self.check_value(default)
        {
            out.push(problem(format!("{at}/default"), why));
        }
    }

    /// Check a value given for this input: by an offer, a control or a
    /// default. Lengths are integer nanometres and angles integer
    /// millidegrees (a control converts what the person types). Options of
    /// `enum_from` are known only at run time, so any text passes here.
    pub fn check_value(&self, value: &Value) -> Result<(), String> {
        use InputType as T;
        match self.ty {
            T::Text => {
                let text = value.as_str().ok_or("give text")?;
                let max = self.max.as_ref().and_then(Number::as_u64).unwrap_or(0);
                if text.len() as u64 > max {
                    return Err(format!("{} bytes; at most {max}", text.len()));
                }
                Ok(())
            }
            T::Length | T::Angle | T::Integer => {
                let n = value.as_i64().ok_or_else(|| match self.ty {
                    T::Length => "give whole nanometres".to_owned(),
                    T::Angle => "give whole millidegrees".to_owned(),
                    _ => "give a whole number".to_owned(),
                })?;
                bounds(n as f64, &self.min, &self.max)
            }
            T::Number => {
                let n = value
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .ok_or("give a finite number")?;
                bounds(n, &self.min, &self.max)
            }
            T::Enum | T::EnumFrom => {
                let chosen: Vec<&str> = match value {
                    Value::String(one) => vec![one.as_str()],
                    Value::Array(list) if self.multiple => list
                        .iter()
                        .map(|v| v.as_str().ok_or("give option values as text"))
                        .collect::<Result<_, _>>()?,
                    _ if self.multiple => return Err("give a list of option values".into()),
                    _ => return Err("give one option value".into()),
                };
                if self.ty == T::Enum
                    && let Some(bad) = chosen
                        .iter()
                        .find(|c| !self.options.iter().any(|o| o.value == **c))
                {
                    let known: Vec<&str> = self.options.iter().map(|o| o.value.as_str()).collect();
                    return Err(format!(
                        "{bad:?} is not an option; options: {}",
                        known.join(", ")
                    ));
                }
                Ok(())
            }
            T::Files => {
                let list = value.as_array().ok_or("give a list of file paths")?;
                let min = self.min.as_ref().and_then(Number::as_u64).unwrap_or(1);
                let max = self.max.as_ref().and_then(Number::as_u64).unwrap_or(1);
                if (list.len() as u64) < min || list.len() as u64 > max {
                    return Err(format!("{} files; give {min} to {max}", list.len()));
                }
                if list.iter().any(|v| v.as_str().is_none_or(str::is_empty)) {
                    return Err("each file is a path as text".into());
                }
                Ok(())
            }
            T::Url => {
                let url = value.as_str().ok_or("give a URL as text")?;
                if !url.starts_with("https://") || url.len() > 2048 {
                    return Err("give an https:// URL of at most 2048 bytes".into());
                }
                Ok(())
            }
            T::PageRanges => {
                let text = value
                    .as_str()
                    .ok_or("give pages as text, such as \"1-5, 31\"")?;
                parse_page_ranges(text).map(|_| ())
            }
        }
    }
}

impl Output {
    /// Check a model step's answer against this shape: every required
    /// field present with its type, no field it does not name.
    pub fn check_value(&self, value: &Value) -> Result<(), Vec<Problem>> {
        let Some(map) = value.as_object() else {
            return Err(vec![problem("", "the answer is an object of named fields")]);
        };
        let mut out = Vec::new();
        for (name, field) in &self.fields {
            match map.get(name) {
                None | Some(Value::Null) if field.optional => {}
                None | Some(Value::Null) => out.push(problem(format!("/{name}"), "missing")),
                Some(v) if !field.ty.admits(v) => out.push(problem(
                    format!("/{name}"),
                    format!("give {}", field.ty.name()),
                )),
                Some(_) => {}
            }
        }
        for name in map.keys() {
            if !self.fields.contains_key(name) {
                let known: Vec<&str> = self.fields.keys().map(String::as_str).collect();
                out.push(problem(
                    format!("/{name}"),
                    format!(
                        "not a field of this answer; its fields: {}",
                        known.join(", ")
                    ),
                ));
            }
        }
        if out.is_empty() { Ok(()) } else { Err(out) }
    }
}

/// Pages as 1-based inclusive ranges, from `"1-5, 31, 32"`.
pub fn parse_page_ranges(text: &str) -> Result<Vec<(u32, u32)>, String> {
    const MAX_RANGES: usize = 64;
    let bad = || format!("{text:?}: give pages such as \"1-5, 31, 32\"");
    let mut ranges = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (a.trim(), b.trim()),
            None => (part, part),
        };
        let a: u32 = a.parse().map_err(|_| bad())?;
        let b: u32 = b.parse().map_err(|_| bad())?;
        if a == 0 || b < a {
            return Err(bad());
        }
        ranges.push((a, b));
    }
    if ranges.is_empty() || ranges.len() > MAX_RANGES {
        return Err(bad());
    }
    Ok(ranges)
}

fn bounds(n: f64, min: &Option<Number>, max: &Option<Number>) -> Result<(), String> {
    if let Some(min) = number(min)
        && n < min
    {
        return Err(format!("{n} is below the least, {min}"));
    }
    if let Some(max) = number(max)
        && n > max
    {
        return Err(format!("{n} is above the greatest, {max}"));
    }
    Ok(())
}

fn number(bound: &Option<Number>) -> Option<f64> {
    bound.as_ref().and_then(Number::as_f64)
}

fn fits_integer(ty: InputType) -> bool {
    ty.integral() || matches!(ty, InputType::Text | InputType::Files)
}

/// `{"literal": …}` alone: a value written as itself.
fn is_literal(map: &serde_json::Map<String, Value>) -> bool {
    map.len() == 1 && map.contains_key("literal")
}

/// `method.md#<anchor>`: the anchor.
fn method_anchor(reference: &str) -> Result<&str, String> {
    match reference.split_once('#') {
        Some((file, anchor))
            if file == METHOD_FILE
                && !anchor.is_empty()
                && anchor.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b)
                }) =>
        {
            Ok(anchor)
        }
        _ => Err(format!(
            "{reference:?}: name a heading of {METHOD_FILE}, as {METHOD_FILE}#the-heading"
        )),
    }
}

/// The anchors of a Markdown text's headings, as GitHub writes them:
/// lowercase, spaces as `-`, punctuation other than `-` and `_` dropped.
pub fn heading_anchors(markdown: &str) -> BTreeSet<String> {
    headings(markdown).map(|(_, _, anchor)| anchor).collect()
}

/// The section of `markdown` under the heading whose anchor is `anchor`:
/// the heading line and everything up to the next heading of the same or a
/// higher level. What a model step reads of `method.md`.
pub fn method_section<'a>(markdown: &'a str, anchor: &str) -> Option<&'a str> {
    let mut found: Option<(usize, usize)> = None;
    for (offset, level, name) in headings(markdown) {
        match found {
            None if name == anchor => found = Some((offset, level)),
            Some((start, at)) if level <= at => return markdown.get(start..offset),
            _ => {}
        }
    }
    found.and_then(|(start, _)| markdown.get(start..))
}

/// Each heading outside code fences: its byte offset, level and anchor.
fn headings(markdown: &str) -> impl Iterator<Item = (usize, usize, String)> + '_ {
    let mut fenced = false;
    let mut offset = 0;
    markdown.split_inclusive('\n').filter_map(move |line| {
        let at = offset;
        offset += line.len();
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            return None;
        }
        if fenced {
            return None;
        }
        let rest = line.strip_prefix('#')?;
        let heading = rest.trim_start_matches('#');
        if !heading.starts_with(' ') {
            return None;
        }
        let level = 1 + rest.len() - heading.len();
        let anchor: String = heading
            .trim()
            .chars()
            .flat_map(char::to_lowercase)
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
                _ => None,
            })
            .collect();
        Some((at, level, anchor))
    })
}

fn item_pointer_rule(out: &mut Vec<Problem>, at: &str, text: &str) {
    if !text.starts_with('/') || text.len() > MAX_TEXT {
        out.push(problem(
            at,
            format!("{text:?}: a pointer inside one item, such as /name"),
        ));
    }
}

fn entry_rule(out: &mut Vec<Problem>, at: &str, name: &str) {
    let valid = name.len() <= 128
        && name
            .split_once('.')
            .is_some_and(|(module, rest)| valid_id(module, true) && !rest.is_empty())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._".contains(&b));
    if !valid {
        out.push(problem(
            at,
            format!("{name:?} is not an entry name such as library.generators"),
        ));
    }
}

fn text_rule(out: &mut Vec<Problem>, at: &str, text: &str, max: usize) {
    if text.trim().is_empty() || text.len() > max {
        out.push(problem(at, format!("give 1 to {max} bytes of text")));
    }
}

fn text_rule_optional(out: &mut Vec<Problem>, at: &str, text: &str, max: usize) {
    if text.len() > max {
        out.push(problem(at, format!("at most {max} bytes")));
    }
}

/// Lowercase `a–z`, `0–9` and `-` (with `wide`, also `_` and `.`), 1 to
/// [`MAX_ID`] bytes, starting with a letter.
fn valid_id(id: &str, wide: bool) -> bool {
    id.len() <= MAX_ID
        && id.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && id.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || b == b'-'
                || (wide && b"_.".contains(&b))
        })
}

fn valid_media_type(media: &str) -> bool {
    media.len() <= 128
        && media.split_once('/').is_some_and(|(a, b)| {
            let part = |s: &str| {
                !s.is_empty()
                    && s.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || b"+-.".contains(&c)
                    })
            };
            part(a) && part(b)
        })
}

/// `"1.0"` as `(1, 0)`.
pub fn parse_version(text: &str) -> Option<(u16, u16)> {
    let (major, minor) = text.split_once('.')?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 5 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(major) || !digits(minor) {
        return None;
    }
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// The JSON Schema of `workflow.json`, for editors and the docs.
pub fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Workflow)).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests;
