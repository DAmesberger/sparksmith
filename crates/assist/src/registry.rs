//! The online parts registry's client (registry R0 §5 and §10): search with
//! the app's own query text, the index and its shards for the offline
//! mirror, and bundles checked against the registry's content hashes
//! before anything uses them. This crate is the one with an HTTP client
//! (`cargo xtask layers`); the mirror's files are io's
//! ([`sparksmith_io::registry_mirror`]).
//!
//! [`RegistryClient`] calls block. [`RegistryWorker`] runs them off the UI
//! thread: searches are debounced, tagged with the caller's generation and
//! dropped when a newer one was asked for (before the request and again
//! when its reply arrives), and bundle reads run on their own thread so a
//! large download never holds up typing. Offline, search runs the core's
//! own index over the mirrored shards and says the index's date.
//!
//! Downloading part data needs a free account ([`account`]): bundle
//! requests carry the sign-in's token, searches and the catalog never do.
pub mod account;

use crate::model::{Endpoint, Secret};
use account::{Auth, DeviceCode};
use serde::Deserialize;
use sparksmith_automation::session::online::{
    OnlineAccount, OnlineHit, OnlineReport, OnlineSource, SignOut,
};
use sparksmith_core::catalog::{
    CatalogContext, Hidden, Hit, Index, SearchOptions, Source, SourceCatalog,
};
use sparksmith_core::ids::{LibraryId, Revision, SourceLineageId};
use sparksmith_core::library::Drafts;
use sparksmith_core::library::bundle::{Bundle, SourceRevision};
use sparksmith_core::model::{NewBoard, Project};
use sparksmith_io::registry_mirror::{self as mirror, Mirror, MirroredIndex, RegistryIndex, Shard};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Bytes of a search reply.
pub const MAX_SEARCH_BYTES: u64 = 4 * 1024 * 1024;
/// Bytes of a bundle reply: the core's packed-asset bound as base64, plus
/// its records.
pub const MAX_BUNDLE_BYTES: u64 = 160 * 1024 * 1024;
/// How long a search may take before it is given up.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(4);
/// How long the index or a shard may take.
pub const SYNC_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a bundle may take.
pub const BUNDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long typing must pause before a search is sent.
pub const DEBOUNCE: Duration = Duration::from_millis(120);

/// One reply row as the registry sends it.
#[derive(Deserialize)]
struct WireRow {
    hit: Hit,
    #[serde(default)]
    id: String,
    #[serde(default)]
    review: String,
}

/// `GET /v1/search` as the registry sends it.
#[derive(Deserialize)]
struct WireSearch {
    #[serde(default)]
    snapshot: String,
    #[serde(default)]
    chips: Vec<String>,
    total: usize,
    #[serde(default)]
    #[allow(dead_code)]
    hidden: Vec<Hidden>,
    results: Vec<WireRow>,
    #[serde(default)]
    took_us: u64,
}

#[derive(Deserialize)]
struct WireBundle {
    format: u32,
    bundle: Bundle,
}

/// The index and shards a client works from: fetched, or the mirror's.
#[derive(Clone, Debug)]
pub struct Synced {
    pub index: RegistryIndex,
    pub shards: Vec<Arc<Shard>>,
    /// When the index was fetched (seconds since the epoch).
    pub fetched: u64,
    /// Read from the mirror because the registry did not answer.
    pub offline: bool,
}

impl Synced {
    /// The shard holding `library`.
    fn shard(&self, library: LibraryId) -> Option<&Arc<Shard>> {
        self.shards.iter().find(|s| s.catalog.library == library)
    }
}

/// A blocking client of one registry.
pub struct RegistryClient {
    endpoint: Endpoint,
    host: String,
    agent: ureq::Agent,
    /// The sign-in: its keychain and what is known of it.
    auth: Auth,
    mirror: Option<Mirror>,
    synced: Mutex<Option<Arc<Synced>>>,
}

impl std::fmt::Debug for RegistryClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryClient")
            .field("base", &self.endpoint.base())
            .finish_non_exhaustive()
    }
}

