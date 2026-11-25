//! Every bound of the contract. A reader refuses what is over one before it
//! keeps or parses it; a writer stays within them.

/// Bytes of the index.
pub const MAX_INDEX_BYTES: u64 = 4 * 1024 * 1024;
/// Shards one index lists.
pub const MAX_SHARDS: usize = 4096;
/// Bytes of one shard.
pub const MAX_SHARD_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes of all shards of one index together.
pub const MAX_CATALOG_BYTES: u64 = MAX_PACKED_BYTES as u64;
/// Bytes of a search reply.
pub const MAX_SEARCH_BYTES: u64 = 4 * 1024 * 1024;
/// Bytes of a search's text.
pub const MAX_QUERY: usize = 512;
/// Rows one search reply holds.
pub const MAX_SEARCH_ROWS: usize = 500;
/// Rows of a search that gives no limit.
pub const DEFAULT_SEARCH_ROWS: usize = 50;
/// Bytes of a bundle reply: [`MAX_PACKED_BYTES`] and its records.
pub const MAX_BUNDLE_BYTES: u64 = 160 * 1024 * 1024;
/// Revision records one bundle carries (a root and its closure).
pub const MAX_BUNDLE_RECORDS: usize = 1024;
/// Links one record lists.
pub const MAX_LINKS: usize = 256;
/// Assets one bundle packs.
pub const MAX_ASSETS: usize = 256;
/// Bytes of one asset.
pub const MAX_ASSET_BYTES: usize = 64 * 1024 * 1024;
/// Bytes all assets of one bundle take as base64 text.
pub const MAX_PACKED_BYTES: usize = 96 * 1024 * 1024;
/// Bytes of an asset's media type.
pub const MAX_MEDIA_TYPE: usize = 128;
/// Bytes of a library's name in a bundle.
pub const MAX_LIBRARY_NAME: usize = 256;
/// Bytes of a device or account request or reply.
pub const MAX_ACCOUNT_BYTES: u64 = 64 * 1024;
/// Characters of the name a client signs in under.
pub const MAX_CLIENT_NAME: usize = 80;
/// Characters of a sign-in code a client shows.
pub const MAX_USER_CODE: usize = 32;
/// Bytes of a request's path and query (the length RFC 9110, 4.1, asks every
/// recipient to support).
pub const MAX_TARGET_BYTES: usize = 8000;

const _: () = assert!(MAX_ASSET_BYTES.div_ceil(3) * 4 <= MAX_PACKED_BYTES);
const _: () = assert!((MAX_PACKED_BYTES as u64) < MAX_BUNDLE_BYTES);
