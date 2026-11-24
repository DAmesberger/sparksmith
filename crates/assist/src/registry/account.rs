//! Signing in to the online library (the registry's accounts contract:
//! device authorization, bearer tokens, sign-out). Searching, the index
//! and its shards need no account; downloading part data does, so the
//! client sends the token on bundle requests only, and only to the
//! registry it was issued by: the token is kept in the system keychain
//! under [`SERVICE`] for the registry's base URL, redirects are never
//! followed, and plain HTTP carries it only to this computer.
//!
//! The token is a [`Secret`]: never written to a file, a log, an error or
//! a recording. The device code that can be exchanged for it is one too.
use super::{RegistryClient, Reply, Request, unreachable};
use crate::model::Secret;
use crate::model::keys::{Keychain, Store};
use serde::Deserialize;
use serde_json::Value;
use sparksmith_automation::session::online::{OnlineAccount, SignOut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The keychain service the online library's sign-in is kept under; the
/// entry is the registry's base URL.
pub const SERVICE: &str = "sparksmith-parts";
/// A download refused for want of a sign-in.
pub const SIGN_IN_NEEDED: &str = "Sign in to the online library in Settings to download parts. \
     Searching works without an account.";
/// Characters of the client name the registry shows on the account page.
pub const MAX_CLIENT_NAME: usize = 80;
/// How long one account request may take.
const ACCOUNT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long revoking the token may take before sign-out goes on without.
const REVOKE_TIMEOUT: Duration = Duration::from_secs(5);
/// Bytes of an account or device reply.
const MAX_ACCOUNT_BYTES: u64 = 64 * 1024;
/// What `slow_down` adds to the polling interval (the contract's 5 s).
const SLOW_DOWN: Duration = Duration::from_secs(5);

/// The account a sign-in belongs to.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Account {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub display_name: String,
}

impl Account {
    /// How Settings names it: the display name, else the email.
    pub fn label(&self) -> &str {
        if self.display_name.trim().is_empty() {
            &self.email
        } else {
            &self.display_name
        }
    }
}

/// A sign-in code the person confirms in the browser.
#[derive(Clone, Debug)]
pub struct DeviceCode {
    device_code: Secret,
    /// What the person types or checks ("BCDF-GHJK").
    pub user_code: String,
    pub pages: VerificationPages,
    /// Seconds the code is valid.
    pub expires_in: u64,
    /// Seconds between polls.
    pub interval: u64,
}

/// Both addresses use the browser URL parser. The button names the origin
/// of the very URL it opens, including a separate identity provider.
#[derive(Clone, Debug)]
pub struct VerificationPages {
    manual: url::Url,
    browser: url::Url,
}

impl VerificationPages {
    pub(super) fn parse(manual: &str, complete: &str) -> Result<Self, String> {
        fn page(input: &str) -> Result<url::Url, String> {
            const INVALID: &str = "The registry supplied an invalid sign-in address.";
            let url = url::Url::parse(input).map_err(|_| INVALID)?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(INVALID.into());
            }
            let origin = url.origin().ascii_serialization();
            let endpoint = crate::model::Endpoint::parse(&origin).map_err(|_| INVALID)?;
            if !endpoint.allows_credentials() {
                return Err("The registry sign-in page needs HTTPS or a loopback address.".into());
            }
            Ok(url)
        }
        let manual = page(manual)?;
        let browser = if complete.is_empty() {
            manual.clone()
        } else {
            page(complete)?
        };
        Ok(Self { manual, browser })
    }

    pub fn manual(&self) -> &url::Url {
        &self.manual
    }
    pub fn browser(&self) -> &url::Url {
        &self.browser
    }

    pub fn button_label(&self) -> String {
        let destination = self.browser.origin();
        let origin = destination.ascii_serialization();
        if self.manual.origin() == destination {
            format!("Open {origin}")
        } else {
            format!("Open identity provider {origin}")
        }
    }
}

#[derive(Deserialize)]
struct WireCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: String,
    expires_in: u64,
    #[serde(default)]
    interval: u64,
}

