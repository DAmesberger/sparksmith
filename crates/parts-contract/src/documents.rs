//! The catalog's documents: the index, its shards, search replies and
//! bundles. `C`, `H` and `B` are the core's `catalog::SourceCatalog`,
//! `catalog::Hit` and `library::bundle::Bundle`; a program that only passes
//! them on may use any type that keeps their JSON.
use crate::hash::bytes_hash;
use crate::ids::{ContentHash, LibraryId, SourceLineageId};
use crate::limits::{
    MAX_BUNDLE_BYTES, MAX_INDEX_BYTES, MAX_SEARCH_BYTES, MAX_SHARD_BYTES, MAX_SHARDS,
};
use crate::version::FORMAT;
use crate::{Error, Result, decode, lenient};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// `GET /v1/index`: the current snapshot and its shards.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Index {
    pub format: u32,
    pub snapshot: String,
    pub shards: Vec<ShardInfo>,
}

/// One shard the index names.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShardInfo {
    pub library: LibraryId,
    pub name: String,
    /// SHA-256 of the shard's exact bytes.
    pub sha256: ContentHash,
    pub entries: usize,
}

/// `GET /v1/shards/{sha256}`: one library's catalog and what the parts
/// library says about each root.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Shard<C> {
    pub format: u32,
    pub catalog: C,
    #[serde(default = "Vec::new")]
    pub entries: Vec<ShardEntry>,
}

/// The facts about one root lineage of a shard.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ShardEntry {
    pub lineage: SourceLineageId,
    /// The entry's id (`publisher/name`).
    pub id: String,
    /// A review word ([`Review`]).
    pub review: String,
    /// Sent by the server; its meaning is not stated, and a client does
    /// not read it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glb: Option<String>,
}

/// What a review word says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Review {
    /// `reviewed`: a person has reviewed the entry.
    Reviewed,
    /// `in_review`.
    InReview,
    /// `unreviewed` and every word a reader does not know.
    NotReviewed,
}

impl Review {
    pub fn of(word: &str) -> Self {
        match word {
            "reviewed" => Self::Reviewed,
            "in_review" => Self::InReview,
            _ => Self::NotReviewed,
        }
    }
    /// The word a server writes.
    pub fn word(self) -> &'static str {
        match self {
            Self::Reviewed => "reviewed",
            Self::InReview => "in_review",
            Self::NotReviewed => "unreviewed",
        }
    }
}

/// Entries hidden from results, by reason. They are counted, never dropped
/// silently.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Hidden {
    pub reason: String,
    pub count: usize,
}

/// `GET /v1/search`: one page of ranked rows. A reader needs `total` and
/// `results`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchReply<H> {
    /// [`FORMAT`].
    pub format: u32,
    #[serde(default)]
    pub snapshot: String,
    /// The text searched for; a v1 client does not read it.
    #[serde(default, deserialize_with = "lenient")]
    pub query: String,
    /// The parsed terms.
    #[serde(default)]
    pub chips: Vec<String>,
    /// Matching entries, before the limit.
    pub total: usize,
    /// Rows before this page; a v1 client does not read it.
    #[serde(default, deserialize_with = "lenient")]
    pub offset: u64,
    #[serde(default)]
    pub hidden: Vec<Hidden>,
    pub results: Vec<SearchRow<H>>,
    /// The search's own time, in microseconds.
    #[serde(default)]
    pub took_us: u64,
}

/// One row of a search reply.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchRow<H> {
    pub hit: H,
    #[serde(default)]
    pub id: String,
    /// A review word ([`Review`]).
    #[serde(default)]
    pub review: String,
}

/// `GET /v1/bundles/{lineage}/{revision}`: a root and its closure.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BundleReply<B> {
    pub format: u32,
    pub bundle: B,
}

/// Decode an index, refusing another format and too many shards.
pub fn decode_index(bytes: &[u8]) -> Result<Index> {
    let index: Index = decode(bytes, MAX_INDEX_BYTES, "the registry index")?;
    if index.format != FORMAT {
        return Err(Error::Invalid(format!(
            "registry index format {} is not supported ({FORMAT} is)",
            index.format
        )));
    }
    if index.shards.len() > MAX_SHARDS {
        return Err(Error::TooLarge(format!(
            "a registry index lists at most {MAX_SHARDS} shards"
        )));
    }
    Ok(index)
}

/// Check a shard's bytes against the hash the index gave and decode it.
pub fn decode_shard<C: DeserializeOwned>(bytes: &[u8], expected: ContentHash) -> Result<Shard<C>> {
    if bytes.len() as u64 > MAX_SHARD_BYTES {
        return Err(Error::TooLarge(format!(
            "registry shard {expected} exceeds {MAX_SHARD_BYTES} bytes"
        )));
    }
    let actual = bytes_hash(bytes);
    if actual != expected {
        return Err(Error::Invalid(format!(
            "registry shard {expected} does not match its hash (it hashes to {actual})"
        )));
    }
    parse_shard(bytes)
}

/// Decode a shard whose bytes were already checked against its hash.
pub fn parse_shard<C: DeserializeOwned>(bytes: &[u8]) -> Result<Shard<C>> {
    let shard: Shard<C> = decode(bytes, MAX_SHARD_BYTES, "a registry shard")?;
    if shard.format != FORMAT {
        return Err(Error::Invalid(format!(
            "registry shard format {} is not supported ({FORMAT} is)",
            shard.format
        )));
    }
    Ok(shard)
}

/// Decode a search reply, refusing another format.
pub fn decode_search<H: DeserializeOwned>(bytes: &[u8]) -> Result<SearchReply<H>> {
    let reply: SearchReply<H> = decode(bytes, MAX_SEARCH_BYTES, "a search reply")?;
    if reply.format != FORMAT {
        return Err(Error::Invalid(format!(
            "registry search reply format {} is not supported ({FORMAT} is)",
            reply.format
        )));
    }
    Ok(reply)
}

/// Decode a bundle reply, refusing another format. The bundle itself is
/// checked by whoever uses it: its root, its records' hashes against the
/// shard's, its assets' against their names.
pub fn decode_bundle<B: DeserializeOwned>(bytes: &[u8]) -> Result<B> {
    let reply: BundleReply<B> = decode(bytes, MAX_BUNDLE_BYTES, "a bundle reply")?;
    if reply.format != FORMAT {
        return Err(Error::Invalid(format!(
            "registry bundle format {} is not supported",
            reply.format
        )));
    }
    Ok(reply.bundle)
}
