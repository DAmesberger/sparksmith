//! The identities of a library, a lineage and a revision, and the content
//! hash. The core uses these types as its own.
#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// An immutable published revision of one lineage, starting at 1.
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(transparent)]
pub struct Revision(pub u32);

impl Revision {
    pub const FIRST: Self = Self(1);
    /// The following revision, or `None` when the space is exhausted.
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// Lower-case hex digits of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        text.push(char::from(DIGITS[usize::from(b >> 4)]));
        text.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    text
}

/// Exactly `N` bytes from `2 * N` lower-case hex digits.
pub fn unhex<const N: usize>(text: &str) -> Option<[u8; N]> {
    fn digit(c: u8) -> Option<u8> {
        match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        }
    }
    let raw = text.as_bytes();
    if raw.len() != N * 2 {
        return None;
    }
    let mut out = [0u8; N];
    for (slot, pair) in out.iter_mut().zip(raw.chunks_exact(2)) {
        let [high, low] = pair else { return None };
        *slot = (digit(*high)? << 4) | digit(*low)?;
    }
    Some(out)
}

macro_rules! random_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// Written as 32 lower-case hex digits. The core never generates one:
        /// `sparksmith-io` draws them from the operating system and passes them
        /// in as values.
        #[derive(
            Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq,
            PartialOrd, Ord, Hash,
        )]
        #[cfg_attr(feature = "schema", derive(JsonSchema))]
        #[serde(try_from = "String", into = "String")]
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        pub struct $name(pub u128);
        impl $name {
            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(u128::from_be_bytes(bytes))
            }
            pub fn parse(text: &str) -> Option<Self> {
                unhex::<16>(text).map(Self::from_bytes)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex(&self.0.to_be_bytes()))
            }
        }
        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.to_string()
            }
        }
        impl TryFrom<String> for $name {
            type Error = String;
            fn try_from(text: String) -> std::result::Result<Self, String> {
                Self::parse(&text).ok_or_else(|| {
                    format!("{} must be 32 lower-case hex digits", stringify!($name))
                })
            }
        }
    };
}

random_id!(
    /// A library source's identity (S3, D1): random, generated once when the
    /// folder is created and stored in its manifest. Never a path or name.
    LibraryId
);
random_id!(
    /// A lineage's identity inside one library folder (S3, D1): random, so
    /// two git authors adding lineages never collide on a counter. Project
    /// copies get fresh project `DefinitionId`s on import.
    SourceLineageId
);

/// One revision of one source lineage.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SourceRevision {
    pub lineage: SourceLineageId,
    pub revision: Revision,
}

/// A SHA-256 digest, written as 64 lower-case hex digits. Names a revision
/// record's canonical content (`library::bundle::content_hash`) or an
/// asset's bytes (`library::bundle::asset_hash`).
#[derive(
    Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", schemars(with = "String"))]
pub struct ContentHash(pub [u8; 32]);

impl ContentHash {
    pub fn parse(text: &str) -> Option<Self> {
        unhex::<32>(text).map(Self)
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(&self.0))
    }
}

impl From<ContentHash> for String {
    fn from(hash: ContentHash) -> String {
        hash.to_string()
    }
}

impl TryFrom<String> for ContentHash {
    type Error = String;
    fn try_from(text: String) -> std::result::Result<Self, String> {
        Self::parse(&text).ok_or_else(|| "a content hash must be 64 lower-case hex digits".into())
    }
}
