//! Cancelling a call, for whoever asked for it: the person's Stop, an MCP
//! client's `notifications/cancelled` or its closed connection.
//!
//! Work that can take long has no time limit of its own; it looks at the
//! [`Cancel`] of the call it runs for. A host enters it on the thread the
//! call runs on ([`Cancel::scope`]), so the layers between need not hand it
//! down; a thread started for the call enters it again.
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One call's cancellation; clones are the same one.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

thread_local! {
    static CURRENT: RefCell<Option<Cancel>> = const { RefCell::new(None) };
}

impl Cancel {
    /// Cancel the call; a second time changes nothing.
    pub fn set(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Run `f` as the call this cancels.
    pub fn scope<R>(&self, f: impl FnOnce() -> R) -> R {
        struct Leave(Option<Cancel>);
        impl Drop for Leave {
            fn drop(&mut self) {
                let outer = self.0.take();
                CURRENT.with(|current| *current.borrow_mut() = outer);
            }
        }
        let _leave = Leave(CURRENT.with(|current| current.borrow_mut().replace(self.clone())));
        f()
    }
}

/// The cancellation of the call this thread runs for.
pub fn current() -> Option<Cancel> {
    CURRENT.with(|current| current.borrow().clone())
}

/// Whether the call this thread runs for was cancelled.
pub fn requested() -> bool {
    current().is_some_and(|cancel| cancel.is_set())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_is_one_threads_and_ends() {
        let outer = Cancel::default();
        let inner = Cancel::default();
        outer.scope(|| {
            inner.scope(|| {
                inner.set();
                assert!(requested());
                let elsewhere = std::thread::spawn(requested).join();
                assert_eq!(elsewhere.ok(), Some(false));
            });
            assert!(!requested());
        });
        assert!(current().is_none());
    }
}
