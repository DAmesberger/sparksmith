//! Committed example documents and their hashes, for both sides' tests. The
//! documents are written by the core, which a test of the client holds them
//! to; a server's test reads and writes the same bytes.
use crate::ids::ContentHash;
use serde::{Deserialize, Serialize};

pub const INDEX: &[u8] = include_bytes!("../vectors/index.json");
pub const SHARD: &[u8] = include_bytes!("../vectors/shard.json");
pub const SEARCH: &[u8] = include_bytes!("../vectors/search.json");
pub const BUNDLE: &[u8] = include_bytes!("../vectors/bundle.json");
pub const CODE_REQUEST: &[u8] = include_bytes!("../vectors/code-request.json");
pub const CODE: &[u8] = include_bytes!("../vectors/code.json");
pub const TOKEN_REQUEST: &[u8] = include_bytes!("../vectors/token-request.json");
pub const TOKEN: &[u8] = include_bytes!("../vectors/token.json");
pub const ACCOUNT: &[u8] = include_bytes!("../vectors/account.json");
pub const RATE_LIMITED: &[u8] = include_bytes!("../vectors/rate-limited.json");
pub const CASES: &[u8] = include_bytes!("../vectors/cases.json");

/// Hashes and request lines both sides must agree on.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cases {
    /// SHA-256 of [`SHARD`], its name.
    pub shard: ContentHash,
    /// The content hash of every record of [`BUNDLE`], in order.
    pub records: Vec<ContentHash>,
    /// The hash of every asset of [`BUNDLE`], in order.
    pub assets: Vec<ContentHash>,
    /// Request lines that are routes, each exactly as a client writes it.
    pub routes: Vec<(String, String)>,
    /// Request lines that are none.
    pub refused: Vec<(String, String)>,
}

pub fn cases() -> serde_json::Result<Cases> {
    serde_json::from_slice(CASES)
}
