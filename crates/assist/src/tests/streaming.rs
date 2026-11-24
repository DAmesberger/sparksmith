//! The loop on streamed replies (L1): the EXR-22 script sent as the event
//! streams of both wire formats grades exactly as the whole replies do; the
//! chat's events come in order; Stop acts between chunks.
use super::*;
use crate::model::stream::fixtures;

fn without_time(mut facts: RunFacts) -> RunFacts {
    facts.elapsed_seconds = 0.0;
    for request in &mut facts.request_log {
        request.seconds = 0.0;
    }
    facts
}

/// The run on whole replies and on the same replies streamed.
fn both(
    config: &RunConfig,
    whole: Vec<Value>,
    streams: Vec<Value>,
    openai_backend: bool,
    sheet: &dyn Fn() -> Fallible<Datasheet>,
) -> Fallible<[(Outcome, Vec<Event>); 2]> {
    let mut out = Vec::new();
    for replies in [whole, streams] {
        let mut session = profiled_session();
        let result = if openai_backend {
            run_with(config, &mut openai(replies, config), sheet()?, &mut session)?
        } else {
            let mut backend = anthropic(replies)?;
            run_with(config, &mut backend, sheet()?, &mut session)?
        };
        out.push(result);
    }
    let [a, b]: [(Outcome, Vec<Event>); 2] = out.try_into().map_err(|_| "two runs")?;
    Ok([a, b])
}

fn assert_same(whole: &Outcome, streamed: &Outcome) {
    assert_eq!(streamed.stop, whole.stop);
    assert_eq!(
        serde_json::to_value(&streamed.report).ok(),
        serde_json::to_value(&whole.report).ok()
    );
    assert_eq!(
        serde_json::to_value(without_time(streamed.facts.clone())).ok(),
        serde_json::to_value(without_time(whole.facts.clone())).ok()
    );
}

#[test]
fn a_streamed_run_grades_as_the_whole_one_on_both_wire_formats() -> Fallible {
    let dir = tempfile::tempdir()?;
    let messages = script()?;
    let streams = messages.iter().map(fixtures::messages).collect();
    let [(whole, _), (streamed, log)] =
        both(&shipped_config()?, messages, streams, false, &|| {
            datasheet(&dir)
        })?;
    assert_eq!(streamed.stop, Stop::Submitted, "{log:?}");
    assert_same(&whole, &streamed);

    let config = local_config(131_072)?;
    let chats = chat_script(&script()?);
    let streams = chats.iter().map(fixtures::chat).collect();
    let [(whole, _), (streamed, log)] =
        both(&config, chats, streams, true, &|| text_datasheet(&dir))?;
    assert_eq!(streamed.stop, Stop::Submitted, "{log:?}");
    assert_same(&whole, &streamed);
    Ok(())
}

#[test]
fn the_chat_gets_deltas_rows_and_the_meter_in_order() -> Fallible {
    let dir = tempfile::tempdir()?;
    let messages = script()?;
    let n = messages.len();
    let streams = messages.iter().map(fixtures::messages).collect();
    let mut backend = anthropic(streams)?;
    let (outcome, log) = run_with(
        &shipped_config()?,
        &mut backend,
        datasheet(&dir)?,
        &mut profiled_session(),
    )?;
    assert_eq!(outcome.stop, Stop::Submitted, "{log:?}");
    // Request 1: the text arrives in pieces, then the reply's whole text
    // replaces them, and the call's row goes Writing, Running, Done.
    let first: Vec<&Event> = log
        .iter()
        .skip_while(|e| !matches!(e, Event::Request { n: 1, .. }))
        .take_while(|e| !matches!(e, Event::Request { n: 2, .. }))
        .collect();
    let deltas: String = first
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    let texts: Vec<&str> = first
        .iter()
        .filter_map(|e| match e {
            Event::Text(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec![deltas.as_str()]);
    assert!(deltas.starts_with("Calling "), "{deltas}");
    let kinds: Vec<&str> = first
        .iter()
        .filter_map(|e| match e {
            Event::Request { .. } => Some("request"),
            Event::Response { .. } => Some("response"),
            Event::Usage(_) => Some("usage"),
            Event::Text(_) => Some("text"),
            Event::Call {
                request: 1,
                call: 0,
                state,
                ..
            } => Some(match state {
                CallState::Writing => "writing",
                CallState::Running { .. } => "running",
                CallState::Done { .. } => "done",
                CallState::Refused { .. } => "refused",
                CallState::NotRun { .. } => "not run",
            }),
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "request", "writing", "response", "usage", "text", "running", "done"
        ]
    );
    // The last meter counts every request and token; the run has no limits.
    let meter = log
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::Usage(m) => Some(*m),
            _ => None,
        })
        .ok_or("a meter")?;
    assert_eq!(meter.requests.used, n as u64);
    assert_eq!(meter.requests.limit, None);
    assert_eq!(meter.output_tokens.used, outcome.facts.output_tokens);
    assert_eq!(
        meter.tokens.used,
        outcome.facts.input_tokens
            + outcome.facts.cache_write_tokens
            + outcome.facts.cache_read_tokens
            + outcome.facts.output_tokens
    );
    assert_eq!(meter.output_tokens.limit, None);
    assert_eq!(meter.tokens.limit, None);
    assert_eq!(meter.minutes.limit, None);
    Ok(())
}

#[test]
fn stop_between_chunks_ends_the_run_with_nothing_run() -> Fallible {
    let dir = tempfile::tempdir()?;
    let streams: Vec<Value> = script()?.iter().map(fixtures::messages).collect();
    let mut backend = anthropic(streams)?;
    let mut session = profiled_session();
    let before = session.engine().revision();
    let cancel = AtomicBool::new(false);
    let mut log = Vec::new();
    // The person presses Stop as the first piece of text shows.
    let outcome = run(
        &goal(datasheet(&dir)?),
        &shipped_config()?,
        &mut backend,
        &mut session,
        &mut |e| {
            if matches!(e, Event::TextDelta(_)) {
                cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            log.push(e);
        },
        &cancel,
    )?;
    assert_eq!(outcome.stop, Stop::Cancelled);
    assert_eq!(session.engine().revision(), before, "nothing ran");
    assert_eq!(outcome.facts.requests, 1);
    assert_eq!(outcome.facts.request_log[0].stop_reason, "stopped");
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, Event::TextDelta(_)))
            .count(),
        1,
        "nothing read after the stop: {log:?}"
    );
    assert!(
        !log.iter().any(|e| matches!(
            e,
            Event::Text(_)
                | Event::Call {
                    state: CallState::Running { .. },
                    ..
                }
        )),
        "the incomplete reply was never used: {log:?}"
    );
    assert_eq!(backend.wire.posts.len(), 1);
    Ok(())
}
