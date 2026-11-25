//! The registry client against a small registry on a local socket that
//! answers with the core's own search over an original fixture shard:
//! search, the offline mirror, refused bundles and dropped stale replies.
use super::*;
use crate::model::keys::Store;
use crate::model::keys::memory::Memory;
use account::{Account, Known, Poll};
use sparksmith_core::catalog::source_catalog;
use sparksmith_core::ids::{DefinitionKind, DefinitionRef};
use sparksmith_parts_contract::account::{DeviceCodeReply, DeviceTokenReply, Token, read_bearer};
use sparksmith_parts_contract::documents::{ShardEntry, ShardInfo};
use sparksmith_parts_contract::hash::bytes_hash;
use sparksmith_parts_contract::limits::MAX_SEARCH_ROWS;
use sparksmith_parts_contract::route::{MEDIA_TYPE, Refused};
use sparksmith_parts_contract::status::codes;
use sparksmith_parts_contract::version::FORMAT;
use std::collections::{BTreeSet, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

const LIBRARY: LibraryId = LibraryId(0x5eed);

/// The ExampleCo resistor Part's bundle and its shard; the Part's
/// reference carries an attached copy, so the bundle packs one asset.
fn fixture() -> std::result::Result<(Bundle, Shard), Box<dyn std::error::Error>> {
    let mut library =
        sparksmith_core::catalog::synthetic::project_with_parts(1, &NewBoard::fixture())?.library;
    let data = b"An original datasheet of the fixture registry.".to_vec();
    let packed = sparksmith_core::library::bundle::PackedAsset {
        hash: bytes_hash(&data),
        media_type: "application/pdf".into(),
        data: sparksmith_core::library::bundle::AssetBytes::new(data),
    };
    let first = library.parts.first_mut().ok_or("no Part")?;
    let reference = first
        .definition
        .references
        .first_mut()
        .ok_or("the synthetic Part has no reference")?;
    reference.asset = Some(packed.hash.to_string());
    library.assets.push(packed);
    let part = library.parts.first().ok_or("no Part")?;
    let root = DefinitionRef {
        kind: DefinitionKind::Part,
        id: part.id,
        revision: part.revision,
    };
    let fresh: Vec<SourceLineageId> = (0..64).map(|i| SourceLineageId(0xa000 + i)).collect();
    let plan = sparksmith_core::library::import::export(
        &library,
        root,
        LIBRARY,
        "Fixture registry",
        &fresh,
    )?;
    let mut catalog = source_catalog(LIBRARY, "Fixture registry", &plan.bundle.records)?;
    for entry in &mut catalog.entries {
        entry.source = Source::Online(LIBRARY);
    }
    let entries = catalog
        .entries
        .iter()
        .filter_map(|e| e.source_lineage)
        .map(|lineage| ShardEntry {
            lineage,
            id: format!("fixture/{lineage}"),
            review: "unreviewed".into(),
            glb: None,
        })
        .collect();
    Ok((
        plan.bundle,
        Shard {
            format: FORMAT,
            catalog,
            entries,
        },
    ))
}

/// What the local registry serves, and how.
struct Registry {
    shard: Arc<Shard>,
    shard_bytes: Vec<u8>,
    bundle: Bundle,
    /// Searches answered, by query text.
    searched: Mutex<Vec<String>>,
    /// A search for this text waits this long first.
    slow: Option<(String, Duration)>,
    tamper: AtomicBool,
    up: AtomicBool,
    /// Requests whose path has this in it are never answered.
    silent: Mutex<String>,
    /// Requests held so, and those of them the client closed.
    held: AtomicU64,
    let_go: AtomicU64,
    /// Every request: method and path, and its `Authorization` header.
    seen: Mutex<Vec<(String, Option<String>)>>,
    /// Bundles need [`TOKEN`] as their bearer (the accounts contract).
    needs_account: AtomicBool,
    /// Bundles answer 429 with `Retry-After: 300`.
    limited: AtomicBool,
    /// Every request answers 401 with a Basic challenge and no body, as a
    /// server behind a password does.
    closed: AtomicBool,
    /// The answers of the next device-token polls, in order; `pending`
    /// once they run out.
    polls: Mutex<VecDeque<&'static str>>,
    /// Request bodies of the device endpoints.
    bodies: Mutex<Vec<serde_json::Value>>,
    /// The tokens it knows: [`TOKEN`] until revoked.
    tokens: Mutex<BTreeSet<String>>,
}

/// The token the fixture registry issues.
const TOKEN: &str = "ssp_fixturetoken0123456789abcdefghijklmnopqrstu";

/// A reply: status, extra headers and body.
type Answer = (u16, Vec<(&'static str, String)>, Vec<u8>);

fn error_body(code: &str, message: &str) -> Vec<u8> {
    ErrorBody::new(code, message).bytes()
}

fn json(value: &impl serde::Serialize) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

/// The bundle of `root`, one of `whole`'s records: it and the records it
/// reaches (records are dependencies first), and the assets they name.
fn closure(whole: &Bundle, root: SourceRevision) -> Bundle {
    let mut wanted = BTreeSet::from([root.lineage]);
    let mut records = Vec::new();
    for record in whole.records.iter().rev() {
        if wanted.contains(&record.lineage) {
            wanted.extend(record.links.iter().map(|l| l.lineage));
            records.push(record.clone());
        }
    }
    records.reverse();
    let named: BTreeSet<_> = records.iter().flat_map(|r| r.assets.iter()).collect();
    Bundle {
        root,
        assets: whole
            .assets
            .iter()
            .filter(|a| named.contains(&a.hash))
            .cloned()
            .collect(),
        records,
        ..whole.clone()
    }
}

/// What the fixture registry answers a sign-in code request with.
fn device_code() -> DeviceCodeReply {
    DeviceCodeReply {
        device_code: Token("dev-secret-code".into()),
        user_code: "BCDF-GHJK".into(),
        verification_uri: "https://parts.example/device".into(),
        verification_uri_complete: "https://parts.example/device?code=BCDF-GHJK".into(),
        expires_in: 900,
        interval: 1,
    }
}

/// An approved sign-in of the fixture registry.
fn approved() -> DeviceTokenReply {
    DeviceTokenReply {
        access_token: Token(TOKEN.into()),
        token_type: "bearer".into(),
        token_id: "t1".into(),
        account: Account {
            email: "maker@example.com".into(),
            display_name: "Maker".into(),
            ..Account::default()
        },
    }
}

/// `GET /v1/account` of the fixture registry.
fn account_reply() -> Account {
    Account {
        id: "a1".into(),
        email: "maker@example.com".into(),
        display_name: String::new(),
        providers: vec!["email".into()],
    }
}

impl Registry {
    /// The accounts contract's routes, then the catalog's.
    fn respond(&self, method: &str, path: &str, auth: Option<&str>, body: &[u8]) -> Answer {
        if let Ok(mut seen) = self.seen.lock() {
            seen.push((format!("{method} {path}"), auth.map(str::to_owned)));
        }
        if self.closed.load(Ordering::SeqCst) {
            return (
                401,
                vec![("www-authenticate", "Basic realm=\"restricted\"".into())],
                Vec::new(),
            );
        }
        let bearer = auth
            .and_then(read_bearer)
            .is_some_and(|token| self.tokens.lock().is_ok_and(|t| t.contains(token)));
        let route = match Route::parse(method, path) {
            Ok(route) => route,
            Err(Refused::Query(message)) => {
                return (400, vec![], error_body(codes::CATALOG_QUERY, &message));
            }
            Err(Refused::NotFound) => {
                return (404, vec![], error_body(codes::NOT_FOUND, "no such thing"));
            }
        };
        let keep = |body: &[u8]| {
            if let Ok(mut b) = self.bodies.lock() {
                b.push(serde_json::from_slice(body).unwrap_or_default());
            }
        };
        match route {
            Route::DeviceCode => {
                keep(body);
                return (200, vec![], json(&device_code()));
            }
            Route::DeviceToken => {
                keep(body);
                let next = self
                    .polls
                    .lock()
                    .ok()
                    .and_then(|mut p| p.pop_front())
                    .unwrap_or("pending");
                if next == "approve" {
                    return (200, vec![], json(&approved()));
                }
                return (400, vec![], error_body(&format!("device.{next}"), next));
            }
            Route::Account if bearer => return (200, vec![], json(&account_reply())),
            Route::Revoke if bearer => {
                if let (Ok(mut tokens), Some(token)) =
                    (self.tokens.lock(), auth.and_then(read_bearer))
                {
                    tokens.remove(token);
                }
                return (204, vec![], Vec::new());
            }
            Route::Account | Route::Revoke => {
                return (401, vec![], error_body(codes::AUTH_REQUIRED, "Sign in"));
            }
            Route::Bundle(_) => {
                if self.limited.load(Ordering::SeqCst) {
                    let mut refusal = ErrorBody::new(codes::RATE_LIMITED, "Too many");
                    refusal.retry_after = Some(300);
                    return (429, vec![("retry-after", "300".into())], refusal.bytes());
                }
                if self.needs_account.load(Ordering::SeqCst) && !bearer {
                    return (
                        401,
                        vec![("www-authenticate", "Bearer".into())],
                        error_body(codes::AUTH_REQUIRED, "Sign in to download parts."),
                    );
                }
            }
            _ => {}
        }
        let (status, body) = self.answer(&route);
        (status, vec![], body)
    }

    fn answer(&self, route: &Route) -> (u16, Vec<u8>) {
        let hash = bytes_hash(&self.shard_bytes);
        match route {
            Route::Index => {
                let index = RegistryIndex {
                    format: FORMAT,
                    snapshot: "fixture-1".into(),
                    shards: vec![ShardInfo {
                        library: LIBRARY,
                        name: self.shard.catalog.name.clone(),
                        sha256: hash,
                        entries: self.shard.catalog.entries.len(),
                    }],
                };
                (200, json(&index))
            }
            Route::Shard(asked) if *asked == hash => (200, self.shard_bytes.clone()),
            Route::Search(query) => {
                let text = query.text.clone();
                if let Some((slow, wait)) = &self.slow
                    && *slow == text
                {
                    std::thread::sleep(*wait);
                }
                if let Ok(mut s) = self.searched.lock() {
                    s.push(text.clone());
                }
                let limit = usize::try_from(query.limit).unwrap_or(MAX_SEARCH_ROWS);
                let offset = usize::try_from(query.offset).unwrap_or(usize::MAX);
                let shards = std::slice::from_ref(&self.shard);
                match search_shards(shards, &text, limit.min(MAX_SEARCH_ROWS), offset) {
                    Ok(report) => {
                        let reply = wire::SearchReply {
                            format: FORMAT,
                            snapshot: "fixture-1".into(),
                            query: text,
                            chips: report.chips,
                            total: report.total,
                            offset: query.offset,
                            hidden: Vec::new(),
                            results: report
                                .results
                                .into_iter()
                                .map(|r| wire::SearchRow {
                                    hit: r.hit,
                                    id: r.id,
                                    review: r.review,
                                })
                                .collect(),
                            took_us: 7,
                        };
                        (200, json(&reply))
                    }
                    Err(message) => (400, error_body(codes::CATALOG_QUERY, &message)),
                }
            }
            Route::Bundle(root)
                if self
                    .bundle
                    .records
                    .iter()
                    .any(|r| r.lineage == root.lineage && r.revision == root.revision) =>
            {
                let mut bundle = closure(&self.bundle, *root);
                if self.tamper.load(Ordering::SeqCst)
                    && let Some(record) = bundle.records.last_mut()
                {
                    record.meta.description.push_str(" (changed on the way)");
                }
                (
                    200,
                    json(&wire::BundleReply {
                        format: FORMAT,
                        bundle,
                    }),
                )
            }
            _ => (404, error_body(codes::NOT_FOUND, "no such thing")),
        }
    }
}

/// A second token the fixture registry knows, for the conformance test to
/// revoke.
const SPEND: &str = "ssp_fixturespend0123456789abcdefghijklmnopqrst";

/// The parts contract's example documents (`crates/parts-contract/vectors`)
/// are what the core and the fixture registry write, byte for byte. Set
/// `SPARKSMITH_BLESS=1` to rewrite them after a deliberate change.
#[test]
fn the_contract_vectors_are_what_the_core_writes() -> TestResult {
    use sparksmith_parts_contract::account::{DeviceCodeRequest, DeviceTokenRequest};
    use sparksmith_parts_contract::vectors;
    let registry = registry(None)?;
    let root = registry.bundle.root;
    let search = Route::Search(SearchQuery {
        text: "EX-R0603".into(),
        limit: 2,
        offset: 0,
    });
    let mut rate_limited = ErrorBody::new(codes::RATE_LIMITED, "Too many");
    rate_limited.retry_after = Some(300);
    let routes = [
        Route::Index,
        Route::Shard(bytes_hash(&registry.shard_bytes)),
        search.clone(),
        Route::Bundle(root),
        Route::DeviceCode,
        Route::DeviceToken,
        Route::Account,
        Route::Revoke,
    ];
    let line = |method: &str, target: String| (method.to_owned(), target);
    let cases = vectors::Cases {
        shard: bytes_hash(&registry.shard_bytes),
        records: registry
            .bundle
            .records
            .iter()
            .map(sparksmith_core::library::bundle::content_hash)
            .collect::<std::result::Result<_, _>>()?,
        assets: registry.bundle.assets.iter().map(|a| a.hash).collect(),
        routes: routes
            .iter()
            .map(|r| line(r.method().as_str(), r.target()))
            .collect(),
        refused: vec![
            line("POST", "/v1/index".into()),
            line("GET", "/v1/index/".into()),
            line("GET", "/v2/index".into()),
            line("GET", "/v1/shards/ABCDEF".into()),
            line("GET", format!("/v1/shards/{}", "A".repeat(64))),
            line(
                "GET",
                format!("/v1/bundles/{}/r{}", root.lineage, root.revision.0),
            ),
            line(
                "GET",
                format!("/v1/bundles/{}/0{}", root.lineage, root.revision.0),
            ),
            line("GET", format!("/v1/bundles/{}/4294967296", root.lineage)),
            line("GET", "/v1/search?q=%E2%28".into()),
            line("GET", "/v1/search?q=ok&limit=many".into()),
            line("GET", format!("/v1/search?q={}", "x".repeat(513))),
            line("DELETE", "/v1/account".into()),
        ],
    };
    let files: [(&str, &[u8], Vec<u8>); 11] = [
        (
            "index.json",
            vectors::INDEX,
            registry.answer(&Route::Index).1,
        ),
        ("shard.json", vectors::SHARD, registry.shard_bytes.clone()),
        ("search.json", vectors::SEARCH, registry.answer(&search).1),
        (
            "bundle.json",
            vectors::BUNDLE,
            registry.answer(&Route::Bundle(root)).1,
        ),
        (
            "code-request.json",
            vectors::CODE_REQUEST,
            json(&DeviceCodeRequest {
                client_name: "Sparksmith on bench".into(),
            }),
        ),
        ("code.json", vectors::CODE, json(&device_code())),
        (
            "token-request.json",
            vectors::TOKEN_REQUEST,
            json(&DeviceTokenRequest {
                device_code: device_code().device_code,
            }),
        ),
        ("token.json", vectors::TOKEN, json(&approved())),
        ("account.json", vectors::ACCOUNT, json(&account_reply())),
        (
            "rate-limited.json",
            vectors::RATE_LIMITED,
            rate_limited.bytes(),
        ),
        (
            "cases.json",
            vectors::CASES,
            serde_json::to_vec_pretty(&cases)?,
        ),
    ];
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../parts-contract/vectors");
    let bless = std::env::var_os("SPARKSMITH_BLESS").is_some();
    for (name, held, written) in files {
        if bless {
            std::fs::write(folder.join(name), &written)?;
        } else {
            assert!(
                held == written.as_slice(),
                "vectors/{name} is not what the core writes; SPARKSMITH_BLESS=1 rewrites it"
            );
        }
    }
    assert!(bless || vectors::cases()? == cases);
    Ok(())
}

/// The fixture registry passes the parts contract's conformance test,
/// asked over its socket as a server's own test-suite would.
#[test]
fn the_fixture_registry_passes_the_contracts_conformance_test() -> TestResult {
    use sparksmith_parts_contract::conformance::{self, Expected, Outcome, Setup};
    use sparksmith_parts_contract::route::Method;
    let registry = registry(None)?;
    registry.needs_account.store(true, Ordering::SeqCst);
    if let Ok(mut tokens) = registry.tokens.lock() {
        tokens.insert(SPEND.into());
    }
    if let Ok(mut polls) = registry.polls.lock() {
        polls.extend(["pending", "slow_down", "expired", "denied", "approve"]);
    }
    let base = serve(registry)?;
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .build(),
    );
    let mut transport =
        |request: &conformance::Request| -> std::result::Result<conformance::Reply, String> {
            let url = format!("{base}{}", request.target);
            let auth = request.authorization.clone().unwrap_or_default();
            let sent = match request.method {
                Method::Get => agent.get(&url).header("authorization", &auth).call(),
                Method::Delete => agent.delete(&url).header("authorization", &auth).call(),
                Method::Post => agent
                    .post(&url)
                    .header("authorization", &auth)
                    .header("content-type", MEDIA_TYPE)
                    .send(request.body.clone().unwrap_or_default()),
            };
            let mut reply = sent.map_err(|e| e.to_string())?;
            let content_type = reply
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            let body = reply
                .body_mut()
                .with_config()
                .limit(request.max_reply)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            Ok(conformance::Reply {
                status: reply.status().as_u16(),
                content_type,
                body,
            })
        };
    let polls = [
        ("dev-secret-code", Expected::SlowDown),
        ("dev-secret-code", Expected::Expired),
        ("dev-secret-code", Expected::Denied),
        ("dev-secret-code", Expected::Approved),
    ];
    let report = conformance::run(
        &mut transport,
        Setup {
            search: "EX-R0603",
            refused_search: Some("pins:many"),
            token: Some(TOKEN),
            spend: Some(SPEND),
            polls: &polls,
        },
    );
    let failures = conformance::failures(&report);
    assert!(failures.is_empty(), "{failures}");
    let skipped: Vec<_> = report
        .iter()
        .filter(|(_, outcome)| *outcome != Outcome::Passed)
        .collect();
    assert!(skipped.is_empty(), "{skipped:?}");
    Ok(())
}

/// Serve `registry` on a local port; the URL.
fn serve(registry: Arc<Registry>) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let url = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let registry = registry.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(match stream.try_clone() {
                    Ok(s) => s,
                    Err(_) => return,
                });
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    return;
                }
                let mut auth = None;
                let mut length = 0usize;
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                    if let Some((name, value)) = header.split_once(':') {
                        let value = value.trim().to_owned();
                        match name.trim().to_ascii_lowercase().as_str() {
                            "authorization" => auth = Some(value),
                            "content-length" => length = value.parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                }
                let mut body = vec![0; length.min(64 * 1024)];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
                if !registry.up.load(Ordering::SeqCst) {
                    // Down: close without an answer, as an unreachable host.
                    return;
                }
                let mut words = line.split_whitespace();
                let method = words.next().unwrap_or("GET").to_owned();
                let path = words.next().unwrap_or("/").to_owned();
                if registry.holds(&path) {
                    // Never answered: until the client closes.
                    registry.held.fetch_add(1, Ordering::SeqCst);
                    let _ = reader.read(&mut [0]);
                    registry.let_go.fetch_add(1, Ordering::SeqCst);
                    return;
                }
                let (status, headers, body) =
                    registry.respond(&method, &path, auth.as_deref(), &body);
                let extra: String = headers
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}\r\n"))
                    .collect();
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            });
        }
    });
    Ok(url)
}