#[derive(Deserialize)]
struct WireToken {
    access_token: String,
    #[serde(default)]
    account: Account,
}

/// One poll of a sign-in code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    /// Not confirmed yet.
    Pending,
    /// Polled too fast: wait longer.
    SlowDown,
    /// Confirmed: the token is kept.
    Approved(Account),
    Expired,
    Denied,
}

/// A sign-in that ended without one.
pub const SIGN_IN_EXPIRED: &str =
    "The sign-in code expired before it was confirmed. Sign in again.";
pub const SIGN_IN_DENIED: &str = "The sign-in was declined in the browser.";
pub const SIGN_IN_CANCELLED: &str = "Sign-in cancelled.";

/// What this computer knows of the sign-in, without the keychain or the
/// network: what Settings draws each frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Known {
    SignedOut,
    /// Signed in; the account once the registry said it.
    SignedIn(Option<Account>),
}

/// The sign-in state a client keeps.
pub(super) struct Auth {
    store: Arc<dyn Store>,
    /// The token: `None` until the keychain was read.
    token: Mutex<Option<Option<Secret>>>,
    account: Mutex<Option<Account>>,
}

impl Auth {
    pub(super) fn system() -> Self {
        Self::with(Arc::new(Keychain))
    }

    pub(super) fn with(store: Arc<dyn Store>) -> Self {
        Self {
            store,
            token: Mutex::new(None),
            account: Mutex::new(None),
        }
    }
}

/// "Sparksmith on <host name>", at most [`MAX_CLIENT_NAME`] characters;
/// the product name alone when the host name says nothing usable.
pub fn client_name() -> String {
    client_name_for(&gethostname::gethostname().to_string_lossy())
}

pub(crate) fn client_name_for(host: &str) -> String {
    let product = sparksmith_core::product::NAME;
    let host = host.trim();
    let host = host.strip_suffix(".local").unwrap_or(host);
    let host: String = host
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ' '))
        .collect();
    let host = host.trim();
    if host.is_empty() {
        return product.chars().take(MAX_CLIENT_NAME).collect();
    }
    format!("{product} on {host}")
        .chars()
        .take(MAX_CLIENT_NAME)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// "in 40 seconds", "in 5 minutes", "in 3 hours".
