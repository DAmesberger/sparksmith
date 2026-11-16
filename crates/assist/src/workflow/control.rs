//! The in-chat controls (`docs/assistant-panel.md` §4): one closed set of
//! ten. The widgets are code, built once in `crates/ui` (C2); a workflow
//! only chooses among them and fills their fields. A workflow naming a
//! control the app does not have fails the load check (serde refuses the
//! unknown name, listing the ten).
//!
//! [`ControlCard`] is what a card shows, the descriptor the runner hands
//! the chat and an MCP client's form is built from (M1); [`ControlAnswer`]
//! is what comes back.
use super::{Input, InputType};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The ten controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// One line or several, with a byte limit.
    Text,
    /// A number with its unit; accepts `0.25`, `"0.25mm"`, `"10mil"`.
    Number,
    /// Single or multiple choice among titled options.
    Choice,
    /// File picker and drop target for documents.
    Datasheet,
    /// A URL, fetched only through an Ask.
    Url,
    /// Pages such as `"1-5, 31, 32"` against the file's page count.
    PageRanges,
    /// A plan's counts, findings and rows; Apply or Decline.
    Impact,
    /// The current step, done of total, Stop.
    Progress,
    /// Requests, tokens and minutes against the run's limits.
    Meter,
    /// What was made; Show, Open in editor, Undo run.
    Result,
}

impl Control {
    pub const ALL: [Self; 10] = [
        Self::Text,
        Self::Number,
        Self::Choice,
        Self::Datasheet,
        Self::Url,
        Self::PageRanges,
        Self::Impact,
        Self::Progress,
        Self::Meter,
        Self::Result,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Choice => "choice",
            Self::Datasheet => "datasheet",
            Self::Url => "url",
            Self::PageRanges => "page_ranges",
            Self::Impact => "impact",
            Self::Progress => "progress",
            Self::Meter => "meter",
            Self::Result => "result",
        }
    }

    /// The control an input of type `ty` uses when it names none.
    pub const fn for_input(ty: InputType) -> Self {
        match ty {
            InputType::Text => Self::Text,
            InputType::Length | InputType::Angle | InputType::Number | InputType::Integer => {
                Self::Number
            }
            InputType::Enum | InputType::EnumFrom => Self::Choice,
            InputType::Files => Self::Datasheet,
            InputType::Url => Self::Url,
            InputType::PageRanges => Self::PageRanges,
        }
    }

    /// Whether this control can show an input of type `ty`.
    pub const fn accepts(self, ty: InputType) -> bool {
        matches!(
            (self, ty),
            (Self::Text, InputType::Text)
                | (
                    Self::Number,
                    InputType::Length | InputType::Angle | InputType::Number | InputType::Integer
                )
                | (Self::Choice, InputType::Enum | InputType::EnumFrom)
                | (Self::Datasheet, InputType::Files)
                | (Self::Url, InputType::Url)
                | (Self::PageRanges, InputType::PageRanges)
        )
    }

    /// Whether the person gives a value through it (the first six).
    pub const fn takes_input(self) -> bool {
        matches!(
            self,
            Self::Text
                | Self::Number
                | Self::Choice
                | Self::Datasheet
                | Self::Url
                | Self::PageRanges
        )
    }
}

/// What a number control's value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Quantity {
    /// Integer nanometres; the person types `0.25mm` or `10mil`.
    Length,
    /// Integer millidegrees; the person types degrees.
    Angle,
    Number,
    Integer,
}

/// The heading of an input card's field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Field {
    /// The input's name in the workflow.
    pub input: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
}

/// A titled option.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Choice {
    pub value: String,
    pub title: String,
}

/// A file already given: what the datasheet control shows of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileChip {
    pub path: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<u32>,
    pub bytes: u64,
}

/// A count on an impact card ("5 pins").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Count {
    pub label: String,
    pub n: u64,
}

/// A finding on an impact or result card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Finding {
    pub blocking: bool,
    pub message: String,
}

/// A changed row the person may leave out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Row {
    pub id: String,
    pub text: String,
}

/// One limit and how much of it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Gauge {
    pub used: u64,
    pub limit: u64,
}

/// A titled value on the result card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Item {
    pub title: String,
    pub value: Value,
}

/// A result card's button: a `show` call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Action {
    pub title: String,
    pub call: String,
    pub params: Value,
}

