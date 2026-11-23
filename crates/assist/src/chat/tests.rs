//! The chat's loop on a scripted model (no network): answers, tool rows,
//! Stop between calls, limits, refusals, and real verbs in one turn that
//! undoes as one step.
use super::*;
use crate::model::{ModelReply, ModelRequest};
use crate::usage::Usage;
use crate::verbs::Agent;
use serde_json::json;
use sparksmith_automation::front::{self, Call, CallCx, NoOne, Surface};
use sparksmith_automation::{Principal, Session};
use std::collections::VecDeque;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A model that answers from a script, recording what it was sent.
#[derive(Default)]
struct Script {
    replies: VecDeque<ModelReply>,
    /// The number of messages each request carried.
    sent: Vec<usize>,
}

impl Script {
    fn new(replies: impl IntoIterator<Item = ModelReply>) -> Self {
        Self {
            replies: replies.into_iter().collect(),
            sent: Vec::new(),
        }
    }
}

impl ModelBackend for Script {
    fn label(&self) -> String {
        "script".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        self.sent.push(request.messages.len());
        self.replies
            .pop_front()
            .ok_or_else(|| Error::Protocol("the script has no more replies".into()))
    }
    fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> Result<Option<u64>> {
        Ok(None)
    }
}

fn reply(content: Vec<Part>, stop: StopReason) -> ModelReply {
    ModelReply {
        model: "scripted".into(),
        content,
        stop,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 10,
            ..Usage::default()
        },
        usage_estimated: false,
    }
}

fn text(text: &str) -> ModelReply {
    reply(vec![Part::text(text)], StopReason::EndTurn)
}

fn call(id: &str, verb: &str, input: Value) -> Part {
    Part::ToolCall {
        id: id.into(),
        name: verb.into(),
        input,
        malformed: None,
        replay: None,
    }
}

fn calls(parts: Vec<Part>) -> ModelReply {
    reply(parts, StopReason::ToolCalls)
}

pub(super) fn config() -> Fallible<ChatConfig> {
    let library = ChatLibrary::locate()?;
    Ok(ChatConfig::new(
        &library,
        crate::model::ModelProfile::openai_compatible("scripted", 32_768),
        Budget::default(),
    )?)
}

fn summary_history() -> Vec<Message> {
    vec![
        Message {
            role: Role::User,
            content: vec![Part::text("a".repeat(8_000))],
        },
        Message {
            role: Role::Assistant,
            content: vec![Part::text("b".repeat(8_000))],
        },
    ]
}

