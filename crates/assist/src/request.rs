//! One authorization/accounting path for every generative request,
//! including the chat's otherwise invisible context summary.
use crate::chat::Hit;
use crate::model::{Budget, Delta, ModelBackend, ModelReply, ModelRequest, StopReason};
use crate::usage::Tally;
use crate::workflow::control::Gauge;
use crate::{CallState, Event, Meter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub(crate) struct Driver<'a> {
    pub backend: &'a mut dyn ModelBackend,
    pub budget: &'a Budget,
    pub started: Instant,
    pub tally: &'a mut Tally,
    pub cancel: &'a AtomicBool,
    pub events: &'a mut dyn FnMut(Event),
}

pub(crate) enum Stopped {
    Cancelled,
    /// Which limit, and the numbers in words.
    Limit(Hit, String),
    Failed(String),
}

impl From<Stopped> for crate::Stop {
    fn from(stop: Stopped) -> Self {
        match stop {
            Stopped::Cancelled => Self::Cancelled,
            Stopped::Limit(_, why) => Self::Budget(why),
            Stopped::Failed(why) => Self::Failed(why),
        }
    }
}

/// What the run has left; `None` is no limit.
pub(crate) struct Allowance {
    pub output: Option<u64>,
    pub duration: Option<Duration>,
}

impl Allowance {
    /// `limit` output tokens, or what is left when that is less.
    pub fn output_within(&self, limit: u32) -> u32 {
        self.output.map_or(limit, |left| {
            u32::try_from(left).map_or(limit, |left| limit.min(left))
        })
    }
}

pub(crate) struct Received {
    pub reply: ModelReply,
    pub n: u32,
    pub seconds: f64,
    pub output_limit: u32,
    /// Preserve completed accounting, but do not use a reply received after a limit.
    pub terminal: Option<Stopped>,
}

impl Driver<'_> {
    pub fn allowance(&self) -> std::result::Result<Allowance, Stopped> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(Stopped::Cancelled);
        }
        if let Some(max) = self.budget.max_requests
            && self.tally.requests >= max
        {
            return Err(Stopped::Limit(Hit::Requests, format!("{max} requests")));
        }
        let mut allowance = Allowance {
            output: None,
            duration: None,
        };
        if let Some(max) = self.budget.max_duration {
            let left = max.saturating_sub(self.started.elapsed());
            if left.is_zero() {
                return Err(Stopped::Limit(
                    Hit::Time,
                    format!("{} seconds of run time", max.as_secs()),
                ));
            }
            allowance.duration = Some(left);
        }
        if let Some(max) = self.budget.max_output_tokens {
            let left = max.saturating_sub(self.tally.usage.output_tokens);
            if left == 0 {
                return Err(out_of_output(max));
            }
            allowance.output = Some(left);
        }
        Ok(allowance)
    }

    /// The time limit, once it has passed, in words for request `n`.
    fn out_of_time(&self, n: u32) -> Option<Stopped> {
        let max = self.budget.max_duration?;
        (self.started.elapsed() >= max).then(|| {
            Stopped::Limit(
                Hit::Time,
                format!("{} seconds of run time, during request {n}", max.as_secs()),
            )
        })
    }

    /// Count the final chat prompt, including private summary requests,
    /// after the history and tool definitions for this dispatch are known.
    pub fn send_chat(
        &mut self,
        mut request: ModelRequest<'_>,
        estimate: u64,
        context: u64,
        visible: bool,
    ) -> std::result::Result<Received, Stopped> {
        self.allowance()?;
        let prompt = self
            .backend
            .count_prompt_tokens(&request)
            .ok()
            .flatten()
            .unwrap_or(estimate);
        request.fit_output(context, prompt);
        if request.max_output_tokens == 0 {
            return Err(Stopped::Limit(
                Hit::Tokens,
                format!(
                    "The prompt needs about {prompt} tokens and leaves no room to answer in the model's {context}-token context."
                ),
            ));
        }
        self.send(&request, prompt, visible)
    }

    /// Recheck after prompt preparation/counting, immediately before the
    /// network call. Summary output is private; its usage is still public.
    pub fn send(
        &mut self,
        request: &ModelRequest<'_>,
        prompt: u64,
        visible: bool,
    ) -> std::result::Result<Received, Stopped> {
        let allowance = self.allowance()?;
        let output = u64::from(request.max_output_tokens);
        if output == 0 {
            return Err(Stopped::Failed(
                "the request leaves the model no output tokens".into(),
            ));
        }
        if let Some(max) = self.budget.max_output_tokens
            && allowance.output.is_some_and(|left| output > left)
        {
            return Err(out_of_output(max));
        }
        let used = self
            .tally
            .usage
            .prompt()
            .saturating_add(self.tally.usage.output_tokens);
        if let Some(max) = self.budget.max_total_tokens
            && used.saturating_add(prompt).saturating_add(output) > max
        {
            return Err(Stopped::Limit(
                Hit::Tokens,
                format!(
                    "{max} tokens: {used} used, and the next request could take {}",
                    prompt.saturating_add(output)
                ),
            ));
        }
        let request = ModelRequest {
            system: request.system,
            tools: request.tools,
            messages: request.messages,
            max_output_tokens: request.max_output_tokens,
            timeout: allowance.duration,
        };
        let n = self.tally.requests.saturating_add(1);
        (self.events)(Event::Request {
            n,
            prompt_tokens: prompt,
        });
        // A failed/cancelled attempt still consumes a request. Token usage
        // is added only when the backend reports it, never guessed as zero.
        self.tally.requests = n;
        let sent = Instant::now();
        let events = &mut *self.events;
        let cancel = self.cancel;
        let mut delta = |delta| {
            if visible && !cancel.load(Ordering::Relaxed) {
                events(match delta {
                    Delta::Text(text) => Event::TextDelta(text),
                    Delta::Reasoning(text) => Event::ReasoningDelta(text),
                    Delta::ToolCall { index, name } => Event::Call {
                        request: n,
                        call: index,
                        name,
                        state: CallState::Writing,
                    },
                });
            }
        };
        let answer = self.backend.stream(&request, &mut delta, self.cancel);
        let seconds = sent.elapsed().as_secs_f64();
        let mut reply = match answer {
            Ok(reply) => reply,
            Err(error) => {
                (self.events)(Event::Usage(meter(self.tally, self.budget, self.started)));
                return Err(if self.cancel.load(Ordering::Relaxed) {
                    Stopped::Cancelled
                } else {
                    self.out_of_time(n)
                        .unwrap_or_else(|| Stopped::Failed(error.to_string()))
                });
            }
        };
        self.tally.usage.add(&reply.usage);
        self.tally.usage_estimated |= reply.usage_estimated;
        (self.events)(Event::Response {
            n,
            stop_reason: reply.stop.label(),
            output_tokens: reply.usage.output_tokens,
            seconds,
        });
        (self.events)(Event::Usage(meter(self.tally, self.budget, self.started)));
        let terminal = if self.cancel.load(Ordering::Relaxed) {
            reply.stop = StopReason::Stopped;
            None
        } else {
            self.out_of_time(n)
        };
        Ok(Received {
            reply,
            n,
            seconds,
            output_limit: request.max_output_tokens,
            terminal,
        })
    }
}

