//! The document a deferred editor action was requested against.
use sparksmith_core::Engine;

#[derive(Debug)]
pub struct DocumentVersion {
    opening: u64,
    revision: u64,
}

impl DocumentVersion {
    pub fn of(engine: &Engine) -> Self {
        Self {
            opening: engine.opening(),
            revision: engine.revision(),
        }
    }

    /// An edit, undo or reopen invalidates the request; fill copper stored
    /// meanwhile does not. The opening tells another project at the same
    /// revision apart.
    pub fn is_current(&self, engine: &Engine) -> bool {
        self.opening == engine.opening() && engine.still_at(self.revision)
    }
}
