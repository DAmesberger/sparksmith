//! Shortening, clearing and condensing on recorded conversation shapes.
use super::*;
use crate::model::{ModelBackend, ModelReply, StopReason};
use crate::usage::Usage;
use serde_json::json;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn user(text: &str) -> Message {
    Message {
        role: Role::User,
        content: vec![Part::text(text)],
    }
}

fn calls(calls: &[(&str, &str, Value)]) -> Message {
    Message {
        role: Role::Assistant,
        content: calls
            .iter()
            .map(|(id, verb, input)| Part::ToolCall {
                id: (*id).into(),
                name: (*verb).into(),
                input: input.clone(),
                malformed: None,
                replay: None,
            })
            .collect(),
    }
}

fn results(replies: &[(&str, String)]) -> Message {
    Message {
        role: Role::User,
        content: replies
            .iter()
            .map(|(id, text)| Part::ToolResult {
                call_id: (*id).into(),
                text: text.clone(),
                error: false,
                images: Vec::new(),
            })
            .collect(),
    }
}

fn answer(text: &str) -> Message {
    Message {
        role: Role::Assistant,
        content: vec![Part::text(text)],
    }
}

/// A bulky listing, as `library.search` or `project.inspect` give.
fn bulky(n: usize) -> String {
    let items: Vec<Value> = (0..n)
        .map(|i| json!({"id": i, "name": format!("Part {i}"), "description": "x".repeat(80)}))
        .collect();
    json!({"results": items}).to_string()
}

