//! Every route: its method, path and query, whether it takes a sign-in, its
//! limits and what may be kept of its reply.
use crate::ids::{ContentHash, Revision, SourceLineageId, SourceRevision};
use crate::limits::{
    DEFAULT_SEARCH_ROWS, MAX_ACCOUNT_BYTES, MAX_BUNDLE_BYTES, MAX_INDEX_BYTES, MAX_QUERY,
    MAX_SEARCH_BYTES, MAX_SHARD_BYTES, MAX_TARGET_BYTES,
};

/// The media type of every request and reply body.
pub const MEDIA_TYPE: &str = "application/json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

/// Whether a request carries `Authorization: Bearer`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    /// Never, signed in or not: the server needs no account.
    Public,
    /// When the client is signed in; the server may answer 401 without it.
    WhenSignedIn,
    /// Always.
    Required,
}

/// What may be kept of a reply. The v1 client sends no conditional request
/// and reads no caching header, so these rules bind a server's headers and
/// anything between the two, not the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caching {
    /// The path names the content for good: it may be kept forever.
    Immutable,
    /// As [`Self::Immutable`], for the person who downloaded it only.
    ImmutablePrivate,
    /// May be kept and must be asked about again before use. A server may
    /// answer with an `ETag`: a strong one, which changes with any byte.
    Revalidate,
    /// Must not be kept.
    Never,
}

impl Caching {
    /// The `Cache-Control` header that says so.
    pub fn header(self) -> &'static str {
        match self {
            Self::Immutable => "public, max-age=31536000, immutable",
            Self::ImmutablePrivate => "private, max-age=31536000, immutable",
            Self::Revalidate => "no-cache",
            Self::Never => "no-store",
        }
    }
}

/// One page of a search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    /// The query as the person typed it, at most [`MAX_QUERY`] bytes.
    pub text: String,
    /// Rows wanted; a server returns at most
    /// [`crate::limits::MAX_SEARCH_ROWS`].
    pub limit: u64,
    /// Ranked rows to skip.
    pub offset: u64,
}

/// A request's route and what its path and query say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// `GET /v1/index`
    Index,
    /// `GET /v1/shards/{sha256}`
    Shard(ContentHash),
    /// `GET /v1/search?q=&limit=&offset=`
    Search(SearchQuery),
    /// `GET /v1/bundles/{lineage}/{revision}`
    Bundle(SourceRevision),
    /// `POST /v1/device/code`
    DeviceCode,
    /// `POST /v1/device/token`
    DeviceToken,
    /// `GET /v1/account`
    Account,
    /// `DELETE /v1/auth/token`
    Revoke,
}

/// Why a request is no route of the contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    /// 404 [`crate::status::codes::NOT_FOUND`].
    NotFound,
    /// 400 [`crate::status::codes::CATALOG_QUERY`], and why.
    Query(String),
}

impl Route {
    pub fn method(&self) -> Method {
        match self {
            Self::DeviceCode | Self::DeviceToken => Method::Post,
            Self::Revoke => Method::Delete,
            Self::Index | Self::Shard(_) | Self::Search(_) | Self::Bundle(_) | Self::Account => {
                Method::Get
            }
        }
    }

    /// The request's path and query.
    pub fn target(&self) -> String {
        match self {
            Self::Index => "/v1/index".into(),
            Self::Shard(hash) => format!("/v1/shards/{hash}"),
            Self::Search(query) => format!(
                "/v1/search?q={}&limit={}&offset={}",
                percent_encode(&query.text),
                query.limit,
                query.offset
            ),
            Self::Bundle(root) => format!("/v1/bundles/{}/{}", root.lineage, root.revision.0),
            Self::DeviceCode => "/v1/device/code".into(),
            Self::DeviceToken => "/v1/device/token".into(),
            Self::Account => "/v1/account".into(),
            Self::Revoke => "/v1/auth/token".into(),
        }
    }

    pub fn auth(&self) -> Auth {
        match self {
            Self::Bundle(_) => Auth::WhenSignedIn,
            Self::Account | Self::Revoke => Auth::Required,
            Self::Index
            | Self::Shard(_)
            | Self::Search(_)
            | Self::DeviceCode
            | Self::DeviceToken => Auth::Public,
        }
    }

