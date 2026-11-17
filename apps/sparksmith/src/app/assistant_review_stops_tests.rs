//! Scripted replays through the real worker, front door and question UI.
//! The script supplies the policy decisions: these tests prove transport,
//! pauses and edits, not that any live model chooses the promised stops.
//! Small edits to the demo stand in for schematic and routing stages;
//! this is not evidence of a complete model-designed board.
use super::*;

fn tool(id: &str, verb: &str, input: Value) -> Part {
    Part::ToolCall {
        id: id.into(),
        name: verb.into(),
        input,
        malformed: None,
        replay: None,
    }
}

fn review(id: &str, summary: &str, accept: &str) -> ModelReply {
    let value = if accept == "Build it" {
        "build"
    } else {
        "continue"
    };
    reply(
        vec![
            Part::text(summary),
            tool(
                id,
                "show",
                json!({"name": "assist.ask", "params": {
                    "title": "Review before continuing",
                    "questions": [{"id": "decision", "title": "Next step", "type": "single",
                        "recommended": value, "options": [
                            {"value": value, "title": accept},
                            {"value": "change", "title": "Change something"}
                        ]}]
                }}),
            ),
        ],
        StopReason::ToolCalls,
    )
}

fn edit(id: &str, revision: u64, op: &str, params: Value) -> ModelReply {
    reply(
        vec![tool(
            id,
            "apply",
            json!({"expected_revision": revision,
            "operations": [{"op": op, "params": params}]}),
        )],
        StopReason::ToolCalls,
    )
}

fn install(ax: &mut Ax, sent: &Sent, replies: Vec<ModelReply>) {
    let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
    let sent = Arc::clone(sent);
    ax.app.assistant.model = Some(ChatModel {
        profile: ModelProfile::openai_compatible("scripted", 32_768),
        connect: Arc::new(move || {
            Ok(Box::new(Script {
                replies: Arc::clone(&replies),
                sent: Arc::clone(&sent),
            }) as Box<dyn ModelBackend>)
        }),
    });
}

fn result(sent: &Sent, id: &str) -> Fallible<Value> {
    let sent = sent.lock().map_err(|e| e.to_string())?;
    let body = sent
        .iter()
        .flat_map(|(_, messages)| messages)
        .flat_map(|m| &m.content)
        .find_map(|p| match p {
            Part::ToolResult { call_id, text, .. } if call_id == id => Some(text),
            _ => None,
        })
        .ok_or_else(|| format!("no result for {id}"))?;
    Ok(serde_json::from_str(body)?)
}

/// Keep driving frames while a card is unanswered: no additional model
/// request or document mutation may get through the worker's pending call.
fn stays_paused(ax: &mut Ax, sent: &Sent, requests: usize, accept: &str) -> Fallible {
    let revision = ax.app.engine.revision();
    let before = serde_json::to_value(ax.app.engine.project())?;
    for _ in 0..12 {
        ax.frame(vec![]);
        std::thread::yield_now();
    }
    assert!(asking(ax));
    ax.find(Role::RadioButton, accept)?;
    ax.find(Role::RadioButton, "Change something")?;
    let button = ax.find(Role::Button, "Answer")?;
    assert!(button.rect.right() <= ax.window.x && button.rect.bottom() <= ax.window.y);
    assert_eq!(sent.lock().map_err(|e| e.to_string())?.len(), requests);
    assert_eq!(ax.app.engine.revision(), revision);
    assert_eq!(serde_json::to_value(ax.app.engine.project())?, before);
    Ok(())
}

