//! The committed example documents read and write as the contract says,
//! and hostile input is refused within its bounds. No core type is used:
//! these are the checks a server that links no core runs too.
use crate::account::{
    self, DevicePoll, decode_account, decode_code, decode_code_request, decode_token_request,
    read_bearer,
};
use crate::conformance::compact;
use crate::documents::{
    Index, Review, ShardInfo, decode_bundle, decode_index, decode_search, decode_shard,
};
use crate::hash::{base64_decode, base64_encode, bytes_hash};
use crate::ids::{ContentHash, LibraryId};
use crate::limits::{MAX_CLIENT_NAME, MAX_INDEX_BYTES, MAX_SHARD_BYTES, MAX_SHARDS};
use crate::route::{Refused, Route, percent_decode, percent_encode};
use crate::status::{ErrorBody, retry_after};
use crate::version::{FORMAT, MAJOR, PREFIX};
use crate::{Error, vectors};
use serde::Deserialize;
use serde_json::Value;
use serde_json::value::RawValue;

type Fallible = Result<(), Box<dyn std::error::Error>>;

#[derive(Deserialize)]
struct Asset {
    hash: ContentHash,
    data: String,
}

#[derive(Deserialize)]
struct Bundle {
    records: Vec<Box<RawValue>>,
    assets: Vec<Asset>,
}

#[test]
fn the_catalog_vectors_read_and_hash_as_committed() -> Fallible {
    let cases = vectors::cases()?;
    let index = decode_index(vectors::INDEX)?;
    assert_eq!(
        serde_json::to_vec(&index)?,
        vectors::INDEX,
        "a writer's bytes"
    );
    assert_eq!(bytes_hash(vectors::SHARD), cases.shard);
    assert_eq!(index.shards.first().map(|s| s.sha256), Some(cases.shard));
    let shard = decode_shard::<Value>(vectors::SHARD, cases.shard)?;
    assert_eq!(
        index.shards[0].entries,
        shard.catalog["entries"].as_array().map_or(0, Vec::len)
    );
    let search = decode_search::<Value>(vectors::SEARCH)?;
    assert!(!search.results.is_empty() && search.total >= search.results.len());
    let bundle: Bundle = decode_bundle(vectors::BUNDLE)?;
    let records: Vec<ContentHash> = bundle
        .records
        .iter()
        .map(|r| bytes_hash(compact(r.get()).as_bytes()))
        .collect();
    assert_eq!(records, cases.records);
    assert!(!bundle.assets.is_empty(), "the vectors carry an asset");
    for (asset, hash) in bundle.assets.iter().zip(&cases.assets) {
        assert_eq!(asset.hash, *hash);
        let data = base64_decode(&asset.data).ok_or("not base64")?;
        assert_eq!(bytes_hash(&data), *hash);
    }
    Ok(())
}

#[test]
fn the_account_vectors_read_and_write_as_committed() -> Fallible {
    let request = decode_code_request(vectors::CODE_REQUEST)?;
    assert_eq!(serde_json::to_vec(&request)?, vectors::CODE_REQUEST);
    let code = decode_code(vectors::CODE)?;
    assert_eq!(serde_json::to_vec(&code)?, vectors::CODE);
    assert!(!format!("{code:?}").contains(code.device_code.expose()));
    let poll = decode_token_request(vectors::TOKEN_REQUEST)?;
    assert_eq!(poll.device_code, code.device_code);
    let DevicePoll::Approved(token) = DevicePoll::read(200, vectors::TOKEN)? else {
        return Err("not approved".into());
    };
    assert_eq!(serde_json::to_vec(&token)?, vectors::TOKEN);
    let account = decode_account(vectors::ACCOUNT)?;
    assert_eq!(serde_json::to_vec(&account)?, vectors::ACCOUNT);
    let limited = ErrorBody::read(vectors::RATE_LIMITED);
    assert_eq!(limited.retry_after, Some(300));
    assert_eq!(limited.bytes(), vectors::RATE_LIMITED);
    Ok(())
}

