//! The online parts registry as a library source of the desktop
//! (registry R0 §10; Settings › Libraries turns it on). Searches and
//! bundle reads run on the registry worker's threads
//! (`sparksmith_assist::registry`): the palette asks with a generation and
//! only the newest reply ever comes back. A registry part is placed like a
//! folder's: its bundle, checked against the registry's content hashes (and
//! mirrored), goes to the same `library.place_from` as one undo step.
use crate::library_sources::{BundlePurpose, FolderPick, ReadBundle};
use sparksmith_assist::registry::{DEBOUNCE, RegistryClient, RegistryEvent, RegistryWorker};
use sparksmith_automation::session::online::OnlineReport;
use sparksmith_core::library::bundle::SourceRevision;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

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

/// One configured, turned-on registry.
pub struct OnlineLibrary {
    worker: RegistryWorker,
    tickets: BTreeMap<u64, (FolderPick, BundlePurpose)>,
    next: u64,
    status: OnlineStatus,
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
        Ok(Self {
            worker,
            tickets: BTreeMap::new(),
            next: 0,
            status: OnlineStatus::Syncing,
        })
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
    pub fn bundle(&mut self, pick: FolderPick, purpose: BundlePurpose) {
        self.next = self.next.wrapping_add(1);
        let root = SourceRevision {
            lineage: pick.lineage,
            revision: pick.revision,
        };
        self.worker.bundle(self.next, pick.library, root);
        self.tickets.insert(self.next, (pick, purpose));
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
                    if let Some((pick, purpose)) = self.tickets.remove(&ticket) {
                        let name = pick.name.clone();
                        bundles.push(ReadBundle {
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
            }
        }
        (searches, bundles)
    }
}
