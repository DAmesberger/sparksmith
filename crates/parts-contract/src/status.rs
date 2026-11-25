//! Statuses, error bodies and their codes.
//!
//! | Status | Meaning | The client |
//! | --- | --- | --- |
//! | 2xx | done | reads the body |
//! | 400 | refused; on a poll, not approved yet ([`codes`]) | shows the message |
//! | 401 on a public route | the server is closed to the public | says so; never asks for a sign-in |
//! | 401 on a route with a bearer | no sign-in, or a token the server does not know | forgets the token, asks to sign in |
//! | 404 | no such thing | shows the message |
//! | 429 | too many requests | says when to try again ([`retry_after`]); a poll slows down |
//! | any other from 300 | refused | shows the message, else the status |
//!
//! A redirect is a refusal: the client never follows one, so a bearer never
//! leaves the server it was issued by.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a server answers a request it refuses with.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorBody {
    pub error: ErrorInfo,
    /// With 429: seconds until the next try is welcome, as `Retry-After`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ErrorInfo {
    /// One of [`codes`], or one this version does not know.
    pub code: String,
    /// For the person, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// The error codes of this version. A reader treats any other by its status.
pub mod codes {
    /// 401: no token, or one the server does not know.
    pub const AUTH_REQUIRED: &str = "auth.required";
    /// 400: a search's text was not understood.
    pub const CATALOG_QUERY: &str = "catalog.query";
    /// 404: no such route, shard, lineage or revision.
    pub const NOT_FOUND: &str = "registry.not_found";
    /// 429.
    pub const RATE_LIMITED: &str = "rate.limited";
    /// 400 on a poll: not confirmed yet.
    pub const DEVICE_PENDING: &str = "device.pending";
    /// 400 on a poll: polled too fast.
    pub const DEVICE_SLOW_DOWN: &str = "device.slow_down";
    /// 400 on a poll: the code is no longer valid.
    pub const DEVICE_EXPIRED: &str = "device.expired";
    /// 400 on a poll: declined.
    pub const DEVICE_DENIED: &str = "device.denied";
}

impl ErrorBody {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            error: ErrorInfo {
                code: code.into(),
                message: Some(message.into()),
            },
            retry_after: None,
        }
    }

    /// What can be read of a refusal's body: any part that is missing or of
    /// another type is absent, and a body that is not JSON says nothing. The
    /// caller has bounded `bytes` by its route's limit.
    pub fn read(bytes: &[u8]) -> Self {
        let value: Value = serde_json::from_slice(bytes).unwrap_or_default();
        let text = |field: &str| value["error"][field].as_str().map(str::to_owned);
        Self {
            error: ErrorInfo {
                code: text("code").unwrap_or_default(),
                message: text("message"),
            },
            retry_after: value["retry_after"].as_u64(),
        }
    }

    /// The bytes a server sends.
    pub fn bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }
}

/// Seconds of a `Retry-After` header: the number form only.
pub fn retry_after(header: &str) -> Option<u64> {
    header.trim().parse().ok()
}