fn retry_in(seconds: u64) -> String {
    let (n, unit) = match seconds {
        0..=89 => (seconds.max(1), "second"),
        90..=5399 => (seconds.div_ceil(60), "minute"),
        _ => (seconds.div_ceil(3600), "hour"),
    };
    format!("in {n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// A 429's words: when to try again.
pub(super) fn rate_limited(reply: &Reply) -> String {
    let after = reply.retry_after.or_else(|| {
        serde_json::from_slice::<Value>(&reply.bytes)
            .ok()
            .and_then(|v| v["retry_after"].as_u64())
    });
    match after {
        Some(seconds) => format!(
            "The online library's download limit is reached. Try again {}.",
            retry_in(seconds)
        ),
        None => "The online library's download limit is reached. Try again later.".into(),
    }
}

/// The error code of a refusal (`device.pending`).
fn error_code(reply: &Reply) -> String {
    serde_json::from_slice::<Value>(&reply.bytes)
        .ok()
        .and_then(|v| v["error"]["code"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Sleep `wait` in short steps; whether `cancel` was set meanwhile.
pub fn sleep_unless(cancel: &AtomicBool, wait: Duration) -> bool {
    let step = Duration::from_millis(100);
    let mut left = wait;
    while !left.is_zero() {
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        let now = left.min(step);
        std::thread::sleep(now);
        left -= now;
    }
    cancel.load(Ordering::SeqCst)
}

impl RegistryClient {
    /// Whether a token may travel to this registry: HTTPS, or plain HTTP
    /// to this computer.
    pub fn may_sign_in(&self) -> bool {
        self.endpoint.allows_credentials()
    }

    /// The kept token, read from the keychain once.
    pub(super) fn token(&self) -> Option<Secret> {
        if !self.may_sign_in() {
            return None;
        }
        let Ok(mut held) = self.auth.token.lock() else {
            return None;
        };
        if held.is_none() {
            // A keychain that cannot be read counts as signed out.
            let read = self
                .auth
                .store
                .get(SERVICE, self.endpoint.base())
                .ok()
                .flatten()
                .and_then(|text| Secret::new(&text));
            *held = Some(read);
        }
        held.clone().flatten()
    }

    /// Forget the token here and in the keychain.
    fn forget(&self) -> Result<bool, String> {
        if let Ok(mut held) = self.auth.token.lock() {
            *held = Some(None);
        }
        if let Ok(mut account) = self.auth.account.lock() {
            *account = None;
        }
        self.auth
            .store
            .delete(SERVICE, self.endpoint.base())
            .map_err(|e| e.to_string())
    }

    /// A download the registry refused because the token it was sent is
    /// unknown, revoked or expired: forget it.
    pub(super) fn token_refused(&self) {
        let _ = self.forget();
    }

    /// What this computer knows of the sign-in, without reading the
    /// keychain: `None` before it was first read.
    pub fn known(&self) -> Option<Known> {
        let token = self.auth.token.lock().ok()?.as_ref()?.is_some();
        if !token {
            return Some(Known::SignedOut);
        }
        let account = self.auth.account.lock().ok().and_then(|a| a.clone());
        Some(Known::SignedIn(account))
    }

    /// Who is signed in: the keychain's token, and the account the
    /// registry last named (no network).
    pub fn account_status(&self) -> OnlineAccount {
        if self.token().is_none() {
            return OnlineAccount::default();
        }
        let account = self.auth.account.lock().ok().and_then(|a| a.clone());
        let nonblank = |text: &str| (!text.trim().is_empty()).then(|| text.to_owned());
        OnlineAccount {
            signed_in: true,
            email: account.as_ref().and_then(|a| nonblank(&a.email)),
            display_name: account.as_ref().and_then(|a| nonblank(&a.display_name)),
        }
    }

    /// Read the kept token and ask the registry whose it is
    /// (`GET /v1/account`). A token the registry no longer knows is
    /// forgotten. `None`: signed out.
    pub fn refresh_account(&self) -> Result<Option<Account>, String> {
        let Some(token) = self.token() else {
            return Ok(None);
        };
        let reply = self.send(
            Request::Read(Some(&token)),
            "/v1/account",
            ACCOUNT_TIMEOUT,
            MAX_ACCOUNT_BYTES,
        )?;
        match reply.status {
            200..=299 => {
                let account: Account = serde_json::from_slice(&reply.bytes)
                    .map_err(|_| "the registry's account reply is unreadable")?;
                if let Ok(mut held) = self.auth.account.lock() {
                    *held = Some(account.clone());
                }
                Ok(Some(account))
            }
            401 => {
                self.forget()?;
                Ok(None)
            }
            _ => Err(self.refusal(&reply)),
        }
    }

    /// Ask for a sign-in code (`POST /v1/device/code`) under `client_name`.
    pub fn start_sign_in(&self, client_name: &str) -> Result<DeviceCode, String> {
        let name: String = client_name.chars().take(MAX_CLIENT_NAME).collect();
        let reply = self.send(
            Request::SignIn(&name),
            "/v1/device/code",
            ACCOUNT_TIMEOUT,
            MAX_ACCOUNT_BYTES,
        )?;
        if reply.status >= 300 {
            return Err(self.refusal(&reply));
        }
        let wire: WireCode = serde_json::from_slice(&reply.bytes)
            .map_err(|_| "the registry's sign-in code is unreadable")?;
        let device_code =
            Secret::new(&wire.device_code).ok_or("the registry sent an empty sign-in code")?;
        let pages =
            VerificationPages::parse(&wire.verification_uri, &wire.verification_uri_complete)?;
        Ok(DeviceCode {
            device_code,
            user_code: wire.user_code.chars().take(32).collect(),
            pages,
            expires_in: wire.expires_in,
            interval: wire.interval,
        })
    }

    /// Poll a sign-in code once (`POST /v1/device/token`). Approved, the
    /// token goes to the keychain before this returns.
    pub fn poll_sign_in(&self, code: &DeviceCode) -> Result<Poll, String> {
        let reply = self.send(
            Request::Poll(&code.device_code),
            "/v1/device/token",
            ACCOUNT_TIMEOUT,
            MAX_ACCOUNT_BYTES,
        )?;
        match reply.status {
            200..=299 => {
                let wire: WireToken = serde_json::from_slice(&reply.bytes)
                    .map_err(|_| "the registry's sign-in reply is unreadable".to_owned())?;
                let token =
                    Secret::new(&wire.access_token).ok_or("the registry sent an empty sign-in")?;
                self.auth
                    .store
                    .set(SERVICE, self.endpoint.base(), token.expose())
                    .map_err(|e| {
                        token.redact(&format!(
                            "The system keychain did not keep the sign-in: {e}"
                        ))
                    })?;
                if let Ok(mut held) = self.auth.token.lock() {
                    *held = Some(Some(token));
                }
                if let Ok(mut held) = self.auth.account.lock() {
                    *held = Some(wire.account.clone());
                }
                Ok(Poll::Approved(wire.account))
            }
            429 => Ok(Poll::SlowDown),
            _ => match error_code(&reply).as_str() {
                "device.pending" => Ok(Poll::Pending),
                "device.slow_down" => Ok(Poll::SlowDown),
                "device.expired" => Ok(Poll::Expired),
                "device.denied" => Ok(Poll::Denied),
                _ => Err(self.refusal(&reply)),
            },
        }
    }

    /// Poll `code` until the person confirms or declines it, it expires
    /// (the registry's `expires_in`) or `cancel` is set. `sleep` waits between polls and says whether `cancel` was
    /// set meanwhile ([`sleep_unless`]). The interval is the registry's,
    /// longer by 5 s after each `slow_down`; a registry that cannot be
    /// reached for a moment is polled again.
    pub fn wait_for_sign_in(
        &self,
        code: &DeviceCode,
        cancel: &AtomicBool,
        sleep: &dyn Fn(&AtomicBool, Duration) -> bool,
    ) -> Result<Account, String> {
        let limit = Duration::from_secs(code.expires_in);
        let mut interval = Duration::from_secs(code.interval.max(1));
        let mut waited = Duration::ZERO;
        loop {
            if waited >= limit {
                return Err(SIGN_IN_EXPIRED.into());
            }
            if sleep(cancel, interval) || cancel.load(Ordering::SeqCst) {
                return Err(SIGN_IN_CANCELLED.into());
            }
            waited = waited.saturating_add(interval);
            match self.poll_sign_in(code) {
                Ok(Poll::Pending) => {}
                Ok(Poll::SlowDown) => interval = interval.saturating_add(SLOW_DOWN),
                Ok(Poll::Approved(account)) => return Ok(account),
                Ok(Poll::Expired) => return Err(SIGN_IN_EXPIRED.into()),
                Ok(Poll::Denied) => return Err(SIGN_IN_DENIED.into()),
                Err(error) if unreachable(&error) => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// Sign out: revoke the token at the registry (`DELETE
    /// /v1/auth/token`, best effort: offline it stays valid until revoked
    /// on the account page) and remove it from the keychain.
    pub fn sign_out(&self) -> Result<SignOut, String> {
        let Some(token) = self.token() else {
            self.forget()?;
            return Ok(SignOut::default());
        };
        let revoked = self.send(
            Request::Revoke(&token),
            "/v1/auth/token",
            REVOKE_TIMEOUT,
            MAX_ACCOUNT_BYTES,
        );
        let (revoked, problem) = match revoked {
            // 401: the registry already does not know it.
            Ok(reply) if reply.status < 300 || reply.status == 401 => (true, None),
            Ok(reply) => (false, Some(self.refusal(&reply))),
            Err(error) => (false, Some(error)),
        };
        self.forget()?;
        Ok(SignOut {
            was_signed_in: true,
            revoked,
            problem,
        })
    }
}
