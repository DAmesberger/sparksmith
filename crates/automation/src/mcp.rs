//! The MCP server of [MCP surface](../../../docs/mcp-surface.md), on the
//! official Rust SDK (`rmcp`), at protocol `2026-07-28` only. `sparksmith
//! mcp` serves it over headless projects and `sparksmith mcp --live` over the
//! project open in the desktop window; both carry the catalog through six
//! verbs ([`tools`]) into the front door
//! ([`crate::front::Surface::call`]).
//!
//! - **One version.** There is no handshake: every request carries its
//!   `_meta` (`protocolVersion`, `clientCapabilities`), checked by rmcp, and a
//!   request without it is refused with `-32602`. A request naming another
//!   version, and `initialize` at any point, is answered with
//!   `UnsupportedProtocolVersionError` (`-32022`) listing `2026-07-28`, and
//!   the connection stays open for a client that then tries again
//!   ([`wire`]).
//! - **`server/discover`** answers the version, the capabilities, the
//!   instructions and `serverInfo`, cached `public` for a day. Every other
//!   result carries `serverInfo` in its `_meta` too, and a `resultType`.
//! - **Lists** are fixed per build: deterministic, unpaginated (a cursor is
//!   refused with `-32602`) and cached `public` for a day. The tools are the
//!   six verbs of [`tools`] (`find`, `describe`, `query`, `apply`, `act`,
//!   `show`); the catalog's entries are data behind them ([`catalog`]). The
//!   prompts are the host's workflows, generated from their data when the
//!   server starts ([`prompts`]). Help resource reads and template completion use the same gated queries.
//! - **Errors** use the spec's codes: `-32602` for an unknown tool or bad
//!   parameters, `-32601` for a method not served, `-32603` when the session
//!   thread is gone, `-32022` for another version. A failure of the call
//!   itself (a refusal of the gate, a stale revision, a diagnostic) is a
//!   tool result with `isError` ([`outcome`]), so the model reads its code.
//! - **The projects** sit behind a thread of their own ([`desk`]): headless
//!   sessions, each behind an opaque handle, whose every call passes the gate
//!   as a remote agent under the server's [`Grant`]; or, live, the window,
//!   whose own gate decides and asks the person.
//!
//! - **The record** of a run is opt-in: [`wirelog`] writes every line read
//!   and written on the server's own pipes, byte for byte, and [`replay`]
//!   turns such a log into the JSON-lines requests that rebuild the project
//!   without a model.
//!
//! Requests are served concurrently, so replies can come back in another
//! order than their requests; the desk runs them one at a time, and an edit
//! names the revision it was made against, so a reordered edit is refused
//! rather than misapplied.
pub mod catalog;
pub mod desk;
pub mod fields;
mod help;
pub mod lenient;
pub mod prompts;
pub mod replay;
mod tap;
pub mod tools;
pub mod trace;
pub mod wire;
pub mod wirelog;

use crate::gate::Grant;
use crate::live;
use crate::session::Session;
use crate::workflow::{Loaded, load};
use desk::Desk;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CompleteRequestParams,
    CompleteResult, ContentBlock, DiscoverResult, GetPromptRequestParams, GetPromptResponse,
    Implementation, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequestParams,
    ReadResourceResponse, Resource, ResourceTemplate, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, ServerInitializeError};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;
use std::borrow::Cow;
use tokio::io::{AsyncRead, AsyncWrite};

/// The one protocol revision served.
pub const VERSIONS: &[ProtocolVersion] = &[ProtocolVersion::V_2026_07_28];
/// How long discovery and the lists stay fresh: they are fixed per build.
pub const LIST_TTL_MS: u64 = 86_400_000;
/// The name in `serverInfo`.
pub const NAME: &str = sparksmith_core::product::SHORT_NAME;
/// Large enough for a complete component draft, small enough to bound memory.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// Claude Code cuts server instructions beyond this many characters.
pub const MAX_INSTRUCTIONS: usize = 4096;
/// The instructions' target in bytes (`docs/mcp-surface.md` §3.1): the
/// draft there is 1,030 characters and 238 Qwen3.8 tokens (4.3 bytes per
/// token), so this holds them near 255 tokens. A live server adds
/// [`LIVE_TARGET`] for the window.
pub const INSTRUCTIONS_TARGET: usize = 1100;
pub const LIVE_TARGET: usize = 400;

/// Read assistant guidance from the selected library.
fn method_text(
    shipped: &sparksmith_io::shipped::ShippedLibrary,
    name: &str,
) -> Result<String, String> {
    use sparksmith_io::shipped::Shelf;
    let path = shipped.shelf(Shelf::Assist).join(name);
    let bytes = shipped
        .read(Shelf::Assist, name, (4 * MAX_INSTRUCTIONS) as u64)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8(bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Err(format!("{} is empty", path.display()));
    }
    Ok(text
        .trim_end()
        .replace("{product}", sparksmith_core::product_name!()))
}

