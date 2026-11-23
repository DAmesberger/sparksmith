//! One developer trace row for a verb, whether called in process or via MCP.
//! The trace stores byte counts and a digest, leaving project contents out.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    InProcess,
    Mcp,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Call {
    pub transport: Transport,
    pub request: Option<u32>,
    pub call: Option<u32>,
    pub verb: String,
    pub entry: String,
    pub request_bytes: usize,
    pub reply_bytes: usize,
    pub visible_bytes: usize,
    pub result_sha256: String,
    pub server_ms: f64,
    pub ok: bool,
}

pub fn entry(args: &Value) -> String {
    args["name"]
        .as_str()
        .or(args["operations"][0]["op"].as_str())
        .unwrap_or_default()
        .to_owned()
}

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
