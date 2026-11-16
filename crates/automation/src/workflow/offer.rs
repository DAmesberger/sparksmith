//! Offering a workflow (`docs/assistant-panel.md` §3): the card's content
//! checked against the host's workflows, whoever asks for it — the model
//! (`assist.offer`), a signal (`offer_when`), the `/` menu or the palette —
//! and the structured signals themselves. Never text matching.
use super::entries::{MAX_QUERY, OfferParams};
use super::event::{OfferedBy, WorkflowEvent};
use super::load::Listed;
use super::{InputType, Signal, Workflow};
use serde_json::{Map, Value};

/// A workflow card as checked: what the chat draws and the run starts from.
#[derive(Clone, Debug, PartialEq)]
pub struct Offer {
    pub workflow: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    /// Inputs already known, each checked against its input.
    pub inputs: Map<String, Value>,
    /// The required inputs the run will still ask for, in the order its
    /// input steps ask.
    pub missing: Vec<String>,
    /// One sentence on why it fits; empty when the person chose it.
    pub why: String,
}

impl Offer {
    /// The card for workflow `id` among `listed` with `inputs` and `why`.
    /// Refused, saying why, when the host has no such workflow, it cannot
    /// run here, an input is unknown or its value does not pass, or `why`
    /// is too long.
    pub fn check(
        listed: &[Listed],
        id: &str,
        inputs: Map<String, Value>,
        why: &str,
    ) -> Result<Self, String> {
        if why.len() > MAX_QUERY {
            return Err(format!("why: give at most {MAX_QUERY} bytes"));
        }
        let Some(found) = listed.iter().find(|l| l.id == id) else {
            let known: Vec<&str> = listed
                .iter()
                .filter(|l| l.available())
                .map(|l| l.id.as_str())
                .collect();
            let shown: String = id.chars().take(64).collect();
            return Err(format!(
                "no workflow {shown:?}; this host has: {}",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            ));
        };
        let Some(loaded) = found.loaded.as_ref().filter(|_| found.available()) else {
            return Err(format!(
                "workflow {id:?} cannot run here: {}",
                found.why_not.join("; ")
            ));
        };
        let workflow = &loaded.workflow;
        for (name, value) in &inputs {
            let input = workflow.input(name).ok_or_else(|| {
                let names: Vec<&str> = workflow.inputs.keys().map(String::as_str).collect();
                format!(
                    "inputs/{name}: not an input of {id}; its inputs: {}",
                    names.join(", ")
                )
            })?;
            input
                .check_value(value)
                .map_err(|why| format!("inputs/{name}: {why}"))?;
        }
        let missing = workflow
            .inputs_in_order()
            .into_iter()
            .filter(|name| {
                workflow
                    .input(name)
                    .is_some_and(|i| !i.optional && i.default.is_none())
                    && !inputs.contains_key(*name)
            })
            .map(str::to_owned)
            .collect();
        Ok(Self {
            workflow: workflow.id.clone(),
            version: workflow.version.clone(),
            title: workflow.title.clone(),
            summary: workflow.summary.clone(),
            inputs,
            missing,
            why: why.to_owned(),
        })
    }

    /// [`Offer::check`] for `assist.offer`'s parameters.
    pub fn from_params(listed: &[Listed], params: &OfferParams) -> Result<Self, String> {
        Self::check(listed, &params.workflow, params.inputs.clone(), &params.why)
    }

    /// The chat's event for this card, `card` in the conversation.
    pub fn event(&self, card: String, by: OfferedBy) -> WorkflowEvent {
        WorkflowEvent::Offered {
            card,
            workflow: self.workflow.clone(),
            version: self.version.clone(),
            title: self.title.clone(),
            summary: self.summary.clone(),
            inputs: self.inputs.clone(),
            missing: self.missing.clone(),
            why: self.why.clone(),
            by,
        }
    }
}

