//! One background search and one latest request; superseded results never
//! reach the composer or a workflow picker. No filesystem work runs in refresh.
use super::{Candidate, rank, traversal};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub query: String,
    pub revision: u64,
    pub folder: Option<PathBuf>,
    pub known: Vec<PathBuf>,
}

pub struct Matches {
    pub candidates: Vec<Candidate>,
    pub ranked: Vec<Candidate>,
    pub limited: bool,
}

struct Active {
    request: Request,
    cancel: Arc<AtomicBool>,
    receive: Receiver<Option<Matches>>,
}

#[derive(Default)]
pub struct Search {
    wanted: Option<Request>,
    active: Option<Active>,
    ready: Option<Result<Matches, String>>,
}

impl Search {
    pub fn visible(&self) -> bool {
        self.wanted.is_some()
    }

    pub fn clear(&mut self) {
        self.wanted = None;
        self.ready = None;
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn result(&self) -> Option<&Result<Matches, String>> {
        self.ready.as_ref()
    }

    pub fn refresh(&mut self, request: Request, ctx: &egui::Context) -> bool {
        let changed = self.wanted.as_ref() != Some(&request);
        self.refresh_with(request, ctx, |request, cancel| {
            traversal::collect(&request.known, request.folder.as_deref(), cancel)
        });
        changed
    }

    fn refresh_with(
        &mut self,
        request: Request,
        ctx: &egui::Context,
        collect: impl FnOnce(&Request, &AtomicBool) -> Option<traversal::Found> + Send + 'static,
    ) {
        if self.wanted.as_ref() != Some(&request) {
            self.clear();
            self.wanted = Some(request);
        }
        let done = self
            .active
            .as_ref()
            .and_then(|active| match active.receive.try_recv() {
                Ok(found) => Some(found),
                Err(TryRecvError::Disconnected) => Some(None),
                Err(TryRecvError::Empty) => None,
            });
        if let Some(done) = done
            && let Some(active) = self.active.take()
            && !active.cancel.load(Ordering::Relaxed)
            && self.wanted.as_ref() == Some(&active.request)
        {
            self.ready = Some(
                done.ok_or_else(|| "File search stopped; type a path to attach a file".into()),
            );
        }
        // A blocked filesystem operation may take time to return. Keep just
        // the newest request until it does, instead of spawning more walkers.
        if self.active.is_some() || self.ready.is_some() {
            return;
        }
        let Some(request) = self.wanted.clone() else {
            return;
        };
        let (send, receive) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let thread_cancel = cancel.clone();
        let input = request.clone();
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("file-mentions".into())
            .spawn(move || {
                let found = collect(&input, &thread_cancel).and_then(|found| {
                    let ranked = rank(&input.query, &found.candidates)
                        .into_iter()
                        .cloned()
                        .collect();
                    (!thread_cancel.load(Ordering::Relaxed)).then_some(Matches {
                        candidates: found.candidates,
                        ranked,
                        limited: found.limited,
                    })
                });
                let _ = send.send(found);
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.active = Some(Active {
                    request,
                    cancel,
                    receive,
                })
            }
            Err(error) => self.ready = Some(Err(format!("Could not search files: {error}"))),
        }
    }
}

impl Drop for Search {
    fn drop(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests;