/// "index of 14 November" for a fetch time in seconds since the epoch.
pub fn index_date(seconds: u64) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let name = usize::try_from(month - 1)
        .ok()
        .and_then(|m| MONTHS.get(m))
        .copied()
        .unwrap_or("?");
    format!("index of {day} {name}")
}

/// Only a read without a bearer may use plaintext transport off this computer.
#[derive(Clone, Copy)]
enum Request<'a> {
    Read(Option<&'a Secret>),
    SignIn(&'a str),
    Poll(&'a Secret),
    Revoke(&'a Secret),
}

impl Request<'_> {
    fn clean(self, message: &str) -> String {
        match self {
            Self::Read(Some(secret)) | Self::Poll(secret) | Self::Revoke(secret) => {
                secret.redact(message)
            }
            Self::Read(None) | Self::SignIn(_) => message.to_owned(),
        }
    }
}

/// A reply of any status.
pub(crate) struct Reply {
    status: u16,
    /// `Retry-After`, in seconds.
    retry_after: Option<u64>,
    bytes: Vec<u8>,
    diagnostic: Option<String>,
}

impl Reply {
    fn received(
        request: Request<'_>,
        status: u16,
        retry_after: Option<u64>,
        bytes: Vec<u8>,
    ) -> Self {
        let diagnostic = if status >= 300 {
            serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|m| request.clean(m)))
        } else {
            None
        };
        Self {
            status,
            retry_after,
            bytes,
            diagnostic,
        }
    }
}

fn get_err(error: ureq::Error) -> String {
    match error {
        ureq::Error::Timeout(_) => "the registry did not answer in time".into(),
        ureq::Error::BodyExceedsLimit(limit) => {
            format!("the registry's reply is larger than {limit} bytes: refused")
        }
        // Anything else on the way (no route, refused, closed): offline.
        other => format!("the registry cannot be reached ({other})"),
    }
}

/// Whether a failure means the registry is unreachable (offline), rather
/// than a refusal it sent.
fn unreachable(message: &str) -> bool {
    message.starts_with("the registry cannot be reached")
        || message.starts_with("the registry did not answer")
}

impl RegistryClient {
    /// A client of the registry at `url` (checked as Settings checks it),
    /// mirroring under `cache` (the user cache directory) when given.
    /// The sign-in is kept in the system keychain.
    pub fn new(url: &str, cache: Option<&Path>) -> Result<Self, String> {
        Self::with_auth(url, cache, Auth::system())
    }

    /// The same, with the sign-in kept in `store` (tests).
    #[cfg(test)]
    pub(crate) fn with_store(
        url: &str,
        cache: Option<&Path>,
        store: Arc<dyn crate::model::keys::Store>,
    ) -> Result<Self, String> {
        Self::with_auth(url, cache, Auth::with(store))
    }

