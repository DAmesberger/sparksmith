//! "Build a small board" through the chat's loop (`turn`) with the real
//! verbs on a fresh project and a scripted model that, like a model, takes
//! every id from the replies it was sent: it explores as run 12's model
//! did, searches the library for generic parts, makes and places them,
//! wires them, updates the PCB, places, routes, pours and checks. The board
//! ends routed with a passing check.
//!
//! The same script runs twice: with the chat's policy and with nothing
//! shortened or cleared (the chat before K1), and the test prints each
//! request's estimated prompt (`--nocapture`), so the two can be compared.
use super::*;
use crate::board_script::wiring::{Verbs, placed};
use crate::board_script::{Scripted, Step, fresh, pin, reply_to, revision};
use crate::verbs::Agent;
use serde_json::json;
use sparksmith_automation::Session;
use sparksmith_automation::front::NoOne;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A build: estimated prompts, characters sent, the session, refused calls.
/// A scripted build: each request's estimated prompt tokens, its
/// characters and those of them a prompt cache could serve, the session
/// and the refused calls.
struct Built {
    prompts: Vec<u64>,
    sent: Vec<usize>,
    cached: Vec<usize>,
    session: Session,
    refused: Vec<String>,
    answer: String,
}

/// One run of the script on a fresh project under `policy`.
fn build(policy: Policy) -> Fallible<Built> {
    let library = ChatLibrary::locate()?;
    let mut config = ChatConfig::new(
        &library,
        crate::model::ModelProfile::anthropic("claude-opus-5-5"),
        Budget::default(),
    )?;
    config.policy = policy;
    let mut session = fresh()?;
    let grant = library.reach.grant();
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let mut model = Scripted::board_build();
    let mut prompts = Vec::new();
    let mut answer = String::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut Vec::new(),
        vec![Part::text(
            "The window shows: {\"workspace\": \"schematic\"}\n\nBuild a small board: a 2-pin header powers a red 0603 LED through a 4.7k 0603 resistor. Put them on the schematic, update the PCB, place them close together, route every net, pour GND on the bottom and check it.",
        )],
        &mut tools,
        &mut |e| match e {
            Event::Request { prompt_tokens, .. } => prompts.push(prompt_tokens),
            Event::Text(text) => answer = text,
            _ => (),
        },
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    let refused = tools.refused;
    Ok(Built {
        answer,
        prompts,
        sent: model.sent,
        cached: model.cached,
        session,
        refused,
    })
}