    /// Bytes of its reply, whatever the status.
    pub fn max_reply(&self) -> u64 {
        match self {
            Self::Index => MAX_INDEX_BYTES,
            Self::Shard(_) => MAX_SHARD_BYTES,
            Self::Search(_) => MAX_SEARCH_BYTES,
            Self::Bundle(_) => MAX_BUNDLE_BYTES,
            Self::DeviceCode | Self::DeviceToken | Self::Account | Self::Revoke => {
                MAX_ACCOUNT_BYTES
            }
        }
    }

    /// Bytes of its request body.
    pub fn max_request(&self) -> u64 {
        match self.method() {
            Method::Post => MAX_ACCOUNT_BYTES,
            Method::Get | Method::Delete => 0,
        }
    }

    pub fn caching(&self) -> Caching {
        match self {
            Self::Index | Self::Search(_) => Caching::Revalidate,
            Self::Shard(_) => Caching::Immutable,
            Self::Bundle(_) => Caching::ImmutablePrivate,
            Self::DeviceCode | Self::DeviceToken | Self::Account | Self::Revoke => Caching::Never,
        }
    }

    /// Read a request line's method and target (a server's side). Unknown
    /// query parameters are ignored; of a repeated one the last counts.
    pub fn parse(method: &str, target: &str) -> std::result::Result<Self, Refused> {
        if target.len() > MAX_TARGET_BYTES {
            return Err(Refused::NotFound);
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let route = match path.strip_prefix("/v1/").unwrap_or_default() {
            "index" => Self::Index,
            "search" => Self::Search(search_query(query)?),
            "device/code" => Self::DeviceCode,
            "device/token" => Self::DeviceToken,
            "account" => Self::Account,
            "auth/token" => Self::Revoke,
            other => item(other).ok_or(Refused::NotFound)?,
        };
        // One spelling per path: `/r01` and `/R1` are not `/1`.
        let exact = matches!(route, Self::Search(_)) || route.target() == path;
        if exact && route.method().as_str() == method {
            Ok(route)
        } else {
            Err(Refused::NotFound)
        }
    }
}

/// A shard's or a bundle's path, after the prefix.
fn item(path: &str) -> Option<Route> {
    if let Some(hash) = path.strip_prefix("shards/") {
        return ContentHash::parse(hash).map(Route::Shard);
    }
    let (lineage, revision) = path.strip_prefix("bundles/")?.split_once('/')?;
    Some(Route::Bundle(SourceRevision {
        lineage: SourceLineageId::parse(lineage)?,
        revision: Revision(revision.parse().ok()?),
    }))
}

fn search_query(query: &str) -> std::result::Result<SearchQuery, Refused> {
    let mut out = SearchQuery {
        text: String::new(),
        limit: DEFAULT_SEARCH_ROWS as u64,
        offset: 0,
    };
    let number = |name: &str, value: &str| {
        value
            .parse::<u64>()
            .map_err(|_| Refused::Query(format!("{name} is not a number")))
    };
    for (name, value) in query.split('&').filter_map(|pair| pair.split_once('=')) {
        match name {
            "q" => {
                out.text = percent_decode(value)
                    .ok_or_else(|| Refused::Query("q is not UTF-8 text".into()))?;
            }
            "limit" => out.limit = number(name, value)?,
            "offset" => out.offset = number(name, value)?,
            _ => {}
        }
    }
    if out.text.len() > MAX_QUERY {
        return Err(Refused::Query(format!(
            "a search has at most {MAX_QUERY} bytes"
        )));
    }
    Ok(out)
}

/// Percent-encoding of a query parameter (RFC 3986 unreserved kept).
pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The text of a query parameter: `%XX` and `+` decoded; `None` when a `%`
/// is not followed by two hex digits or the bytes are not UTF-8.
pub fn percent_decode(text: &str) -> Option<String> {
    let mut out = Vec::with_capacity(text.len());
    let mut bytes = text.bytes();
    while let Some(b) = bytes.next() {
        out.push(match b {
            b'%' => {
                let mut digit = || char::from(bytes.next()?).to_digit(16);
                let value = digit()? * 16 + digit()?;
                u8::try_from(value).ok()?
            }
            b'+' => b' ',
            other => other,
        });
    }
    String::from_utf8(out).ok()
}
