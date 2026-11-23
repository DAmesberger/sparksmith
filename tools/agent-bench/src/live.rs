//! A real Messages API backend with explicit per-run limits and a persisted
//! provider reservation before each billable attempt.
use crate::{
    Result,
    run::Arm,
    spend::{Budget as PriceBudget, Ledger},
};
use model::{
    stream::{SseEvent, Streamed},
    wire::{AuthStyle, HttpWire, Wire},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sparksmith_assist::{
    Error,
    model::{
        self, Budget, Endpoint, KeySource, ModelBackend, ModelProfile, ModelReply, ModelRequest,
    },
};
use sparksmith_automation::session::SchemaDetail;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Operator {
    pub pricing: PriceBudget,

    pub requests: u32,
    pub cumulative_output_tokens: u64,
    pub cumulative_tokens: u64,
    pub seconds: u64,
    pub max_output_per_request: u32,
    pub effort: String,
}

impl Operator {
    pub fn arm(self, dir: &Path, ledger: &Path, detail: SchemaDetail) -> Result<Arm> {
        if self.pricing.provider != "anthropic" {
            return Err("this live runner requires an Anthropic rate card".into());
        }
        if self.requests == 0
            || self.seconds == 0
            || self.cumulative_tokens == 0
            || self.max_output_per_request == 0
            || self.cumulative_output_tokens == 0
        {
            return Err("set the bench request, output, total token and time allowances".into());
        }
        let mut profile = ModelProfile::anthropic(&self.pricing.model);
        profile.effort = Some(self.effort.clone());
        profile.output.per_request = self.max_output_per_request;
        profile.output.after_cut = self.max_output_per_request;
        profile.capabilities.server_fallback = false;
        profile.request_timeout = Duration::from_secs(self.seconds);
        profile.check()?;
        let endpoint = Endpoint::parse(model::profile::ANTHROPIC_BASE_URL)?;
        let key = KeySource::Env("ANTHROPIC_API_KEY".into()).resolve(&endpoint)?;
        let http = HttpWire::new(endpoint, key, AuthStyle::ApiKeyHeader)?;
        let wire = ReceiptWire {
            inner: http,
            model: self.pricing.model.clone(),
        };
        let backend = Box::new(model::anthropic::AnthropicBackend::new(
            profile.clone(),
            wire,
        ));
        let budget = Budget {
            max_requests: self.requests,
            max_total_tokens: self.cumulative_tokens,
            max_output_tokens: self.cumulative_output_tokens,
            max_duration: Duration::from_secs(self.seconds),
        };
        let run = std::path::absolute(dir)?.to_string_lossy().into_owned();
        let backend = Box::new(Paid {
            inner: backend,
            pricing: self.pricing,
            path: ledger.to_owned(),
            run,
            request: 0,
            context: profile.capabilities.context_tokens,
        });
        Ok(Arm {
            runner: "live".into(),
            backend,
            profile,
            budget,
            detail,
            reports_usage: true,
        })
    }
}
struct Paid {
    inner: Box<dyn ModelBackend>,
    pricing: PriceBudget,
    path: PathBuf,
    run: String,
    request: u32,
    context: u64,
}
impl ModelBackend for Paid {
    fn label(&self) -> String {
        self.inner.label()
    }
    fn send(&mut self, req: &ModelRequest<'_>) -> sparksmith_assist::Result<ModelReply> {
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| Error::Config("request counter overflow".into()))?;
        let ledger = Ledger::new(&self.path, &self.pricing);
        let problem = |err: Box<dyn std::error::Error>| Error::Config(err.to_string());
        ledger
            .reserve(&self.run, self.request, self.context, req.max_output_tokens)
            .map_err(problem)?;
        let reply = self.inner.send(req)?;
        if reply.usage_estimated || reply.model != self.pricing.model || reply.usage.prompt() == 0 {
            return Err(Error::Config(
                "usage or model could not be confirmed; the ledger holds the maximum charge".into(),
            ));
        }
        ledger
            .settle(&self.run, self.request, reply.usage)
            .map_err(problem)?;
        Ok(reply)
    }
    fn count_prompt_tokens(
        &mut self,
        _: &ModelRequest<'_>,
    ) -> sparksmith_assist::Result<Option<u64>> {
        Ok(None)
    }
}

/// The developer bench uses whole replies so the provider's billing fields
/// are checked before the product's neutral parser supplies defaults.
struct ReceiptWire {
    inner: HttpWire,
    model: String,
}
impl Wire for ReceiptWire {
    fn label(&self) -> String {
        self.inner.label()
    }
    fn post(
        &mut self,
        path: &str,
        headers: &[(&str, &str)],
        body: &Value,
        timeout: Duration,
    ) -> sparksmith_assist::Result<Value> {
        if path != "/v1/messages" || body.get("fallbacks").is_some() || body["model"] != self.model
        {
            return Err(Error::Config(
                "the request is outside the operator's rate card".into(),
            ));
        }
        let mut body = body.clone();
        body["service_tier"] = serde_json::json!("standard_only");
        body["inference_geo"] = serde_json::json!("global");
        let response = self.inner.post(path, headers, &body, timeout)?;
        check_receipt(&response, &self.model)?;
        Ok(response)
    }
    fn post_stream(
        &mut self,
        _: &str,
        _: &[(&str, &str)],
        _: &Value,
        _: Duration,
        _: &mut dyn FnMut(SseEvent) -> sparksmith_assist::Result<bool>,
    ) -> sparksmith_assist::Result<Streamed> {
        Err(Error::Config(
            "the bench requires a whole billing receipt".into(),
        ))
    }
    fn clean_reply(&self, reply: ModelReply) -> sparksmith_assist::Result<ModelReply> {
        self.inner.clean_reply(reply)
    }
}

