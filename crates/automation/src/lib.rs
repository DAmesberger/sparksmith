//! Headless automation over the one Sparksmith command engine.
//!
//! [`Session`] owns an [`sparksmith_core::Engine`] plus the file effects
//! automation needs (open, save, native export). Every transport reaches it
//! through the front door ([`front::Surface::call`]): the CLI's JSON-lines
//! transport and the MCP servers are thin adapters over it, so every caller
//! reaches the same typed operations, the same gate and the same error.
pub mod convert_host;
pub mod front;
pub mod gate;
pub mod kicad_import;
pub mod lines;
pub mod live;
pub mod mcp;
pub mod review;
pub mod session;
pub mod workflow;

pub use front::{Answer, Asker, Call, CallCx, Surface};
pub use gate::{Caller, Decision, Grant, Limits, Mode, Principal, Question};
pub use session::{Request, Session, part_files};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Document(#[from] sparksmith_core::Error),
    #[error(transparent)]
    File(#[from] sparksmith_io::Error),
    #[error(transparent)]
    Native(#[from] sparksmith_solidcraft::Error),
    #[error("invalid request: {0}")]
    Request(#[from] serde_json::Error),
    /// A call the front door refused or that failed, in the one error shape.
    #[error("{}", .0.message)]
    Refused(Box<front::Error>),
}

impl Error {
    /// `error` from the front door.
    pub fn refused(error: Box<front::Error>) -> Self {
        Self::Refused(error)
    }

    pub fn diagnostic(&self) -> sparksmith_core::Diagnostic {
        match self {
            Self::Document(error) => error.diagnostic(),
            Self::File(error) => error.diagnostic(),
            Self::Native(error) => sparksmith_core::Diagnostic {
                code: error.code().into(),
                rule: None,
                message: error.to_string(),
                expected_revision: None,
                actual_revision: None,
            },
            Self::Request(_) => sparksmith_core::Diagnostic {
                code: "malformed_request".into(),
                rule: None,
                message: self.to_string(),
                expected_revision: None,
                actual_revision: None,
            },
            Self::Refused(error) => sparksmith_core::Diagnostic {
                code: error.code.clone(),
                rule: error.rule.clone(),
                message: error.message.clone(),
                expected_revision: error.expected_revision,
                actual_revision: error.actual_revision,
            },
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
