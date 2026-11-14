//! The six tools called synchronously, for tests that check what a model
//! reaches through them without a protocol client: the same
//! [`super::tools::call`] the server answers `tools/call` with.
use super::{Server, tools};
use crate::gate::Grant;
use crate::session::Session;
use serde_json::{Value, json};

pub(crate) type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// A server with its own runtime.
pub(crate) struct Tools {
    runtime: tokio::runtime::Runtime,
    server: Server,
}

impl Tools {
    /// `session` as the only project, gated under `grant`.
    pub(crate) fn new(session: Session, grant: Grant) -> Fallible<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            server: Server::new(session, grant)?,
        })
    }

    /// The window `client` reaches, as `mcp --live` serves it.
    pub(crate) fn live(client: crate::live::Client) -> Fallible<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            server: Server::live(client)?,
        })
    }

    /// Calls `tool` with `arguments`: whether it failed, and its JSON.
    pub(crate) fn call(&self, tool: &str, arguments: Value) -> (bool, Value) {
        let arguments = match arguments {
            Value::Object(arguments) => Some(arguments),
            _ => None,
        };
        let desk = self.server.desk().clone();
        let result = self
            .runtime
            .block_on(async move { tools::call(&desk, tool, arguments).await });
        match result {
            Ok(result) => {
                let payload = result
                    .content
                    .first()
                    .and_then(|c| c.as_text())
                    .and_then(|t| serde_json::from_str(&t.text).ok())
                    .unwrap_or(Value::Null);
                (result.is_error == Some(true), payload)
            }
            Err(error) => (true, json!({"protocol": error.message})),
        }
    }

    /// A call that must succeed.
    pub(crate) fn ok(&self, tool: &str, arguments: Value) -> Fallible<Value> {
        match self.call(tool, arguments.clone()) {
            (false, payload) => Ok(payload),
            (true, payload) => Err(format!("{tool} {arguments}: {payload}").into()),
        }
    }

    /// `query {name, params}`.
    pub(crate) fn query(&self, name: &str, params: Value) -> (bool, Value) {
        self.call(tools::QUERY, json!({"name": name, "params": params}))
    }

    /// `act {name, params}` at the project's revision.
    pub(crate) fn act(&self, name: &str, params: Value) -> (bool, Value) {
        let revision = self.revision();
        self.call(
            tools::ACT,
            json!({"name": name, "params": params, "expected_revision": revision}),
        )
    }

    /// `apply` of one operation at the project's revision.
    pub(crate) fn apply(&self, op: &str, params: Value) -> (bool, Value) {
        let revision = self.revision();
        self.call(
            tools::APPLY,
            json!({"expected_revision": revision, "operations": [{"op": op, "params": params}]}),
        )
    }

    /// The first project's session, read on the desk.
    pub(crate) fn with_session<R: Send + 'static>(
        &self,
        read: impl FnOnce(&Session) -> R + Send + 'static,
    ) -> Option<R> {
        let desk = self.server.desk().clone();
        self.runtime
            .block_on(async move {
                desk.run(move |d| {
                    let handle = d.project(None).ok()?;
                    d.session(&handle).map(read)
                })
                .await
            })
            .ok()
            .flatten()
    }

    /// The first project's revision.
    pub(crate) fn revision(&self) -> u64 {
        self.with_session(|s| s.engine().revision()).unwrap_or(0)
    }
}
