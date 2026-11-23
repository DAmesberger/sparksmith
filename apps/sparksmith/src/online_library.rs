//! The online parts registry as a library source of the desktop
//! (registry R0 §10; Settings › Libraries turns it on). Searches and
//! bundle reads run on the registry worker's threads
//! (`sparksmith_assist::registry`): the palette asks with a generation and
//! only the newest reply ever comes back. A registry part is placed like a
//! folder's: its bundle, checked against the registry's content hashes (and
//! mirrored), goes to the same `library.place_from` as one undo step.
//!
//! Downloading part data needs a free account: Settings › Libraries signs
//! in with a code the person confirms in the browser, polled on the
//! worker's own thread, and signs out ([`OnlineLibrary::sign_in`]).
use crate::document_version::DocumentVersion;
use crate::library_sources::{BundlePurpose, FolderPick, ReadBundle};
use sparksmith_assist::registry::account::{self, DeviceCode, Known};
use sparksmith_assist::registry::{
    AccountEvent, DEBOUNCE, RegistryClient, RegistryEvent, RegistryWorker,
};
use sparksmith_automation::session::online::OnlineReport;
use sparksmith_core::library::bundle::SourceRevision;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Open an `http(s)` page in the system browser, without waiting for it;
/// whether it was handed over. Anything else is refused.
pub fn open_in_browser(url: &str) -> bool {
    let plain = url.chars().all(|c| c.is_ascii_graphic());
    if !plain || !(url.starts_with("https://") || url.starts_with("http://")) {
        return false;
    }
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("explorer")
    } else {
        std::process::Command::new("xdg-open")
    };
    // Spawned, never waited on: a slow browser cannot block the UI.
    command.arg(url).spawn().is_ok()
}

/// Rows one palette search asks the registry for.
pub const ONLINE_ROWS: usize = 30;

/// A finished palette search.
#[derive(Clone, Debug)]
pub struct OnlineSearch {
    pub generation: u64,
    pub text: String,
    pub result: Result<OnlineReport, String>,
}

/// How the registry's index stands.
#[derive(Clone, Debug, PartialEq)]
pub enum OnlineStatus {
    Syncing,
    /// Searchable; `offline` names the mirror's index date.
    Ready {
        entries: usize,
        offline: Option<String>,
    },
    Failed(String),
}

/// The sign-in as Settings shows it.
#[derive(Clone, Debug)]
pub enum AccountView {
    /// The kept sign-in is being read.
    Reading,
    SignedOut,
    /// Asking the registry for a code.
    Requesting,
    /// Waiting for the person to confirm `code` in the browser.
    Code(DeviceCode),
    /// Signed in; the account's name or email once known.
    SignedIn(Option<String>),
    SigningOut,
}

/// A sign-in button pressed in Settings › Libraries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountAction {
    SignIn,
    Cancel,
    SignOut,
    /// Open the sign-in page with the code filled in.
    OpenBrowser(String),
    /// Copy the sign-in code.
    Copy(String),
}

/// A sign-in under way.
struct Attempt {
    number: u64,
    cancel: Arc<AtomicBool>,
    code: Option<DeviceCode>,
}

impl Drop for OnlineLibrary {
    /// A registry turned off or changed stops polling its sign-in.
    fn drop(&mut self) {
        self.cancel_sign_in();
    }
}

/// One configured, turned-on registry.
pub struct OnlineLibrary {
    worker: RegistryWorker,
    tickets: BTreeMap<u64, (FolderPick, BundlePurpose, DocumentVersion)>,
    next: u64,
    status: OnlineStatus,
    attempt: Option<Attempt>,
    attempts: u64,
    signing_out: bool,
    /// What the last sign-in or sign-out said, and whether it is a problem.
    account_note: Option<(String, bool)>,
}