fn out_of_output(max: u64) -> Stopped {
    Stopped::Limit(Hit::OutputTokens, format!("{max} output tokens"))
}

/// The run's use against its budget, for the chat's meter.
fn meter(tally: &Tally, budget: &Budget, started: Instant) -> Meter {
    Meter {
        requests: Gauge {
            used: tally.requests.into(),
            limit: budget.max_requests.map(u64::from),
        },
        output_tokens: Gauge {
            used: tally.usage.output_tokens,
            limit: budget.max_output_tokens,
        },
        tokens: Gauge {
            used: tally
                .usage
                .prompt()
                .saturating_add(tally.usage.output_tokens),
            limit: budget.max_total_tokens,
        },
        minutes: Gauge {
            used: started.elapsed().as_secs() / 60,
            limit: budget.max_duration.map(|d| d.as_secs().div_ceil(60)),
        },
        reported: tally.usage,
        usage_estimated: tally.usage_estimated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Part;
    use crate::usage::Usage;

    struct Model {
        calls: usize,
        fail: bool,
    }
    impl ModelBackend for Model {
        fn label(&self) -> String {
            "script".into()
        }
        fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> crate::Result<Option<u64>> {
            Ok(None)
        }
        fn send(&mut self, request: &ModelRequest<'_>) -> crate::Result<ModelReply> {
            self.calls += 1;
            assert!(request.timeout.is_some_and(|t| t <= Duration::from_secs(2)));
            if self.fail {
                return Err(crate::Error::Protocol("scripted failure".into()));
            }
            Ok(ModelReply {
                model: "script".into(),
                content: vec![Part::text("private summary")],
                stop: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    ..Usage::default()
                },
                usage_estimated: false,
            })
        }
    }

    #[test]
    fn hidden_requests_report_usage_and_failed_attempts_consume_a_slot() {
        for fail in [false, true] {
            let mut model = Model { calls: 0, fail };
            let budget = Budget {
                max_requests: Some(1),
                max_duration: Some(Duration::from_secs(2)),
                ..Budget::default()
            };
            let mut tally = Tally::default();
            let mut events = Vec::new();
            let cancel = AtomicBool::new(false);
            let mut observe = |event| events.push(event);
            let mut driver = Driver {
                backend: &mut model,
                budget: &budget,
                started: Instant::now(),
                tally: &mut tally,
                cancel: &cancel,
                events: &mut observe,
            };
            let request = ModelRequest {
                system: "",
                tools: &[],
                messages: &[],
                max_output_tokens: 100,
                timeout: None,
            };
            assert_eq!(driver.send(&request, 10, false).is_err(), fail);
            assert!(matches!(
                driver.send(&request, 10, true),
                Err(Stopped::Limit(..))
            ));
            assert_eq!(model.calls, 1);
            assert_eq!(tally.requests, 1);
            assert_eq!(tally.usage.output_tokens, if fail { 0 } else { 5 });
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, Event::TextDelta(_) | Event::ReasoningDelta(_)))
            );
            assert!(events.iter().any(|e| matches!(e, Event::Usage(_))));
        }
    }

    #[test]
    fn every_cap_and_initial_cancel_prevent_the_backend_call() {
        for case in 0..5 {
            let mut model = Model {
                calls: 0,
                fail: false,
            };
            let mut budget = Budget::default();
            match case {
                0 => budget.max_requests = Some(0),
                1 => budget.max_output_tokens = Some(9),
                2 => budget.max_total_tokens = Some(19),
                3 => budget.max_duration = Some(Duration::ZERO),
                _ => {}
            }
            let mut tally = Tally::default();
            let cancel = AtomicBool::new(case == 4);
            let mut ignore = |_| {};
            let mut driver = Driver {
                backend: &mut model,
                budget: &budget,
                started: Instant::now(),
                tally: &mut tally,
                cancel: &cancel,
                events: &mut ignore,
            };
            let request = ModelRequest {
                system: "",
                tools: &[],
                messages: &[],
                max_output_tokens: 10,
                timeout: None,
            };
            assert!(driver.send(&request, 10, false).is_err());
            assert_eq!(model.calls, 0);
            assert_eq!(tally.requests, 0);
        }
    }
}