impl Workflow {
    /// The inputs in the order its input steps ask for them, then any no
    /// step asks for.
    pub fn inputs_in_order(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self
            .steps
            .iter()
            .filter(|s| s.kind == super::StepKind::Input)
            .flat_map(|s| s.inputs.iter().map(String::as_str))
            .collect();
        for name in self.inputs.keys() {
            if !names.contains(&name.as_str()) {
                names.push(name);
            }
        }
        names
    }

    /// The inputs a run starts from: those of the input steps before its
    /// first other step, in order. The chat's one confirmation shows these;
    /// an input step later in the run asks for its own when it comes.
    pub fn start_inputs(&self) -> Vec<&str> {
        self.steps
            .iter()
            .take_while(|s| s.kind == super::StepKind::Input)
            .flat_map(|s| s.inputs.iter().map(String::as_str))
            .collect()
    }

    /// Fill each `files` start input that `inputs` lacks from `attached`
    /// (each a path and its media type, read from its first bytes): the
    /// files of a type it takes, in order, up to its `max`. An input that
    /// would get fewer than its `min` stays unfilled, and each file fills
    /// one input at most. The pre-fill a conversation's attachments give,
    /// without a model.
    pub fn fill_files(&self, inputs: &mut Map<String, Value>, attached: &[(String, String)]) {
        let mut used = vec![false; attached.len()];
        for name in self.start_inputs() {
            let Some(input) = self.input(name) else {
                continue;
            };
            if input.ty != InputType::Files || inputs.contains_key(name) {
                continue;
            }
            let bound = |b: &Option<serde_json::Number>| {
                b.as_ref()
                    .and_then(serde_json::Number::as_u64)
                    .map_or(1, |n| usize::try_from(n).unwrap_or(usize::MAX))
            };
            let (min, max) = (bound(&input.min), bound(&input.max));
            let taken: Vec<usize> = attached
                .iter()
                .enumerate()
                .filter(|(i, (_, media))| {
                    !used[*i] && input.accept.iter().any(|a| a.eq_ignore_ascii_case(media))
                })
                .map(|(i, _)| i)
                .take(max)
                .collect();
            if taken.is_empty() || taken.len() < min {
                continue;
            }
            let paths = taken
                .iter()
                .map(|i| {
                    used[*i] = true;
                    Value::String(attached[*i].0.clone())
                })
                .collect();
            inputs.insert(name.to_owned(), Value::Array(paths));
        }
    }

    /// Whether `signal` offers this workflow (`offer_when`). A media type
    /// compares without case.
    pub fn offered_on(&self, signal: &Signal) -> bool {
        self.offer_when.iter().any(|own| match (own, signal) {
            (Signal::Attachment(a), Signal::Attachment(b)) => a.eq_ignore_ascii_case(b),
            (Signal::Workspace(a), Signal::Workspace(b)) | (Signal::Issue(a), Signal::Issue(b)) => {
                a == b
            }
            _ => false,
        })
    }

    /// The `files` input an attachment of `media` type fills: the first,
    /// in the order the inputs are asked, that accepts it.
    pub fn input_for_attachment(&self, media: &str) -> Option<&str> {
        self.inputs_in_order().into_iter().find(|name| {
            self.input(name).is_some_and(|i| {
                i.ty == InputType::Files && i.accept.iter().any(|a| a.eq_ignore_ascii_case(media))
            })
        })
    }
}

/// The workflows of `listed` that run here and `signal` offers, in id
/// order.
pub fn offered_on<'a>(listed: &'a [Listed], signal: &Signal) -> Vec<&'a super::Loaded> {
    let mut found: Vec<&super::Loaded> = listed
        .iter()
        .filter(|l| l.available())
        .filter_map(|l| l.loaded.as_ref())
        .filter(|l| l.workflow.offered_on(signal))
        .collect();
    found.sort_by(|a, b| a.workflow.id.cmp(&b.workflow.id));
    found
}

/// A PDF's media type: a datasheet's.
pub const PDF: &str = "application/pdf";

/// The media type of a dropped or attached file, from its first bytes:
/// only the types a workflow can ask for are known. A name's extension is
/// never trusted on its own.
pub fn media_type(head: &[u8]) -> Option<&'static str> {
    head.starts_with(b"%PDF-").then_some(PDF)
}
