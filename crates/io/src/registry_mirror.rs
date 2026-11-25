//! The offline mirror of an online parts registry (part-search §6.4,
//! registry R0 §10): `<cache>/registry/origins/<origin-digest>/` holds the last index the
//! registry named, its shards (each a `SourceCatalog` by SHA-256 of its
//! bytes), and every revision record and asset a placed or previewed part
//! needed, by content hash. Never authoritative and never part of a
//! project: placing imports an exact snapshot, so a project opens the same
//! without the network.
//!
//! Every byte is checked before use. A shard must hash to the name the
//! index gave it; a bundle's records must hash to the content hashes its
//! shard lists for exactly those revisions (`SourceCatalog::revisions`),
//! and its assets to their names ([`validate_bundle`]). A mismatch is
//! refused, naming the record, and nothing is stored. Reading the mirror
//! back checks the same again, so a changed cache file is never trusted.
//!
//! The wire types and their bounds are the parts contract's
//! (`sparksmith_parts_contract`). Run off the UI thread.
use crate::persistence::atomic_write_with;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use sparksmith_core::catalog::{Source, SourceCatalog};
use sparksmith_core::ids::{ContentHash, LibraryId, Revision, SourceLineageId};
use sparksmith_core::library::bundle::{
    AssetBytes, Bundle, PackedAsset, RevisionRecord, SourceRevision, asset_hash, content_hash,
    validate_bundle,
};
use sparksmith_parts_contract::documents::{self, Index};
use sparksmith_parts_contract::limits::{MAX_INDEX_BYTES, MAX_SHARD_BYTES};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub struct ShardBudget(pub u64);

impl ShardBudget {
    pub fn charge(&mut self, bytes: usize) -> Result<()> {
        self.0 = self
            .0
            .checked_sub(bytes as u64)
            .ok_or_else(|| Error::TooLarge("registry catalog budget exceeded".into()))?;
        Ok(())
    }

    /// Missing or damaged files can be fetched again. A valid file consumes the
    /// same budget as a fresh download, including repeated index entries.
    pub fn cached(&mut self, mirror: &Mirror, hash: ContentHash) -> Result<Option<Shard>> {
        let Ok(bytes) = read_limited(&mirror.shard_path(hash), MAX_SHARD_BYTES) else {
            return Ok(None);
        };
        if asset_hash(&bytes) != hash {
            return Ok(None);
        }
        self.charge(bytes.len())?;
        match documents::parse_shard(&bytes) {
            Ok(shard) => Ok(Some(online(shard))),
            Err(_) => {
                self.0 += bytes.len() as u64;
                Ok(None)
            }
        }
    }
}

/// Bytes of one mirrored record file.
pub const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;
/// Bytes of one mirrored asset.
pub const MAX_ASSET_BYTES: u64 = sparksmith_core::library::bundle::MAX_ASSET_BYTES as u64;
/// Bytes of a registry host key (the mirror's folder name).
pub const MAX_HOST_KEY: usize = 96;

/// `GET /v1/shards/{sha256}`, its entries marked [`Source::Online`].
pub type Shard = documents::Shard<SourceCatalog>;

/// The mirror's own copy of the index, with when it was fetched.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MirroredIndex {
    pub index: Index,
    /// Seconds since the Unix epoch when the index was fetched.
    pub fetched: u64,
}

/// A mirrored bundle: its records and assets by hash.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct BundleFile {
    library: LibraryId,
    name: String,
    root: SourceRevision,
    records: Vec<ContentHash>,
    assets: Vec<(ContentHash, String)>,
}

/// One registry's mirror directory.
#[derive(Clone, Debug)]
pub struct Mirror {
    root: PathBuf,
    origin: String,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    origin: String,
    snapshot: MirroredIndex,
}

/// Canonical scheme, host and effective port of an already checked registry URL.
fn origin(url: &str) -> Result<String> {
    let bad = || Error::Invalid("The registry URL needs a valid host and port".into());
    let uri = url.parse::<http::Uri>().map_err(|_| bad())?;
    let authority = uri.authority().ok_or_else(bad)?;
    let scheme = uri.scheme_str().ok_or_else(bad)?;
    let host = authority.host();
    let port = match authority.as_str().strip_prefix(host).ok_or_else(bad)? {
        "" if scheme == "https" => 443,
        "" => 80,
        suffix => suffix
            .strip_prefix(':')
            .ok_or_else(bad)?
            .parse::<u16>()
            .map_err(|_| bad())?,
    };
    let host = if let Some(literal) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        format!(
            "[{}]",
            literal.parse::<std::net::Ipv6Addr>().map_err(|_| bad())?
        )
    } else {
        host.to_ascii_lowercase()
    };
    Ok(format!("{scheme}://{host}:{port}"))
}

