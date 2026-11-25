//! The wire contract between Sparksmith and an online parts library: what
//! the client and the server both build against, so that each shape, limit
//! and hash is written once (`docs/parts-contract.md`).
//!
//! Nothing here does I/O, opens a connection, reads a clock or waits. The
//! documents a library's parts are made of (a catalog, a search hit, a
//! bundle) are the core's; the wire types around them are generic over
//! them, so a server that only stores and serves them needs no more than
//! this crate, and one that searches or checks them names the core's types.
//!
//! | Module | What it fixes |
//! | --- | --- |
//! | [`version`] | the contract's version and what may change within it |
//! | [`route`] | every route: method, path, query, sign-in, limits, caching |
//! | [`documents`] | index, shard, search reply, bundle reply |
//! | [`account`] | device sign-in, account, token |
//! | [`status`] | statuses, error bodies and their codes |
//! | [`limits`] | every bound, as data |
//! | [`ids`], [`hash`] | identities, content hashes, asset text |
//! | `vectors`, `conformance` | example documents and the server's test (feature `test-fixtures`) |
#![forbid(unsafe_code)]
pub mod account;
pub mod documents;
pub mod hash;
pub mod ids;
pub mod limits;
pub mod route;
pub mod status;
pub mod version;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "test-fixtures"))]
pub mod conformance;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod vectors;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer};
use std::fmt;

/// Why a document was refused.
#[derive(Debug)]
pub enum Error {
    /// Over one of [`limits`].
    TooLarge(String),
    /// Read, and not what it must be.
    Invalid(String),
    /// Not the JSON it must be.
    Unreadable(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(message) | Self::Invalid(message) => f.write_str(message),
            Self::Unreadable(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Unreadable(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Decode `bytes`, at most `limit` of them, as `what`. Fields the type does
/// not name are ignored ([`version`]).
pub(crate) fn decode<T: DeserializeOwned>(bytes: &[u8], limit: u64, what: &str) -> Result<T> {
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge(format!("{what} exceeds {limit} bytes")));
    }
    Ok(serde_json::from_slice(bytes)?)
}

/// A field a reader only passes on or ignores: any value of another type
/// reads as absent, so it never costs the rest of the document.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}