fn registry(
    slow: Option<(&str, Duration)>,
) -> std::result::Result<Arc<Registry>, Box<dyn std::error::Error>> {
    let (bundle, shard) = fixture()?;
    Ok(Arc::new(Registry {
        shard_bytes: serde_json::to_vec(&shard)?,
        shard: Arc::new(shard),
        bundle,
        searched: Mutex::new(Vec::new()),
        slow: slow.map(|(t, d)| (t.to_owned(), d)),
        tamper: AtomicBool::new(false),
        up: AtomicBool::new(true),
        silent: Mutex::new(String::new()),
        held: AtomicU64::new(0),
        let_go: AtomicU64::new(0),
        seen: Mutex::new(Vec::new()),
        needs_account: AtomicBool::new(false),
        limited: AtomicBool::new(false),
        closed: AtomicBool::new(false),
        polls: Mutex::new(VecDeque::new()),
        bodies: Mutex::new(Vec::new()),
        tokens: Mutex::new(BTreeSet::from([TOKEN.to_owned()])),
    }))
}

/// A client of `url` whose sign-in lives in `store`, never the system
/// keychain.
fn client_with(
    url: &str,
    cache: Option<&Path>,
    store: &Arc<Memory>,
) -> std::result::Result<RegistryClient, String> {
    RegistryClient::with_store(url, cache, store.clone())
}

