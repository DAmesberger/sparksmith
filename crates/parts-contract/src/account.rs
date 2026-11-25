//! Signing in: a device code the person confirms in a browser, the token it
//! becomes, the account it belongs to and its revocation. The token is a
//! bearer, sent only where [`crate::route::Auth`] says.
//!
//! The client asks for a code, shows `user_code` and opens
//! `verification_uri_complete` (else `verification_uri`), then polls at
//! `interval` seconds, [`SLOW_DOWN_SECONDS`] longer after each "slow down",
//! until it is approved, declined or `expires_in` seconds have passed.
use crate::limits::{MAX_ACCOUNT_BYTES, MAX_CLIENT_NAME, MAX_USER_CODE};
use crate::status::{ErrorBody, codes};
use crate::{Error, Result, decode, lenient};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Seconds each "slow down" adds to the polling interval.
pub const SLOW_DOWN_SECONDS: u64 = 5;
/// Seconds between polls when the server names none.
pub const MIN_INTERVAL_SECONDS: u64 = 1;

/// A token or a device code: never in a debug output.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Token(pub String);

impl Token {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token([redacted])")
    }
}

/// The `Authorization` header of `token`.
pub fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// The token of an `Authorization` header.
pub fn read_bearer(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// `POST /v1/device/code`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCodeRequest {
    /// Who signs in, as the account page shows it; at most
    /// [`MAX_CLIENT_NAME`] characters.
    pub client_name: String,
}

/// The code a sign-in starts with.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceCodeReply {
    /// What the client polls with: a secret.
    pub device_code: Token,
    /// What the person types or checks.
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: String,
    /// Seconds the code is valid.
    pub expires_in: u64,
    /// Seconds between polls.
    #[serde(default)]
    pub interval: u64,
}

impl DeviceCodeReply {
    /// The code as a client shows it.
    pub fn shown_code(&self) -> String {
        self.user_code.chars().take(MAX_USER_CODE).collect()
    }
}

/// `POST /v1/device/token`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceTokenRequest {
    pub device_code: Token,
}

/// An approved sign-in.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceTokenReply {
    pub access_token: Token,
    /// `bearer`; a v1 client does not read it.
    #[serde(default, deserialize_with = "lenient")]
    pub token_type: String,
    /// The token's name on the account page; a v1 client does not read it.
    #[serde(default, deserialize_with = "lenient")]
    pub token_id: String,
    #[serde(default)]
    pub account: Account,
}

/// `GET /v1/account`, and the account of an approved sign-in.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Account {
    /// A v1 client does not read it.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "String::is_empty"
    )]
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
    /// How the account signs in on the site; a v1 client does not read it.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub providers: Vec<String>,
}

impl Account {
    /// How it is named: the display name, else the email.
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.email
        } else {
            &self.display_name
        }
    }
}

/// One answer to `POST /v1/device/token`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DevicePoll {
    Approved(DeviceTokenReply),
    /// Not confirmed yet.
    Pending,
    /// Polled too fast.
    SlowDown,
    Expired,
    Denied,
    /// Refused for another reason: read the status and the body.
    Refused,
}

impl DevicePoll {
    /// Read an answer. 429 is "slow down" whatever its body says.
    pub fn read(status: u16, bytes: &[u8]) -> Result<Self> {
        Ok(match status {
            200..=299 => Self::Approved(decode(bytes, MAX_ACCOUNT_BYTES, "a sign-in reply")?),
            429 => Self::SlowDown,
            _ => match ErrorBody::read(bytes).error.code.as_str() {
                codes::DEVICE_PENDING => Self::Pending,
                codes::DEVICE_SLOW_DOWN => Self::SlowDown,
                codes::DEVICE_EXPIRED => Self::Expired,
                codes::DEVICE_DENIED => Self::Denied,
                _ => Self::Refused,
            },
        })
    }
}

/// Decode the request of a sign-in code (a server's side).
pub fn decode_code_request(bytes: &[u8]) -> Result<DeviceCodeRequest> {
    let request: DeviceCodeRequest = decode(bytes, MAX_ACCOUNT_BYTES, "a sign-in request")?;
    if request.client_name.chars().count() > MAX_CLIENT_NAME {
        return Err(Error::TooLarge(format!(
            "a client name has at most {MAX_CLIENT_NAME} characters"
        )));
    }
    Ok(request)
}

/// Decode a poll (a server's side).
pub fn decode_token_request(bytes: &[u8]) -> Result<DeviceTokenRequest> {
    decode(bytes, MAX_ACCOUNT_BYTES, "a sign-in poll")
}

/// Decode a sign-in code (the client's side).
pub fn decode_code(bytes: &[u8]) -> Result<DeviceCodeReply> {
    decode(bytes, MAX_ACCOUNT_BYTES, "a sign-in code")
}

/// Decode an account (the client's side).
pub fn decode_account(bytes: &[u8]) -> Result<Account> {
    decode(bytes, MAX_ACCOUNT_BYTES, "an account")
}