    fn with_auth(url: &str, cache: Option<&Path>, auth: Auth) -> Result<Self, String> {
        sparksmith_io::settings::check_registry_url(url).map_err(|e| e.to_string())?;
        let endpoint = Endpoint::parse(url).map_err(|e| e.to_string())?;
        let base = endpoint.base();
        let host = base.split_once("://").map_or(base, |(_, h)| h).to_owned();
        let mut config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0);
        if endpoint.is_loopback() {
            config = config.proxy(None);
        }
        let mirror = cache
            .map(|c| Mirror::open(c, url))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            endpoint,
            host,
            agent: config.build().into(),
            auth,
            mirror,
            synced: Mutex::new(None),
        })
    }

    /// The registry's display name: its host.
    pub fn host(&self) -> &str {
        &self.host
    }

    fn request_url(&self, request: Request<'_>, path: &str) -> Result<String, String> {
        if !path.starts_with('/') {
            return Err("A registry request path must start with /.".into());
        }
        if !matches!(request, Request::Read(None)) && !self.may_sign_in() {
            return Err(format!(
                "Signing in to {} needs https://: a sign-in never travels over plain HTTP to \
                 another computer.",
                self.host
            ));
        }
        Ok(format!("{}{path}", self.endpoint.base()))
    }

    /// Check the destination before constructing or sending the HTTP request.
    fn send(
        &self,
        request: Request<'_>,
        path: &str,
        timeout: Duration,
        limit: u64,
    ) -> Result<Reply, String> {
        let url = self.request_url(request, path)?;
        let clean = |error| request.clean(&get_err(error));
        let post = |body: serde_json::Value| {
            self.agent
                .post(&url)
                .config()
                .timeout_global(Some(timeout))
                .build()
                .send_json(body)
        };
        let read = |req: ureq::RequestBuilder<_>, token: Option<&Secret>| {
            let mut req = req.config().timeout_global(Some(timeout)).build();
            if let Some(token) = token {
                req = req.header("authorization", format!("Bearer {}", token.expose()));
            }
            req.call()
        };
        let sent = match request {
            Request::SignIn(name) => post(serde_json::json!({"client_name": name})),
            Request::Poll(code) => post(serde_json::json!({"device_code": code.expose()})),
            Request::Read(token) => read(self.agent.get(&url), token),
            Request::Revoke(token) => read(self.agent.delete(&url), Some(token)),
        };
        let mut response = sent.map_err(clean)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse().ok());
        let bytes = response
            .body_mut()
            .with_config()
            .limit(limit)
            .read_to_vec()
            .map_err(clean)?;
        Ok(Reply::received(request, status, retry_after, bytes))
    }

    /// A refusal in words: the sign-in a download needs, when to try again
    /// after a limit, else the registry's own message.
    fn refusal(&self, reply: &Reply) -> String {
        match reply.status {
            401 => account::SIGN_IN_NEEDED.into(),
            429 => account::rate_limited(reply),
            status => {
                let message = reply
                    .diagnostic
                    .clone()
                    .unwrap_or_else(|| format!("status {status}"));
                format!("the registry refused: {message}")
            }
        }
    }

    /// A public read: never with the sign-in. Searching and the index
    /// need no account, so a 401 here is the server closed to the public
    /// (a password in front of it), not a missing sign-in.
    fn get(&self, path: &str, timeout: Duration, limit: u64) -> Result<Vec<u8>, String> {
        let reply = self.send(Request::Read(None), path, timeout, limit)?;
        if reply.status == 401 {
            return Err(format!(
                "{} is not open to the public: it asked for a password to search, which needs no account. Use another online library, or wait until this one opens",
                self.host
            ));
        }
        if reply.status >= 300 {
            return Err(self.refusal(&reply));
        }
        Ok(reply.bytes)
    }

    /// A download of part data: with the sign-in's token when there is
    /// one. A token the registry refuses is forgotten.
    fn download(&self, path: &str, timeout: Duration, limit: u64) -> Result<Vec<u8>, String> {
        let token = self.token();
        let reply = self.send(Request::Read(token.as_ref()), path, timeout, limit)?;
        if reply.status >= 300 {
            if reply.status == 401 && token.is_some() {
                self.token_refused();
            }
            return Err(self.refusal(&reply));
        }
        Ok(reply.bytes)
    }

    /// The index and shards: fetched now (the missing shards downloaded,
    /// checked and mirrored), else the mirror's last ones when the registry
    /// cannot be reached.
    pub fn sync(&self) -> Result<Arc<Synced>, String> {
        match self.fetch_sync() {
            Ok(synced) => Ok(self.keep(synced)),
            Err(error) if unreachable(&error) => match self.mirrored() {
                Some(synced) => Ok(self.keep(synced)),
                None => Err(error),
            },
            Err(error) => Err(error),
        }
    }

    fn keep(&self, synced: Synced) -> Arc<Synced> {
        let synced = Arc::new(synced);
        if let Ok(mut slot) = self.synced.lock() {
            *slot = Some(synced.clone());
        }
        synced
    }

    /// The last sync, or a new one.
    fn synced(&self) -> Result<Arc<Synced>, String> {
        let held = self.synced.lock().ok().and_then(|s| s.clone());
        match held {
            Some(synced) => Ok(synced),
            None => self.sync(),
        }
    }

    fn fetch_sync(&self) -> Result<Synced, String> {
        let index =
            mirror::decode_index(&self.get("/v1/index", SYNC_TIMEOUT, mirror::MAX_INDEX_BYTES)?)
                .map_err(|e| e.to_string())?;
        let mut shards = Vec::with_capacity(index.shards.len());
        for info in &index.shards {
            let shard = match &self.mirror {
                Some(m) if m.has_shard(info.sha256) => m.shard(info.sha256).ok(),
                _ => None,
            };
            let shard = match shard {
                Some(shard) => shard,
                None => {
                    let bytes = self.get(
                        &format!("/v1/shards/{}", info.sha256),
                        SYNC_TIMEOUT,
                        mirror::MAX_SHARD_BYTES,
                    )?;
                    match &self.mirror {
                        Some(m) => m.store_shard(&bytes, info.sha256),
                        None => mirror::decode_shard(&bytes, info.sha256),
                    }
                    .map_err(|e| e.to_string())?
                }
            };
            shards.push(Arc::new(shard));
        }
        let fetched = mirror::now();
        if let Some(m) = &self.mirror {
            m.store_index(&index, fetched).map_err(|e| e.to_string())?;
        }
        Ok(Synced {
            index,
            shards,
            fetched,
            offline: false,
        })
    }

    fn mirrored(&self) -> Option<Synced> {
        let m = self.mirror.as_ref()?;
        let MirroredIndex { index, fetched } = m.index()?;
        let shards = m.shards().into_iter().map(Arc::new).collect();
        Some(Synced {
            index,
            shards,
            fetched,
            offline: true,
        })
    }

    /// `library.search` of `text` on the registry; offline, on the mirror.
    pub fn search(&self, text: &str, limit: usize, offset: usize) -> Result<OnlineReport, String> {
        let query: String = url_encode(text);
        let path = format!("/v1/search?q={query}&limit={limit}&offset={offset}");
        match self.get(&path, SEARCH_TIMEOUT, MAX_SEARCH_BYTES) {
            Ok(bytes) => {
                let wire: WireSearch = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("the registry's search reply is unreadable: {e}"))?;
                Ok(OnlineReport {
                    source: self.host.clone(),
                    snapshot: wire.snapshot,
                    chips: wire.chips,
                    total: wire.total,
                    results: wire
                        .results
                        .into_iter()
                        .filter_map(|row| online_row(row.hit, row.id, row.review))
                        .collect(),
                    offline: None,
                    took_us: wire.took_us,
                })
            }
            Err(error) if unreachable(&error) => self
                .offline_search(text, limit, offset)
                .map_err(|offline| if offline.is_empty() { error } else { offline }),
            Err(error) => Err(error),
        }
    }

    /// The same search over the mirrored shards, with the core's own
    /// index: what works without the network. `offset` pages as the
    /// registry's search does, so offline page two follows page one.
    pub fn offline_search(
        &self,
        text: &str,
        limit: usize,
        offset: usize,
    ) -> Result<OnlineReport, String> {
        let synced = match self.synced.lock().ok().and_then(|s| s.clone()) {
            Some(s) => s,
            None => Arc::new(self.mirrored().ok_or_else(String::new)?),
        };
        let mut report = search_shards(&synced.shards, text, limit, offset)?;
        report.source = self.host.clone();
        report.snapshot = synced.index.snapshot.clone();
        report.offline = Some(index_date(synced.fetched));
        Ok(report)
    }

    /// The checked bundle of a registry root: the mirror's when it holds
    /// it, else fetched, checked against the shard's content hashes and
    /// mirrored. A record that does not match is refused, naming it.
    pub fn bundle(&self, library: LibraryId, root: SourceRevision) -> Result<Bundle, String> {
        let synced = self.synced()?;
        let shard = synced
            .shard(library)
            .ok_or_else(|| format!("{} has no library {library}", self.host))?;
        let catalog: &SourceCatalog = &shard.catalog;
        if let Some(m) = &self.mirror
            && let Ok(Some(bundle)) = m.bundle(root, catalog)
        {
            return Ok(bundle);
        }
        let bytes = self.download(
            &format!("/v1/bundles/{}/{}", root.lineage, root.revision.0),
            BUNDLE_TIMEOUT,
            MAX_BUNDLE_BYTES,
        )?;
        let wire: WireBundle =
            serde_json::from_slice(&bytes).map_err(|_| "the registry's bundle is unreadable")?;
        if wire.format != mirror::FORMAT {
            return Err(format!(
                "registry bundle format {} is not supported",
                wire.format
            ));
        }
        match &self.mirror {
            Some(m) => m.store_bundle(&wire.bundle, catalog),
            None => mirror::verify_bundle(&wire.bundle, root, catalog),
        }
        .map_err(|e| e.to_string())?;
        if wire.bundle.root != root {
            return Err(format!(
                "the registry sent another root for {}",
                root.lineage
            ));
        }
        Ok(wire.bundle)
    }

    /// Whether `library` is one of this registry's (after a sync).
    pub fn holds(&self, library: LibraryId) -> bool {
        self.synced
            .lock()
            .ok()
            .and_then(|s| s.clone())
            .is_some_and(|s| s.shard(library).is_some())
    }

    /// The registry's review word for a root lineage, from its shard.
    pub fn review(&self, library: LibraryId, lineage: SourceLineageId) -> Option<String> {
        let synced = self.synced.lock().ok().and_then(|s| s.clone())?;
        synced
            .shard(library)?
            .entries
            .iter()
            .find(|e| e.lineage == lineage)
            .map(|e| e.review.clone())
    }
}

