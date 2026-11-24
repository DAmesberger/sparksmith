//! An online parts registry as a session source (part-search §5.2, registry
//! R0 §10). The session holds no HTTP client: the host installs an
//! [`OnlineSource`] (the desktop's registry client), and `library.search`
//! with `src:online` adds its rows after the local ones, ranked last. A
//! registry root is then placed or imported like a folder's
//! (`library.place_from_folder`, `library.import_from_folder`): its bundle
//! is fetched, checked against the registry's content hashes and imported
//! as an exact snapshot, so the project never depends on the network.
use super::Session;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparksmith_core::catalog::{Hit, term_spans};
use sparksmith_core::ids::{LibraryId, Revision, SourceLineageId};
use sparksmith_core::library::bundle::Bundle;
use std::sync::Arc;

/// Said when no registry is installed.
const NO_REGISTRY: &str = "no online registry is turned on (Settings › Libraries)";

fn refused(message: String) -> crate::Error {
    crate::Error::Document(sparksmith_core::Error::Invalid(message))
}

/// Rows one online search returns at most.
pub const MAX_ONLINE_ROWS: usize = 100;

/// One registry row: the core's hit (its entry marked
/// `Source::Online`) and what the registry says about the entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OnlineHit {
    pub hit: Hit,
    /// The registry's entry id (`publisher/name`).
    #[serde(default)]
    pub id: String,
    /// `reviewed`, `in_review`, or anything else: not reviewed.
    #[serde(default)]
    pub review: String,
}

impl OnlineHit {
    /// Whether a person has reviewed the entry.
    pub fn reviewed(&self) -> bool {
        self.review == "reviewed"
    }
    /// The review badge's words.
    pub fn review_label(&self) -> &'static str {
        match self.review.as_str() {
            "reviewed" => "reviewed",
            "in_review" => "in review",
            _ => "not reviewed",
        }
    }
}

/// One online search: the same query text read by the registry with the
/// same core, or by the mirror's last index when offline.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct OnlineReport {
    /// The registry's display name (its host).
    pub source: String,
    pub snapshot: String,
    pub chips: Vec<String>,
    /// Matching entries before the limit.
    pub total: usize,
    pub results: Vec<OnlineHit>,
    /// Offline: the mirror answered, with the index's date ("index of 14
    /// November"); `None` when the registry did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline: Option<String>,
    /// The registry's own search time, in microseconds (0 offline).
    #[serde(default)]
    pub took_us: u64,
}

/// Who is signed in to the registry, as far as this computer knows:
/// searching needs no account, downloading part data needs one.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct OnlineAccount {
    pub signed_in: bool,
    /// The account's email, once the registry said it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// What signing out did.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq, schemars::JsonSchema)]
pub struct SignOut {
    /// There was a sign-in to remove.
    pub was_signed_in: bool,
    /// The registry revoked the token; `false` when it could not be
    /// reached (the token is removed from this computer either way).
    pub revoked: bool,
    /// Why the registry did not revoke it, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// What a host gives a session to search and fetch from an online
/// registry. Calls block; the session runs them on its own thread.
pub trait OnlineSource: Send + Sync {
    /// The registry's display name.
    fn name(&self) -> String;
    /// `library.search` of `text` on the registry.
    fn search(&self, text: &str, limit: usize) -> Result<OnlineReport, String>;
    /// The checked bundle of a registry root; `Ok(None)` when `library` is
    /// not this registry's.
    fn bundle(
        &self,
        library: LibraryId,
        lineage: SourceLineageId,
        revision: Option<Revision>,
    ) -> Result<Option<Bundle>, String>;
    /// Who is signed in, from this computer's sign-in (no network).
    fn account(&self) -> Result<OnlineAccount, String>;
    /// Revoke the sign-in at the registry (best effort) and remove it from
    /// this computer.
    fn sign_out(&self) -> Result<SignOut, String>;
}

/// Whether `text` asks for online rows: an unquoted `src:online` term.
pub fn asks_online(text: &str) -> bool {
    term_spans(text).is_ok_and(|spans| {
        spans
            .into_iter()
            .filter_map(|r| text.get(r))
            .any(|t| t.eq_ignore_ascii_case("src:online"))
    })
}

impl Session {
    /// Install (or remove) the online registry `library.search` reaches
    /// with `src:online`.
    pub fn set_online(&mut self, online: Option<Arc<dyn OnlineSource>>) {
        self.online = online;
    }

