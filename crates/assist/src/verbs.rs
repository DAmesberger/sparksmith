//! The built-in agent on the surface (extension ABI S6): a run's tools are
//! the six verbs, `find`, `describe`, `query`, `apply`, `act` and `show`,
//! with the definitions MCP's `tools/list` serves, built by the same
//! function ([`tools`]). Every call runs the same verb code MCP runs
//! (`sparksmith_automation::mcp::tools::dispatch`), here on the run's own
//! session in process: each becomes one [`Call`] through the front door as
//! [`Principal::LocalAgent`] under the run's grant. No MCP framing, client
//! or socket sits between the agent and the engine. Nobody can be asked
//! during a run, so what needs a person is refused (`gate.needs_person`).
//!
//! The run's reach profile ([`crate::profile::ReachProfile`]) narrows the
//! catalog: it is the [`Grant`] the front door's gate holds every call to,
//! its modules and what outside the document an entry may touch (an entry
//! that touches files, settings, … which it does not list is refused with
//! `gate.reach`), and `find` lists only what the grant reaches. A path in a
//! call is the gate's too: it makes the call Ask, which no module of the
//! datasheet profile reaches, and a run has no folders.
use crate::model::ToolSpec;
use serde_json::{Value, json};
use sparksmith_automation::front::{self, Call, CallCx, NoOne, Surface};
use sparksmith_automation::mcp::desk::Calls;
use sparksmith_automation::mcp::tools as mcp;
use sparksmith_automation::session::metadata::{self, EntryMeta};
use sparksmith_automation::{Grant, Principal, Session};

pub use mcp::{ACT, APPLY, DESCRIBE, FIND, NAMES, QUERY, SHOW};

/// The six verbs as a run offers them: MCP's `tools/list` tools, name,
/// description and input schema unchanged, in their fixed order (so the
/// prompt prefix caches).
pub fn tools() -> Vec<ToolSpec> {
    mcp::tools()
        .into_iter()
        .map(|tool| ToolSpec {
            name: tool.name.into_owned(),
            description: tool.description.map(|d| d.into_owned()).unwrap_or_default(),
            input_schema: Value::Object((*tool.input_schema).clone()),
        })
        .collect()
}

/// A run's way to its session: the verbs' calls as the built-in agent.
pub struct Agent<'a> {
    pub session: &'a mut Session,
    pub grant: &'a Grant,
    /// The open turn every call joins (a workflow run's), when there is one.
    pub turn: Option<&'a str>,
}

impl Agent<'_> {
    /// Runs the verb `name` with `input`, answering in the transport
    /// envelope; a name that is no verb is refused, naming the verbs.
    pub fn verb(&mut self, name: &str, input: Value) -> Value {
        mcp::dispatch(self, name, input).unwrap_or_else(|| {
            failure(
                "tool.unknown",
                format!(
                    "there is no tool {}: the tools are {}",
                    name.chars().take(64).collect::<String>(),
                    NAMES.join(", ")
                ),
            )
        })
    }

    /// One call through the front door as the built-in agent, in the run's
    /// turn when it has one.
    pub fn run(&mut self, mut call: Call) -> Value {
        if call.turn.is_none() {
            call.turn = self.turn.map(str::to_owned);
        }
        let cx = CallCx {
            who: Principal::LocalAgent { grant: self.grant },
            asker: &mut NoOne,
            verbose: false,
        };
        front::envelope(self.session.call(cx, call))
    }
}

impl Calls for Agent<'_> {
    fn call(&mut self, _project: Option<&str>, call: Call) -> Value {
        // One project: the run's own session.
        self.run(call)
    }

    fn catalog(&mut self, call: Call) -> Value {
        self.run(call)
    }

    fn open(&mut self, mut call: Call) -> Value {
        // A new or opened project replaces the run's own.
        call.expected_revision
            .get_or_insert(self.session.engine().revision());
        self.run(call)
    }

    fn is_live(&self) -> bool {
        false
    }

    fn reaches(&self, entry: &EntryMeta) -> bool {
        self.grant.reaches(metadata::declared_reach(&entry.name))
            && self
                .grant
                .modules
                .get(entry.module)
                .is_some_and(|ceiling| entry.class <= *ceiling)
    }
}

/// The transport envelope of a failure with `code`.
pub fn failure(code: &str, message: String) -> Value {
    json!({"ok": false, "error": {"code": code, "message": message}})
}