#[test]
fn summary_requests_share_the_turn_budget_and_usage() -> Fallible {
    let mut config = config()?;
    config.policy.trigger = 0.10;
    config.policy.target = 0.05;
    config.budget.max_requests = 1;
    let mut model = Script::new([text("Summary."), text("Answer.")]);
    let mut history = summary_history();
    let mut tools = Recorder {
        calls: vec![],
        answer: |_, _| Value::Null,
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Next")],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(model.sent.len(), 1, "the summary consumes the one request");
    assert_eq!(outcome.usage.requests, 1);
    assert_eq!(outcome.usage.tokens, 110);
    assert!(
        matches!(
            outcome.end,
            End::Limit {
                hit: Hit::Requests,
                ..
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[test]
fn cancelled_turn_never_starts_a_summary() -> Fallible {
    let mut config = config()?;
    config.policy.trigger = 0.10;
    config.policy.target = 0.05;
    let mut model = Script::new([text("Summary.")]);
    let mut history = summary_history();
    let mut tools = Recorder {
        calls: vec![],
        answer: |_, _| Value::Null,
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Next")],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(true),
    );
    assert!(model.sent.is_empty());
    assert_eq!(outcome.usage.requests, 0);
    assert_eq!(outcome.end, End::Stopped);
    Ok(())
}

#[test]
fn summary_output_uses_the_same_remaining_token_budget() -> Fallible {
    let mut config = config()?;
    config.policy.trigger = 0.10;
    config.policy.target = 0.05;
    config.budget.max_output_tokens = 10;
    let mut model = Script::new([text("Summary."), text("Answer.")]);
    let mut history = summary_history();
    let mut tools = Recorder {
        calls: vec![],
        answer: |_, _| Value::Null,
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Next")],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(model.sent.len(), 1);
    assert_eq!(outcome.usage.output_tokens, 10);
    assert!(
        matches!(
            outcome.end,
            End::Limit {
                hit: Hit::OutputTokens,
                ..
            }
        ),
        "{outcome:?}"
    );
    Ok(())
}

#[test]
fn cancelled_summary_counts_usage_but_does_not_replace_history() -> Fallible {
    struct Cancels<'a>(&'a AtomicBool);
    impl ModelBackend for Cancels<'_> {
        fn label(&self) -> String {
            "cancels".into()
        }
        fn send(&mut self, _: &ModelRequest<'_>) -> Result<ModelReply> {
            self.0.store(true, Ordering::Relaxed);
            Ok(text("This cancelled summary must not be kept."))
        }
        fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> Result<Option<u64>> {
            Ok(None)
        }
    }
    let mut config = config()?;
    config.policy.trigger = 0.10;
    config.policy.target = 0.05;
    let cancel = AtomicBool::new(false);
    let mut model = Cancels(&cancel);
    let mut history = summary_history();
    let mut tools = Recorder {
        calls: vec![],
        answer: |_, _| Value::Null,
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Next")],
        &mut tools,
        &mut |_| {},
        &cancel,
    );
    assert_eq!(outcome.end, End::Stopped);
    assert_eq!(outcome.usage.requests, 1);
    assert_eq!(outcome.usage.tokens, 110);
    let kept = serde_json::to_string(&history)?;
    assert!(!kept.contains("This cancelled summary"));
    assert!(kept.contains(&"a".repeat(8_000)));
    Ok(())
}

#[test]
fn late_replies_are_counted_without_using_answers_or_summaries() -> Fallible {
    for summary in [false, true] {
        let mut config = config()?;
        config.budget.max_duration = std::time::Duration::from_secs(2);
        if summary {
            config.policy.trigger = 0.10;
            config.policy.target = 0.05;
        }
        let mut history = if summary {
            summary_history()
        } else {
            Vec::new()
        };
        let original = history.clone();
        let mut model = Script::new([text("Late content must not enter the conversation.")]);
        let mut tools = Recorder {
            calls: vec![],
            answer: |_, _| Value::Null,
        };
        let mut events = Vec::new();
        let outcome = turn(
            &config,
            &mut model,
            &mut history,
            vec![Part::text("Next")],
            &mut tools,
            &mut |event| {
                if matches!(event, Event::Response { .. }) {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                events.push(event);
            },
            &AtomicBool::new(false),
        );
        assert!(
            matches!(outcome.end, End::Limit { hit: Hit::Time, .. }),
            "{outcome:?}"
        );
        assert_eq!(outcome.usage.requests, 1);
        assert_eq!(outcome.usage.tokens, 110);
        assert_eq!(&history[..original.len()], original.as_slice());
        assert!(!serde_json::to_string(&history)?.contains("Late content"));
        assert!(tools.calls.is_empty());
        assert!(!events.iter().any(|event| matches!(event, Event::Text(_))));
    }
    Ok(())
}

/// Tools that answer from a closure and record each call.
struct Recorder<F: FnMut(&str, &Value) -> Value> {
    calls: Vec<String>,
    answer: F,
}

impl<F: FnMut(&str, &Value) -> Value> Tools for Recorder<F> {
    fn call(&mut self, verb: &str, input: &Value) -> Value {
        self.calls.push(verb.to_owned());
        (self.answer)(verb, input)
    }
}

fn ok(result: Value) -> Value {
    json!({"ok": true, "result": result})
}

#[test]
fn the_shipped_chat_data_loads_and_its_reach_steers_the_window() -> Fallible {
    let library = ChatLibrary::locate()?;
    assert!(library.system.contains(sparksmith_core::product_name!()));
    assert!(!library.system.contains(PRODUCT_PLACEHOLDER));
    let grant = library.reach.grant();
    assert_eq!(
        grant.modules.get("view"),
        Some(&sparksmith_core::Class::Show)
    );
    assert_eq!(
        grant.modules.get("layout"),
        Some(&sparksmith_core::Class::Edit)
    );
    use sparksmith_core::permission::{Reach, Reaches};
    assert!(grant.reaches(Reaches::NONE.with(Reach::Window)));
    assert!(!grant.reaches(Reaches::NONE.with(Reach::Settings)));
    Ok(())
}

#[test]
fn an_answer_without_tools_ends_the_turn_and_the_history_keeps_both_sides() -> Fallible {
    let config = config()?;
    let mut model = Script::new([text("The board has 4 layers.")]);
    let mut history = Vec::new();
    let mut events = Vec::new();
    let mut tools = Recorder {
        calls: Vec::new(),
        answer: |_: &str, _: &Value| ok(json!({})),
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("How many layers?")],
        &mut tools,
        &mut |e| events.push(e),
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    assert_eq!(outcome.usage.requests, 1);
    assert_eq!(outcome.usage.tokens, 110);
    assert_eq!(history.len(), 2);
    assert!(tools.calls.is_empty());
    assert!(matches!(events.first(), Some(Event::Request { n: 1, .. })));
    assert!(events.contains(&Event::Text("The board has 4 layers.".into())));
    Ok(())
}

#[test]
fn a_zero_nudge_allowance_follows_each_metered_read_but_resets_on_an_edit() -> Fallible {
    let mut config = config()?;
    config.policy.nudge_tokens = 0;
    let mut model = Script::new([
        calls(vec![call(
            "read",
            "query",
            json!({"name":"project.issues"}),
        )]),
        calls(vec![call("edit", "apply", json!({"operations":[]}))]),
        calls(vec![call(
            "read2",
            "query",
            json!({"name":"project.issues"}),
        )]),
        text("Done."),
    ]);
    let mut tools = Recorder {
        calls: Vec::new(),
        answer: |_: &str, _: &Value| ok(json!({})),
    };
    let mut nudges = Vec::new();
    let mut history = Vec::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Check, edit, then check.")],
        &mut tools,
        &mut |event| {
            if let Event::Nudged { tokens, .. } = event {
                nudges.push(tokens);
            }
        },
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    assert_eq!(nudges, [110, 110]);
    let nudged_after: Vec<_> = history
        .iter()
        .filter_map(|message| {
            if message
                .content
                .iter()
                .any(|p| matches!(p, Part::Text {text,..} if text == &config.nudge))
            {
                message.content.iter().find_map(|p| match p {
                    Part::ToolResult { call_id, .. } => Some(call_id.as_str()),
                    _ => None,
                })
            } else {
                None
            }
        })
        .collect();
    assert_eq!(nudged_after, ["read", "read2"]);
    Ok(())
}

#[test]
fn tool_calls_run_in_order_and_their_rows_end_done_or_refused() -> Fallible {
    let config = config()?;
    let mut model = Script::new([
        calls(vec![
            call("a", "query", json!({"name": "project.issues"})),
            call("b", "apply", json!({"operations": []})),
        ]),
        text("Done."),
    ]);
    let mut history = Vec::new();
    let mut events = Vec::new();
    let mut tools = Recorder {
        calls: Vec::new(),
        answer: |verb: &str, _: &Value| {
            if verb == "query" {
                ok(json!({"open": 0}))
            } else {
                json!({"ok": false, "error": {"code": "malformed_request", "message": "no operations"}})
            }
        },
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Check it.")],
        &mut tools,
        &mut |e| events.push(e),
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    assert_eq!(tools.calls, ["query", "apply"]);
    let states: Vec<(u32, &str)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Call { call, state, .. } => Some((
                *call,
                match state {
                    CallState::Writing => "writing",
                    CallState::Running { .. } => "running",
                    CallState::Done { .. } => "done",
                    CallState::Refused { .. } => "refused",
                    CallState::NotRun { .. } => "not run",
                },
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            (0, "writing"),
            (1, "writing"),
            (0, "running"),
            (0, "done"),
            (1, "running"),
            (1, "refused")
        ]
    );
    // User, assistant (calls), user (two results), assistant (answer).
    assert_eq!(history.len(), 4);
    let errors: Vec<bool> = history[2]
        .content
        .iter()
        .filter_map(|p| match p {
            Part::ToolResult { error, .. } => Some(*error),
            _ => None,
        })
        .collect();
    assert_eq!(errors, [false, true]);
    Ok(())
}

#[test]
fn stop_between_calls_leaves_the_rest_unrun_and_the_history_answered() -> Fallible {
    let config = config()?;
    let mut model = Script::new([
        calls(vec![
            call("a", "query", json!({"name": "project.issues"})),
            call("b", "query", json!({"name": "project.issues"})),
        ]),
        text("never sent"),
    ]);
    let cancel = AtomicBool::new(false);
    let mut history = Vec::new();
    let mut events = Vec::new();
    let mut tools = Recorder {
        calls: Vec::new(),
        answer: |_: &str, _: &Value| {
            // The person presses Stop while the first call runs.
            cancel.store(true, Ordering::Relaxed);
            ok(json!({}))
        },
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Go.")],
        &mut tools,
        &mut |e| events.push(e),
        &cancel,
    );
    assert_eq!(outcome.end, End::Stopped);
    assert_eq!(tools.calls.len(), 1);
    assert_eq!(model.sent.len(), 1);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Call { call: 1, state: CallState::NotRun { why }, .. } if why == "stopped"
    )));
    // Every call has its result, so the next message can follow.
    assert_eq!(history.len(), 3);
    assert_eq!(history[2].role, Role::User);
    // The next message joins that last message of the person.
    cancel.store(false, Ordering::Relaxed);
    let mut model = Script::new([text("Fine.")]);
    turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Carry on.")],
        &mut tools,
        &mut |_| {},
        &cancel,
    );
    assert_eq!(history.len(), 4);
    assert_eq!(history[2].content.len(), 3);
    Ok(())
}

#[test]
fn a_turn_stops_at_its_request_limit_and_a_failed_backend_says_why() -> Fallible {
    let mut config = config()?;
    config.budget.max_requests = 1;
    let mut model = Script::new([calls(vec![call("a", "find", json!({"text": "via"}))])]);
    let mut history = Vec::new();
    let mut tools = Recorder {
        calls: Vec::new(),
        answer: |_: &str, _: &Value| ok(json!([])),
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Find vias.")],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(
        outcome.end,
        End::Limit {
            hit: Hit::Requests,
            why: "1 requests".into()
        }
    );
    config.budget.max_requests = 5;
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Again.")],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert!(matches!(outcome.end, End::Failed(why) if why.contains("no more replies")));
    Ok(())
}

#[test]
fn a_model_without_tool_calling_is_refused_before_any_request() -> Fallible {
    let mut config = config()?;
    config.profile.capabilities.tool_calling = false;
    let mut model = Script::new([text("unused")]);
    let outcome = turn(
        &config,
        &mut model,
        &mut Vec::new(),
        vec![Part::text("Hi.")],
        &mut Recorder {
            calls: Vec::new(),
            answer: |_: &str, _: &Value| ok(json!({})),
        },
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert!(matches!(outcome.end, End::Failed(why) if why.contains("without tool calling")));
    assert!(model.sent.is_empty());
    Ok(())
}

/// Real verbs through the front door as the built-in agent, every call in
/// the message's turn: two edits undo as one step with the turn's label.
#[test]
fn the_verbs_of_one_message_join_its_turn_and_undo_as_one_step() -> Fallible {
    let config = config()?;
    let library = ChatLibrary::locate()?;
    let mut session = Session::new(sparksmith_core::Engine::demo()?);
    let grant = library.reach.grant();
    let label = "AI assistant: two vias";
    let begun = front::envelope(session.call(
        CallCx {
            who: Principal::LocalAgent { grant: &grant },
            asker: &mut NoOne,
            verbose: false,
            window: None,
        },
        Call {
            entry: "project.turn_begin".into(),
            params: json!({"label": label}),
            project: None,
            expected_revision: None,
            dry_run: false,
            confirm: None,
            turn: None,
            job: false,
        },
    ));
    let turn_id = begun["result"]["turn"]
        .as_str()
        .ok_or("no turn")?
        .to_owned();
    let net = session.engine().project().nets.first().map_or(0, |n| n.id);
    let via = |x: f64, revision: u64| {
        json!({"expected_revision": revision, "operations": [{"op": "layout.add_via", "params":
            {"via": {"net": net, "position": [x, 1.0], "drill": 0.3, "diameter": 0.6,
            "span": "through", "tented": true}}}]})
    };
    let start = session.engine().revision();
    let vias = session.engine().project().vias.len();
    let mut model = Script::new([
        calls(vec![call("a", "apply", via(1.0, start))]),
        calls(vec![call("b", "apply", via(2.0, start + 1))]),
        text("Added two vias."),
    ]);
    struct Verbs<'a> {
        session: &'a mut Session,
        grant: &'a sparksmith_automation::Grant,
        turn: &'a str,
    }
    impl Tools for Verbs<'_> {
        fn call(&mut self, verb: &str, input: &Value) -> Value {
            Agent {
                session: self.session,
                grant: self.grant,
                window: None,
                turn: Some(self.turn),
                asker: &mut NoOne,
            }
            .verb(verb, input.clone())
        }
    }
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        turn: &turn_id,
    };
    let mut events = Vec::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut Vec::new(),
        vec![Part::text("Add two vias.")],
        &mut tools,
        &mut |e| events.push(e),
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered, "{events:#?}");
    assert_eq!(session.engine().revision(), start + 2);
    assert_eq!(
        session.engine().undo_step().map(|s| s.line()),
        Some(label.to_owned())
    );
    assert_eq!(session.engine().project().vias.len(), vias + 2);
    // One undo takes back both.
    let mut fork = session.engine().fork();
    fork.undo()?;
    assert_eq!(fork.project().vias.len(), vias);
    Ok(())
}