/// A row whose hit is marked as this registry's; `None` for one that
/// names no source lineage (nothing could place it).
fn online_row(mut hit: Hit, id: String, review: String) -> Option<OnlineHit> {
    let library = hit.entry.source.library()?;
    hit.entry.source = Source::Online(library);
    hit.entry.source_lineage?;
    Some(OnlineHit { hit, id, review })
}

/// The core's search over shards (offline).
pub fn search_shards(
    shards: &[Arc<Shard>],
    text: &str,
    limit: usize,
    offset: usize,
) -> Result<OnlineReport, String> {
    let catalogs: Vec<Arc<SourceCatalog>> =
        shards.iter().map(|s| Arc::new(s.catalog.clone())).collect();
    let project = Project::new(&NewBoard::without_board());
    let index =
        Index::with_sources(&project, &Drafts::default(), &catalogs).map_err(|e| e.to_string())?;
    let query = index.parse(text).map_err(|e| e.to_string())?;
    let report = index.search(
        &query,
        &CatalogContext::default(),
        SearchOptions {
            limit: limit.max(1),
            // The ranked search skips the rows before the page itself, so
            // a page past its result limit is never empty while more
            // match (review, offline registry).
            offset,
            include_drafts: false,
        },
    );
    let review = |hit: &Hit| {
        let lineage = hit.entry.source_lineage?;
        let library = hit.entry.source.library()?;
        shards
            .iter()
            .find(|s| s.catalog.library == library)?
            .entries
            .iter()
            .find(|e| e.lineage == lineage)
            .map(|e| (e.id.clone(), e.review.clone()))
    };
    Ok(OnlineReport {
        chips: report.chips,
        total: report.total,
        results: report
            .results
            .into_iter()
            .filter_map(|hit| {
                let (id, word) = review(&hit).unwrap_or_default();
                online_row(hit, id, word)
            })
            .collect(),
        ..OnlineReport::default()
    })
}

