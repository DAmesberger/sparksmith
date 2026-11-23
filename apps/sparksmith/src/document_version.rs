//! The document a deferred editor action was requested against.
use sparksmith_core::{Engine, model::Project};
use std::sync::{Arc, Weak};

#[derive(Debug)]
pub struct DocumentVersion {
    project: Weak<Project>,
    revision: u64,
}

impl DocumentVersion {
    pub fn of(engine: &Engine) -> Self {
        Self {
            project: Arc::downgrade(engine.project_arc()),
            revision: engine.revision(),
        }
    }

    /// An edit, undo or reopen invalidates the request. Holding the weak
    /// reference keeps the allocation identity distinct even if the user
    /// closes the project and another engine has the same revision.
    pub fn is_current(&self, engine: &Engine) -> bool {
        self.revision == engine.revision()
            && self.project.as_ptr() == Arc::as_ptr(engine.project_arc())
    }
}