#[test]
fn the_chat_builds_a_small_board_routed_and_checked_in_small_requests() -> Fallible {
    let Built {
        answer,
        prompts,
        sent,
        cached,
        session,
        refused,
    } = build(crate::chat::tests::config()?.policy)?;
    assert!(refused.is_empty(), "{refused:#?}");
    let project = session.engine().project();
    assert_eq!(project.instances.len(), 3);
    assert_eq!(project.tracks.len(), 7);
    assert_eq!(project.fills.len(), 1);
    // Every placed symbol is inside its sheet's frame.
    for occurrence in project.occurrences.iter() {
        let sheet = project.sheet(occurrence.sheet).ok_or("no sheet")?;
        let [left, top] = sheet.top_left();
        let [width, height] = sheet.size_mm();
        let [x, y] = occurrence.position.to_mm_f64();
        assert!(
            x > left && x < left + width && y < top && y > top - height,
            "({x}, {y}) is off the sheet"
        );
    }
    let report = session.engine().query(sparksmith_core::Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    })?;
    let report = serde_json::to_value(report)?;
    assert_eq!(report["result"]["counts"]["error"], 0, "{report}");
    assert_eq!(report["result"]["counts"]["to_do"], 3, "{report}");
    assert_eq!(
        answer,
        "Checked the board. Open findings 4 (errors 0, warnings 1, to-dos 3, informational 0)."
    );
    // One warning: the board has surface-mount parts and no fiducials,
    // which its PCBWay profile asks three of for assembly.
    assert_eq!(report["result"]["counts"]["warning"], 1, "{report}");
    let rows = report["result"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        rows.iter()
            .all(|r| r["finding"]["code"] == "purchasing.no_part"
                || r["finding"]["code"] == "board.fiducials_too_few"),
        "{report}"
    );
    // The values the search gave went onto the placed parts: the BOM reads
    // them.
    let bom = sparksmith_core::completeness::bom(project)?;
    let value_of = |designator: &str| {
        bom.lines
            .iter()
            .find(|l| l.designators.iter().any(|d| d == designator))
            .map(|l| l.values.iter().map(|v| v.text.as_str()).collect::<Vec<_>>())
    };
    assert_eq!(value_of("R1"), Some(vec!["4.7 kΩ"]), "{bom:?}");
    assert_eq!(value_of("D1"), Some(vec!["Red"]), "{bom:?}");
    let csv = sparksmith_core::fabrication::assembly::bom_csv(project)?;
    assert!(
        csv.lines()
            .any(|l| l.contains("R1") && l.contains("4.7 kΩ")),
        "{csv}"
    );
    let whole = build(Policy::KEEP_ALL)?;
    // Tokens a prompt cache cannot serve: what differs from the request
    // before, at 3 characters a token.
    let uncached = |sent: &[usize], cached: &[usize]| -> Vec<usize> {
        sent.iter()
            .zip(cached)
            .map(|(s, c)| crate::usage::estimated_tokens(s.saturating_sub(*c) as u64) as usize)
            .collect()
    };
    let fresh = uncached(&sent, &cached);
    let fresh_whole = uncached(&whole.sent, &whole.cached);
    println!(
        "request  estimated prompt tokens, K1 policy | kept whole; not served by a prompt cache, K1 | kept whole"
    );
    for n in 0..prompts.len() {
        println!(
            "{:>7}  {:>8} | {:>8}   {:>8} | {:>8}",
            n + 1,
            prompts[n],
            whole.prompts.get(n).unwrap_or(&0),
            fresh[n],
            fresh_whole.get(n).unwrap_or(&0)
        );
    }
    let total = |v: &[u64]| v.iter().sum::<u64>();
    let total_fresh = |v: &[usize]| v.iter().sum::<usize>();
    println!(
        "  total  {:>8} | {:>8}   {:>8} | {:>8}   (largest {} | {})",
        total(&prompts),
        total(&whole.prompts),
        total_fresh(&fresh),
        total_fresh(&fresh_whole),
        prompts.iter().max().unwrap_or(&0),
        whole.prompts.iter().max().unwrap_or(&0)
    );
    println!(
        "characters sent: {} | {}",
        sent.iter().sum::<usize>(),
        whole.sent.iter().sum::<usize>()
    );
    // The target: a request of a board build stays under about 25 k tokens.
    assert!(prompts.iter().all(|p| *p < 25_000), "{prompts:?}");
    // Measured 22 November 2026: 166 198 prompt tokens, 25 852 of them past
    // the cacheable prefix, before the generators' compact listing, the
    // scoped pins read and the earlier clearing; 138 942 and 19 817 after.
    assert!(total(&prompts) < 145_000, "{}", total(&prompts));
    assert!(total_fresh(&fresh) < 21_000, "{}", total_fresh(&fresh));
    Ok(())
}

/// Earlier turns of a long conversation: `n` messages that each read a
/// 12 000-character reply, about 4 000 tokens apiece.
fn long_past(history: &mut Vec<Message>, n: usize) {
    for k in 0..n {
        let id = format!("old{k}");
        history.push(Message {
            role: Role::User,
            content: vec![Part::text(format!("Earlier question {k}."))],
        });
        history.push(Message {
            role: Role::Assistant,
            content: vec![Part::ToolCall {
                id: id.clone(),
                name: "query".into(),
                input: json!({"name": "project.inspect", "params": {"at": "/project/instances"}}),
                malformed: None,
                replay: None,
            }],
        });
        history.push(Message {
            role: Role::User,
            content: vec![Part::ToolResult {
                call_id: id,
                text: format!("{{\"rows\": \"{}\"}}", "x".repeat(12_000)),
                error: false,
                images: Vec::new(),
            }],
        });
        history.push(Message {
            role: Role::Assistant,
            content: vec![Part::text("Done.")],
        });
    }
}

