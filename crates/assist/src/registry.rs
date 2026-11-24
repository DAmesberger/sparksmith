//! The online parts registry's client (registry R0 §5 and §10): search with
//! the app's own query text, the index and its shards for the offline
//! mirror, and bundles checked against the registry's content hashes
//! before anything uses them. This crate is the one with an HTTP client
//! (`cargo xtask layers`); the mirror's files are io's
//! ([`sparksmith_io::registry_mirror`]).
//!
//! [`RegistryClient`] calls block, and end when their call is cancelled or
//! the registry sends nothing for as long as the person allows
//! (`settings::Network`). [`RegistryWorker`] runs each on a thread of its
//! own: searches are debounced and tagged with the caller's generation, and
//! a newer one ends the request of the one before, so a silent registry
//! holds up neither typing nor a download. Offline, search runs the core's
//! own index over the mirrored shards and says the index's date.
//!
//! Downloading part data needs a free account ([`account`]): bundle
//! requests carry the sign-in's token, searches and the catalog never do.
pub mod account;

use crate::http::{self, Agents, Next, Silence};
use crate::model::{Endpoint, Secret};
use crate::settings::{Network, Wait};
use account::{Auth, DeviceCode};
use serde::Deserialize;
use sparksmith_automation::cancel::Cancel;
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
use std::collections::BTreeMap;
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
/// A request its call was cancelled during.
const STOPPED: &str = "the request was stopped";
/// Who a silent request waited for, in words.
const REGISTRY: &str = "the online library";
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
    agents: Agents,
    /// How long its requests may hear nothing.
    network: Mutex<Network>,
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

fn get_err(error: ureq::Error, silence: Silence) -> String {
    match error {
        error if http::silent(&error) => {
            format!("the registry did not answer. {}", silence.why(REGISTRY))
        }
        ureq::Error::BodyExceedsLimit(limit) => {
            format!("the registry's reply is larger than {limit} bytes: refused")
        }
        // Anything else on the way (no route, refused, closed): offline.
        other => format!("the registry cannot be reached ({other})"),
    }
}

/// A request as its thread sends it.
enum Sent {
    Post(serde_json::Value),
    Get(Option<Secret>),
    Delete(Secret),
}

impl From<Request<'_>> for Sent {
    fn from(request: Request<'_>) -> Self {
        match request {
            Request::SignIn(name) => Self::Post(serde_json::json!({"client_name": name})),
            Request::Poll(code) => Self::Post(serde_json::json!({"device_code": code.expose()})),
            Request::Read(token) => Self::Get(token.cloned()),
            Request::Revoke(token) => Self::Delete(token.clone()),
        }
    }
}

/// What came back: the status, `Retry-After` and the body.
type Answer = (u16, Option<u64>, Vec<u8>);

