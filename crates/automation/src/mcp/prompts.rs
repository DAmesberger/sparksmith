//! MCP prompts: the workflows ([`crate::workflow`]) any client can start,
//! generated from their data (`docs/assistant-panel.md` §2.5). A
//! workflow's prompt is named after its id (`datasheet-to-component` is
//! `datasheet_to_component`), titled and described by its `title` and
//! `summary`, and takes its inputs as arguments, in the order its input
//! steps ask for them. `prompts/get` serves its `method.md` as written, then
//! the task built from the arguments. Nothing about a workflow is written
//! here: a new workflow is a folder of data.
//!
//! An external client has its own loop: it reads the method and calls the
//! verbs itself, while the in-app runner drives the same workflow step by
//! step. A test holds the order of the two the same.
use crate::workflow::{InputType, Loaded, StepKind, Workflow};
use rmcp::model::{GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role};
use serde_json::Value;

/// Bytes of one prompt argument.
const MAX_ARGUMENT: usize = 16 * 1024;

/// The prompt name of workflow `id`: its id with `_` for `-`.
pub fn name_of(id: &str) -> String {
    id.replace('-', "_")
}

/// The inputs of `workflow` in the order its input steps ask for them, then
/// any no step asks for.
fn inputs_in_order(workflow: &Workflow) -> Vec<&str> {
    let mut names: Vec<&str> = workflow
        .steps
        .iter()
        .filter(|s| s.kind == StepKind::Input)
        .flat_map(|s| s.inputs.iter().map(String::as_str))
        .collect();
    for name in workflow.inputs.keys() {
        if !names.contains(&name.as_str()) {
            names.push(name);
        }
    }
    names
}

/// What a prompt argument says of input `name`: its title and help, and
/// how a client writes a value of its type as text.
fn describe(workflow: &Workflow, name: &str) -> String {
    let Some(input) = workflow.input(name) else {
        return String::new();
    };
    let mut text = input.title.clone();
    if !input.help.is_empty() {
        text.push_str(": ");
        text.push_str(&input.help);
    }
    let how = match input.ty {
        InputType::Files => format!(
            "File paths or URLs ({}), one per line. You read them with your own tools; {} does not.",
            input.accept.join(", "),
            sparksmith_core::product_name!()
        ),
        InputType::PageRanges => "Pages such as 1-5, 31, 32.".to_owned(),
        InputType::Length => "A length in integer nanometres.".to_owned(),
        InputType::Angle => "An angle in integer millidegrees.".to_owned(),
        InputType::Enum => format!(
            "One of: {}.",
            input
                .options
                .iter()
                .map(|o| o.value.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        InputType::Url => "An https URL.".to_owned(),
        InputType::Text | InputType::Number | InputType::Integer | InputType::EnumFrom => {
            String::new()
        }
    };
    if !how.is_empty() {
        text.push_str(". ");
        text.push_str(&how);
    }
    text
}

/// `prompts/list`: one prompt per workflow.
pub fn list(workflows: &[Loaded]) -> Vec<Prompt> {
    workflows
        .iter()
        .map(|loaded| {
            let workflow = &loaded.workflow;
            let arguments = inputs_in_order(workflow)
                .into_iter()
                .filter_map(|name| {
                    let input = workflow.input(name)?;
                    Some(
                        PromptArgument::new(name)
                            .with_description(describe(workflow, name))
                            .with_required(!input.optional),
                    )
                })
                .collect();
            Prompt::new(
                name_of(&workflow.id),
                Some(workflow.summary.clone()),
                Some(arguments),
            )
            .with_title(workflow.title.clone())
        })
        .collect()
}

/// `prompts/get`: the workflow's method, then the task built from the
/// arguments. A missing required argument, one that is not text or is too
/// long, and one the workflow has no input for are refused, naming it.
pub fn get(
    workflows: &[Loaded],
    name: &str,
    arguments: Option<&JsonObject>,
) -> Result<GetPromptResult, String> {
    let Some(loaded) = workflows.iter().find(|w| name_of(&w.workflow.id) == name) else {
        let shown: String = name.chars().take(64).collect();
        return Err(format!("unknown prompt {shown:?}; prompts/list names them"));
    };
    let workflow = &loaded.workflow;
    if let Some(unknown) = arguments
        .into_iter()
        .flat_map(|a| a.keys())
        .find(|key| workflow.input(key).is_none())
    {
        let shown: String = unknown.chars().take(64).collect();
        return Err(format!(
            "{name} has no argument {shown:?}; its arguments: {}",
            inputs_in_order(workflow).join(", ")
        ));
    }
    let mut task = format!("## Task\n\n{}\n", workflow.summary);
    let mut missing = Vec::new();
    let mut first_text = None;
    for key in inputs_in_order(workflow) {
        let Some(input) = workflow.input(key) else {
            continue;
        };
        let text = match arguments.and_then(|a| a.get(key)) {
            None | Some(Value::Null) => "",
            Some(Value::String(text)) => text.trim(),
            Some(_) => return Err(format!("argument {key} must be text")),
        };
        let limit = match input.ty {
            InputType::Text => input
                .max
                .as_ref()
                .and_then(serde_json::Number::as_u64)
                .map_or(MAX_ARGUMENT, |m| usize::try_from(m).unwrap_or(MAX_ARGUMENT))
                .min(MAX_ARGUMENT),
            _ => MAX_ARGUMENT,
        };
        if text.len() > limit {
            return Err(format!("argument {key} is over {limit} bytes"));
        }
        if text.is_empty() {
            if !input.optional {
                missing.push(key);
            }
            continue;
        }
        if input.ty == InputType::Files {
            task.push_str(&format!("\n{}:\n", input.title));
            for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                task.push_str(&format!("- {line}\n"));
            }
        } else {
            if input.ty == InputType::Text && first_text.is_none() {
                first_text = Some(text.to_owned());
            }
            task.push_str(&format!("\n{}: {text}\n", input.title));
        }
    }
    if !missing.is_empty() {
        return Err(format!("give {}", missing.join(" and ")));
    }
    let text = format!("{}\n{task}", loaded.method.trim_end());
    let description = match first_text {
        Some(subject) => format!("{}: {subject}", workflow.title),
        None => workflow.title.clone(),
    };
    Ok(
        GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
            .with_description(description),
    )
}