impl OnlineLibrary {
    /// Start the registry at `url`, mirrored under `cache`, and sync its
    /// index in the background. `repaint` asks for a frame when something
    /// arrives.
    pub fn start(
        url: &str,
        cache: Option<&Path>,
        repaint: Option<egui::Context>,
    ) -> Result<Self, String> {
        let client = Arc::new(RegistryClient::new(url, cache)?);
        let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            if let Some(ctx) = &repaint {
                ctx.request_repaint();
            }
        });
        let worker = RegistryWorker::start(client, DEBOUNCE, notify);
        worker.sync();
        worker.read_account();
        Ok(Self {
            worker,
            tickets: BTreeMap::new(),
            next: 0,
            status: OnlineStatus::Syncing,
            attempt: None,
            attempts: 0,
            signing_out: false,
            account_note: None,
        })
    }

    /// The sign-in as Settings shows it.
    pub fn account(&self) -> AccountView {
        if let Some(attempt) = &self.attempt {
            return match &attempt.code {
                Some(code) => AccountView::Code(code.clone()),
                None => AccountView::Requesting,
            };
        }
        if self.signing_out {
            return AccountView::SigningOut;
        }
        match self.worker.client().known() {
            None => AccountView::Reading,
            Some(Known::SignedOut) => AccountView::SignedOut,
            Some(Known::SignedIn(account)) => AccountView::SignedIn(
                account
                    .map(|a| a.label().to_owned())
                    .filter(|l| !l.trim().is_empty()),
            ),
        }
    }

    /// What the last sign-in or sign-out said, and whether it is a problem.
    pub fn account_note(&self) -> Option<&(String, bool)> {
        self.account_note.as_ref()
    }

    /// Whether this registry may be signed in to (HTTPS, or this computer).
    pub fn may_sign_in(&self) -> bool {
        self.worker.client().may_sign_in()
    }

    /// Start signing in: a code comes back to show, and the registry is
    /// polled until the person confirms it in the browser.
    pub fn sign_in(&mut self) {
        if let Some(old) = self.attempt.take() {
            old.cancel.store(true, Ordering::SeqCst);
        }
        self.attempts = self.attempts.wrapping_add(1);
        let cancel = Arc::new(AtomicBool::new(false));
        self.worker
            .sign_in(self.attempts, account::client_name(), cancel.clone());
        self.attempt = Some(Attempt {
            number: self.attempts,
            cancel,
            code: None,
        });
        self.account_note = None;
    }

    /// Stop waiting for the sign-in.
    pub fn cancel_sign_in(&mut self) {
        if let Some(attempt) = self.attempt.take() {
            attempt.cancel.store(true, Ordering::SeqCst);
        }
        self.account_note = None;
    }

    /// Sign out: the registry revokes the token and the keychain forgets it.
    pub fn sign_out(&mut self) {
        self.cancel_sign_in();
        self.signing_out = true;
        self.worker.sign_out();
    }

    fn account_event(&mut self, event: AccountEvent) {
        let current = |attempt: u64, held: &Option<Attempt>| {
            held.as_ref().is_some_and(|a| a.number == attempt)
        };
        match event {
            AccountEvent::Read(Ok(())) => {}
            AccountEvent::Read(Err(error)) => self.account_note = Some((error, true)),
            AccountEvent::Code { attempt, code } => {
                if current(attempt, &self.attempt)
                    && let Some(held) = &mut self.attempt
                {
                    held.code = Some(code);
                }
            }
            AccountEvent::SignedIn { attempt, result } => {
                if !current(attempt, &self.attempt) {
                    return;
                }
                self.attempt = None;
                self.account_note = match result {
                    Ok(_) => None,
                    Err(error) => Some((error, true)),
                };
            }
            AccountEvent::SignedOut(result) => {
                self.signing_out = false;
                self.account_note = match result {
                    Ok(out) => out.problem.map(|problem| {
                        (
                            format!(
                                "Signed out on this computer. The online library could not be \
                                 told ({problem}); revoke the sign-in on your account page."
                            ),
                            false,
                        )
                    }),
                    Err(error) => Some((error, true)),
                };
            }
        }
    }

    /// The registry's display name: its host.
    pub fn name(&self) -> String {
        self.worker.client().host().to_owned()
    }

    pub fn status(&self) -> &OnlineStatus {
        &self.status
    }

    /// The blocking client, for work already off the UI thread (previews,
    /// the chat's session).
    pub fn client(&self) -> Arc<RegistryClient> {
        self.worker.client().clone()
    }

    /// Search `text` as `generation`; older replies are dropped.
    pub fn search(&self, generation: u64, text: &str) {
        self.worker.search(generation, text, ONLINE_ROWS);
    }

    /// Drop every search asked up to `generation`.
    pub fn cancel(&self, generation: u64) {
        self.worker.cancel(generation);
    }

    /// Fetch a registry root's bundle for `purpose`; it comes back from
    /// [`Self::receive`] as a [`ReadBundle`].
    pub fn bundle(&mut self, pick: FolderPick, purpose: BundlePurpose, version: DocumentVersion) {
        self.next = self.next.wrapping_add(1);
        let root = SourceRevision {
            lineage: pick.lineage,
            revision: pick.revision,
        };
        self.worker.bundle(self.next, pick.library, root);
        self.tickets.insert(self.next, (pick, purpose, version));
    }

    /// What arrived: the newest palette search and finished bundle reads.
    pub fn receive(&mut self) -> (Vec<OnlineSearch>, Vec<ReadBundle>) {
        let mut searches = Vec::new();
        let mut bundles = Vec::new();
        for event in self.worker.receive() {
            match event {
                RegistryEvent::Search {
                    generation,
                    text,
                    result,
                    ..
                } => searches.push(OnlineSearch {
                    generation,
                    text,
                    result,
                }),
                RegistryEvent::Bundle { ticket, result, .. } => {
                    if let Some((pick, purpose, version)) = self.tickets.remove(&ticket) {
                        let name = pick.name.clone();
                        bundles.push(ReadBundle {
                            version,
                            pick,
                            purpose,
                            result: result.map_err(|e| format!("Fetching {name} failed: {e}")),
                        });
                    }
                }
                RegistryEvent::Synced(result) => {
                    self.status = match result {
                        Ok(synced) => OnlineStatus::Ready {
                            entries: synced.shards.iter().map(|s| s.catalog.entries.len()).sum(),
                            offline: synced
                                .offline
                                .then(|| sparksmith_assist::registry::index_date(synced.fetched)),
                        },
                        Err(error) => OnlineStatus::Failed(error),
                    };
                }
                RegistryEvent::Account(event) => self.account_event(event),
            }
        }
        (searches, bundles)
    }
}