/// A client with no sign-in, in a keychain of its own.
fn plain(url: &str, cache: Option<&Path>) -> std::result::Result<RegistryClient, String> {
    client_with(url, cache, &Arc::new(Memory::default()))
}

#[test]
fn sync_charges_downloaded_and_cached_shards() -> TestResult {
    let registry = registry(None)?;
    let size = registry.shard_bytes.len() as u64;
    let cache = tempfile::tempdir()?;
    let client = plain(&serve(registry)?, Some(cache.path()))?;
    assert!(matches!(client.fetch_sync(size - 1), Err(m) if m.contains("registry catalog budget")));
    assert!(client.fetch_sync(size).is_ok());
    assert!(matches!(client.fetch_sync(size - 1), Err(m) if m.contains("registry catalog budget")));
    Ok(())
}

#[test]
fn online_search_reads_the_same_query_and_marks_rows_online() -> TestResult {
    let registry = registry(None)?;
    let client = plain(&serve(registry.clone())?, None)?;
    let report = client.search("EX-R0603 kind:part", 10, 0)?;
    assert_eq!(report.offline, None);
    assert_eq!(report.total, 1, "{report:?}");
    let row = report.results.first().ok_or("no row")?;
    assert_eq!(row.hit.entry.source, Source::Online(LIBRARY));
    assert_eq!(row.review_label(), "not reviewed");
    // The registry's refusal of a term comes back as its message.
    let refused = client.search("pins:many", 10, 0);
    assert!(
        matches!(&refused, Err(m) if m.contains("pins")),
        "{refused:?}"
    );
    Ok(())
}

