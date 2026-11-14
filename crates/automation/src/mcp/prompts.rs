//! MCP prompts: workflows any client can start. Each is a skill file in the
//! repository's `skills/` folder, served without its front matter, so an
//! agent reading the skill and one using the prompt follow the same text.
//! The skills extension and `describe {name: "guide:…"}` come with P4.
use rmcp::model::{GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role};
use serde_json::Value;

/// The datasheet-to-component workflow (`skills/sparksmith-datasheet-part`).
pub const DATASHEET_PART_SKILL: &str =
    include_str!("../../../../skills/sparksmith-datasheet-part/SKILL.md");
/// Its prompt name.
pub const DATASHEET_PART: &str = "datasheet_to_component";
/// Bytes of one prompt argument.
const MAX_ARGUMENT: usize = 16 * 1024;

/// A skill file's body: the text after its `---` front matter.
pub fn skill_body(skill: &str) -> &str {
    skill
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(skill, |(_, body)| body)
        .trim_start()
}

/// `prompts/list`.
pub fn list() -> Vec<Prompt> {
    let argument = |name: &str, description: &str, required: bool| {
        PromptArgument::new(name)
            .with_description(description)
            .with_required(required)
    };
    vec![
        Prompt::new(
            DATASHEET_PART,
            Some("Author one component (pins and functions, symbol, package, footprint, binding, 3D model) from its manufacturer datasheets as drafts, with every value cited and checked by library.review_check, for a person to review and publish."),
            Some(vec![
                argument(
                    "part",
                    "The part name the drafts take, e.g. its manufacturer part number.",
                    true,
                ),
                argument(
                    "datasheets",
                    concat!(
                        "The datasheets: file paths or URLs, one per line. You read them; ",
                        sparksmith_core::product_name!(),
                        " does not."
                    ),
                    true,
                ),
                argument(
                    "notes",
                    "Anything else: which variant or package, what to include or leave out.",
                    false,
                ),
            ]),
        )
        .with_title("Create a component from datasheets"),
    ]
}

/// `prompts/get`: the workflow, then the task built from the arguments.
pub fn get(name: &str, arguments: Option<&JsonObject>) -> Result<GetPromptResult, String> {
    if name != DATASHEET_PART {
        let shown: String = name.chars().take(64).collect();
        return Err(format!("unknown prompt {shown:?}; prompts/list names them"));
    }
    let argument = |key: &str| -> Result<String, String> {
        let text = match arguments.and_then(|a| a.get(key)) {
            None | Some(Value::Null) => "",
            Some(Value::String(text)) => text.trim(),
            Some(_) => return Err(format!("argument {key} must be text")),
        };
        if text.len() > MAX_ARGUMENT {
            return Err(format!("argument {key} is over {MAX_ARGUMENT} bytes"));
        }
        Ok(text.to_owned())
    };
    let part = argument("part")?;
    let datasheets = argument("datasheets")?;
    let notes = argument("notes")?;
    if part.is_empty() || datasheets.is_empty() {
        return Err("give part and datasheets".into());
    }
    let mut task = format!("## Task\n\nCreate the part \"{part}\" from these datasheets:\n\n");
    for line in datasheets.lines().map(str::trim).filter(|l| !l.is_empty()) {
        task.push_str(&format!("- {line}\n"));
    }
    if !notes.is_empty() {
        task.push_str(&format!("\nNotes: {notes}\n"));
    }
    let text = format!("{}\n{task}", skill_body(DATASHEET_PART_SKILL));
    Ok(
        GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
            .with_description(format!("Create {part} from its datasheets")),
    )
}
