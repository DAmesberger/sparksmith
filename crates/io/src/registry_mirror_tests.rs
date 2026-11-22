//! The registry mirror: shards by hash, bundles checked record by record
//! against their shard, refusals that store nothing, and reuse offline.
//! Fixtures are original (the synthetic ExampleCo catalog).
use super::*;
use crate::library_folder::folder_tests::part_bundle;
use crate::library_folder::{create_folder, write_bundle};
use sparksmith_core::catalog::source_catalog;

fn fixture(dir: &Path) -> Result<(Bundle, Shard, Vec<u8>, ContentHash)> {
    let folder = create_folder(&dir.join("folder"), "Registry fixture")?;
    let (bundle, _) = part_bundle(&folder, b"original datasheet bytes", 100)?;
    write_bundle(&folder, &bundle)?;
    let records: Vec<RevisionRecord> = crate::library_folder::read_records(&folder)?
        .into_iter()
        .map(|s| s.record)
        .collect();
    let catalog = source_catalog(folder.manifest.library, &folder.manifest.name, &records)?;
    let shard = Shard {
        format: FORMAT,
        catalog,
        entries: Vec::new(),
    };
    let bytes = serde_json::to_vec(&shard)?;
    let hash = asset_hash(&bytes);
    Ok((bundle, shard, bytes, hash))
}

#[test]
fn host_keys_are_folder_names() -> Result<()> {
    assert_eq!(host_key("http://localhost:8787")?, "localhost_8787");
    assert_eq!(
        host_key("https://Parts.Sparksmith.io/v1")?,
        "parts.sparksmith.io"
    );
    assert!(host_key("ftp://x").is_err());
    assert!(host_key("http://").is_err());
    Ok(())
}

#[test]
fn shards_are_checked_against_their_name_and_marked_online() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (_, shard, bytes, hash) = fixture(dir.path())?;
    let mirror = Mirror::open(dir.path(), "http://localhost:8787")?;
    let stored = mirror.store_shard(&bytes, hash)?;
    assert!(!stored.catalog.entries.is_empty());
    assert!(
        stored
            .catalog
            .entries
            .iter()
            .all(|e| e.source == Source::Online(shard.catalog.library))
    );
    assert!(mirror.has_shard(hash));
    // Bytes that do not hash to the name are refused and never stored.
    let other = asset_hash(b"another shard");
    let refused = mirror.store_shard(&bytes, other);
    assert!(matches!(refused, Err(Error::Invalid(ref m)) if m.contains("does not match its hash")));
    assert!(!mirror.has_shard(other));
    Ok(())
}

/// A damaged mirrored shard (its bytes no longer hash to its name) is
/// left out offline, and the next valid download of it replaces it.
#[test]
fn a_valid_download_replaces_a_damaged_shard() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (_, _, bytes, hash) = fixture(dir.path())?;
    let mirror = Mirror::open(dir.path(), "http://localhost:8787")?;
    mirror.store_shard(&bytes, hash)?;
    let path = mirror.shard_path(hash);
    let mut damaged = bytes.clone();
    damaged.truncate(bytes.len() / 2);
    std::fs::write(&path, &damaged)?;
    assert!(mirror.shard(hash).is_err(), "the damage is found");
    mirror.store_shard(&bytes, hash)?;
    assert_eq!(std::fs::read(&path)?, bytes);
    assert!(mirror.shard(hash).is_ok());
    // A good copy is kept as it is.
    mirror.store_shard(&bytes, hash)?;
    assert_eq!(std::fs::read(&path)?, bytes);
    Ok(())
}

#[test]
fn a_bundle_whose_record_hash_differs_is_refused_and_nothing_is_stored() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (bundle, shard, _, _) = fixture(dir.path())?;
    let mirror = Mirror::open(dir.path(), "http://localhost:8787")?;
    let mut tampered = bundle.clone();
    let record = tampered
        .records
        .last_mut()
        .ok_or_else(|| Error::Invalid("empty fixture".into()))?;
    record.meta.description.push_str(" (changed in transit)");
    let lineage = record.lineage.to_string();
    let refused = mirror.store_bundle(&tampered, &shard.catalog);
    let message = match refused {
        Err(Error::Invalid(message)) => message,
        other => return Err(Error::Invalid(format!("not refused: {other:?}"))),
    };
    assert!(
        message.contains("does not match its content hash"),
        "{message}"
    );
    assert!(message.contains(&lineage), "{message}");
    assert!(mirror.bundle(bundle.root, &shard.catalog)?.is_none());
    // A record the shard does not list is refused too.
    let mut stranger = bundle.clone();
    if let Some(first) = stranger.records.first_mut() {
        first.revision = Revision(first.revision.0 + 40);
    }
    assert!(verify_bundle(&stranger, stranger.root, &shard.catalog).is_err());
    Ok(())
}

#[test]
fn a_stored_bundle_is_reused_offline_and_a_changed_cache_file_is_refused() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let (bundle, shard, bytes, hash) = fixture(dir.path())?;
    let mirror = Mirror::open(dir.path(), "http://localhost:8787")?;
    mirror.store_index(
        &RegistryIndex {
            format: FORMAT,
            snapshot: "s1".into(),
            shards: vec![ShardInfo {
                library: shard.catalog.library,
                name: shard.catalog.name.clone(),
                sha256: hash,
                entries: shard.catalog.entries.len(),
            }],
        },
        now(),
    )?;
    mirror.store_shard(&bytes, hash)?;
    mirror.store_bundle(&bundle, &shard.catalog)?;
    // A second mirror over the same directory (a later, offline session).
    let offline = Mirror::open(dir.path(), "http://localhost:8787/")?;
    assert_eq!(offline.shards().len(), 1);
    let again = offline
        .bundle(bundle.root, &shard.catalog)?
        .ok_or_else(|| Error::Invalid("not mirrored".into()))?;
    assert_eq!(again, bundle);
    // Records are kept once by content hash.
    let first = bundle
        .records
        .first()
        .ok_or_else(|| Error::Invalid("empty fixture".into()))?;
    let path = offline.record_path(content_hash(first)?);
    assert!(path.is_file());
    let mut changed: RevisionRecord = serde_json::from_slice(&std::fs::read(&path)?)?;
    changed.meta.name.push('!');
    std::fs::write(&path, serde_json::to_vec(&changed)?)?;
    assert!(offline.bundle(bundle.root, &shard.catalog).is_err());
    Ok(())
}