#[test]
fn a_tampered_bundle_is_refused_naming_the_record_and_never_mirrored() -> TestResult {
    let registry = registry(None)?;
    registry.tamper.store(true, Ordering::SeqCst);
    let cache = tempfile::tempdir()?;
    let url = serve(registry.clone())?;
    let client = plain(&url, Some(cache.path()))?;
    let refused = client.bundle(LIBRARY, registry.bundle.root);
    let message = refused.err().ok_or("a tampered bundle was accepted")?;
    assert!(
        message.contains("does not match its content hash"),
        "{message}"
    );
    let mirror = Mirror::open(cache.path(), &url)?;
    assert!(
        mirror
            .bundle(registry.bundle.root, &registry.shard.catalog)?
            .is_none()
    );
    Ok(())
}

#[test]
fn the_mirror_answers_search_and_bundles_when_the_registry_is_down() -> TestResult {
    let registry = registry(None)?;
    let cache = tempfile::tempdir()?;
    let url = serve(registry.clone())?;
    let client = plain(&url, Some(cache.path()))?;
    client.sync()?;
    let fetched = client.bundle(LIBRARY, registry.bundle.root)?;
    assert_eq!(fetched, registry.bundle);
    registry.up.store(false, Ordering::SeqCst);
    // A new client (the next session) on the same cache, offline.
    let offline = plain(&url, Some(cache.path()))?;
    let report = offline.search("EX-R0603 kind:part", 10, 0)?;
    assert!(
        report
            .offline
            .as_deref()
            .is_some_and(|d| d.starts_with("index of ")),
        "{report:?}"
    );
    assert_eq!(report.total, 1);
    let again = OnlineSource::bundle(&offline, LIBRARY, registry.bundle.root.lineage, None)?;
    assert_eq!(again.as_ref(), Some(&registry.bundle));
    // Another registry's library is not this one's.
    assert!(
        OnlineSource::bundle(&offline, LibraryId(1), registry.bundle.root.lineage, None)?.is_none()
    );
    Ok(())
}

