//! A server's conformance test: what a v1 client depends on, asked through
//! the caller's own transport (a socket, or a server's router called in
//! process). It names no core type, so a server that links no core runs it
//! too.
//!
//! [`run`] changes nothing on the server but what it is given to spend: the
//! sign-in codes it asks for and [`Setup::spend`]. A check whose setup is
//! missing is reported as skipped, never as passed.
use crate::account::{
    DeviceCodeRequest, DevicePoll, DeviceTokenRequest, Token, bearer, decode_account, decode_code,
};
use crate::documents::{Index, decode_bundle, decode_index, decode_search, decode_shard};
use crate::hash::{base64_decode, bytes_hash};
use crate::ids::{ContentHash, LibraryId, Revision, SourceLineageId, SourceRevision};
use crate::limits::{
    MAX_ASSETS, MAX_BUNDLE_RECORDS, MAX_CATALOG_BYTES, MAX_LIBRARY_NAME, MAX_MEDIA_TYPE, MAX_QUERY,
};
use crate::route::{MEDIA_TYPE, Method, Route, SearchQuery};
use crate::status::{ErrorBody, codes};
use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::value::RawValue;

/// A request as the test sends it. Follow no redirect.
#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    /// Path and query.
    pub target: String,
    /// The whole `Authorization` header.
    pub authorization: Option<String>,
    /// JSON, with `content-type: application/json`.
    pub body: Option<Vec<u8>>,
    /// Bytes of reply the test accepts.
    pub max_reply: u64,
}

/// What came back, of any status.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub status: u16,
    /// The `content-type` header.
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// Sends one request to the server under test.
pub trait Transport {
    fn send(&mut self, request: &Request) -> Result<Reply, String>;
}

impl<F: FnMut(&Request) -> Result<Reply, String>> Transport for F {
    fn send(&mut self, request: &Request) -> Result<Reply, String> {
        self(request)
    }
}

/// What the test may use.
#[derive(Clone, Copy, Debug)]
pub struct Setup<'a> {
    /// A search that finds at least one entry of the served catalog.
    pub search: &'a str,
    /// A search the server refuses as not understood.
    pub refused_search: Option<&'a str>,
    /// A token the server knows.
    pub token: Option<&'a str>,
    /// Another token it knows, to be revoked by the test.
    pub spend: Option<&'a str>,
    /// A device code and the answer its next poll gives.
    pub polls: &'a [(&'a str, Expected)],
}

/// What a poll is expected to answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expected {
    Approved,
    SlowDown,
    Expired,
    Denied,
}

/// What became of one check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    /// Not run, and why.
    Skipped(String),
    Failed(String),
}

/// A check, by name, and what became of it.
pub type Report = Vec<(&'static str, Outcome)>;

enum Stop {
    Skip(String),
    Fail(String),
}

impl From<String> for Stop {
    fn from(why: String) -> Self {
        Self::Fail(why)
    }
}

impl From<crate::Error> for Stop {
    fn from(error: crate::Error) -> Self {
        Self::Fail(error.to_string())
    }
}

type Checked = Result<(), Stop>;

struct Run<'a> {
    transport: &'a mut dyn Transport,
    setup: Setup<'a>,
}