fn texts(history: &[Message]) -> Vec<String> {
    history
        .iter()
        .flat_map(|m| &m.content)
        .filter_map(|p| match p {
            Part::ToolResult { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_long_reply_is_shortened_as_json_and_says_how_to_ask_for_less() {
    let value: Value = serde_json::from_str(&bulky(400)).unwrap_or_default();
    let text = shrink(&value, 4_000);
    assert!(text.len() <= 4_000, "{}", text.len());
    let read: Value = serde_json::from_str(&text).unwrap_or_default();
    assert!(
        read["_shortened"]
            .as_str()
            .is_some_and(|n| n.contains("narrower")),
        "{text}"
    );
    // The first items stay whole; the rest are counted.
    assert_eq!(read["results"][0]["name"], "Part 0");
    assert!(text.contains("more"), "{text}");
    // A reply that fits is as it was.
    assert_eq!(shrink(&json!({"ok": 1}), 4_000), r#"{"ok":1}"#);
}

#[test]
fn bulky_replies_read_and_moved_past_are_cleared_together_and_name_their_call() {
    let policy = Policy {
        batch_chars: 20_000,
        ..Policy::CHAT
    };
    let search = json!({"name": "library.search", "params": {"query": "resistor"}});
    let mut history = vec![
        user("Build it."),
        calls(&[("a", "query", search.clone())]),
        results(&[("a", bulky(120))]),
        calls(&[("b", "query", json!({"name": "project.issues"}))]),
        results(&[("b", "{\"open\":0}".into())]),
        calls(&[("c", "query", search)]),
        results(&[("c", bulky(120))]),
    ];
    // One bulky reply behind the kept rounds is not yet enough.
    assert_eq!(clear_settled(&mut history, &policy), 0);
    history.push(calls(&[("d", "query", json!({"name": "project.issues"}))]));
    history.push(results(&[("d", "{\"open\":0}".into())]));
    history.push(calls(&[("e", "query", json!({"name": "project.issues"}))]));
    history.push(results(&[("e", "{\"open\":0}".into())]));
    // Two are: both go at once; small replies and the kept rounds stay.
    assert_eq!(clear_settled(&mut history, &policy), 2);
    let texts = texts(&history);
    assert!(texts[0].starts_with(STUB), "{}", texts[0]);
    assert!(
        texts[0].contains(r#"query {"name":"library.search""#),
        "{}",
        texts[0]
    );
    assert_eq!(texts[1], "{\"open\":0}");
    assert!(texts[2].starts_with(STUB));
    assert_eq!(texts[4], "{\"open\":0}");
    // Cleared once only.
    assert_eq!(clear_settled(&mut history, &policy), 0);
}

#[test]
fn condensing_keeps_the_ids_of_edits_and_the_person_s_words_and_drops_listings() -> Fallible {
    let place =
        json!({"expected_revision": 3, "operations": [{"op": "component.place", "params": {}}]});
    let placed = json!({"revision": 4, "outcomes": [{"instance_placed": {
        "instance": 12, "occurrence": 20, "designator": "U1"}}], "padding": "y".repeat(800)});
    let mut history = vec![
        user("The window shows: {…}\n\nPlace U1 and route it."),
        calls(&[("a", "query", json!({"name": "project.inspect"}))]),
        results(&[("a", bulky(60))]),
        calls(&[("b", "apply", place)]),
        results(&[("b", placed.to_string())]),
        answer("Placed U1."),
        user("Now route."),
        calls(&[("c", "query", json!({"name": "project.issues"}))]),
        results(&[("c", bulky(10))]),
    ];
    let estimate = |h: &[Message]| {
        h.iter()
            .map(|m| crate::model::appended_chars(&m.content))
            .sum::<u64>()
            / 3
    };
    // Below the trigger nothing changes.
    let before = history.clone();
    assert_eq!(
        condense(&mut history, &Policy::CHAT, 1_000_000, &estimate),
        None
    );
    assert_eq!(history, before);
    // The last round (the issues query) stays.
    let policy = Policy {
        keep_rounds: 1,
        ..Policy::CHAT
    };
    let condensed = condense(&mut history, &policy, 4_000, &estimate).ok_or("not condensed")?;
    assert_eq!(condensed.cleared, 2);
    assert!(condensed.after_tokens < condensed.before_tokens);
    let record = condensed.record.unwrap_or_default();
    assert!(record.contains("Place U1 and route it."), "{record}");
    assert!(!record.contains("The window shows"), "{record}");
    assert!(record.contains("designator \"U1\""), "{record}");
    assert!(record.contains("occurrence 20"), "{record}");
    // The edit's stub keeps what it made; the person's words stay.
    let texts = texts(&history);
    assert!(
        texts[1].starts_with(STUB) && texts[1].contains("instance 12"),
        "{}",
        texts[1]
    );
    assert_eq!(history[6], user("Now route."));
    Ok(())
}

/// A model that summarises.
struct Summariser(Option<String>);

impl ModelBackend for Summariser {
    fn label(&self) -> String {
        "summariser".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> crate::Result<ModelReply> {
        assert!(request.tools.is_empty());
        self.0 = request
            .messages
            .first()
            .and_then(|m| match m.content.first() {
                Some(Part::Text { text, .. }) => Some(text.clone()),
                _ => None,
            });
        Ok(ModelReply {
            model: "s".into(),
            content: vec![Part::text("The person wants a 3.3 V supply; U1 is placed.")],
            stop: StopReason::EndTurn,
            usage: Usage::default(),
            usage_estimated: false,
        })
    }
    fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> crate::Result<Option<u64>> {
        Ok(None)
    }
}

#[test]
fn a_summary_replaces_the_earlier_turns_with_the_record_and_keeps_the_latest() -> Fallible {
    let mut history = vec![
        user("Build a 3.3 V supply."),
        calls(&[("a", "apply", json!({"operations": []}))]),
        results(&[(
            "a",
            json!({"outcomes": [{"designator": "U1", "instance": 12}]}).to_string(),
        )]),
        answer("Placed U1."),
        user("Now route it."),
    ];
    let mut model = Summariser(None);
    let done = summarise(
        &mut history,
        "Condense.",
        1_000,
        Duration::from_secs(5),
        |request| model.send(request).ok(),
    );
    let (turns, kept) = done.ok_or("no summary")?;
    assert_eq!(turns, 1);
    assert!(
        kept.contains("U1 is placed") && kept.contains("instance 12"),
        "{kept}"
    );
    // One message: the record, then the latest words, unchanged.
    assert_eq!(history.len(), 1);
    assert!(matches!(&history[0].content[0], Part::Text { text, .. } if text.starts_with(RECORD)));
    assert!(matches!(&history[0].content[1], Part::Text { text, .. } if text == "Now route it."));
    // The summariser read the record and the transcript.
    assert!(model.0.is_some_and(|t| t.contains("Build a 3.3 V supply.")));
    // A later summary carries the earlier record's lines.
    history.push(answer("Routed."));
    history.push(user("Check it."));
    let again = summarise(
        &mut history,
        "Condense.",
        1_000,
        Duration::from_secs(5),
        |request| Summariser(None).send(request).ok(),
    );
    assert!(again.is_some_and(
        |(_, kept)| kept.contains("instance 12") && kept.contains("Build a 3.3 V supply.")
    ));
    Ok(())
}