/// Percent-encoding of a query parameter (RFC 3986 unreserved kept).
fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl OnlineSource for RegistryClient {
    fn name(&self) -> String {
        self.host.clone()
    }
    fn search(&self, text: &str, limit: usize) -> Result<OnlineReport, String> {
        RegistryClient::search(self, text, limit, 0)
    }
    fn bundle(
        &self,
        library: LibraryId,
        lineage: SourceLineageId,
        revision: Option<Revision>,
    ) -> Result<Option<Bundle>, String> {
        let synced = self.synced()?;
        let Some(shard) = synced.shard(library) else {
            return Ok(None);
        };
        let revision = match revision {
            Some(r) => r,
            None => shard
                .catalog
                .revisions
                .iter()
                .filter(|r| r.lineage == lineage)
                .map(|r| r.revision)
                .max()
                .ok_or_else(|| format!("{} has no lineage {lineage}", self.host))?,
        };
        RegistryClient::bundle(self, library, SourceRevision { lineage, revision }).map(Some)
    }
    fn account(&self) -> Result<OnlineAccount, String> {
        Ok(self.account_status())
    }
    fn sign_out(&self) -> Result<SignOut, String> {
        RegistryClient::sign_out(self)
    }
}

/// What the worker reports.
#[derive(Debug)]
pub enum RegistryEvent {
    /// The search of `text` asked as `generation`: only ever the newest.
    Search {
        generation: u64,
        text: String,
        result: Result<OnlineReport, String>,
        /// From the request being sent to its reply, in milliseconds.
        round_trip_ms: f64,
    },
    /// A bundle asked for with `ticket`.
    Bundle {
        ticket: u64,
        library: LibraryId,
        root: SourceRevision,
        result: Result<Bundle, String>,
    },
    /// A sync finished: shards held, or why not.
    Synced(Result<Arc<Synced>, String>),
    /// The sign-in: read, started, finished or removed.
    Account(AccountEvent),
}