/// The same method opens the chat's guidance and MCP discovery.
pub fn agent_method(shipped: &sparksmith_io::shipped::ShippedLibrary) -> Result<String, String> {
    method_text(shipped, "method.md")
}

/// Instructions for the headless server or the open window.
pub fn instructions(live: bool) -> Result<String, String> {
    let shipped = sparksmith_io::shipped::ShippedLibrary::locate().map_err(|e| e.to_string())?;
    instructions_for(&shipped, live)
}

fn instructions_for(
    shipped: &sparksmith_io::shipped::ShippedLibrary,
    live: bool,
) -> Result<String, String> {
    let mut text = agent_method(shipped)?;
    if live {
        text.push_str("\n\n");
        text.push_str(&method_text(shipped, "mcp-live.md")?);
    }
    if text.chars().count() > MAX_INSTRUCTIONS {
        return Err(format!(
            "{}: MCP instructions exceed {MAX_INSTRUCTIONS} characters",
            shipped
                .shelf(sparksmith_io::shipped::Shelf::Assist)
                .display()
        ));
    }
    Ok(text)
}

/// The `2026-07-28` server, over headless projects or the open window.
pub struct Server {
    desk: desk::Handle,
    bench: Option<(tap::Tap, crate::session::SchemaDetail)>,
    /// How it was started, for a wire log's first row.
    started: wirelog::Started,
    wire: Option<wirelog::WireLog>,
    instructions: String,
    /// The workflows its prompts are made from.
    workflows: Vec<Loaded>,
}

impl Server {
    /// Starts the session thread with `session` as the first project,
    /// gated by `policy`; projects opened later start from
    /// [`Session::lending`] and open or create their document.
    pub fn new(session: Session, policy: Grant) -> Result<Self, String> {
        Self::with_new_projects(session, policy, Box::new(Session::lending))
    }

    /// [`Self::new`], with the sessions later projects start from made by
    /// `new_session` (one with the host's library sources, say).
    pub fn with_new_projects(
        session: Session,
        policy: Grant,
        new_session: desk::NewSession,
    ) -> Result<Self, String> {
        let started = wirelog::Started {
            live: false,
            mode: Some(policy.mode.name().to_owned()),
            folders: policy
                .folders
                .iter()
                .map(|folder| folder.display().to_string())
                .collect(),
            project: session.path().map(|path| {
                std::path::absolute(path)
                    .unwrap_or_else(|_| path.to_path_buf())
                    .display()
                    .to_string()
            }),
        };
        Ok(Self {
            instructions: instructions(false)?,
            bench: None,
            started,
            wire: None,
            desk: Desk::start(session, policy, new_session)?,
            workflows: load::available(&load::host()),
        })
    }

    /// A server whose calls run in the open window `client` reaches, whose
    /// own gate decides and asks the person.
    pub fn live(client: live::Client) -> Result<Self, String> {
        Ok(Self {
            instructions: instructions(true)?,
            bench: None,
            started: wirelog::Started {
                live: true,
                ..wirelog::Started::default()
            },
            wire: None,
            desk: Desk::start_live(client)?,
            workflows: load::available(&load::host()),
        })
    }

    /// Capture this headless run's tool-call traffic into a new trace.
    pub fn with_bench_trace(
        mut self,
        path: &std::path::Path,
        detail: crate::session::SchemaDetail,
    ) -> std::io::Result<Self> {
        self.bench = Some((tap::Tap::new(std::fs::File::create_new(path)?), detail));
        Ok(self)
    }

    /// Record every line read from the client and written to it in a new
    /// wire log at `path` ([`wirelog`]); an existing file is refused.
    pub fn with_wire_log(mut self, path: &std::path::Path) -> std::io::Result<Self> {
        self.wire = Some(wirelog::WireLog::create(path, &self.started)?);
        Ok(self)
    }

    /// This server with its prompts made from `workflows` instead of the
    /// host's.
    #[must_use]
    pub fn with_workflows(mut self, workflows: Vec<Loaded>) -> Self {
        self.workflows = workflows;
        self
    }

    /// The workflows its prompts are made from.
    pub fn workflows(&self) -> &[Loaded] {
        &self.workflows
    }

    /// The session thread, for the tools built on it.
    pub fn desk(&self) -> &desk::Handle {
        &self.desk
    }

    /// Who answers: the `serverInfo` of discovery and of every result.
    pub fn implementation() -> Implementation {
        Implementation::new(NAME, env!("CARGO_PKG_VERSION"))
            .with_title(sparksmith_core::product::NAME)
    }