#[test]
fn routes_are_one_spelling_each() -> Fallible {
    let cases = vectors::cases()?;
    for (method, target) in &cases.routes {
        let route = Route::parse(method, target).map_err(|e| format!("{target}: {e:?}"))?;
        assert_eq!(route.method().as_str(), method);
        assert_eq!(&route.target(), target);
        assert!(target.starts_with(&format!("{PREFIX}/")));
    }
    for (method, target) in &cases.refused {
        assert!(Route::parse(method, target).is_err(), "{method} {target}");
    }
    assert_eq!(PREFIX, format!("/v{MAJOR}"));
    let long = format!("/v1/search?q=a&pad={}", "x".repeat(8000));
    assert_eq!(Route::parse("GET", &long), Err(Refused::NotFound));
    // Unknown parameters are ignored; the last of a repeated one counts.
    let Ok(Route::Search(query)) = Route::parse("GET", "/v1/search?q=a&new=1&q=b+c%21") else {
        return Err("not a search".into());
    };
    assert_eq!(
        (query.text.as_str(), query.limit, query.offset),
        ("b c!", 50, 0)
    );
    Ok(())
}

#[test]
fn percent_encoding_round_trips_and_refuses_noise() {
    for text in ["", "EX-R0603 kind:part", "10 kΩ ±1%", "a&b=c+d"] {
        assert_eq!(percent_decode(&percent_encode(text)).as_deref(), Some(text));
    }
    for noise in ["%4", "%zz", "%ff", "%"] {
        assert_eq!(percent_decode(noise), None, "{noise}");
    }
}

fn index_with(shards: usize, format: u32) -> Result<Vec<u8>, serde_json::Error> {
    let info = ShardInfo {
        library: LibraryId(1),
        name: "x".into(),
        sha256: ContentHash::default(),
        entries: 0,
    };
    serde_json::to_vec(&Index {
        format,
        snapshot: "s".into(),
        shards: vec![info; shards],
    })
}