#[path = "board_trace.rs"]
mod board_trace;

#[path = "board_chat.rs"]
mod board_chat;

#[test]
fn chat_and_mcp_share_one_copy_of_the_method() -> Fallible {
    let library = ShippedLibrary::locate()?;
    let method = sparksmith_automation::mcp::agent_method(&library)?;
    let chat = ChatLibrary::load(&library)?;
    assert_eq!(chat.system.matches(&method).count(), 1);
    assert_eq!(sparksmith_automation::mcp::instructions(false)?, method);
    assert!(sparksmith_automation::mcp::instructions(true)?.starts_with(&method));
    Ok(())
}

#[test]
fn summary_and_chat_fit_the_counted_prompt_in_the_declared_context() -> Fallible {
    struct Counted {
        input: u64,
        counted: Vec<bool>,
        sent: Vec<(bool, u32)>,
    }
    impl ModelBackend for Counted {
        fn label(&self) -> String {
            "offline counter".into()
        }
        fn count_prompt_tokens(&mut self, req: &ModelRequest<'_>) -> Result<Option<u64>> {
            self.counted.push(req.tools.is_empty());
            Ok(Some(self.input))
        }
        fn send(&mut self, req: &ModelRequest<'_>) -> Result<ModelReply> {
            self.sent
                .push((req.tools.is_empty(), req.max_output_tokens));
            Ok(text("Finished"))
        }
    }
    let mut config = config()?;
    config.profile.capabilities.context_tokens = 4096;
    config.profile.output.per_request = 2048;
    config.policy.trigger = 0.10;
    config.policy.target = 0.05;
    for (input, run) in [(3500, true), (4096, false)] {
        let mut model = Counted {
            input,
            counted: Vec::new(),
            sent: Vec::new(),
        };
        let mut tools = Recorder {
            calls: vec![],
            answer: |_, _| Value::Null,
        };
        let result = turn(
            &config,
            &mut model,
            &mut summary_history(),
            vec![Part::text("Next")],
            &mut tools,
            &mut |_| {},
            &AtomicBool::new(false),
        );
        assert_eq!(model.counted, [true, false], "summary then chat");
        if run {
            assert_eq!(model.sent, [(true, 596), (false, 596)]);
            assert_eq!(result.end, End::Answered);
        } else {
            assert!(model.sent.is_empty());
            assert!(matches!(
                result.end,
                End::Limit {
                    hit: Hit::Tokens,
                    ..
                }
            ));
        }
    }
    Ok(())
}