type Check = fn(&mut Run<'_>) -> Checked;

const CHECKS: &[(&str, Check)] = &[
    ("index", |r| r.index()),
    ("shards", |r| r.shards()),
    ("search", |r| r.search()),
    ("search pages", |r| r.search_pages()),
    ("search refusal", |r| r.search_refusal()),
    ("unknown query parameter", |r| r.unknown_parameter()),
    ("unknown things", |r| r.unknown_things()),
    ("bundle", |r| r.bundle()),
    ("bundle without a sign-in", |r| r.bundle_without_sign_in()),
    ("account", |r| r.account()),
    ("account without a sign-in", |r| r.account_without_sign_in()),
    ("sign-in code", |r| r.sign_in_code()),
    ("sign-in polls", |r| r.polls()),
    ("revoke", |r| r.revoke()),
];

/// Run every check, each on its own.
pub fn run(transport: &mut dyn Transport, setup: Setup<'_>) -> Report {
    let mut run = Run { transport, setup };
    CHECKS
        .iter()
        .map(|(name, check)| {
            let outcome = match check(&mut run) {
                Ok(()) => Outcome::Passed,
                Err(Stop::Skip(why)) => Outcome::Skipped(why),
                Err(Stop::Fail(why)) => Outcome::Failed(why),
            };
            (*name, outcome)
        })
        .collect()
}

/// The report's failures, one per line; empty when nothing failed.
pub fn failures(report: &Report) -> String {
    let mut out = String::new();
    for (name, outcome) in report {
        if let Outcome::Failed(why) = outcome {
            out.push_str(&format!("{name}: {why}\n"));
        }
    }
    out
}

#[derive(Deserialize)]
struct CatalogView {
    library: LibraryId,
    entries: Vec<IgnoredAny>,
    revisions: Vec<RecordView>,
}

#[derive(Deserialize)]
struct RecordView {
    lineage: SourceLineageId,
    revision: Revision,
    content_hash: ContentHash,
}

#[derive(Deserialize)]
struct BundleView {
    library: LibraryId,
    name: String,
    root: SourceRevision,
    records: Vec<Box<RawValue>>,
    assets: Vec<AssetView>,
}

#[derive(Deserialize)]
struct RecordHead {
    lineage: SourceLineageId,
    revision: Revision,
}

#[derive(Deserialize)]
struct AssetView {
    hash: ContentHash,
    media_type: String,
    data: String,
}

/// JSON text without the spaces between its tokens.
pub(crate) fn compact(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let (mut quoted, mut escaped) = (false, false);
    for c in json.chars() {
        if quoted {
            quoted = escaped || c != '"';
            escaped = !escaped && c == '\\';
        } else {
            quoted = c == '"';
            if c.is_ascii_whitespace() {
                continue;
            }
        }
        out.push(c);
    }
    out
}

fn ensure(holds: bool, why: impl FnOnce() -> String) -> Checked {
    if holds {
        Ok(())
    } else {
        Err(Stop::Fail(why()))
    }
}

fn needed<T>(given: Option<T>, what: &str) -> Result<T, Stop> {
    given.ok_or_else(|| Stop::Skip(format!("the setup has no {what}")))
}

/// A path no server has.
const NOWHERE: ContentHash = ContentHash([0; 32]);

impl Run<'_> {
    fn exchange(&mut self, request: &Request) -> Result<Reply, Stop> {
        let reply = self.transport.send(request)?;
        let place = format!("{} {}", request.method.as_str(), request.target);
        ensure(reply.body.len() as u64 <= request.max_reply, || {
            format!("{place}: the reply exceeds {} bytes", request.max_reply)
        })?;
        ensure(!(300..400).contains(&reply.status), || {
            format!("{place}: a redirect ({})", reply.status)
        })?;
        let json = reply.content_type.as_deref().is_some_and(|t| {
            t.split(';')
                .next()
                .is_some_and(|t| t.trim().eq_ignore_ascii_case(MEDIA_TYPE))
        });
        ensure(json || reply.body.is_empty(), || {
            format!(
                "{place}: the content type is {:?}, not {MEDIA_TYPE}",
                reply.content_type
            )
        })?;
        Ok(reply)
    }

    fn ask(
        &mut self,
        route: &Route,
        token: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<Reply, Stop> {
        self.exchange(&Request {
            method: route.method(),
            target: route.target(),
            authorization: token.map(bearer),
            body,
            max_reply: route.max_reply(),
        })
    }

    /// The body of a request that must succeed.
    fn ok(&mut self, route: &Route, token: Option<&str>) -> Result<Vec<u8>, Stop> {
        let reply = self.ask(route, token, None)?;
        ensure((200..300).contains(&reply.status), || {
            format!("{}: status {}", route.target(), reply.status)
        })?;
        Ok(reply.body)
    }

    /// A request that must be refused with `status` and `code`.
    fn refused(&mut self, route: &Route, token: Option<&str>, status: u16, code: &str) -> Checked {
        let reply = self.ask(route, token, None)?;
        let place = route.target();
        ensure(reply.status == status, || {
            format!("{place}: status {}, not {status}", reply.status)
        })?;
        let error = ErrorBody::read(&reply.body).error;
        ensure(error.code == code, || {
            format!("{place}: code {:?}, not {code:?}", error.code)
        })?;
        ensure(error.message.is_some_and(|m| !m.trim().is_empty()), || {
            format!("{place}: no message for the person")
        })
    }

    fn read_index(&mut self) -> Result<Index, Stop> {
        Ok(decode_index(&self.ok(&Route::Index, None)?)?)
    }

    fn read_shard(
        &mut self,
        hash: ContentHash,
        budget: &mut u64,
    ) -> Result<crate::documents::Shard<CatalogView>, Stop> {
        let bytes = self.ok(&Route::Shard(hash), None)?;
        *budget = budget
            .checked_sub(bytes.len() as u64)
            .ok_or_else(|| Stop::Fail(format!("the shards exceed {MAX_CATALOG_BYTES} bytes")))?;
        Ok(decode_shard(&bytes, hash)?)
    }

    fn index(&mut self) -> Checked {
        let index = self.read_index()?;
        for (i, info) in index.shards.iter().enumerate() {
            // A client looks a library's revisions up in its one shard.
            ensure(
                index
                    .shards
                    .iter()
                    .take(i)
                    .all(|s| s.library != info.library),
                || format!("library {} has two shards", info.library),
            )?;
        }
        Ok(())
    }

    fn shards(&mut self) -> Checked {
        let mut budget = MAX_CATALOG_BYTES;
        for info in self.read_index()?.shards {
            let shard = self.read_shard(info.sha256, &mut budget)?;
            ensure(shard.catalog.library == info.library, || {
                format!(
                    "shard {}: library {}, and the index says {}",
                    info.sha256, shard.catalog.library, info.library
                )
            })?;
            ensure(shard.catalog.entries.len() == info.entries, || {
                format!(
                    "shard {}: {} entries, and the index says {}",
                    info.sha256,
                    shard.catalog.entries.len(),
                    info.entries
                )
            })?;
        }
        Ok(())
    }

    fn find(&mut self, limit: u64, offset: u64) -> Result<(usize, Vec<Box<RawValue>>), Stop> {
        let route = Route::Search(SearchQuery {
            text: self.setup.search.into(),
            limit,
            offset,
        });
        let reply = decode_search::<Box<RawValue>>(&self.ok(&route, None)?)?;
        ensure(reply.results.len() as u64 <= limit, || {
            format!("{} rows, and {limit} were asked for", reply.results.len())
        })?;
        Ok((
            reply.total,
            reply.results.into_iter().map(|r| r.hit).collect(),
        ))
    }

    fn search(&mut self) -> Checked {
        let (total, rows) = self.find(10, 0)?;
        ensure(!rows.is_empty() && total >= rows.len(), || {
            format!("{} rows of {total}", rows.len())
        })
    }

    fn search_pages(&mut self) -> Checked {
        let (total, first) = self.find(2, 0)?;
        if total < 2 {
            return Err(Stop::Skip("the search finds one entry only".into()));
        }
        let (_, one) = self.find(1, 0)?;
        let (_, two) = self.find(1, 1)?;
        let text = |rows: &[Box<RawValue>]| -> Vec<String> {
            rows.iter().map(|r| compact(r.get())).collect()
        };
        ensure(
            text(&first) == [text(&one), text(&two)].concat() && !two.is_empty(),
            || "two pages of one row are not the page of two".into(),
        )
    }

    fn search_refusal(&mut self) -> Checked {
        let text = needed(self.setup.refused_search, "refused search")?;
        let query = |text: &str| {
            Route::Search(SearchQuery {
                text: text.into(),
                limit: 1,
                offset: 0,
            })
        };
        self.refused(&query(text), None, 400, codes::CATALOG_QUERY)?;
        let long = "x".repeat(MAX_QUERY + 1);
        self.refused(&query(&long), None, 400, codes::CATALOG_QUERY)
    }

    fn unknown_parameter(&mut self) -> Checked {
        let route = Route::Search(SearchQuery {
            text: self.setup.search.into(),
            limit: 1,
            offset: 0,
        });
        let reply = self.exchange(&Request {
            method: route.method(),
            target: format!("{}&not_of_this_version=1", route.target()),
            authorization: None,
            body: None,
            max_reply: route.max_reply(),
        })?;
        ensure(reply.status == 200, || {
            format!("status {} for one more parameter", reply.status)
        })
    }

    fn unknown_things(&mut self) -> Checked {
        self.refused(&Route::Shard(NOWHERE), None, 404, codes::NOT_FOUND)?;
        let token = needed(self.setup.token, "token")?;
        let nothing = Route::Bundle(SourceRevision {
            lineage: SourceLineageId(0),
            revision: Revision::FIRST,
        });
        self.refused(&nothing, Some(token), 404, codes::NOT_FOUND)
    }

    /// A revision the index lists, and its shard.
    fn listed(&mut self) -> Result<(SourceRevision, CatalogView), Stop> {
        let mut budget = MAX_CATALOG_BYTES;
        for info in self.read_index()?.shards {
            let shard = self.read_shard(info.sha256, &mut budget)?;
            if let Some(entry) = shard.entries.first()
                && let Some(record) = shard
                    .catalog
                    .revisions
                    .iter()
                    .filter(|r| r.lineage == entry.lineage)
                    .max_by_key(|r| r.revision)
            {
                let root = SourceRevision {
                    lineage: record.lineage,
                    revision: record.revision,
                };
                return Ok((root, shard.catalog));
            }
        }
        Err(Stop::Skip("the index lists no entry".into()))
    }

    fn bundle(&mut self) -> Checked {
        let token = needed(self.setup.token, "token")?;
        let (root, catalog) = self.listed()?;
        let bytes = self.ok(&Route::Bundle(root), Some(token))?;
        let bundle: BundleView = decode_bundle(&bytes)?;
        ensure(bundle.root == root, || "another root".into())?;
        ensure(bundle.library == catalog.library, || {
            "another library".into()
        })?;
        ensure(bundle.name.len() <= MAX_LIBRARY_NAME, || {
            "too long a name".into()
        })?;
        ensure(
            (1..=MAX_BUNDLE_RECORDS).contains(&bundle.records.len())
                && bundle.assets.len() <= MAX_ASSETS,
            || "too many records or assets, or none".into(),
        )?;
        let mut rooted = false;
        for record in &bundle.records {
            let head: RecordHead = serde_json::from_str(record.get()).map_err(|e| e.to_string())?;
            rooted |= head.lineage == root.lineage && head.revision == root.revision;
            let listed = catalog
                .revisions
                .iter()
                .find(|r| r.lineage == head.lineage && r.revision == head.revision);
            let hash = bytes_hash(compact(record.get()).as_bytes());
            ensure(listed.is_some_and(|r| r.content_hash == hash), || {
                format!(
                    "record {} {} is not the one the shard lists",
                    head.lineage, head.revision
                )
            })?;
        }
        ensure(rooted, || "the root is not among the records".into())?;
        for asset in &bundle.assets {
            let held = base64_decode(&asset.data).map(|data| bytes_hash(&data));
            ensure(held == Some(asset.hash), || {
                format!("asset {} does not hash to its name", asset.hash)
            })?;
            ensure(
                (1..=MAX_MEDIA_TYPE).contains(&asset.media_type.len()),
                || format!("asset {}: no usable media type", asset.hash),
            )?;
        }
        Ok(())
    }

    fn bundle_without_sign_in(&mut self) -> Checked {
        let (root, _) = self.listed()?;
        let route = Route::Bundle(root);
        match self.ask(&route, None, None)?.status {
            200 => Ok(()),
            _ => self.refused(&route, None, 401, codes::AUTH_REQUIRED),
        }
    }

    fn account(&mut self) -> Checked {
        let token = needed(self.setup.token, "token")?;
        decode_account(&self.ok(&Route::Account, Some(token))?)?;
        Ok(())
    }

    fn account_without_sign_in(&mut self) -> Checked {
        for route in [Route::Account, Route::Revoke] {
            self.refused(&route, None, 401, codes::AUTH_REQUIRED)?;
            let unknown = Some("a-token-no-server-issued");
            self.refused(&route, unknown, 401, codes::AUTH_REQUIRED)?;
        }
        Ok(())
    }

    fn sign_in_code(&mut self) -> Checked {
        let request = DeviceCodeRequest {
            client_name: "Conformance test".into(),
        };
        let mut body = serde_json::to_value(&request).map_err(crate::Error::from)?;
        body["not_of_this_version"] = true.into();
        let body = serde_json::to_vec(&body).map_err(crate::Error::from)?;
        let reply = self.ask(&Route::DeviceCode, None, Some(body))?;
        ensure(reply.status == 200, || format!("status {}", reply.status))?;
        let code = decode_code(&reply.body)?;
        ensure(
            !code.device_code.expose().trim().is_empty()
                && !code.user_code.trim().is_empty()
                && code.user_code == code.shown_code()
                && code.expires_in > 0,
            || "an unusable sign-in code".into(),
        )?;
        ensure(
            self.poll(code.device_code.clone())? == DevicePoll::Pending,
            || "a code nobody confirmed is not pending".into(),
        )
    }

    fn poll(&mut self, device_code: Token) -> Result<DevicePoll, Stop> {
        let body =
            serde_json::to_vec(&DeviceTokenRequest { device_code }).map_err(crate::Error::from)?;
        let reply = self.ask(&Route::DeviceToken, None, Some(body))?;
        Ok(DevicePoll::read(reply.status, &reply.body)?)
    }

    fn polls(&mut self) -> Checked {
        if self.setup.polls.is_empty() {
            return Err(Stop::Skip("the setup has no polls".into()));
        }
        for (code, expected) in self.setup.polls {
            let got = match self.poll(Token((*code).into()))? {
                DevicePoll::Approved(reply) => {
                    ensure(!reply.access_token.expose().trim().is_empty(), || {
                        "an approved sign-in without a token".into()
                    })?;
                    Some(Expected::Approved)
                }
                DevicePoll::SlowDown => Some(Expected::SlowDown),
                DevicePoll::Expired => Some(Expected::Expired),
                DevicePoll::Denied => Some(Expected::Denied),
                DevicePoll::Pending | DevicePoll::Refused => None,
            };
            ensure(got == Some(*expected), || {
                format!("a poll answered {got:?}, not {expected:?}")
            })?;
        }
        Ok(())
    }

    fn revoke(&mut self) -> Checked {
        let token = needed(self.setup.spend, "token to revoke")?;
        self.ok(&Route::Revoke, Some(token))?;
        self.refused(&Route::Account, Some(token), 401, codes::AUTH_REQUIRED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compacting_keeps_text() {
        assert_eq!(
            compact("{ \"a b\" : [ 1 ,\n \"c \\\" d\\\\\" , \" \" ] }"),
            "{\"a b\":[1,\"c \\\" d\\\\\",\" \"]}"
        );
    }
}