/// The offline fallback pages as the registry does: page two of a search
/// follows page one instead of repeating it.
#[test]
fn the_offline_fallback_keeps_the_page_offset() -> TestResult {
    let registry = registry(None)?;
    let cache = tempfile::tempdir()?;
    let url = serve(registry.clone())?;
    let client = plain(&url, Some(cache.path()))?;
    client.sync()?;
    registry.up.store(false, Ordering::SeqCst);
    let offline = plain(&url, Some(cache.path()))?;
    let ids = |report: &OnlineReport| -> Vec<String> {
        report.results.iter().map(|r| r.id.clone()).collect()
    };
    let all = offline.search("kind:part", 10, 0)?;
    assert!(all.offline.is_some(), "{all:?}");
    for offset in 0..=all.results.len() {
        let page = offline.search("kind:part", 1, offset)?;
        assert_eq!(
            ids(&page),
            ids(&all)
                .into_iter()
                .skip(offset)
                .take(1)
                .collect::<Vec<_>>(),
            "offline page at {offset}"
        );
        assert_eq!(page.total, all.total, "the total is the whole search's");
    }
    Ok(())
}

fn wait_for(
    worker: &RegistryWorker,
    until: impl Fn(&[RegistryEvent]) -> bool,
) -> Vec<RegistryEvent> {
    let started = Instant::now();
    let mut all = Vec::new();
    while started.elapsed() < Duration::from_secs(5) {
        all.extend(worker.receive());
        if until(&all) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    all
}

#[test]
fn a_slow_older_search_is_dropped_and_typing_is_debounced() -> TestResult {
    let registry = registry(Some(("EX-R", Duration::from_millis(400))))?;
    let client = Arc::new(plain(&serve(registry.clone())?, None)?);
    let worker = RegistryWorker::start(client, Duration::from_millis(40), Arc::new(|| {}));
    // Typing quickly: only the last text of a burst is sent.
    worker.search(1, "E", 10);
    worker.search(2, "EX", 10);
    worker.search(3, "EX-R", 10);
    // The slow request is in flight when the next text arrives.
    std::thread::sleep(Duration::from_millis(150));
    worker.search(4, "EX-C", 10);
    let events = wait_for(&worker, |e| {
        e.iter()
            .any(|e| matches!(e, RegistryEvent::Search { generation: 4, .. }))
    });
    // Let the slow reply land too, then look again.
    std::thread::sleep(Duration::from_millis(500));
    let mut events = events;
    events.extend(worker.receive());
    let generations: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            RegistryEvent::Search { generation, .. } => Some(*generation),
            _ => None,
        })
        .collect();
    assert_eq!(generations, [4], "only the newest search is reported");
    let searched = registry
        .searched
        .lock()
        .map(|s| s.clone())
        .unwrap_or_default();
    assert!(
        !searched.iter().any(|t| t == "E" || t == "EX"),
        "debounced: {searched:?}"
    );
    if let Some(RegistryEvent::Search { result, text, .. }) = events.first() {
        assert_eq!(text, "EX-C");
        let report = result.as_ref().map_err(|e| e.clone())?;
        assert!(report.results.iter().all(|r| {
            r.hit.entry.name.contains("EX-C")
                || r.hit
                    .entry
                    .mpn
                    .as_deref()
                    .is_some_and(|m| m.contains("EX-C"))
        }));
    }
    Ok(())
}

#[test]
fn index_dates_read_as_people_write_them() {
    assert_eq!(index_date(0), "index of 1 January");
    // 2026-11-14 00:00 UTC.
    assert_eq!(index_date(1_794_614_400), "index of 14 November");
}

impl Registry {
    /// Never answer a request whose path has `part` in it (empty: all are).
    fn hold(&self, part: &str) {
        if let Ok(mut silent) = self.silent.lock() {
            *silent = part.to_owned();
        }
    }

    fn holds(&self, path: &str) -> bool {
        self.silent
            .lock()
            .is_ok_and(|part| !part.is_empty() && path.contains(part.as_str()))
    }

    fn held(&self) -> u64 {
        self.held.load(Ordering::SeqCst)
    }

    fn wait_let_go(&self, requests: u64) {
        while self.let_go.load(Ordering::SeqCst) < requests {
            std::thread::yield_now();
        }
    }
}

fn limited(seconds: u32) -> Network {
    Network {
        search: Wait::Limit(seconds),
        index: Wait::Limit(seconds),
        download: Wait::Limit(seconds),
        account: Wait::Limit(seconds),
        sign_out: Wait::Limit(seconds),
        ..Network::NO_LIMITS
    }
}

/// A silent registry is given up at the person's limit, which the message
/// names with its place; the mirror then answers a search. A limit set
/// later holds from then on.
#[test]
fn a_silent_registry_is_given_up_at_the_persons_limit() -> TestResult {
    let registry = registry(None)?;
    let cache = tempfile::tempdir()?;
    let client = plain(&serve(registry.clone())?, Some(cache.path()))?;
    client.sync()?;
    client.set_network(limited(1));
    registry.hold("/v1/");
    let offline = client.search("EX-R0603", 10, 0)?;
    assert!(offline.offline.is_some(), "the last index answered");
    let refused = client
        .bundle(LIBRARY, registry.bundle.root)
        .err()
        .ok_or("a silent download ended")?;
    assert!(
        refused.contains("for 1 second, the limit set in Settings › Network"),
        "{refused}"
    );
    registry.wait_let_go(2);
    // Tried again, it is there.
    registry.hold("");
    client.bundle(LIBRARY, registry.bundle.root)?;
    Ok(())
}

/// With "No limit", a silent registry is waited for until the call is
/// cancelled, which closes the connection; the next request is answered.
#[test]
fn without_a_limit_a_cancel_ends_the_wait() -> TestResult {
    let registry = registry(None)?;
    let client = Arc::new(plain(&serve(registry.clone())?, None)?);
    client.sync()?;
    registry.hold("/v1/bundles");
    let cancel = Cancel::default();
    let (own, call, root) = (client.clone(), cancel.clone(), registry.bundle.root);
    let request = sparksmith_core::library::standards::carried(move || {
        call.scope(|| own.bundle(LIBRARY, root).map(|_| ()))
    });
    let waiting = std::thread::spawn(request);
    while registry.held() == 0 {
        std::thread::yield_now();
    }
    cancel.set();
    let ended = waiting.join().map_err(|_| "the request's thread")?;
    assert_eq!(ended.err().as_deref(), Some(STOPPED));
    registry.wait_let_go(1);
    registry.hold("");
    client.bundle(LIBRARY, registry.bundle.root)?;
    Ok(())
}