/// The folder name for a registry URL: its host and port, lower case,
/// every other character `_` (`http://localhost:8787` → `localhost_8787`).
pub fn host_key(url: &str) -> Result<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| {
            Error::Invalid(format!(
                "{url}: a registry URL starts with http:// or https://"
            ))
        })?;
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() {
        return Err(Error::Invalid(format!(
            "{url}: the registry URL names no host"
        )));
    }
    let key: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if key.len() > MAX_HOST_KEY || key.starts_with('.') {
        return Err(Error::Invalid(format!(
            "{url}: the registry host is not usable as a folder name"
        )));
    }
    Ok(key)
}

/// Seconds since the Unix epoch, 0 before it.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Check a shard's bytes against the hash the index gave and decode it,
/// with every entry marked as coming from this registry.
pub fn decode_shard(bytes: &[u8], expected: ContentHash) -> Result<Shard> {
    Ok(online(documents::decode_shard(bytes, expected)?))
}

fn online(mut shard: Shard) -> Shard {
    let library = shard.catalog.library;
    for entry in &mut shard.catalog.entries {
        entry.source = Source::Online(library);
    }
    shard
}

/// Check a registry bundle before it is used or stored: it is the asked
/// root of `catalog`'s library, structurally valid, its assets hash to
/// their names, and every record hashes to the content hash `catalog`
/// lists for that exact revision. Refusals name the record.
pub fn verify_bundle(bundle: &Bundle, root: SourceRevision, catalog: &SourceCatalog) -> Result<()> {
    if bundle.root != root {
        return Err(Error::Invalid(format!(
            "the registry sent {} {} for {} {}",
            bundle.root.lineage, bundle.root.revision, root.lineage, root.revision
        )));
    }
    if bundle.library != catalog.library {
        return Err(Error::Invalid(format!(
            "the registry bundle of {} is not from library {}",
            root.lineage, catalog.name
        )));
    }
    validate_bundle(bundle)?;
    for record in &bundle.records {
        expected_hash(catalog, record.lineage, record.revision).and_then(|expected| {
            let actual = content_hash(record)?;
            if actual == expected {
                Ok(())
            } else {
                Err(Error::Invalid(format!(
                    "registry record {} {} does not match its content hash: refused",
                    record.lineage, record.revision
                )))
            }
        })?;
    }
    Ok(())
}

fn expected_hash(
    catalog: &SourceCatalog,
    lineage: SourceLineageId,
    revision: Revision,
) -> Result<ContentHash> {
    catalog
        .revisions
        .iter()
        .find(|r| r.lineage == lineage && r.revision == revision)
        .map(|r| r.content_hash)
        .ok_or_else(|| {
            Error::Invalid(format!(
                "registry record {lineage} {revision} is not in the index of {}: refused",
                catalog.name
            ))
        })
}