fn check_receipt(response: &Value, model: &str) -> sparksmith_assist::Result<()> {
    let usage = &response["usage"];
    let priced = [
        "input_tokens",
        "output_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .all(|field| usage[*field].as_u64().is_some());
    let standard = [
        ("service_tier", "standard"),
        ("inference_geo", "global"),
        ("speed", "standard"),
    ]
    .iter()
    .all(|(field, expected)| usage[*field].is_null() || usage[*field] == *expected);
    let one_hour = &usage["cache_creation"]["ephemeral_1h_input_tokens"];
    let cache_receipt = usage["cache_creation"].is_null()
        || usage["cache_creation"].is_object()
            && (one_hour.is_null() || one_hour.as_u64() == Some(0));
    let extra_tools = &usage["server_tool_use"];
    let no_paid_tools = extra_tools.is_null()
        || extra_tools
            .as_object()
            .is_some_and(|counts| counts.values().all(|v| v.as_u64() == Some(0)));
    if response["model"] != model || !priced || !standard || !cache_receipt || !no_paid_tools {
        return Err(Error::Protocol("the provider receipt has incomplete usage or a different rate class; the ledger holds the maximum charge".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn incomplete_usage_and_a_different_rate_class_never_release_the_hold() {
        let receipt = json!({"model":"example","usage":{"input_tokens":3,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":2}});
        assert!(check_receipt(&receipt, "example").is_ok());
        let mut absent = receipt.clone();
        absent["usage"]["input_tokens"] = Value::Null;
        assert!(check_receipt(&absent, "example").is_err());
        let mut wrong = receipt.clone();
        wrong["usage"]["inference_geo"] = json!("us");
        assert!(check_receipt(&wrong, "example").is_err());
        assert!(check_receipt(&receipt, "another").is_err());
    }
    #[test]
    fn an_interrupted_post_keeps_its_hold_and_is_never_replayed() -> Result<()> {
        use crate::spend::Rates;
        use std::io::{self, Read};
        use std::net::TcpListener;
        use std::thread;
        use std::time::Instant;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let endpoint = Endpoint::parse(&format!("http://{}", listener.local_addr()?))?;
        let server = listener.try_clone()?;
        let observe = thread::spawn(move || -> io::Result<()> {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut peer = loop {
                match server.accept() {
                    Ok((peer, _)) => break peer,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => return Err(error),
                }
            };
            peer.set_nonblocking(false)?;
            peer.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut received = Vec::new();
            let mark = b"interrupted-post-fixture";
            while received.len() < 65_536 {
                let mut chunk = [0; 2048];
                let n = peer.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                received.extend_from_slice(&chunk[..n]);
                if received.starts_with(b"POST /v1/messages ")
                    && received.windows(mark.len()).any(|part| part == mark)
                {
                    // Discard the connection after accepting the POST body.
                    return Ok(());
                }
            }
            Err(io::Error::other(
                "the loopback peer did not receive the fixture request",
            ))
        });
        let profile = ModelProfile::anthropic("test-model");
        let wire = ReceiptWire {
            inner: HttpWire::new(endpoint, None, AuthStyle::ApiKeyHeader)?,
            model: profile.model.clone(),
        };
        let dir = tempfile::tempdir()?;
        let mut paid = Paid {
            inner: Box::new(model::anthropic::AnthropicBackend::new(profile, wire)),
            pricing: PriceBudget {
                provider: "anthropic".into(),
                model: "test-model".into(),
                limit_cents: 1,
                rates: Rates {
                    input_nusd: 10,
                    cache_write_nusd: 20,
                    cache_read_nusd: 1,
                    output_nusd: 30,
                },
                rate_source: "test fixture".into(),
            },
            path: dir.path().join("ledger.jsonl"),
            run: "run".into(),
            request: 0,
            context: 100,
        };
        let request = ModelRequest {
            system: "interrupted-post-fixture",
            tools: &[],
            messages: &[],
            max_output_tokens: 3,
            timeout: Duration::from_secs(2),
        };
        assert!(paid.send(&request).is_err());
        observe.join().map_err(|_| "loopback peer failed")??;
        assert!(paid.send(&request).is_err());
        let ledger = Ledger::new(&paid.path, &paid.pricing).balances()?;
        assert_eq!(ledger["run"].attempts, 1);
        assert_eq!(ledger["run"].held_nusd, 2090);
        assert_eq!(ledger["run"].confirmed_nusd, 0);
        assert!(matches!(listener.accept(),Err(error) if error.kind()==io::ErrorKind::WouldBlock));
        Ok(())
    }
}
