//! Canonical JSON: the one encoder and decoder the host and every module use.
//!
//! Canonical form:
//! - object keys sorted by their UTF-8 bytes, each key once;
//! - no whitespace outside strings;
//! - numbers are integers (`i64` or `u64`) in shortest decimal form; no
//!   floats, exponents, `-0` or leading zeros;
//! - strings escaped as `serde_json` writes them (`\"`, `\\`, control
//!   characters), everything else literal UTF-8; with the `nfc` feature,
//!   strings must also be in Unicode NFC;
//! - absent optional fields are omitted, never `null`.
//!
//! [`decode`] accepts only bytes that [`encode`] would write for the decoded
//! value, so two different byte strings never decode to the same value.
//!
//! The codec has its own reader, writer and serde (de)serializer over an
//! integer-only tree ([`Canon`]), not `serde_json`: no path reaches
//! floating-point code, so a module built with it contains no float
//! instruction, which the host refuses in plugins whose results are hashed
//! (part-generation slice B3).
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

mod de;
mod ser;
pub mod tree;

pub use tree::Canon;

/// Deepest nesting of arrays and objects the codec accepts.
pub const MAX_DEPTH: usize = 64;

/// Total JSON values plus object keys accepted by one read. A tree with
/// `n` such nodes takes at least `2n - 1` bytes, so this admits every tree
/// within ABI 1's largest (1 MiB) response while bounding allocations even
/// when a caller gives a larger byte allowance.
pub const MAX_NODES: usize = 512 * 1024;

/// Total decoded UTF-8 bytes in all keys and string values in one read.
/// Decoding JSON escapes never expands their byte length, so this also
/// admits every string within ABI 1's 1 MiB response bound.
pub const MAX_STRING_BYTES: usize = 1024 * 1024;

/// Why bytes were not canonical JSON of the expected shape, in plain words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodecError {
    /// The input is longer than the caller's bound.
    TooLarge { bytes: usize, limit: usize },
    /// The bytes are not JSON.
    Syntax(String),
    /// A number is not an integer.
    Float,
    /// Nesting deeper than [`MAX_DEPTH`].
    TooDeep,
    /// More JSON values and object keys than [`MAX_NODES`].
    TooManyNodes,
    /// More decoded string bytes than [`MAX_STRING_BYTES`].
    TooMuchText,
    /// A string is not in Unicode NFC (with the `nfc` feature).
    NotNfc,
    /// Valid JSON, but not in canonical form (key order, whitespace,
    /// duplicate keys, `null` for an absent field, number spelling).
    NotCanonical,
    /// Canonical JSON whose shape does not match the expected type.
    Shape(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes, limit } => {
                write!(f, "the data is {bytes} bytes, more than the {limit} allowed")
            }
            Self::Syntax(detail) => write!(f, "the data is not valid JSON ({detail})"),
            Self::Float => f.write_str("a number is not a whole number; only integers are allowed"),
            Self::TooDeep => write!(f, "the data nests deeper than {MAX_DEPTH} levels"),
            Self::TooManyNodes => write!(f, "the data has more than {MAX_NODES} values and object keys"),
            Self::TooMuchText => write!(f, "the data has more than {MAX_STRING_BYTES} decoded text bytes"),
            Self::NotNfc => f.write_str("a text value is not in Unicode normal form C"),
            Self::NotCanonical => f.write_str(
                "the data is not in canonical form (sorted keys, no spaces, no nulls, each key once)",
            ),
            Self::Shape(detail) => write!(f, "the data has the wrong shape ({detail})"),
        }
    }
}

/// The canonical bytes of `value`.
pub fn encode<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    let tree = ser::to_canon(value).map_err(|e| CodecError::Shape(e.0))?;
    let mut out = Vec::new();
    tree::write(&tree, &mut out, 0)?;
    Ok(out)
}

/// Decode canonical bytes of at most `limit` bytes into `T`, also bounded
/// by [`MAX_DEPTH`], [`MAX_NODES`] and [`MAX_STRING_BYTES`].
pub fn decode<T: DeserializeOwned + Serialize>(
    bytes: &[u8],
    limit: usize,
) -> Result<T, CodecError> {
    if bytes.len() > limit {
        return Err(CodecError::TooLarge {
            bytes: bytes.len(),
            limit,
        });
    }
    let value = tree::read(bytes)?;
    let mut again = Vec::with_capacity(bytes.len());
    tree::write(&value, &mut again, 0)?;
    if again != bytes {
        return Err(CodecError::NotCanonical);
    }
    let typed: T = de::from_canon(value).map_err(|e| CodecError::Shape(e.0))?;
    // A field the type fills by default, or a `null` it reads as absent,
    // would encode differently: refuse it as not canonical.
    if encode(&typed)? != bytes {
        return Err(CodecError::NotCanonical);
    }
    Ok(typed)
}