#[test]
fn scripted_plan_schematic_and_routing_reviews_pause_then_resume_real_edits() -> Fallible {
    for size in [SMALL, LARGE] {
        let (mut ax, sent) = chat(size, vec![])?;
        let start = ax.app.engine.revision();
        let before = serde_json::to_value(ax.app.engine.project())?;
        let net = ax.app.engine.project().nets.first().ok_or("no net")?.id;
        let vias = ax.app.engine.project().vias.len();
        install(
            &mut ax,
            &sent,
            vec![
                review(
                    "plan",
                    "I will name the signal net REVIEW_SIGNAL, then add a via for it.",
                    "Build it",
                ),
                edit(
                    "schematic",
                    start,
                    "schematic.net_rename",
                    json!({"net": net, "name": "REVIEW_SIGNAL"}),
                ),
                review(
                    "schematic_review",
                    "The net is now REVIEW_SIGNAL. Its existing connections are unchanged.",
                    "Continue to the board",
                ),
                edit(
                    "routing",
                    start + 1,
                    "layout.add_via",
                    json!({"via": {
                "net": net, "position": [1.0, 1.0], "drill": 0.3, "diameter": 0.6,
                "span": "through", "tented": true}}),
                ),
                review(
                    "board_review",
                    "Added one through via to REVIEW_SIGNAL. Review its position before finishing.",
                    "Looks good",
                ),
                text("The requested changes are reviewed."),
            ],
        );
        send(
            &mut ax,
            "Rename the signal net and add a via; show me the plan first.",
        );
        ax.until("plan confirmation", asking)?;
        stays_paused(&mut ax, &sent, 1, "Build it")?;
        assert!(
            ax.texts
                .iter()
                .any(|t| t.contains("then add a via for it."))
        );
        assert_eq!(serde_json::to_value(ax.app.engine.project())?, before);
        ax.click(Role::Button, "Answer")?;
        ax.until("schematic review", |ax| {
            asking(ax) && ax.app.engine.revision() == start + 1
        })?;
        stays_paused(&mut ax, &sent, 3, "Continue to the board")?;
        assert!(
            ax.texts
                .iter()
                .any(|t| t.contains("Its existing connections are unchanged."))
        );
        assert_eq!(
            ax.app.engine.project().nets.first().ok_or("no net")?.name,
            "REVIEW_SIGNAL"
        );
        assert_eq!(ax.app.engine.project().vias.len(), vias);
        assert_eq!(result(&sent, "plan")?["answers"]["decision"], "build");
        assert_eq!(result(&sent, "schematic")?["revision"], start + 1);
        // A plain answer goes back to the pending call, not a new turn.
        send(&mut ax, "Continue to the board.");
        ax.until("board review", |ax| {
            asking(ax) && ax.app.engine.revision() == start + 2
        })?;
        stays_paused(&mut ax, &sent, 5, "Looks good")?;
        assert!(
            ax.texts
                .iter()
                .any(|t| t.contains("Review its position before finishing."))
        );
        assert_eq!(ax.app.engine.project().vias.len(), vias + 1);
        assert_eq!(
            result(&sent, "schematic_review")?["said"],
            "Continue to the board."
        );
        assert_eq!(result(&sent, "routing")?["revision"], start + 2);
        ax.click(Role::Button, "Answer")?;
        ax.until("reviewed turn ending", idle)?;
        assert_eq!(
            result(&sent, "board_review")?["answers"]["decision"],
            "continue"
        );
        assert_eq!(
            ax.app
                .assistant
                .conversation
                .items
                .iter()
                .filter(|i| matches!(i, Item::You { .. }))
                .count(),
            1
        );
        // Pauses and answers preserve the original turn's one-step undo.
        let mut undone = ax.app.engine.fork();
        undone.undo()?;
        assert_eq!(serde_json::to_value(undone.project())?, before);
        let sent = sent.lock().map_err(|e| e.to_string())?;
        let system = &sent.first().ok_or("no model request")?.0;
        for words in [
            "## Plan confirmation and review stops",
            "**Build it**",
            "**Change something**",
            "After the schematic",
            "After placement and routing",
            "go ahead without stopping",
            "A skipped card is not approval",
        ] {
            assert!(system.contains(words), "the delivered prompt lacks {words}");
        }
    }
    Ok(())
}

#[test]
fn scripted_change_request_and_skip_leave_the_plan_unbuilt() -> Fallible {
    let (mut ax, sent) = chat(
        SMALL,
        vec![
            review("plan", "I will add a via to the signal net.", "Build it"),
            text("What would you like to change?"),
            review(
                "revised_plan",
                "I will rename the signal net and leave the copper unchanged.",
                "Build it",
            ),
            text("I will wait for your go-ahead before changing the design."),
        ],
    )?;
    let before = serde_json::to_value(ax.app.engine.project())?;
    let revision = ax.app.engine.revision();
    send(&mut ax, "Add a via; show me the plan.");
    ax.until("plan confirmation", asking)?;
    stays_paused(&mut ax, &sent, 1, "Build it")?;
    ax.click(Role::RadioButton, "Change something")?;
    ax.click(Role::Button, "Answer")?;
    ax.until("change request", idle)?;
    assert_eq!(result(&sent, "plan")?["answers"]["decision"], "change");
    assert_eq!(serde_json::to_value(ax.app.engine.project())?, before);
    send(
        &mut ax,
        "Just rename the signal net; leave the copper unchanged.",
    );
    ax.until("revised plan confirmation", asking)?;
    stays_paused(&mut ax, &sent, 3, "Build it")?;
    ax.click(Role::Button, "Skip")?;
    ax.until("skipped plan ending", idle)?;
    assert_eq!(result(&sent, "revised_plan")?["skipped"], true);
    assert_eq!(ax.app.engine.revision(), revision);
    assert_eq!(serde_json::to_value(ax.app.engine.project())?, before);
    Ok(())
}

#[test]
fn scripted_explicit_no_stops_can_edit_without_a_review_card() -> Fallible {
    let (mut ax, sent) = chat(SMALL, vec![])?;
    let start = ax.app.engine.revision();
    let net = ax.app.engine.project().nets.first().ok_or("no net")?.id;
    install(
        &mut ax,
        &sent,
        vec![
            edit(
                "rename",
                start,
                "schematic.net_rename",
                json!({"net": net, "name": "REVIEW_SIGNAL"}),
            ),
            text("Renamed the net to REVIEW_SIGNAL."),
        ],
    );
    send(
        &mut ax,
        "Rename the first net REVIEW_SIGNAL; go ahead without stopping.",
    );
    ax.until("uninterrupted edit", idle)?;
    assert_eq!(ax.app.engine.revision(), start + 1);
    assert_eq!(result(&sent, "rename")?["revision"], start + 1);
    assert!(
        !ax.app
            .assistant
            .conversation
            .items
            .iter()
            .any(|i| matches!(i, Item::Question(_)))
    );
    Ok(())
}