    /// Serves one client until `read` closes, or the client stops before
    /// sending anything but discovery.
    pub async fn serve<R, W>(self, read: R, write: W) -> Result<(), String>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        use wirelog::Direction::{In, Out};
        // The wire log sits on the pipes themselves, under the bench tap.
        let wire = self.wire.clone();
        let bench = self.bench.as_ref().map(|(tap, _)| tap.clone());
        match (wire, bench) {
            (None, None) => self.serve_pipes(read, write).await,
            (Some(wire), None) => {
                let (read, write) = (wire.stream(read, In), wire.stream(write, Out));
                self.serve_pipes(read, write).await
            }
            (None, Some(tap)) => {
                let (read, write) = (tap.stream(read), tap.stream(write));
                self.serve_pipes(read, write).await
            }
            (Some(wire), Some(tap)) => {
                let read = tap.stream(wire.stream(read, In));
                let write = tap.stream(wire.stream(write, Out));
                self.serve_pipes(read, write).await
            }
        }
    }

    async fn serve_pipes<R, W>(self, read: R, write: W) -> Result<(), String>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let transport = wire::ModernOnly::new(
            AsyncRwTransport::new_server(read, write).with_max_line_length(MAX_LINE),
        );
        let running = match ServiceExt::serve(self, transport).await {
            Ok(running) => running,
            Err(ServerInitializeError::ConnectionClosed(_)) => return Ok(()),
            Err(error) => return Err(format!("MCP: {error}")),
        };
        running
            .waiting()
            .await
            .map(|_| ())
            .map_err(|e| format!("MCP: {e}"))
    }

    /// [`Self::serve`] on the process's stdin and stdout, on a runtime of
    /// its own.
    pub fn serve_stdio(self) -> Result<(), String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("cannot start the MCP runtime: {e}"))?;
        let (read, write) = rmcp::transport::stdio();
        runtime.block_on(self.serve(read, write))
    }
}

/// The tools, in their fixed order: `find`, `describe`, `query`, `apply`,
/// `act` and `show`.
pub fn tools() -> Vec<Tool> {
    tools::tools()
}

/// A list request may not ask for a page: every list is one page.
fn one_page(request: Option<&PaginatedRequestParams>) -> Result<(), ErrorData> {
    match request.and_then(|r| r.cursor.as_ref()) {
        None => Ok(()),
        Some(cursor) => Err(ErrorData::invalid_params(
            format!("invalid cursor {cursor}: every list is one page"),
            None,
        )),
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts()
            .enable_resources()
            .enable_completions()
            .build();
        if let Some(tools) = capabilities.tools.as_mut() {
            tools.list_changed = Some(false);
        }
        if let Some(prompts) = capabilities.prompts.as_mut() {
            prompts.list_changed = Some(false);
        }
        let mut info = ServerConfig::new(capabilities).with_instructions(self.instructions.clone());
        info.protocol_version = ProtocolVersion::V_2026_07_28;
        info.server_info = Self::implementation();
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(VERSIONS)
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, ErrorData> {
        let mut result = DiscoverResult::from_server_info(VERSIONS.to_vec(), self.get_info());
        result.ttl_ms = LIST_TTL_MS;
        result.cache_scope = CacheScope::Public;
        Ok(result)
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        one_page(request.as_ref())?;
        Ok(ListToolsResult::with_all_items(tools())
            .with_ttl_ms(LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        tools().into_iter().find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let mut args = request.arguments;
        if let Some((_, detail)) = &self.bench
            && let Some(fields) = args.as_mut()
        {
            tools::set_default_detail(&request.name, fields, *detail);
        }
        tools::call(&self.desk, &request.name, args)
            .await
            .map(CallToolResponse::from)
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        one_page(request.as_ref())?;
        Ok(
            ListPromptsResult::with_all_items(prompts::list(&self.workflows))
                .with_ttl_ms(LIST_TTL_MS)
                .with_cache_scope(CacheScope::Public),
        )
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        prompts::get(&self.workflows, &request.name, request.arguments.as_ref())
            .map(GetPromptResponse::from)
            .map_err(|why| ErrorData::invalid_params(why, None))
    }

    // Help resources use the query verbs behind the same grant.
    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        one_page(request.as_ref())?;
        let resource =
            Resource::new(help::INDEX, "Offline help index").with_mime_type("application/json");
        Ok(ListResourcesResult::with_all_items(vec![resource])
            .with_ttl_ms(LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }
    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        one_page(request.as_ref())?;
        let template =
            ResourceTemplate::new(help::TEMPLATE, "Help topic").with_mime_type("application/json");
        Ok(ListResourceTemplatesResult::with_all_items(vec![template])
            .with_ttl_ms(LIST_TTL_MS)
            .with_cache_scope(CacheScope::Public))
    }
    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        self.help_resource(&request.uri)
            .await
            .map(ReadResourceResponse::from)
    }
    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, ErrorData> {
        self.help_completion(&request).await
    }
}

/// A transport envelope (`{"ok", "result" | "error"}`) as a tool result.
/// A failure is a result with `isError`, carrying the error, so the model
/// reads its code and message; only the protocol's own failures are
/// JSON-RPC errors.
pub fn outcome(reply: &Value) -> CallToolResult {
    let failed = reply["ok"] != true;
    let payload = if failed {
        &reply["error"]
    } else {
        &reply["result"]
    };
    let content = vec![ContentBlock::text(payload.to_string())];
    if failed {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    }
}

#[cfg(test)]
pub(crate) mod scenario;
#[cfg(test)]
pub(crate) mod testing;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tool_tests;
#[cfg(test)]
mod workflow_tests;