/// Read JSON that need not be canonical (spaces, any key order), such as a
/// generator's `presets.json`, into `T`; integers only. The same structural
/// and decoded-text bounds as [`decode`] apply even if `limit` is larger.
pub fn read<T: DeserializeOwned>(bytes: &[u8], limit: usize) -> Result<T, CodecError> {
    if bytes.len() > limit {
        return Err(CodecError::TooLarge {
            bytes: bytes.len(),
            limit,
        });
    }
    de::from_canon(tree::read(bytes)?).map_err(|e| CodecError::Shape(e.0))
}

/// SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The result hash: SHA-256 of a generation's canonical bytes.
pub fn result_hash(canonical_generation: &[u8]) -> [u8; 32] {
    sha256(canonical_generation)
}

/// Lower-case hexadecimal of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;
    use alloc::vec;

    #[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        zeta: i64,
        alpha: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        map: BTreeMap<String, i64>,
    }

    fn sample() -> Sample {
        let mut map = BTreeMap::new();
        map.insert("b".into(), -2);
        map.insert("a".into(), 1);
        Sample {
            zeta: 7,
            alpha: "Ω \"q\"\n".into(),
            note: None,
            map,
        }
    }

    #[test]
    fn encodes_sorted_compact_and_round_trips() -> Result<(), CodecError> {
        let bytes = encode(&sample())?;
        assert_eq!(
            core::str::from_utf8(&bytes).unwrap_or(""),
            r#"{"alpha":"Ω \"q\"\n","map":{"a":1,"b":-2},"zeta":7}"#
        );
        assert_eq!(decode::<Sample>(&bytes, 1024)?, sample());
        Ok(())
    }

    #[test]
    fn refuses_every_non_canonical_spelling() {
        let refused = [
            (
                &br#"{"zeta":7,"alpha":"x","map":{}}"#[..],
                CodecError::NotCanonical,
            ),
            (
                &br#"{"alpha":"x", "map":{},"zeta":7}"#[..],
                CodecError::NotCanonical,
            ),
            (
                &br#"{"alpha":"x","map":{},"note":null,"zeta":7}"#[..],
                CodecError::NotCanonical,
            ),
            (
                &br#"{"alpha":"x","map":{},"zeta":7.0}"#[..],
                CodecError::Float,
            ),
            (
                &br#"{"alpha":"x","map":{},"zeta":7e0}"#[..],
                CodecError::Float,
            ),
            // `-0` is no integer's shortest form.
            (
                &br#"{"alpha":"x","map":{},"zeta":-0}"#[..],
                CodecError::Float,
            ),
            (
                &br#"{"alpha":"x","alpha":"x","map":{},"zeta":7}"#[..],
                CodecError::NotCanonical,
            ),
            // An escape where the character itself is canonical.
            (
                &br#"{"alpha":"\/","map":{},"zeta":7}"#[..],
                CodecError::NotCanonical,
            ),
        ];
        for (bytes, expected) in refused {
            assert_eq!(
                decode::<Sample>(bytes, 1024).err(),
                Some(expected),
                "{}",
                core::str::from_utf8(bytes).unwrap_or("")
            );
        }
        assert!(matches!(
            decode::<Sample>(br#"{"alpha":"x","map":{},"other":1,"zeta":7}"#, 1024),
            Err(CodecError::Shape(_))
        ));
        assert!(matches!(
            decode::<Sample>(b"{", 1024),
            Err(CodecError::Syntax(_))
        ));
    }

    #[test]
    fn bounds_size_and_depth() {
        let long = vec![b' '; 2048];
        assert_eq!(
            decode::<Vec<u8>>(&long, 1024).err(),
            Some(CodecError::TooLarge {
                bytes: 2048,
                limit: 1024
            })
        );
        let mut deep = Canon::Bool(true);
        for _ in 0..=MAX_DEPTH {
            deep = Canon::Arr(vec![deep]);
        }
        let mut out = Vec::new();
        assert_eq!(
            tree::write(&deep, &mut out, 0).err(),
            Some(CodecError::TooDeep)
        );
        let text = alloc::format!(
            "{}true{}",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        );
        assert_eq!(tree::read(text.as_bytes()).err(), Some(CodecError::TooDeep));
    }

    #[test]
    fn a_large_map_keeps_keys_and_duplicate_detection_uses_decoded_keys() -> Result<(), CodecError>
    {
        let expected: BTreeMap<String, u8> = (0..60_000)
            .map(|i| (alloc::format!("k{i:05}"), 0))
            .collect();
        let bytes = encode(&expected)?;
        assert_eq!(bytes.len(), 660_001);
        assert_eq!(
            decode::<BTreeMap<String, u8>>(&bytes, 1024 * 1024)?,
            expected
        );
        // The second value is intentionally invalid: the duplicate key
        // must be refused before that value is visited or allocated.
        assert_eq!(
            tree::read(br#"{"key":0,"\u006bey":!}"#).err(),
            Some(CodecError::NotCanonical),
        );
        let value = tree::read(br#"{"z":0,"a":1}"#)?;
        assert_eq!(
            value,
            Canon::Obj(vec![
                ("z".into(), Canon::Int(0)),
                ("a".into(), Canon::Int(1))
            ])
        );
        assert!(read::<BTreeMap<String, u8>>(br#"{"z":0,"a":1}"#, 100).is_ok());
        Ok(())
    }

    #[test]
    fn structural_budget_admits_the_densest_one_mib_tree_and_refuses_one_more()
    -> Result<(), CodecError> {
        // Root + MAX_NODES-1 integers: 2*MAX_NODES-1 bytes, the largest
        // node count that fits the default ABI response byte allowance.
        let bytes = alloc::format!("[{}0]", "0,".repeat(MAX_NODES - 2));
        assert_eq!(bytes.len(), 1024 * 1024 - 1);
        let values: Vec<u8> = decode(bytes.as_bytes(), 1024 * 1024)?;
        assert_eq!(values.len(), MAX_NODES - 1);
        assert!(values.iter().all(|&v| v == 0));
        // No parsing of the next value, even with an unlimited wire cap.
        let over = alloc::format!("[{}!]", "0,".repeat(MAX_NODES - 1));
        for error in [
            read::<Vec<u8>>(over.as_bytes(), usize::MAX).err(),
            decode::<Vec<u8>>(over.as_bytes(), usize::MAX).err(),
        ] {
            assert_eq!(error, Some(CodecError::TooManyNodes));
        }
        Ok(())
    }

    #[test]
    fn decoded_text_budget_is_shared_by_keys_and_values() -> Result<(), CodecError> {
        let payload = "x".repeat(MAX_STRING_BYTES - 3);
        // The key is one byte; the escaped character decodes to two.
        let at = alloc::format!("{{\"k\":\"{payload}\\u00e9\"}}");
        let value: BTreeMap<String, String> = read(at.as_bytes(), usize::MAX)?;
        assert_eq!(value.get("k").map(String::len), Some(MAX_STRING_BYTES - 1));
        let over = alloc::format!("{{\"k\":\"{payload}\\u00e9x");
        assert_eq!(
            tree::read(over.as_bytes()).err(),
            Some(CodecError::TooMuchText)
        );
        // Different values share the same budget, rather than each
        // receiving a fresh allowance. Keys are charged too.
        let half = "x".repeat(MAX_STRING_BYTES / 2 - 1);
        let at = alloc::format!("{{\"a\":\"{half}\",\"b\":\"{half}\"}}");
        assert!(tree::read(at.as_bytes()).is_ok());
        let over = alloc::format!("{{\"a\":\"{half}\",\"b\":\"{half}x");
        assert_eq!(
            tree::read(over.as_bytes()).err(),
            Some(CodecError::TooMuchText)
        );
        Ok(())
    }

    #[test]
    fn structural_budget_counts_keys_and_is_shared_across_nested_objects() {
        // Each object contributes its object, key and integer nodes.
        // MAX_NODES = root + 174762*3 + one final node.
        let objects = "{\"a\":0},".repeat((MAX_NODES - 1) / 3);
        let at = alloc::format!("[{objects}0]");
        assert!(tree::read(at.as_bytes()).is_ok());
        // The last object spends the final node: its key must be refused
        // before attempting the intentionally invalid value.
        let over = alloc::format!("[{objects}{{\"a\":!}}]");
        assert_eq!(
            tree::read(over.as_bytes()).err(),
            Some(CodecError::TooManyNodes)
        );
    }

    #[cfg(feature = "nfc")]
    #[test]
    fn nfc_refuses_decomposed_text() {
        // "é" as e + combining acute: not NFC.
        assert_eq!(
            decode::<String>("\"e\u{301}\"".as_bytes(), 64).err(),
            Some(CodecError::NotNfc)
        );
        assert!(decode::<String>("\"\u{e9}\"".as_bytes(), 64).is_ok());
    }

    #[test]
    fn hashes_are_sha256_in_hex() {
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