#[test]
fn wiring_placed_parts_takes_one_read_and_one_apply_on_a_long_conversation() -> Fallible {
    let (config, mut session, mut history) = placed()?;
    // About 80 000 tokens of earlier turns: a 1 000 000-token model's
    // trigger alone would never condense them.
    long_past(&mut history, 20);
    let grant = ChatLibrary::locate()?.reach.grant();
    let mut steps: Vec<Step> = Vec::new();
    // 1. The parts' pins, one compact read.
    steps.push(Box::new(|_| {
        vec![(
            "query",
            json!({"name": "schematic.pins", "params": {"designators": ["J1", "R1", "D1"]}}),
        )]
    }));
    // 2. One apply: J1.1 to R1.1, R1.2 to the LED's anode, its cathode to J1.2.
    steps.push(Box::new(|m| {
        let parts = reply_to(m, "c1_0")["result"].clone();
        let end = |k: usize, name: &str| {
            json!({"instance": parts[k]["instance"], "terminal": pin(&parts[k], name)})
        };
        let wires = [
            (end(0, "1"), end(1, "1")),
            (end(1, "2"), end(2, "A")),
            (end(2, "K"), end(0, "2")),
        ];
        let ops: Vec<Value> = wires
            .into_iter()
            .map(|(from, to)| json!({"op": "schematic.connect", "params": {"from": from, "to": to}}))
            .collect();
        vec![(
            "apply",
            json!({"expected_revision": revision(m), "operations": ops}),
        )]
    }));
    let mut model = Scripted::from_checked_steps(steps);
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let mut condensed = None;
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text(
            "The window shows: {\"workspace\": \"schematic\"}\n\nWire them: the header powers the LED through the resistor.",
        )],
        &mut tools,
        &mut |e| {
            if let Event::Condensed(c) = e {
                condensed.get_or_insert(c);
            }
        },
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered, "{:#?}", tools.refused);
    assert!(tools.refused.is_empty(), "{:#?}", tools.refused);
    println!(
        "wiring turn: {} requests, {} tokens",
        outcome.usage.requests, outcome.usage.tokens
    );
    // The earlier turns were condensed before the first request.
    let condensed = condensed.ok_or("the long past was not condensed")?;
    assert!(condensed.cleared >= 20, "{condensed:?}");
    assert!(condensed.after_tokens < 56_000, "{condensed:?}");
    // Two tool requests and the answer, in a small budget: about 80 000
    // tokens a request was re-sent before (E2E run 13).
    assert_eq!(outcome.usage.requests, 3);
    assert!(
        outcome.usage.tokens < 80_000,
        "{} tokens: {:?}",
        outcome.usage.tokens,
        model.sent
    );
    // The three nets join what was asked.
    let project = session.engine().project();
    let joined = project
        .nets
        .iter()
        .filter(|n| n.terminals.len() == 2)
        .count();
    assert_eq!(joined, 3, "{:?}", project.nets);
    Ok(())
}

/// A turn that reads without changing anything is nudged, not stopped:
/// past the policy's allowance the harness tells the model, with the next
/// tool results, to act or ask the person, and the turn goes on to the
/// model's answer.
#[test]
fn reading_without_a_change_nudges_the_model_and_the_turn_goes_on() -> Fallible {
    let (config, mut session, mut history) = placed()?;
    let grant = ChatLibrary::locate()?.reach.grant();
    let nudge = config.nudge.clone();
    // The model reads until it is nudged, then answers.
    let steps: Vec<Step> = (0..200)
        .map(|_| -> Step {
            let nudge = nudge.clone();
            Box::new(move |m: &[Message]| {
                let nudged = m.last().is_some_and(|last| {
                    last.content
                        .iter()
                        .any(|p| matches!(p, Part::Text { text, .. } if *text == nudge))
                });
                if nudged {
                    Vec::new()
                } else {
                    vec![(
                        "query",
                        json!({"name": "project.inspect", "params": {"at": "/project"}}),
                    )]
                }
            })
        })
        .collect();
    let mut model = Scripted::from_steps(steps);
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let mut nudges = Vec::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text("Wire the reset, boot, LED and I2C circuits.")],
        &mut tools,
        &mut |e| {
            if let Event::Nudged { tokens, .. } = e {
                nudges.push(tokens);
            }
        },
        &AtomicBool::new(false),
    );
    assert_eq!(
        outcome.end,
        End::Answered,
        "the turn is not ended by reading"
    );
    assert!(nudge.contains("assist.ask"), "{nudge}");
    let allowance = config.policy.nudge_tokens;
    assert_eq!(nudges.len(), 1, "{nudges:?}");
    assert!(nudges[0] > allowance, "{nudges:?}");
    // The model answered on the request after the nudge.
    assert!(
        outcome.usage.tokens < allowance + 60_000,
        "{} tokens",
        outcome.usage.tokens
    );
    Ok(())
}

/// The generators' listing fits in one reply the chat keeps whole: it was
/// about 14 000 characters with every preset key and was cut at 12 000.
#[test]
fn the_generators_listing_is_not_cut_in_the_chat() -> Fallible {
    let mut session = fresh()?;
    let grant = ChatLibrary::locate()?.reach.grant();
    let answer = Agent {
        session: &mut session,
        grant: &grant,
        window: None,
        turn: None,
        asker: &mut NoOne,
    }
    .verb("query", json!({"name": "library.generators", "params": {}}));
    assert_eq!(answer["ok"], true, "{answer}");
    let size = answer["result"].to_string().len();
    assert!(
        size < crate::chat::tests::config()?.policy.reply_chars,
        "{size} characters"
    );
    Ok(())
}