/// One card, as the chat draws it and an MCP form is built from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "control", rename_all = "snake_case")]
pub enum ControlCard {
    Text {
        field: Field,
        multiline: bool,
        max_bytes: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    Number {
        field: Field,
        quantity: Quantity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<serde_json::Number>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<serde_json::Number>,
    },
    Choice {
        field: Field,
        multiple: bool,
        options: Vec<Choice>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        value: Vec<String>,
    },
    Datasheet {
        field: Field,
        accept: Vec<String>,
        min: u64,
        max: u64,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<FileChip>,
    },
    Url {
        field: Field,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    PageRanges {
        field: Field,
        /// The page count of the file the pages are of, when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pages: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    Impact {
        title: String,
        counts: Vec<Count>,
        findings: Vec<Finding>,
        /// Rows the person may leave out; empty when none may be.
        rows: Vec<Row>,
    },
    Progress {
        step: String,
        done: u32,
        total: u32,
    },
    Meter {
        requests: Gauge,
        output_tokens: Gauge,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        total_tokens: Option<Gauge>,
        minutes: Gauge,
    },
    Result {
        title: String,
        items: Vec<Item>,
        findings: Vec<Finding>,
        actions: Vec<Action>,
        /// Whether Undo run is offered (the run's turn is still the last
        /// undo step).
        undo: bool,
    },
}

impl ControlCard {
    /// The control this card is.
    pub fn control(&self) -> Control {
        match self {
            Self::Text { .. } => Control::Text,
            Self::Number { .. } => Control::Number,
            Self::Choice { .. } => Control::Choice,
            Self::Datasheet { .. } => Control::Datasheet,
            Self::Url { .. } => Control::Url,
            Self::PageRanges { .. } => Control::PageRanges,
            Self::Impact { .. } => Control::Impact,
            Self::Progress { .. } => Control::Progress,
            Self::Meter { .. } => Control::Meter,
            Self::Result { .. } => Control::Result,
        }
    }

    /// The card for input `name`, with the value given so far. The options
    /// of an `enum_from` input are the runner's to fill (`options`); a
    /// value of the wrong shape is left out, for the person to give.
    pub fn for_input(
        name: &str,
        input: &Input,
        value: Option<&Value>,
        options: Vec<Choice>,
    ) -> Self {
        let field = Field {
            input: name.to_owned(),
            title: input.title.clone(),
            help: input.help.clone(),
            optional: input.optional,
        };
        let value = value.or(input.default.as_ref());
        let text = || value.and_then(Value::as_str).map(str::to_owned);
        let count = |bound: &Option<serde_json::Number>| {
            bound
                .as_ref()
                .and_then(serde_json::Number::as_u64)
                .unwrap_or(1)
        };
        match input.control() {
            Control::Text => Self::Text {
                field,
                multiline: input.multiline,
                max_bytes: count(&input.max),
                value: text(),
            },
            Control::Number => Self::Number {
                field,
                quantity: match input.ty {
                    InputType::Length => Quantity::Length,
                    InputType::Angle => Quantity::Angle,
                    InputType::Integer => Quantity::Integer,
                    _ => Quantity::Number,
                },
                min: input.min.clone(),
                max: input.max.clone(),
                value: value.and_then(|v| v.as_number().cloned()),
            },
            Control::Choice => Self::Choice {
                field,
                multiple: input.multiple,
                options: if input.ty == InputType::Enum {
                    input
                        .options
                        .iter()
                        .map(|o| Choice {
                            value: o.value.clone(),
                            title: o.title.clone(),
                        })
                        .collect()
                } else {
                    options
                },
                value: match value {
                    Some(Value::String(one)) => vec![one.clone()],
                    Some(Value::Array(list)) => list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect(),
                    _ => Vec::new(),
                },
            },
            Control::Datasheet => Self::Datasheet {
                field,
                accept: input.accept.clone(),
                min: count(&input.min),
                max: count(&input.max),
                files: Vec::new(),
            },
            Control::Url => Self::Url {
                field,
                value: text(),
            },
            // The load check gives every input one of the six above; a page
            // range is what is left.
            Control::PageRanges
            | Control::Impact
            | Control::Progress
            | Control::Meter
            | Control::Result => Self::PageRanges {
                field,
                pages: None,
                value: text(),
            },
        }
    }
}

/// What the person answers on a card.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ControlAnswer {
    /// Input cards: the values given, by input name.
    Inputs {
        values: serde_json::Map<String, Value>,
    },
    /// A notify card's Change: the person's choice.
    Change { value: Value },
    /// An impact card's Apply, leaving out these rows.
    Apply {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        leave_out: Vec<String>,
    },
    /// An impact card's Decline, or Esc on an input card.
    Decline,
    /// Progress's Stop.
    Stop,
    /// The result card's Undo run.
    UndoRun,
}