/// What became of the sign-in. `attempt` tells a sign-in's events from an
/// earlier, cancelled one's.
#[derive(Debug)]
pub enum AccountEvent {
    /// The kept sign-in was read and the registry asked whose it is
    /// (`Ok`: [`RegistryClient::known`] says the rest).
    Read(Result<(), String>),
    /// A code to show; the registry is being polled.
    Code {
        attempt: u64,
        code: DeviceCode,
    },
    /// The sign-in ended: signed in, or why not.
    SignedIn {
        attempt: u64,
        result: Result<account::Account, String>,
    },
    SignedOut(Result<SignOut, String>),
}

enum SearchJob {
    Search {
        generation: u64,
        text: String,
        limit: usize,
    },
}

enum FetchJob {
    Bundle {
        ticket: u64,
        library: LibraryId,
        root: SourceRevision,
    },
    Sync,
}

/// The client on two threads: one for searches (debounced, newest wins),
/// one for the index and bundles.
pub struct RegistryWorker {
    client: Arc<RegistryClient>,
    searches: Sender<SearchJob>,
    fetches: Sender<FetchJob>,
    /// For the sign-in's threads.
    sender: Sender<RegistryEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
    events: Receiver<RegistryEvent>,
    /// The newest search generation asked for; replies of others are
    /// dropped.
    latest: Arc<AtomicU64>,
}

impl RegistryWorker {
    /// Start the threads. `notify` runs after each event is sent (the app
    /// asks for a repaint).
    pub fn start(
        client: Arc<RegistryClient>,
        debounce: Duration,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (searches, search_jobs) = mpsc::channel::<SearchJob>();
        let (fetches, fetch_jobs) = mpsc::channel::<FetchJob>();
        let (sender, events) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        {
            let client = client.clone();
            let sender = sender.clone();
            let latest = latest.clone();
            let notify = notify.clone();
            let context = sparksmith_core::library::standards::Context::current().ok();
            std::thread::spawn(move || {
                let _scope = context.as_ref().map(|c| c.enter());
                search_loop(&client, &search_jobs, &sender, &latest, debounce, &*notify);
            });
        }
        {
            let client = client.clone();
            let sender = sender.clone();
            let notify = notify.clone();
            let context = sparksmith_core::library::standards::Context::current().ok();
            std::thread::spawn(move || {
                let _scope = context.as_ref().map(|c| c.enter());
                while let Ok(job) = fetch_jobs.recv() {
                    let event = match job {
                        FetchJob::Bundle {
                            ticket,
                            library,
                            root,
                        } => RegistryEvent::Bundle {
                            ticket,
                            library,
                            root,
                            result: client.bundle(library, root),
                        },
                        FetchJob::Sync => RegistryEvent::Synced(client.sync()),
                    };
                    if sender.send(event).is_err() {
                        return;
                    }
                    notify();
                }
            });
        }
        Self {
            client,
            searches,
            fetches,
            sender,
            notify,
            events,
            latest,
        }
    }

    pub fn client(&self) -> &Arc<RegistryClient> {
        &self.client
    }

