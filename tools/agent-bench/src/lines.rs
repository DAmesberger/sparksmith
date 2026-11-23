//! The board script must read a matching find hit and signature before its
//! first edit. The rest of the scenario and its independent verifier are shared.
use serde_json::{Value, json};
use sparksmith_assist::{
    Error, Result,
    board_script::Scripted,
    model::{Message, ModelBackend, ModelReply, ModelRequest, Part},
};

pub(super) struct Board {
    board: Scripted,
    started: bool,
    discovery: Option<(String, String)>,
}
impl Board {
    pub(super) fn new() -> Self {
        Self {
            board: Scripted::board_build(),
            started: false,
            discovery: None,
        }
    }
}
fn delivered(messages: &[Message], id: &str) -> Result<Value> {
    let body = messages
        .iter()
        .rev()
        .flat_map(|message| message.content.iter())
        .find_map(|part| match part {
            Part::ToolResult { call_id, text, .. } if call_id == id => Some(text),
            _ => None,
        })
        .ok_or_else(|| Error::Protocol("missing catalog reply before board edits".into()))?;
    Ok(serde_json::from_str(body)?)
}
impl ModelBackend for Board {
    fn label(&self) -> String {
        self.board.label()
    }
    fn count_prompt_tokens(&mut self, request: &ModelRequest<'_>) -> Result<Option<u64>> {
        self.board.count_prompt_tokens(request)
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        if let Some((describe, find)) = self.discovery.take() {
            let described = delivered(request.messages, &describe)?;
            let found = delivered(request.messages, &find)?;
            let hit = &found["hits"][0];
            if described["line"].as_str().is_none_or(str::is_empty)
                || described.get("schema").is_some()
                || ["name", "line", "class", "person", "reach", "asks_with_path"]
                    .iter()
                    .any(|key| hit.get(*key) != described.get(*key))
            {
                return Err(Error::Protocol("the board script needs matching find and describe signatures before its first edit".into()));
            }
        }
        let mut reply = self.board.send(request)?;
        if !self.started {
            let (describe, entry) = reply
                .content
                .iter()
                .find_map(|part| match part {
                    Part::ToolCall {
                        name, id, input, ..
                    } if name == "describe" => {
                        Some((id.clone(), input["name"].as_str()?.to_owned()))
                    }
                    _ => None,
                })
                .ok_or_else(|| Error::Protocol("the first board step omitted describe".into()))?;
            let find = reply
                .content
                .iter_mut()
                .find_map(|part| match part {
                    Part::ToolCall {
                        name, id, input, ..
                    } if name == "find" => {
                        *input = json!({"text":entry,"limit":1});
                        Some(id.clone())
                    }
                    _ => None,
                })
                .ok_or_else(|| Error::Protocol("the first board step omitted find".into()))?;
            self.discovery = Some((describe, find));
            self.started = true;
        }
        Ok(reply)
    }
}