    /// Add the registry's rows to a `library.search` reply when the text
    /// asks for them: after the local results, counted in `total`, with the
    /// registry's facts under `online`. A registry that cannot answer is
    /// reported there, never an error of the search.
    pub(super) fn add_online(&self, text: &str, limit: Option<usize>, reply: &mut Value) {
        if !asks_online(text) {
            return;
        }
        let Some(online) = &self.online else {
            reply["online"] = json!({"error": NO_REGISTRY});
            return;
        };
        let limit = limit
            .unwrap_or(sparksmith_core::catalog::DEFAULT_LIMIT)
            .min(MAX_ONLINE_ROWS);
        match online.search(text, limit) {
            Ok(report) => {
                let rows: Vec<Value> = report
                    .results
                    .iter()
                    .filter_map(|r| serde_json::to_value(&r.hit).ok())
                    .collect();
                let total = reply["result"]["total"].as_u64().unwrap_or(0) + report.total as u64;
                if let Some(results) = reply["result"]["results"].as_array_mut() {
                    results.extend(rows);
                }
                reply["result"]["total"] = json!(total);
                reply["online"] = json!({
                    "source": report.source,
                    "snapshot": report.snapshot,
                    "total": report.total,
                    "offline": report.offline,
                    "entries": report.results.iter().map(|r| json!({
                        "lineage": r.hit.entry.source_lineage,
                        "id": r.id,
                        "review": r.review_label(),
                    })).collect::<Vec<_>>(),
                    "place": "act library.place_from_folder {library, lineage} (the entry's source library and source_lineage)",
                });
            }
            Err(error) => {
                reply["online"] = json!({"source": online.name(), "error": error});
            }
        }
    }

    /// `library.account`: the registry and who is signed in to it.
    pub(super) fn online_account(&self) -> crate::Result<Value> {
        let Some(online) = &self.online else {
            return Ok(json!({
                "registry": null,
                "signed_in": false,
                "note": NO_REGISTRY,
            }));
        };
        let account = online
            .account()
            .map_err(|why| refused(format!("{}: {why}", online.name())))?;
        let mut reply = json!({"registry": online.name()});
        if let (Value::Object(reply), Ok(Value::Object(fields))) =
            (&mut reply, serde_json::to_value(&account))
        {
            reply.extend(fields);
        }
        if !account.signed_in {
            reply["note"] = json!(format!(
                "Downloading parts from {} needs a free account; the person signs in \
                 in Settings › Libraries. Searching works without one.",
                online.name()
            ));
        }
        Ok(reply)
    }

    /// `library.sign_out`: revoke and forget the registry sign-in.
    pub(super) fn online_sign_out(&self) -> crate::Result<Value> {
        let Some(online) = &self.online else {
            return Err(refused(NO_REGISTRY.into()));
        };
        let out = online
            .sign_out()
            .map_err(|why| refused(format!("{}: {why}", online.name())))?;
        let mut reply = serde_json::to_value(&out)?;
        reply["registry"] = json!(online.name());
        reply["signed_in"] = json!(false);
        Ok(reply)
    }