#[test]
fn hostile_catalog_documents_are_refused_within_their_bounds() -> Fallible {
    let oversized = vec![b' '; usize::try_from(MAX_INDEX_BYTES)? + 1];
    assert!(matches!(decode_index(&oversized), Err(Error::TooLarge(_))));
    assert!(matches!(decode_index(&index_with(1, 2)?), Err(Error::Invalid(m)) if m.contains('2')));
    assert!(matches!(
        decode_index(&index_with(MAX_SHARDS + 1, FORMAT)?),
        Err(Error::TooLarge(_))
    ));
    assert!(decode_index(&index_with(MAX_SHARDS, FORMAT)?).is_ok());
    // Unknown fields are ignored; missing and mistyped ones refused.
    assert!(decode_index(br#"{"format":1,"snapshot":"s","shards":[],"new":{}}"#).is_ok());
    assert!(matches!(
        decode_index(br#"{"format":1,"shards":[]}"#),
        Err(Error::Unreadable(_))
    ));
    let upper = format!(
        r#"{{"format":1,"snapshot":"s","shards":[{{"library":"{}","name":"x","sha256":"{}","entries":0}}]}}"#,
        "0".repeat(32),
        "A".repeat(64)
    );
    assert!(matches!(
        decode_index(upper.as_bytes()),
        Err(Error::Unreadable(_))
    ));
    let shard = br#"{"format":1,"catalog":{}}"#;
    let refused = decode_shard::<Value>(shard, ContentHash::default());
    assert!(matches!(refused, Err(Error::Invalid(m)) if m.contains("does not match its hash")));
    let shard = decode_shard::<Value>(shard, bytes_hash(shard))?;
    assert!(shard.entries.is_empty());
    let other = br#"{"format":7,"catalog":{}}"#;
    assert!(matches!(
        decode_shard::<Value>(other, bytes_hash(other)),
        Err(Error::Invalid(_))
    ));
    let huge = vec![b' '; usize::try_from(MAX_SHARD_BYTES)? + 1];
    assert!(matches!(
        decode_shard::<Value>(&huge, bytes_hash(&huge)),
        Err(Error::TooLarge(_))
    ));
    // A search reply's fields a v1 client does not read never cost it.
    let reply = br#"{"format":1,"query":5,"offset":-1,"total":0,"results":[]}"#;
    assert!(decode_search::<Value>(reply).is_ok());
    assert!(decode_search::<Value>(br#"{"format":1,"results":[]}"#).is_err());
    // Another format is refused, naming it, as the other documents are.
    let other = br#"{"format":2,"total":0,"results":[]}"#;
    assert!(matches!(decode_search::<Value>(other), Err(Error::Invalid(m)) if m.contains('2')));
    assert!(decode_search::<Value>(br#"{"total":0,"results":[]}"#).is_err());
    assert!(matches!(
        decode_bundle::<Value>(br#"{"format":2,"bundle":{}}"#),
        Err(Error::Invalid(_))
    ));
    Ok(())
}

#[test]
fn hostile_account_documents_are_refused() -> Fallible {
    let long = serde_json::to_vec(&account::DeviceCodeRequest {
        client_name: "é".repeat(MAX_CLIENT_NAME + 1),
    })?;
    assert!(matches!(
        decode_code_request(&long),
        Err(Error::TooLarge(_))
    ));
    let fits = serde_json::to_vec(&account::DeviceCodeRequest {
        client_name: "é".repeat(MAX_CLIENT_NAME),
    })?;
    assert!(decode_code_request(&fits).is_ok());
    assert!(decode_account(br#"{"id":5,"providers":"email","email":"a@b"}"#).is_ok());
    assert!(decode_code(br#"{"device_code":"d","user_code":"u"}"#).is_err());
    let code = decode_code(
        br#"{"device_code":"d","user_code":"0123456789012345678901234567890123456789","verification_uri":"https://x","expires_in":1}"#,
    )?;
    assert_eq!(code.shown_code().chars().count(), 32);
    assert_eq!(code.interval, 0);
    assert_eq!(DevicePoll::read(400, b"not json")?, DevicePoll::Refused);
    assert_eq!(DevicePoll::read(429, b"")?, DevicePoll::SlowDown);
    assert_eq!(
        DevicePoll::read(400, br#"{"error":{"code":"device.pending"}}"#)?,
        DevicePoll::Pending
    );
    assert!(DevicePoll::read(200, b"{}").is_err());
    Ok(())
}

#[test]
fn refusals_headers_and_words_read_leniently() {
    assert_eq!(ErrorBody::read(b"<html>"), ErrorBody::default());
    assert_eq!(
        ErrorBody::read(br#"{"error":"x","retry_after":"soon"}"#),
        ErrorBody::default()
    );
    assert_eq!(retry_after(" 120 "), Some(120));
    assert_eq!(retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    assert_eq!(read_bearer("Bearer abc"), Some("abc"));
    assert_eq!(read_bearer("bearer  abc "), Some("abc"));
    assert_eq!(read_bearer("Basic abc"), None);
    assert_eq!(read_bearer("Bearer "), None);
    assert_eq!(Review::of("reviewed"), Review::Reviewed);
    assert_eq!(Review::of("in_review"), Review::InReview);
    assert_eq!(Review::of("approved by the moon"), Review::NotReviewed);
}

#[test]
fn base64_round_trips_and_rejects_noise() {
    for len in 0..10 {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 37 + 5) as u8).collect();
        let text = base64_encode(&bytes);
        assert_eq!(base64_decode(&text), Some(bytes));
    }
    assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_decode("Zm9=x"), None);
    assert_eq!(base64_decode("Zm9"), None);
    assert_eq!(base64_decode("Zm9="), None, "stray bits under padding");
}