/// Send `sent` to `url` on `agent` and read at most `limit` bytes.
fn exchange(
    agent: &ureq::Agent,
    url: &str,
    sent: &Sent,
    silence: Silence,
    limit: u64,
) -> Result<Answer, ureq::Error> {
    let after = silence.after;
    let post = |body: &serde_json::Value| {
        agent
            .post(url)
            .config()
            .timeout_per_read(after)
            .timeout_per_write(after)
            .timeout_connect(after)
            .timeout_resolve(after)
            .build()
            .send_json(body)
    };
    let read = |request: ureq::RequestBuilder<_>, token: Option<&Secret>| {
        let mut request = request
            .config()
            .timeout_per_read(after)
            .timeout_per_write(after)
            .timeout_connect(after)
            .timeout_resolve(after)
            .build();
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {}", token.expose()));
        }
        request.call()
    };
    let mut response = match sent {
        Sent::Post(body) => post(body)?,
        Sent::Get(token) => read(agent.get(url), token.as_ref())?,
        Sent::Delete(token) => read(agent.delete(url), Some(token))?,
    };
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
        .read_to_vec()?;
    Ok((status, retry_after, bytes))
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
        let mirror = cache
            .map(|c| Mirror::open(c, url))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            agents: Agents::new(endpoint.is_loopback()),
            network: Mutex::new(Network::NO_LIMITS),
            endpoint,
            host,
            auth,
            mirror,
            synced: Mutex::new(None),
        })
    }

    /// How long its requests may hear nothing, from now on.
    pub fn set_network(&self, network: Network) {
        if let Ok(mut held) = self.network.lock() {
            *held = network;
        }
    }

    fn silence(&self, wait: impl FnOnce(&Network) -> Wait) -> Silence {
        let network = self.network.lock().map_or(Network::NO_LIMITS, |n| *n);
        Network::silence(wait(&network))
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

    /// Check the destination before constructing or sending the HTTP
    /// request, which runs on a thread of its own ([`crate::http`]).
    fn send(
        &self,
        request: Request<'_>,
        path: &str,
        silence: Silence,
        limit: u64,
    ) -> Result<Reply, String> {
        let url = self.request_url(request, path)?;
        let sent = Sent::from(request);
        let running = self.agents.start(move |agent, said| {
            let _ = said.send(exchange(agent, &url, &sent, silence, limit));
        })?;
        match running.next() {
            Next::Step(answer) => {
                running.done();
                let (status, retry_after, bytes) =
                    answer.map_err(|e| request.clean(&get_err(e, silence)))?;
                Ok(Reply::received(request, status, retry_after, bytes))
            }
            Next::Cancelled => Err(STOPPED.into()),
            Next::Gone => Err("the registry cannot be reached (no answer)".into()),
        }
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
    fn get(&self, path: &str, silence: Silence, limit: u64) -> Result<Vec<u8>, String> {
        let reply = self.send(Request::Read(None), path, silence, limit)?;
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
    fn download(&self, path: &str, silence: Silence, limit: u64) -> Result<Vec<u8>, String> {
        let token = self.token();
        let reply = self.send(Request::Read(token.as_ref()), path, silence, limit)?;
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
        match self.fetch_sync(mirror::MAX_CATALOG_BYTES) {
            Ok(synced) => Ok(self.keep(synced)),
            Err(error) if unreachable(&error) => match self.mirrored()? {
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

    fn fetch_sync(&self, limit: u64) -> Result<Synced, String> {
        let silence = self.silence(|n| n.index);
        let index =
            mirror::decode_index(&self.get("/v1/index", silence, mirror::MAX_INDEX_BYTES)?)
                .map_err(|e| e.to_string())?;
        let mut shards = Vec::with_capacity(index.shards.len());
        let mut budget = mirror::ShardBudget(limit);
        for info in &index.shards {
            let shard = match &self.mirror {
                Some(m) => budget.cached(m, info.sha256).map_err(|e| e.to_string())?,
                _ => None,
            };
            let shard = match shard {
                Some(shard) => shard,
                None => {
                    let bytes = self.get(
                        &format!("/v1/shards/{}", info.sha256),
                        silence,
                        mirror::MAX_SHARD_BYTES,
                    )?;
                    budget.charge(bytes.len()).map_err(|e| e.to_string())?;
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

    fn mirrored(&self) -> Result<Option<Synced>, String> {
        let Some((m, MirroredIndex { index, fetched })) =
            self.mirror.as_ref().and_then(|m| Some((m, m.index()?)))
        else {
            return Ok(None);
        };
        let shards = m
            .shards(&index, mirror::MAX_CATALOG_BYTES)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(Arc::new)
            .collect();
        Ok(Some(Synced {
            index,
            shards,
            fetched,
            offline: true,
        }))
    }

    /// `library.search` of `text` on the registry; offline, on the mirror.
    pub fn search(&self, text: &str, limit: usize, offset: usize) -> Result<OnlineReport, String> {
        let query: String = url_encode(text);
        let path = format!("/v1/search?q={query}&limit={limit}&offset={offset}");
        match self.get(&path, self.silence(|n| n.search), MAX_SEARCH_BYTES) {
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
            None => Arc::new(self.mirrored()?.ok_or_else(String::new)?),
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
            self.silence(|n| n.download),
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
    /// No search is wanted any more.
    Drop,
}

/// Work that runs on a thread of its own, to be stopped by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Work {
    Sync,
    Account,
    SignOut,
    Bundle(u64),
}

/// The client off the UI thread: each request on a thread of its own, so a
/// silent registry holds up no other one. Searches are debounced and the
/// newest wins: a newer one ends the request of the one before. Dropping it
/// ends every request.
pub struct RegistryWorker {
    client: Arc<RegistryClient>,
    searches: Sender<SearchJob>,
    sender: Sender<RegistryEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
    events: Receiver<RegistryEvent>,
    /// The newest search generation asked for; replies of others are
    /// dropped.
    latest: Arc<AtomicU64>,
    running: Mutex<BTreeMap<Work, Cancel>>,
}

impl Drop for RegistryWorker {
    fn drop(&mut self) {
        if let Ok(running) = self.running.lock() {
            running.values().for_each(Cancel::set);
        }
    }
}

impl RegistryWorker {
    /// Start the searches' thread. `notify` runs after each event is sent
    /// (the app asks for a repaint).
    pub fn start(
        client: Arc<RegistryClient>,
        debounce: Duration,
        notify: Arc<dyn Fn() + Send + Sync>,
    ) -> Self {
        let (searches, search_jobs) = mpsc::channel::<SearchJob>();
        let (sender, events) = mpsc::channel();
        let latest = Arc::new(AtomicU64::new(0));
        let search = Search {
            client: client.clone(),
            sender: sender.clone(),
            latest: latest.clone(),
            notify: notify.clone(),
        };
        std::thread::spawn(sparksmith_core::library::standards::carried(move || {
            search_loop(&search, &search_jobs, debounce);
        }));
        Self {
            client,
            searches,
            sender,
            notify,
            events,
            latest,
            running: Mutex::default(),
        }
    }

    /// Run `request` as `work` on a thread of its own, ending an earlier
    /// one that still runs.
    fn run(
        &self,
        work: Work,
        request: impl FnOnce(&RegistryClient) -> RegistryEvent + Send + 'static,
    ) -> bool {
        let cancel = Cancel::default();
        if let Ok(mut running) = self.running.lock()
            && let Some(earlier) = running.insert(work, cancel.clone())
        {
            earlier.set();
        }
        let client = self.client.clone();
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        std::thread::Builder::new()
            .name("registry-request".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                let event = cancel.scope(|| request(&client));
                if sender.send(event).is_ok() {
                    notify();
                }
            }))
            .is_ok()
    }

    fn stop(&self, work: Work) {
        if let Ok(mut running) = self.running.lock()
            && let Some(cancel) = running.remove(&work)
        {
            cancel.set();
        }
    }

    /// Stop the bundle asked for with `ticket`: its event says so.
    pub fn stop_bundle(&self, ticket: u64) {
        self.stop(Work::Bundle(ticket));
    }

    /// Stop reading the index.
    pub fn stop_sync(&self) {
        self.stop(Work::Sync);
    }

    /// Stop telling the registry of the sign-out, which this computer has
    /// done either way.
    pub fn stop_sign_out(&self) {
        self.stop(Work::SignOut);
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

    /// Drop every search asked so far (the palette closed): the request in
    /// flight ends.
    pub fn cancel(&self, generation: u64) {
        if self.latest.fetch_max(generation, Ordering::SeqCst) < generation {
            let _ = self.searches.send(SearchJob::Drop);
        }
    }

    pub fn bundle(&self, ticket: u64, library: LibraryId, root: SourceRevision) {
        let started = self.run(Work::Bundle(ticket), move |client| RegistryEvent::Bundle {
            ticket,
            library,
            root,
            result: client.bundle(library, root),
        });
        if !started {
            let result = Err("the download could not be started".into());
            let _ = self.sender.send(RegistryEvent::Bundle {
                ticket,
                library,
                root,
                result,
            });
        }
    }

    pub fn sync(&self) {
        if !self.run(Work::Sync, |client| RegistryEvent::Synced(client.sync())) {
            let result = Err("the index could not be read: no thread".into());
            let _ = self.sender.send(RegistryEvent::Synced(result));
        }
    }

    /// Read the kept sign-in and ask whose it is.
    pub fn read_account(&self) {
        self.account(Work::Account, |client| {
            AccountEvent::Read(client.refresh_account().map(|_| ()))
        });
    }

    /// Sign in as `client_name` (attempt `attempt`), on a thread of its
    /// own: the code arrives first, then the end. Setting `cancel` stops
    /// the polling.
    pub fn sign_in(&self, attempt: u64, client_name: String, cancel: Arc<AtomicBool>) {
        let sender = self.sender.clone();
        let notify = self.notify.clone();
        self.account(Work::Account, move |client| {
            // The person's Cancel ends a request in flight too.
            Cancel::from(cancel.clone()).scope(|| {
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
            })
        });
    }

    /// Sign out.
    pub fn sign_out(&self) {
        self.account(Work::SignOut, |client| {
            AccountEvent::SignedOut(client.sign_out())
        });
    }

    fn account(
        &self,
        work: Work,
        request: impl FnOnce(&RegistryClient) -> AccountEvent + Send + 'static,
    ) {
        if !self.run(work, |client| RegistryEvent::Account(request(client))) {
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
        let events: Vec<_> = self
            .events
            .try_iter()
            .filter(
                |e| !matches!(e, RegistryEvent::Search { generation, .. } if *generation != latest),
            )
            .collect();
        if let Ok(mut running) = self.running.lock() {
            for event in &events {
                if let RegistryEvent::Bundle { ticket, .. } = event {
                    running.remove(&Work::Bundle(*ticket));
                }
            }
        }
        events
    }
}

/// What a search's thread needs.
#[derive(Clone)]
struct Search {
    client: Arc<RegistryClient>,
    sender: Sender<RegistryEvent>,
    latest: Arc<AtomicU64>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Search {
    fn run(&self, generation: u64, text: String, limit: usize) {
        let started = Instant::now();
        let result = self.client.search(&text, limit, 0);
        let round_trip_ms = started.elapsed().as_secs_f64() * 1000.0;
        // A newer search was asked while this one ran: drop its reply.
        if generation != self.latest.load(Ordering::SeqCst) {
            return;
        }
        let event = RegistryEvent::Search {
            generation,
            text,
            result,
            round_trip_ms,
        };
        if self.sender.send(event).is_ok() {
            (self.notify)();
        }
    }
}

fn search_loop(search: &Search, jobs: &Receiver<SearchJob>, debounce: Duration) {
    /// The request in flight, ended when a newer is asked or none is left.
    #[derive(Default)]
    struct InFlight(Cancel);
    impl Drop for InFlight {
        fn drop(&mut self) {
            self.0.set();
        }
    }
    let mut in_flight = InFlight::default();
    while let Ok(mut job) = jobs.recv() {
        // Debounce: wait for typing to pause, keeping only the newest.
        loop {
            match jobs.recv_timeout(debounce) {
                Ok(newer) => job = newer,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        in_flight.0.set();
        in_flight = InFlight::default();
        let SearchJob::Search {
            generation,
            text,
            limit,
        } = job
        else {
            continue;
        };
        if generation != search.latest.load(Ordering::SeqCst) {
            continue;
        }
        let (own, cancel) = (search.clone(), in_flight.0.clone());
        let text_shown = text.clone();
        let started = std::thread::Builder::new()
            .name("registry-search".into())
            .spawn(sparksmith_core::library::standards::carried(move || {
                cancel.scope(|| own.run(generation, text, limit));
            }));
        if let Err(error) = started {
            let _ = search.sender.send(RegistryEvent::Search {
                generation,
                text: text_shown,
                result: Err(format!("the search could not be started: {error}")),
                round_trip_ms: 0.0,
            });
            (search.notify)();
        }
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