fn read_limited(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(Error::TooLarge(format!(
            "{} exceeds {limit} bytes",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::TooLarge(format!(
            "{} exceeds {limit} bytes",
            path.display()
        )));
    }
    Ok(bytes)
}

/// Keep `bytes`, already checked against the content hash `path` is
/// named by, at `path`. A file already there with the same bytes is kept;
/// one with other bytes is damaged (its name is their hash) and is
/// replaced atomically, so a valid download repairs it.
fn write_verified(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.is_file() && read_limited(path, bytes.len() as u64).is_ok_and(|held| held == bytes) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    atomic_write_with(path, |file| Ok(file.write_all(bytes)?))
}

impl Mirror {
    /// The mirror of the registry at `url` under `cache` (the user cache
    /// directory): `<cache>/registry/origins/<origin-digest>/`. Nothing is created yet.
    pub fn open(cache: &Path, url: &str) -> Result<Self> {
        crate::settings::check_registry_url(url)?;
        let origin = origin(url)?;
        Ok(Self {
            root: cache
                .join("registry")
                .join("origins")
                .join(asset_hash(origin.as_bytes()).to_string()),
            origin,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn shard_path(&self, hash: ContentHash) -> PathBuf {
        self.root.join("shards").join(format!("{hash}.json"))
    }
    fn record_path(&self, hash: ContentHash) -> PathBuf {
        self.root.join("records").join(format!("{hash}.json"))
    }
    fn asset_path(&self, hash: ContentHash) -> PathBuf {
        self.root.join("assets").join(hash.to_string())
    }
    fn bundle_path(&self, root: SourceRevision) -> PathBuf {
        self.root
            .join("bundles")
            .join(root.lineage.to_string())
            .join(format!("r{:04}.json", root.revision.0))
    }

    /// The last index stored, if any and readable.
    pub fn index(&self) -> Option<MirroredIndex> {
        let bytes = read_limited(&self.root.join("index.json"), MAX_INDEX_BYTES).ok()?;
        let file: IndexFile = serde_json::from_slice(&bytes).ok()?;
        (file.origin == self.origin).then_some(file.snapshot)
    }

    /// Keep `index` as the mirror's current one.
    pub fn store_index(&self, index: &Index, fetched: u64) -> Result<()> {
        let bytes = serde_json::to_vec(&IndexFile {
            origin: self.origin.clone(),
            snapshot: MirroredIndex {
                index: index.clone(),
                fetched,
            },
        })?;
        std::fs::create_dir_all(&self.root)?;
        atomic_write_with(&self.root.join("index.json"), |file| {
            Ok(file.write_all(&bytes)?)
        })
    }

    /// Whether a shard's bytes are already mirrored.
    pub fn has_shard(&self, hash: ContentHash) -> bool {
        self.shard_path(hash).is_file()
    }

    /// Check and keep a shard's bytes; returns it decoded.
    pub fn store_shard(&self, bytes: &[u8], hash: ContentHash) -> Result<Shard> {
        let shard = decode_shard(bytes, hash)?;
        write_verified(&self.shard_path(hash), bytes)?;
        Ok(shard)
    }

    /// A mirrored shard, checked again against its name.
    pub fn shard(&self, hash: ContentHash) -> Result<Shard> {
        decode_shard(
            &read_limited(&self.shard_path(hash), MAX_SHARD_BYTES)?,
            hash,
        )
    }

    /// Every shard of the stored index that is mirrored and still checks,
    /// in index order. Missing or changed ones are left out.
    pub fn shards(&self, index: &Index, limit: u64) -> Result<Vec<Shard>> {
        let mut budget = ShardBudget(limit);
        let mut shards = Vec::new();
        for info in &index.shards {
            if let Some(shard) = budget.cached(self, info.sha256)? {
                shards.push(shard);
            }
        }
        Ok(shards)
    }

    /// Check `bundle` against `catalog` ([`verify_bundle`]) and keep its
    /// records and assets by hash. Nothing is stored when it fails.
    pub fn store_bundle(&self, bundle: &Bundle, catalog: &SourceCatalog) -> Result<()> {
        verify_bundle(bundle, bundle.root, catalog)?;
        let mut records = Vec::with_capacity(bundle.records.len());
        for record in &bundle.records {
            let hash = content_hash(record)?;
            write_verified(&self.record_path(hash), &serde_json::to_vec(record)?)?;
            records.push(hash);
        }
        for asset in &bundle.assets {
            write_verified(&self.asset_path(asset.hash), asset.data.as_slice())?;
        }
        let file = BundleFile {
            library: bundle.library,
            name: bundle.name.clone(),
            root: bundle.root,
            records,
            assets: bundle
                .assets
                .iter()
                .map(|a| (a.hash, a.media_type.clone()))
                .collect(),
        };
        let path = self.bundle_path(bundle.root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(&file)?;
        atomic_write_with(&path, |f| Ok(f.write_all(&bytes)?))
    }

    /// A mirrored bundle of `root`, rebuilt from its records and assets and
    /// checked against `catalog` again; `Ok(None)` when it was never
    /// mirrored.
    pub fn bundle(&self, root: SourceRevision, catalog: &SourceCatalog) -> Result<Option<Bundle>> {
        let path = self.bundle_path(root);
        if !path.is_file() {
            return Ok(None);
        }
        let file: BundleFile = serde_json::from_slice(&read_limited(&path, MAX_RECORD_BYTES)?)?;
        let mut records = Vec::with_capacity(file.records.len());
        for hash in &file.records {
            let record: RevisionRecord =
                serde_json::from_slice(&read_limited(&self.record_path(*hash), MAX_RECORD_BYTES)?)?;
            records.push(record);
        }
        let mut assets = Vec::with_capacity(file.assets.len());
        for (hash, media_type) in &file.assets {
            assets.push(PackedAsset {
                hash: *hash,
                media_type: media_type.clone(),
                data: AssetBytes::new(read_limited(&self.asset_path(*hash), MAX_ASSET_BYTES)?),
            });
        }
        let bundle = Bundle {
            library: file.library,
            name: file.name,
            root: file.root,
            records,
            assets,
        };
        verify_bundle(&bundle, root, catalog)?;
        Ok(Some(bundle))
    }
}

#[cfg(test)]
#[path = "registry_mirror_tests.rs"]
mod tests;
