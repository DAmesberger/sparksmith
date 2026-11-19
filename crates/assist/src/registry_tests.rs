//! The registry client against a small registry on a local socket that
//! answers with the core's own search over an original fixture shard:
//! search, the offline mirror, refused bundles and dropped stale replies.
use super::*;
use sparksmith_core::catalog::source_catalog;
use sparksmith_core::ids::{DefinitionKind, DefinitionRef};
use sparksmith_io::registry_mirror::{ShardEntry, ShardInfo};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicBool;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

const LIBRARY: LibraryId = LibraryId(0x5eed);

/// The ExampleCo resistor Part's bundle and its shard.
fn fixture() -> std::result::Result<(Bundle, Shard), Box<dyn std::error::Error>> {
    let library =
        sparksmith_core::catalog::synthetic::project_with_parts(1, &NewBoard::fixture())?.library;
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
            format: mirror::FORMAT,
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
}

fn decode(text: &str) -> String {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = text.get(i + 1..i + 3).unwrap_or("20");
                out.push(u8::from_str_radix(hex, 16).unwrap_or(b' '));
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Registry {
    fn answer(&self, path: &str) -> (u16, Vec<u8>) {
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        let params: BTreeMap<&str, String> = query
            .split('&')
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k, decode(v)))
            .collect();
        let hash = sparksmith_core::library::bundle::asset_hash(&self.shard_bytes);
        if route == "/v1/index" {
            let index = RegistryIndex {
                format: mirror::FORMAT,
                snapshot: "fixture-1".into(),
                shards: vec![ShardInfo {
                    library: LIBRARY,
                    name: self.shard.catalog.name.clone(),
                    sha256: hash,
                    entries: self.shard.catalog.entries.len(),
                }],
            };
            return (200, serde_json::to_vec(&index).unwrap_or_default());
        }
        if route == format!("/v1/shards/{hash}") {
            return (200, self.shard_bytes.clone());
        }
        if route == "/v1/search" {
            let text = params.get("q").cloned().unwrap_or_default();
            if let Some((slow, wait)) = &self.slow
                && *slow == text
            {
                std::thread::sleep(*wait);
            }
            if let Ok(mut s) = self.searched.lock() {
                s.push(text.clone());
            }
            return match search_shards(std::slice::from_ref(&self.shard), &text, 30) {
                Ok(report) => {
                    let rows: Vec<serde_json::Value> = report
                        .results
                        .iter()
                        .map(|r| serde_json::json!({"hit": r.hit, "id": r.id, "review": r.review}))
                        .collect();
                    let body = serde_json::json!({"format": 1, "snapshot": "fixture-1", "query": text,
                        "chips": report.chips, "total": report.total, "offset": 0, "results": rows, "took_us": 7});
                    (200, serde_json::to_vec(&body).unwrap_or_default())
                }
                Err(message) => (
                    400,
                    serde_json::to_vec(&serde_json::json!({"error": {"code": "catalog.query", "message": message}}))
                        .unwrap_or_default(),
                ),
            };
        }
        let root = self.bundle.root;
        if route == format!("/v1/bundles/{}/{}", root.lineage, root.revision.0) {
            let mut bundle = self.bundle.clone();
            if self.tamper.load(Ordering::SeqCst)
                && let Some(record) = bundle.records.last_mut()
            {
                record.meta.description.push_str(" (changed on the way)");
            }
            let body = serde_json::json!({"format": 1, "bundle": bundle});
            return (200, serde_json::to_vec(&body).unwrap_or_default());
        }
        (
            404,
            br#"{"error":{"code":"registry.not_found","message":"no such thing"}}"#.to_vec(),
        )
    }
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
                loop {
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
                        break;
                    }
                }
                if !registry.up.load(Ordering::SeqCst) {
                    // Down: close without an answer, as an unreachable host.
                    return;
                }
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (status, body) = registry.answer(&path);
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
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
    }))
}

#[test]
fn online_search_reads_the_same_query_and_marks_rows_online() -> TestResult {
    let registry = registry(None)?;
    let client = RegistryClient::new(&serve(registry.clone())?, None)?;
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
    let client = RegistryClient::new(&url, Some(cache.path()))?;
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
    let client = RegistryClient::new(&url, Some(cache.path()))?;
    client.sync()?;
    let fetched = client.bundle(LIBRARY, registry.bundle.root)?;
    assert_eq!(fetched, registry.bundle);
    registry.up.store(false, Ordering::SeqCst);
    // A new client (the next session) on the same cache, offline.
    let offline = RegistryClient::new(&url, Some(cache.path()))?;
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
    let client = Arc::new(RegistryClient::new(&serve(registry.clone())?, None)?);
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

/// Against a running registry (`SPARKSMITH_REGISTRY_URL`, e.g. the
/// sparksmith-parts server on http://localhost:8787): sync, search with the
/// app's own query text, fetch, check and mirror a bundle, then reuse it
/// offline. `cargo test -p sparksmith-assist live_registry -- --ignored`.
#[test]
#[ignore = "needs a running registry"]
fn live_registry_round_trip() -> TestResult {
    let url = std::env::var("SPARKSMITH_REGISTRY_URL")?;
    let cache = tempfile::tempdir()?;
    let client = RegistryClient::new(&url, Some(cache.path()))?;
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
        let offline = search_shards(&synced.shards, text, 30)?;
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