/// A silent search and a silent download hold up no other request: a newer
/// search ends the older one's, and Stop the download's.
#[test]
fn a_silent_request_holds_up_no_other() -> TestResult {
    let registry = registry(None)?;
    let client = Arc::new(plain(&serve(registry.clone())?, None)?);
    client.sync()?;
    registry.hold("SILENT");
    let worker = RegistryWorker::start(client, Duration::ZERO, Arc::new(|| {}));
    worker.search(1, "SILENT", 10);
    while registry.held() == 0 {
        std::thread::yield_now();
    }
    worker.search(2, "EX-C", 10);
    registry.wait_let_go(1);
    let events = wait_for(&worker, |e| !e.is_empty());
    assert!(
        matches!(
            events.as_slice(),
            [RegistryEvent::Search {
                generation: 2,
                result: Ok(_),
                ..
            }]
        ),
        "{events:?}"
    );
    registry.hold("/v1/bundles");
    let root = registry.bundle.root;
    worker.bundle(7, LIBRARY, root);
    while registry.held() < 2 {
        std::thread::yield_now();
    }
    worker.stop_bundle(7);
    registry.wait_let_go(2);
    let events = wait_for(&worker, |e| !e.is_empty());
    assert!(
        matches!(
            events.as_slice(),
            [RegistryEvent::Bundle { ticket: 7, result: Err(why), .. }] if why == STOPPED
        ),
        "{events:?}"
    );
    Ok(())
}

fn seen(registry: &Registry) -> Vec<(String, Option<String>)> {
    registry.seen.lock().map(|s| s.clone()).unwrap_or_default()
}