    /// Ask for `text` as `generation` (larger than any before). Older
    /// searches still waiting are dropped; an older reply is never reported.
    pub fn search(&self, generation: u64, text: &str, limit: usize) {
        self.latest.fetch_max(generation, Ordering::SeqCst);
        let _ = self.searches.send(SearchJob::Search {
            generation,
            text: text.to_owned(),
            limit,
        });
    }

    /// Drop every search asked so far (the palette closed): replies still
    /// in flight are not reported.
    pub fn cancel(&self, generation: u64) {
        self.latest.fetch_max(generation, Ordering::SeqCst);
    }

    pub fn bundle(&self, ticket: u64, library: LibraryId, root: SourceRevision) {
        let _ = self.fetches.send(FetchJob::Bundle {
            ticket,
            library,
            root,
        });
    }

    pub fn sync(&self) {
        let _ = self.fetches.send(FetchJob::Sync);
    }

    /// Read the kept sign-in and ask whose it is, on a thread of its own.
    pub fn read_account(&self) {
        let client = self.client.clone();
        self.account_thread(move || AccountEvent::Read(client.refresh_account().map(|_| ())));
    }

    /// Sign in as `client_name` (attempt `attempt`), on a thread of its
    /// own: the code arrives first, then the end. Setting `cancel` stops
    /// the polling.
    pub fn sign_in(&self, attempt: u64, client_name: String, cancel: Arc<AtomicBool>) {
        let client = self.client.clone();
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        self.account_thread(move || {
            let code = match client.start_sign_in(&client_name) {
                Ok(code) => code,
                Err(error) => {
                    return AccountEvent::SignedIn {
                        attempt,
                        result: Err(error),
                    };
                }
            };
            let event = AccountEvent::Code {
                attempt,
                code: code.clone(),
            };
            if sender.send(RegistryEvent::Account(event)).is_ok() {
                notify();
            }
            AccountEvent::SignedIn {
                attempt,
                result: client.wait_for_sign_in(&code, &cancel, &account::sleep_unless),
            }
        });
    }

    /// Sign out, on a thread of its own.
    pub fn sign_out(&self) {
        let client = self.client.clone();
        self.account_thread(move || AccountEvent::SignedOut(client.sign_out()));
    }

    fn account_thread(&self, work: impl FnOnce() -> AccountEvent + Send + 'static) {
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        let started = std::thread::Builder::new()
            .name("registry-account".into())
            .spawn(move || {
                if sender.send(RegistryEvent::Account(work())).is_ok() {
                    notify();
                }
            });
        if started.is_err() {
            let event = AccountEvent::Read(Err("could not start the sign-in's thread".into()));
            if self.sender.send(RegistryEvent::Account(event)).is_ok() {
                (self.notify)();
            }
        }
    }

    /// Events since the last call; a search reply that is no longer the
    /// newest is dropped here too.
    pub fn receive(&self) -> Vec<RegistryEvent> {
        let latest = self.latest.load(Ordering::SeqCst);
        self.events
            .try_iter()
            .filter(
                |e| !matches!(e, RegistryEvent::Search { generation, .. } if *generation != latest),
            )
            .collect()
    }
}

fn search_loop(
    client: &RegistryClient,
    jobs: &Receiver<SearchJob>,
    sender: &Sender<RegistryEvent>,
    latest: &AtomicU64,
    debounce: Duration,
    notify: &(dyn Fn() + Send + Sync),
) {
    while let Ok(mut job) = jobs.recv() {
        // Debounce: wait for typing to pause, keeping only the newest.
        loop {
            match jobs.recv_timeout(debounce) {
                Ok(newer) => job = newer,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let SearchJob::Search {
            generation,
            text,
            limit,
        } = job;
        if generation != latest.load(Ordering::SeqCst) {
            continue;
        }
        let started = Instant::now();
        let result = client.search(&text, limit, 0);
        let round_trip_ms = started.elapsed().as_secs_f64() * 1000.0;
        // A newer search was asked while this one ran: drop its reply.
        if generation != latest.load(Ordering::SeqCst) {
            continue;
        }
        if sender
            .send(RegistryEvent::Search {
                generation,
                text,
                result,
                round_trip_ms,
            })
            .is_err()
        {
            return;
        }
        notify();
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
