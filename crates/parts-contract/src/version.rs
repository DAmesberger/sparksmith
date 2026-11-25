//! The contract's version.
//!
//! A version has two numbers. The **major** is what both sides say on the
//! wire: the client by the path it asks ([`PREFIX`], `/v1/…`), the server by
//! the `format` of every catalog document it sends ([`FORMAT`]). The
//! **minor** is this crate's and is never on the wire: within one major a
//! client and a server of any two minors understand each other.
//!
//! Within a major these may change, as a new minor:
//! - a new optional field in a reply or a request body, a new optional query
//!   parameter, a new route, a new error code, a new review word. A reader
//!   ignores fields and parameters it does not know, shows an unknown review
//!   word as "not reviewed", and reads an unknown error code by its status.
//! - a larger limit on what a *reader* accepts. A writer uses a larger limit
//!   only when every reader it serves has it.
//!
//! These need a new major, its routes beside the old ones: removing or
//! renaming a field, parameter, code or route; another type, meaning or
//! required sign-in; a field that becomes required; another content hash;
//! a smaller limit.
//!
//! Strictness, per direction. A **client** reading a reply ignores unknown
//! fields, refuses a missing or mistyped field it needs, and refuses another
//! `format`, naming it. A **server** reading a request ignores unknown fields
//! and query parameters and refuses a missing or mistyped one it needs. The
//! records inside a bundle are the exception in both directions: the core
//! reads them strictly, since a field dropped from one would change its
//! content hash. A document inside another (`catalog`, `hit`, `bundle`) is
//! therefore as strict as the core's own type.

/// The contract this crate is: the major is on the wire, the minor is not.
pub const MAJOR: u32 = 1;
/// Counts the additions made within [`MAJOR`].
pub const MINOR: u32 = 0;
/// What every path starts with.
pub const PREFIX: &str = "/v1";
/// The `format` of the index, of shards, of search replies and of bundle
/// replies.
pub const FORMAT: u32 = 1;