/// The sign-in's token goes with bundle downloads and nothing else: not
/// with a search, the index or a shard, and never to another registry.
#[test]
fn the_token_goes_with_bundles_only_and_only_to_its_registry() -> TestResult {
    let registry = registry(None)?;
    registry.needs_account.store(true, Ordering::SeqCst);
    let url = serve(registry.clone())?;
    let store = Arc::new(Memory::default());
    store.set(account::SERVICE, &url, TOKEN)?;
    let signed_in = client_with(&url, None, &store)?;
    signed_in.search("EX-R0603 kind:part", 10, 0)?;
    let bundle = signed_in.bundle(LIBRARY, registry.bundle.root)?;
    assert_eq!(bundle, registry.bundle);
    let requests = seen(&registry);
    for (request, auth) in &requests {
        if request.starts_with("GET /v1/bundles/") {
            assert_eq!(auth.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
        } else {
            assert_eq!(auth, &None, "{request} carried the token");
        }
    }
    assert!(
        requests
            .iter()
            .any(|(r, _)| r.starts_with("GET /v1/search"))
    );
    assert!(requests.iter().any(|(r, _)| r == "GET /v1/index"));

    // Another registry, same keychain: its own entry is empty, so nothing
    // is sent, and its refusal says to sign in.
    let other = self::registry(None)?;
    other.needs_account.store(true, Ordering::SeqCst);
    let elsewhere = client_with(&serve(other.clone())?, None, &store)?;
    let refused = elsewhere.bundle(LIBRARY, other.bundle.root);
    assert_eq!(refused.err().as_deref(), Some(account::SIGN_IN_NEEDED));
    assert!(seen(&other).iter().all(|(_, auth)| auth.is_none()));
    Ok(())
}

/// A download without an account is refused with the words saying where
/// to sign in; a limit says when to try again. Search still works.
#[test]
fn refusals_say_to_sign_in_or_when_to_retry() -> TestResult {
    let registry = registry(None)?;
    registry.needs_account.store(true, Ordering::SeqCst);
    let client = plain(&serve(registry.clone())?, None)?;
    assert_eq!(client.search("EX-R0603 kind:part", 10, 0)?.total, 1);
    let refused = client.bundle(LIBRARY, registry.bundle.root).err();
    assert_eq!(refused.as_deref(), Some(account::SIGN_IN_NEEDED));
    registry.limited.store(true, Ordering::SeqCst);
    let limited = client.bundle(LIBRARY, registry.bundle.root).err();
    assert_eq!(
        limited.as_deref(),
        Some("The online library's download limit is reached. Try again in 5 minutes.")
    );
    Ok(())
}

/// A token the registry no longer knows is forgotten when a download is
/// refused, and Settings then shows signed out.
#[test]
fn a_refused_token_is_forgotten() -> TestResult {
    let registry = registry(None)?;
    registry.needs_account.store(true, Ordering::SeqCst);
    let url = serve(registry.clone())?;
    let store = Arc::new(Memory::default());
    store.set(account::SERVICE, &url, "ssp_revoked")?;
    let client = client_with(&url, None, &store)?;
    assert!(client.account_status().signed_in);
    let refused = client.bundle(LIBRARY, registry.bundle.root).err();
    assert_eq!(refused.as_deref(), Some(account::SIGN_IN_NEEDED));
    assert_eq!(store.get(account::SERVICE, &url)?, None);
    assert_eq!(client.known(), Some(Known::SignedOut));
    Ok(())
}

#[test]
fn sign_in_button_names_the_origin_of_its_parsed_destination() -> TestResult {
    use account::VerificationPages;
    let manual = "https://parts.example/device";
    for (complete, origin, separate) in [
        ("", "https://parts.example", false),
        (
            "https://PARTS.example:443/approve?c=123",
            "https://parts.example",
            false,
        ),
        (
            "https://identity.example:8443/approve?c=123",
            "https://identity.example:8443",
            true,
        ),
    ] {
        let pages = VerificationPages::parse(manual, complete)?;
        assert_eq!(pages.browser().origin().ascii_serialization(), origin);
        assert!(pages.button_label().contains(origin));
        assert_eq!(pages.button_label().contains("identity provider"), separate);
    }
    for invalid in [
        "file:///device",
        "https://user@example.org/device",
        "http://remote.example/device",
        "/relative",
    ] {
        assert!(VerificationPages::parse(invalid, manual).is_err());
        assert!(VerificationPages::parse(manual, invalid).is_err());
    }
    Ok(())
}

/// The device flow: a code under the client's name, polled at the
/// registry's interval, 5 s longer after `slow_down`, until approved; the
/// token goes to the keychain and the account is known.
#[test]
fn signing_in_polls_until_approved_and_keeps_the_token() -> TestResult {
    let registry = registry(None)?;
    if let Ok(mut polls) = registry.polls.lock() {
        polls.extend(["pending", "slow_down", "pending", "approve"]);
    }
    let url = serve(registry.clone())?;
    let store = Arc::new(Memory::default());
    let client = client_with(&url, None, &store)?;
    assert_eq!(
        client.known(),
        None,
        "the keychain is not read before asked"
    );
    let code = client.start_sign_in("Sparksmith on bench")?;
    assert_eq!(code.user_code, "BCDF-GHJK");
    assert_eq!(
        code.pages.browser().as_str(),
        "https://parts.example/device?code=BCDF-GHJK"
    );
    assert!(!format!("{code:?}").contains("dev-secret-code"));
    let waits = Mutex::new(Vec::new());
    let sleep = |_: &AtomicBool, wait: Duration| {
        if let Ok(mut w) = waits.lock() {
            w.push(wait.as_secs());
        }
        false
    };
    let account = client.wait_for_sign_in(&code, &AtomicBool::new(false), &sleep)?;
    assert_eq!(account.email, "maker@example.com");
    assert_eq!(account.label(), "Maker");
    assert_eq!(
        waits.lock().map(|w| w.clone()).unwrap_or_default(),
        [1, 1, 6, 6]
    );
    assert_eq!(store.get(account::SERVICE, &url)?.as_deref(), Some(TOKEN));
    assert_eq!(client.known(), Some(Known::SignedIn(Some(account))));
    let status = client.account_status();
    assert!(status.signed_in);
    assert_eq!(status.email.as_deref(), Some("maker@example.com"));
    let bodies = registry
        .bodies
        .lock()
        .map(|b| b.clone())
        .unwrap_or_default();
    assert_eq!(bodies[0]["client_name"], "Sparksmith on bench");
    assert_eq!(bodies[1]["device_code"], "dev-secret-code");
    // Polls never carry a token.
    assert!(seen(&registry).iter().all(|(_, auth)| auth.is_none()));
    Ok(())
}

/// Declined, expired, past its time or cancelled: no token is kept.
#[test]
fn a_declined_expired_or_cancelled_sign_in_keeps_nothing() -> TestResult {
    let registry = registry(None)?;
    let url = serve(registry.clone())?;
    let store = Arc::new(Memory::default());
    let client = client_with(&url, None, &store)?;
    let no_wait = |_: &AtomicBool, _: Duration| false;
    let go = AtomicBool::new(false);
    for (answer, said) in [
        ("denied", account::SIGN_IN_DENIED),
        ("expired", account::SIGN_IN_EXPIRED),
    ] {
        if let Ok(mut polls) = registry.polls.lock() {
            polls.push_back(answer);
        }
        let code = client.start_sign_in("Sparksmith")?;
        let ended = client.wait_for_sign_in(&code, &go, &no_wait);
        assert_eq!(ended.err().as_deref(), Some(said));
    }
    assert_eq!(
        client.poll_sign_in(&client.start_sign_in("Sparksmith")?)?,
        Poll::Pending
    );
    // Pending past `expires_in`: given up after that long.
    let mut code = client.start_sign_in("Sparksmith")?;
    code.expires_in = 3;
    let polls = registry.bodies.lock().map(|b| b.len()).unwrap_or_default();
    let ended = client.wait_for_sign_in(&code, &go, &no_wait);
    assert_eq!(ended.err().as_deref(), Some(account::SIGN_IN_EXPIRED));
    let after = registry.bodies.lock().map(|b| b.len()).unwrap_or_default();
    assert_eq!(after - polls, 3, "one poll a second for three seconds");
    // Cancelled while waiting.
    let stop = AtomicBool::new(true);
    let ended = client.wait_for_sign_in(&code, &stop, &account::sleep_unless);
    assert_eq!(ended.err().as_deref(), Some(account::SIGN_IN_CANCELLED));
    assert_eq!(store.get(account::SERVICE, &url)?, None);
    assert_eq!(client.known(), None);
    Ok(())
}

/// Sign-out revokes the token at the registry and removes it from the
/// keychain; offline, it is removed all the same and says it was not
/// revoked.
#[test]
fn signing_out_revokes_and_forgets_the_token() -> TestResult {
    let registry = registry(None)?;
    let url = serve(registry.clone())?;
    let store = Arc::new(Memory::default());
    store.set(account::SERVICE, &url, TOKEN)?;
    let client = client_with(&url, None, &store)?;
    assert_eq!(
        client.refresh_account()?,
        Some(Account {
            id: "a1".into(),
            email: "maker@example.com".into(),
            display_name: String::new(),
            providers: vec!["email".into()],
        })
    );
    let out = client.sign_out()?;
    assert!(
        out.was_signed_in && out.revoked && out.problem.is_none(),
        "{out:?}"
    );
    assert!(
        seen(&registry)
            .iter()
            .any(|(r, auth)| r == "DELETE /v1/auth/token"
                && auth.as_deref() == Some(format!("Bearer {TOKEN}").as_str()))
    );
    assert_eq!(store.get(account::SERVICE, &url)?, None);
    assert!(!client.account_status().signed_in);
    assert!(!client.sign_out()?.was_signed_in);

    store.set(account::SERVICE, &url, TOKEN)?;
    let offline = client_with(&url, None, &store)?;
    registry.up.store(false, Ordering::SeqCst);
    let out = offline.sign_out()?;
    assert!(
        out.was_signed_in && !out.revoked && out.problem.is_some(),
        "{out:?}"
    );
    assert!(!out.problem.unwrap_or_default().contains(TOKEN));
    assert_eq!(store.get(account::SERVICE, &url)?, None);
    Ok(())
}

/// A registry over plain HTTP on another computer is never sent a token
/// and cannot be signed in to.
#[test]
fn no_token_travels_over_plain_http_to_another_computer() -> TestResult {
    let url = "http://192.168.1.20:8787";
    let store = Arc::new(Memory::default());
    store.set(account::SERVICE, url, TOKEN)?;
    let client = client_with(url, None, &store)?;
    assert!(!client.may_sign_in());
    assert!(client.token().is_none());
    assert!(!client.account_status().signed_in);
    let refused = client.start_sign_in("Sparksmith").err().unwrap_or_default();
    assert!(refused.contains("https://"), "{refused}");
    Ok(())
}

/// Prepare the same destinations as the HTTP sender, without DNS, sockets
/// or a system keychain. Only an anonymous read is public.
#[test]
fn registry_credentials_follow_the_parsed_endpoint() -> TestResult {
    let token = Secret::new("inert-test-marker").ok_or("missing marker")?;
    let poll = Secret::new("inert-code-marker").ok_or("missing marker")?;
    for (base, local, allowed) in [
        ("http://LOCALHOST:8787", true, true),
        ("http://127.1.2.3", true, true),
        ("http://[::1]:8787", true, true),
        ("http://[::ffff:127.0.0.1]", true, true),
        ("http://localhost.parts.example", false, false),
        ("http://127.0.0.1.parts.example", false, false),
        ("http://parts.example", false, false),
        ("http://192.0.2.1", false, false),
        ("https://parts.example", false, true),
    ] {
        let client = plain(base, None)?;
        assert_eq!(client.endpoint.is_loopback(), local, "{base}");
        assert_eq!(client.may_sign_in(), allowed, "{base}");
        assert_eq!(
            client.request_url(Request::Read(None), &Route::Index)?,
            format!("{base}/v1/index")
        );
        let root = SourceRevision {
            lineage: SourceLineageId(1),
            revision: Revision(1),
        };
        for (request, route) in [
            (Request::SignIn("test"), Route::DeviceCode),
            (Request::Poll(&poll), Route::DeviceToken),
            (Request::Read(Some(&token)), Route::Bundle(root)),
            (Request::Read(Some(&token)), Route::Account),
            (Request::Revoke(&token), Route::Revoke),
        ] {
            let url = client.request_url(request, &route);
            let path = route.target();
            if allowed {
                assert_eq!(url?, format!("{base}{path}"));
            } else {
                assert!(url.is_err_and(|e| e.contains("https://")), "{base}{path}");
            }
        }
    }
    Ok(())
}

#[test]
fn the_client_name_names_this_computer_within_its_bound() {
    let product = sparksmith_core::product::NAME;
    assert_eq!(
        account::client_name_for("bench-mac.local"),
        format!("{product} on bench-mac")
    );
    assert_eq!(account::client_name_for("  "), product);
    assert_eq!(
        account::client_name_for("\u{1b}[31m"),
        format!("{product} on 31m")
    );
    let long = account::client_name_for(&"a".repeat(200));
    assert_eq!(long.chars().count(), account::MAX_CLIENT_NAME);
    assert!(account::client_name().starts_with(product));
}

/// Against a running registry (`SPARKSMITH_REGISTRY_URL`, e.g. the
/// sparksmith-parts server on http://localhost:8787): sync, search with the
/// app's own query text, fetch, check and mirror a bundle, then reuse it
/// offline. `cargo test -p sparksmith-assist live_registry -- --ignored`.
#[test]
#[ignore = "needs a running registry"]
fn live_registry_round_trip() -> TestResult {
    let url = std::env::var("SPARKSMITH_REGISTRY_URL")?;
    let cache = tempfile::tempdir()?;
    let client = plain(&url, Some(cache.path()))?;
    let started = Instant::now();
    let synced = client.sync()?;
    println!(
        "sync: {} shards in {:?}",
        synced.shards.len(),
        started.elapsed()
    );
    for text in [
        "usb",
        "header",
        "pins:10",
        "has:3d kind:part",
        "resistor",
        "src:online led",
    ] {
        let started = Instant::now();
        let report = client.search(text, 30, 0)?;
        println!(
            "{text:>18}: {:>3} of {:>3}, server {} µs, round trip {:.2} ms",
            report.results.len(),
            report.total,
            report.took_us,
            started.elapsed().as_secs_f64() * 1000.0
        );
        let offline = search_shards(&synced.shards, text, 30, 0)?;
        assert_eq!(
            offline.total, report.total,
            "{text}: mirror and registry agree"
        );
    }
    let row = client
        .search("usb", 1, 0)?
        .results
        .into_iter()
        .next()
        .ok_or("no usb row")?;
    let library = row.hit.entry.source.library().ok_or("no library")?;
    let root = SourceRevision {
        lineage: row.hit.entry.source_lineage.ok_or("no lineage")?,
        revision: row.hit.entry.revision.ok_or("no revision")?,
    };
    let started = Instant::now();
    let bundle = client.bundle(library, root)?;
    println!(
        "bundle: {} records, {} assets in {:?}",
        bundle.records.len(),
        bundle.assets.len(),
        started.elapsed()
    );
    let mirror = Mirror::open(cache.path(), &url)?;
    let shard = synced.shard(library).ok_or("no shard")?;
    assert_eq!(mirror.bundle(root, &shard.catalog)?.as_ref(), Some(&bundle));
    Ok(())
}

/// A server behind a password answers 401 to a search: the refusal says
/// the library is not open, never that searching needs a sign-in (E2E
/// run 13 finding 3).
#[test]
fn a_registry_behind_a_password_says_it_is_not_open()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let registry = self::registry(None)?;
    registry.closed.store(true, Ordering::SeqCst);
    let store = Arc::new(Memory::default());
    let client = client_with(&serve(registry)?, None, &store)?;
    let refused = client.search("resistor", 5, 0).err().unwrap_or_default();
    assert!(refused.contains("is not open to the public"), "{refused}");
    assert_ne!(refused, account::SIGN_IN_NEEDED);
    Ok(())
}

#[test]
fn registry_diagnostics_redact_device_codes_and_bearers_in_memory() -> TestResult {
    let marker = Secret::new("fixture-sensitive-text").ok_or("missing marker")?;
    let message = "could not use fixture-sensitive-text";
    let client = plain("https://parts.example", None)?;
    for request in [
        Request::Poll(&marker),
        Request::Read(Some(&marker)),
        Request::Revoke(&marker),
    ] {
        assert_eq!(request.clean(message), "could not use [redacted]");
        let reply = Reply::received(
            request,
            403,
            None,
            serde_json::to_vec(&serde_json::json!({"error": {"message": message}}))?,
        );
        assert_eq!(
            client.refusal(&reply),
            "the registry refused: could not use [redacted]"
        );
    }
    Ok(())
}
