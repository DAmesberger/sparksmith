//! Content hashes and the text assets travel as.
//!
//! A hash is the SHA-256 of exact bytes: a shard's are the bytes served, an
//! asset's the decoded bytes, a record's its **canonical JSON**. That is what
//! the core writes for the record: compact (no space between tokens), fields
//! in the order the type declares, maps by key, floats as the shortest text
//! that reads back the same. `serde_json` writes it, with `float_roundtrip`
//! and without `preserve_order` or `arbitrary_precision`: a program that
//! turns one of those on anywhere changes the hash of a record it reads and
//! writes again.
use crate::ids::ContentHash;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// SHA-256 of `bytes`.
pub fn bytes_hash(bytes: &[u8]) -> ContentHash {
    ContentHash(Sha256::digest(bytes).into())
}

/// SHA-256 of a record's canonical JSON.
pub fn record_hash(record: &impl Serialize) -> serde_json::Result<ContentHash> {
    Ok(bytes_hash(&serde_json::to_vec(record)?))
}

/// Bytes of the base64 text of `n` bytes.
pub fn encoded_len(n: usize) -> usize {
    n.div_ceil(3).saturating_mul(4)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding, as an asset's `data` is written.
pub fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 0x3f).unwrap_or(0);
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of [`base64_encode`]'s text and of no other text.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        ALPHABET
            .iter()
            .position(|a| *a == c)
            .and_then(|p| u32::try_from(p).ok())
    }
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / 4 * 3);
    let chunks = raw.len() / 4;
    for (index, chunk) in raw.chunks_exact(4).enumerate() {
        let padding = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if padding > 2 || (padding > 0 && index + 1 != chunks) {
            return None;
        }
        let mut n = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            let v = if i >= 4 - padding { 0 } else { value(*c)? };
            n = (n << 6) | v;
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..4 - padding]);
    }
    // Reject non-canonical encodings (stray bits under the padding).
    (base64_encode(&out) == text).then_some(out)
}