    /// The bundle of a folder root, else of a registry root.
    pub(super) fn source_bundle(
        &self,
        library: LibraryId,
        lineage: SourceLineageId,
        revision: Option<Revision>,
    ) -> crate::Result<Bundle> {
        match self.sources.bundle(library, lineage, revision) {
            Ok(bundle) => Ok(bundle),
            Err(folder) => match &self.online {
                Some(online) => match online.bundle(library, lineage, revision) {
                    Ok(Some(bundle)) => Ok(bundle),
                    Ok(None) => Err(folder),
                    Err(why) => Err(crate::Error::Document(sparksmith_core::Error::Invalid(
                        format!("{}: {why}", online.name()),
                    ))),
                },
                None => Err(folder),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sparksmith_core::Engine;
    use sparksmith_core::catalog::{MatchTier, Source, source_catalog};
    use sparksmith_core::ids::{DefinitionKind, DefinitionRef};
    use std::sync::atomic::{AtomicUsize, Ordering};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const REGISTRY: LibraryId = LibraryId(0x0e1);

    /// A registry of one synthetic component, counting its searches.
    struct Fake {
        bundle: Bundle,
        hit: Hit,
        searches: AtomicUsize,
        signed_in: std::sync::atomic::AtomicBool,
    }

    impl OnlineSource for Fake {
        fn name(&self) -> String {
            "registry.test".into()
        }
        fn search(&self, _text: &str, _limit: usize) -> Result<OnlineReport, String> {
            self.searches.fetch_add(1, Ordering::SeqCst);
            Ok(OnlineReport {
                source: self.name(),
                total: 1,
                results: vec![OnlineHit {
                    hit: self.hit.clone(),
                    id: "sparksmith/fixture".into(),
                    review: "unreviewed".into(),
                }],
                ..OnlineReport::default()
            })
        }
        fn bundle(
            &self,
            library: LibraryId,
            lineage: SourceLineageId,
            _revision: Option<Revision>,
        ) -> Result<Option<Bundle>, String> {
            Ok((library == REGISTRY && lineage == self.bundle.root.lineage)
                .then(|| self.bundle.clone()))
        }
        fn account(&self) -> Result<OnlineAccount, String> {
            let signed_in = self.signed_in.load(Ordering::SeqCst);
            Ok(OnlineAccount {
                signed_in,
                email: signed_in.then(|| "maker@example.com".into()),
                display_name: None,
            })
        }
        fn sign_out(&self) -> Result<SignOut, String> {
            Ok(SignOut {
                was_signed_in: self.signed_in.swap(false, Ordering::SeqCst),
                revoked: true,
                problem: None,
            })
        }
    }

    fn fake() -> std::result::Result<Fake, Box<dyn std::error::Error>> {
        let library = sparksmith_core::catalog::synthetic::project_with_parts(
            1,
            &sparksmith_core::NewBoard::fixture(),
        )?
        .library;
        let component = library
            .latest_components()
            .first()
            .map(|c| (c.id, c.revision))
            .ok_or("no component")?;
        let root = DefinitionRef {
            kind: DefinitionKind::Component,
            id: component.0,
            revision: component.1,
        };
        let fresh: Vec<SourceLineageId> = (0..64).map(|i| SourceLineageId(0xb000 + i)).collect();
        let bundle =
            sparksmith_core::library::import::export(&library, root, REGISTRY, "Registry", &fresh)?
                .bundle;
        let mut entry = source_catalog(REGISTRY, "Registry", &bundle.records)?
            .entries
            .into_iter()
            .find(|e| e.source_lineage == Some(bundle.root.lineage))
            .ok_or("no entry for the root")?;
        entry.source = Source::Online(REGISTRY);
        Ok(Fake {
            bundle,
            hit: Hit {
                entry,
                tier: MatchTier::Filter,
                favourite: false,
            },
            searches: AtomicUsize::new(0),
            signed_in: std::sync::atomic::AtomicBool::new(true),
        })
    }

    fn call(session: &mut Session, command: &str, params: Value) -> Value {
        session.dispatch(json!({"command": command, "params": params}))
    }

    /// The chat's path: `library.search … src:online` lists the registry's
    /// rows after the local ones, and `library.place_from_folder` places
    /// one from its checked bundle in one undo step. Without `src:online`
    /// the registry is not asked.
    #[test]
    fn src_online_searches_the_registry_and_places_from_it() -> TestResult {
        let mut session = Session::new(Engine::demo()?);
        let none = call(
            &mut session,
            "project.query",
            json!({"query": "library.search", "params": {"query": "src:online"}}),
        );
        assert_eq!(none["ok"], true, "{none}");
        assert!(none["result"]["online"]["error"].is_string(), "{none}");
        let fake = Arc::new(fake()?);
        session.set_online(Some(fake.clone()));
        let local = call(
            &mut session,
            "project.query",
            json!({"query": "library.search", "params": {"query": "resistor"}}),
        );
        assert_eq!(local["ok"], true, "{local}");
        assert_eq!(
            fake.searches.load(Ordering::SeqCst),
            0,
            "not asked without src:online"
        );
        let found = call(
            &mut session,
            "project.query",
            json!({"query": "library.search", "params": {"query": "src:online"}}),
        );
        let result = &found["result"]["result"];
        assert_eq!(result["total"], 1, "{found}");
        let entry = &result["results"][0]["entry"];
        assert_eq!(entry["source"]["online"], json!(REGISTRY), "{entry}");
        assert_eq!(
            found["result"]["online"]["entries"][0]["review"],
            "not reviewed"
        );
        let revision = session.engine().revision();
        let placed = call(
            &mut session,
            "library.place_from_folder",
            json!({
                "expected_revision": revision,
                "library": REGISTRY,
                "lineage": entry["source_lineage"],
                "position": [20.0, 20.0],
            }),
        );
        assert_eq!(placed["ok"], true, "{placed}");
        assert_eq!(
            placed["result"]["outcome"]["outcome"], "placed_from",
            "{placed}"
        );
        assert_eq!(session.engine().revision(), revision + 1, "one undo step");
        assert!(
            session
                .engine()
                .project()
                .library
                .origins
                .iter()
                .any(|o| o.origin.library == REGISTRY),
            "the copy records the registry as its origin"
        );
        Ok(())
    }

    /// `library.account` names the registry and who is signed in, without
    /// a registry says none is turned on; `library.sign_out` signs out and
    /// is refused without a registry.
    #[test]
    fn the_account_entries_report_and_sign_out() -> TestResult {
        let mut session = Session::new(Engine::demo()?);
        let none = call(&mut session, "library.account", json!({}));
        assert_eq!(none["ok"], true, "{none}");
        assert_eq!(none["result"]["signed_in"], false, "{none}");
        assert!(none["result"]["registry"].is_null(), "{none}");
        let refused = call(&mut session, "library.sign_out", json!({}));
        assert_eq!(refused["ok"], false, "{refused}");

        let fake = Arc::new(fake()?);
        session.set_online(Some(fake.clone()));
        let status = call(&mut session, "library.account", json!({}));
        let result = &status["result"];
        assert_eq!(result["registry"], "registry.test", "{status}");
        assert_eq!(result["signed_in"], true, "{status}");
        assert_eq!(result["email"], "maker@example.com", "{status}");
        assert!(result["note"].is_null(), "{status}");

        let out = call(&mut session, "library.sign_out", json!({}));
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["result"]["was_signed_in"], true, "{out}");
        assert_eq!(out["result"]["revoked"], true, "{out}");
        assert_eq!(out["result"]["signed_in"], false, "{out}");
        let after = call(&mut session, "library.account", json!({}));
        assert_eq!(after["result"]["signed_in"], false, "{after}");
        assert!(
            after["result"]["note"]
                .as_str()
                .is_some_and(|n| n.contains("Settings › Libraries")),
            "{after}"
        );
        Ok(())
    }

    #[test]
    fn only_an_unquoted_online_source_term_asks_online() {
        assert!(asks_online("usb src:online"));
        assert!(asks_online("SRC:ONLINE pins:4"));
        assert!(!asks_online("usb"));
        assert!(!asks_online("\"src:online\" usb"));
        assert!(!asks_online("src:library"));
    }
}
