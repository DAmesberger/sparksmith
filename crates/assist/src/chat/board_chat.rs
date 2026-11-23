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
use super::board_trace::fresh;
use super::*;
use crate::model::{ModelReply, ModelRequest};
use crate::usage::Usage;
use crate::verbs::Agent;
use serde_json::json;
use sparksmith_automation::front::NoOne;
use sparksmith_automation::{Grant, Session};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The reply to call `id` in the last message, read as JSON.
fn reply_to(messages: &[Message], id: &str) -> Value {
    messages
        .last()
        .into_iter()
        .flat_map(|m| &m.content)
        .find_map(|p| match p {
            Part::ToolResult { call_id, text, .. } if call_id == id => {
                Some(serde_json::from_str(text).unwrap_or(Value::Null))
            }
            _ => None,
        })
        .unwrap_or(Value::Null)
}

/// The revision of the latest reply that names one, as a model reads it.
fn revision(messages: &[Message]) -> Value {
    messages
        .iter()
        .rev()
        .flat_map(|m| m.content.iter().rev())
        .find_map(|p| match p {
            Part::ToolResult { text, .. } => serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.get("revision").cloned()),
            _ => None,
        })
        .unwrap_or(Value::Null)
}

type Step = Box<dyn FnMut(&[Message]) -> Vec<(&'static str, Value)>>;

/// A model that answers from steps, each reading the replies it is sent.
struct Scripted {
    steps: std::collections::VecDeque<Step>,
    n: u32,
    /// Characters of each request: system, tools and messages.
    sent: Vec<usize>,
    /// Of those, the characters equal to the start of the request before
    /// it: what a prompt cache can serve (a prefix match, as a provider's
    /// cache is).
    cached: Vec<usize>,
    last: String,
}

impl ModelBackend for Scripted {
    fn label(&self) -> String {
        "scripted board".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let whole = format!(
            "{}{}{}",
            serde_json::to_string(request.tools).unwrap_or_default(),
            request.system,
            serde_json::to_string(request.messages).unwrap_or_default()
        );
        let chars = whole.len();
        if std::env::var_os("SPARKSMITH_RECORD_REQUEST_DIGESTS").is_some() {
            use sha2::{Digest, Sha256};
            println!(
                "request-sha256 {} {:x}",
                self.n + 1,
                Sha256::digest(whole.as_bytes())
            );
        }
        self.sent.push(chars);
        let same = whole
            .bytes()
            .zip(self.last.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        self.cached.push(same);
        self.last = whole;
        self.n += 1;
        let usage = Usage {
            input_tokens: (chars / 3) as u64,
            output_tokens: 200,
            ..Usage::default()
        };
        let Some(mut step) = self.steps.pop_front() else {
            return Ok(ModelReply {
                model: "scripted".into(),
                content: vec![Part::text("The board is built, routed and checked.")],
                stop: StopReason::EndTurn,
                usage,
                usage_estimated: false,
            });
        };
        let content = step(request.messages)
            .into_iter()
            .enumerate()
            .map(|(k, (verb, input))| Part::ToolCall {
                id: format!("c{}_{k}", self.n),
                name: verb.into(),
                input,
                malformed: None,
                replay: None,
            })
            .collect();
        Ok(ModelReply {
            model: "scripted".into(),
            content,
            stop: StopReason::ToolCalls,
            usage,
            usage_estimated: false,
        })
    }
    fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> Result<Option<u64>> {
        Ok(None)
    }
}

struct Verbs<'a> {
    session: &'a mut Session,
    grant: &'a Grant,
    refused: Vec<String>,
}

impl Tools for Verbs<'_> {
    fn call(&mut self, verb: &str, input: &Value) -> Value {
        let answer = Agent {
            session: self.session,
            grant: self.grant,
            window: None,
            turn: None,
            asker: &mut NoOne,
        }
        .verb(verb, input.clone());
        if answer["ok"] != true {
            self.refused
                .push(format!("{verb} {input}: {}", answer["error"]));
        }
        answer
    }
}

/// The parts: search words, designator.
const PARTS: [(&str, &str); 3] = [
    ("header 1x2", "J1"),
    ("resistor 0603 4.7k", "R1"),
    ("led 0603 red", "D1"),
];

fn search(words: &str) -> (&'static str, Value) {
    (
        "query",
        json!({"name": "library.search", "params": {"query": words}}),
    )
}

/// The script: what a model sends, step by step.
fn steps() -> Vec<Step> {
    let apply = |messages: &[Message], ops: Vec<Value>| {
        (
            "apply",
            json!({"expected_revision": revision(messages), "operations": ops}),
        )
    };
    // The ids learnt so far, shared between steps as a model's memory of
    // what it read.
    let learnt = std::rc::Rc::new(std::cell::RefCell::new(json!({})));
    let mut out: Vec<Step> = Vec::new();
    // 1. Look around, as run 12's model did.
    out.push(Box::new(|_| {
        vec![
            ("query", json!({"name": "project.inspect", "params": {}})),
            ("query", json!({"name": "library.generators", "params": {}})),
            // The fab profiles: ids, options, choices and defaults.
            ("query", json!({"name": "rules.fab_profiles", "params": {}})),
            ("describe", json!({"name": "component.place"})),
            (
                "find",
                json!({"text": "place a component on the schematic"}),
            ),
        ]
    }));
    // 2. The fab profile by the id and option the listing gives, in one
    // step.
    out.push(Box::new(move |m| {
        vec![apply(
            m,
            vec![json!({"op": "rules.set_fab_profile", "params": {
                "profile": "pcbway-2-layer", "options": {"limits": "minimum"}}})],
        )]
    }));
    // 3. Search for the parts.
    out.push(Box::new(|_| PARTS.iter().map(|(w, _)| search(w)).collect()));
    // 4. Make the generic parts the search offers: each make goes alone,
    // one revision after the other.
    out.push(Box::new(move |m| {
        let start = revision(m).as_u64().unwrap_or_default();
        (0..PARTS.len())
            .map(|k| {
                let make = reply_to(m, &format!("c3_{k}"))["generic"][0]["make"].clone();
                (
                    "apply",
                    json!({"expected_revision": start + k as u64, "operations": [make]}),
                )
            })
            .collect()
    }));
    // 5. Search again: now held, with their place data.
    out.push(Box::new(|_| PARTS.iter().map(|(w, _)| search(w)).collect()));
    // 6. Place them; position left to the sheet.
    out.push(Box::new(move |m| {
        let ops = PARTS
            .iter()
            .enumerate()
            .map(|(k, (_, designator))| {
                let hint = &reply_to(m, &format!("c5_{k}"))["generic"][0];
                let mut params = json!({"component": hint["component"], "physical": hint["physical"],
                    "designator": designator});
                if let Some(properties) = hint.get("properties") {
                    params["properties"] = properties.clone();
                }
                json!({"op": "component.place", "params": params})
            })
            .collect();
        vec![apply(m, ops)]
    }));
    // 7. The placed parts' pins, in one scoped read (the playbook's
    // schematic.pins).
    let l = learnt.clone();
    out.push(Box::new(move |m| {
        let placed = reply_to(m, "c6_0");
        let instances: Vec<Value> = placed["outcomes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|o| o["instance"].clone())
            .collect();
        l.borrow_mut()["instances"] = json!(instances);
        let designators: Vec<&str> = PARTS.iter().map(|(_, d)| *d).collect();
        vec![(
            "query",
            json!({"name": "schematic.pins", "params": {"designators": designators}}),
        )]
    }));
    // 8. Wire: J1.1 to R1.1 (VIN), R1.2 to D1 anode, D1 cathode to J1.2.
    out.push(Box::new(move |m| {
        let parts = reply_to(m, "c7_0")["result"].clone();
        let end = |k: usize, name: &str| {
            json!({"instance": parts[k]["instance"], "terminal": pin(&parts[k], name)})
        };
        let wires = [
            (end(0, "1"), end(1, "1")),
            (end(1, "2"), end(2, "A")),
            (end(2, "K"), end(0, "2")),
        ];
        let ops = wires
            .into_iter()
            .map(
                |(from, to)| json!({"op": "schematic.connect", "params": {"from": from, "to": to}}),
            )
            .collect();
        vec![apply(m, ops)]
    }));
    // 9. The board and Update PCB.
    out.push(Box::new(move |m| {
        vec![apply(
            m,
            vec![
                json!({"op": "layout.set_board_size", "params": {"size": [30.0, 20.0]}}),
                json!({"op": "layout.sync", "params": {}}),
            ],
        )]
    }));
    // 10. Place the parts close together.
    let l = learnt.clone();
    out.push(Box::new(move |m| {
        let ids = l.borrow()["instances"].clone();
        let ops = [
            ([5.0, 10.0], 90.0),
            ([15.0, 6.0], 0.0),
            ([24.0, 10.0], 270.0),
        ]
        .iter()
        .enumerate()
        .map(|(k, (at, rotation))| {
            json!({"op": "layout.move", "params": {
                "instance": ids[k], "position": at, "rotation_deg": rotation, "layer": "Top"}})
        })
        .collect();
        vec![apply(m, ops)]
    }));
    // 11. Where the pads are.
    out.push(Box::new(|_| {
        vec![("query", json!({"name": "layout.pads", "params": {}}))]
    }));
    // 12. Route each net with L-shaped legs between its pads.
    out.push(Box::new(move |m| {
        let pads = reply_to(m, "c11_0");
        let pads = pads["result"].as_array().cloned().unwrap_or_default();
        let at = |designator: &str, pad: &str| -> (Value, [f64; 2]) {
            pads.iter()
                .find(|p| p["designator"] == designator && p["pad"] == pad)
                .map(|p| {
                    let xy = [
                        p["position"][0].as_f64().unwrap_or_default(),
                        p["position"][1].as_f64().unwrap_or_default(),
                    ];
                    (p["net"].clone(), xy)
                })
                .unwrap_or((Value::Null, [0.0, 0.0]))
        };
        let (vin, j1) = at("J1", "1");
        let (_, r1a) = at("R1", "1");
        let (led, r1b) = at("R1", "2");
        let (_, anode) = at("D1", "2");
        let (gnd, j2) = at("J1", "2");
        let (_, cathode) = at("D1", "1");
        let paths = [
            (vin, vec![j1, [j1[0], r1a[1]], r1a]),
            (led, vec![r1b, [anode[0], r1b[1]], anode]),
            (gnd, vec![j2, [j2[0], 14.0], [cathode[0], 14.0], cathode]),
        ];
        let mut ops = Vec::new();
        for (net, path) in paths {
            for leg in path.windows(2) {
                ops.push(json!({"op": "layout.route", "params": {"request": {
                    "net": net, "start": leg[0], "end": leg[1], "width": 0.25, "layer": "Top"}}}));
            }
        }
        vec![apply(m, ops)]
    }));
    // 13. A GND fill on the bottom, on the net of J1's pad 2 (from the
    // pads reply, two requests back).
    out.push(Box::new(move |m| {
        let gnd = m
            .iter()
            .rev()
            .flat_map(|m| &m.content)
            .find_map(|p| match p {
                Part::ToolResult { text, .. } => serde_json::from_str::<Value>(text)
                    .ok()
                    .and_then(|v| v["result"].as_array().cloned())
                    .and_then(|rows| {
                        rows.iter()
                            .find(|r| r["designator"] == "J1" && r["pad"] == "2")
                            .map(|r| r["net"].clone())
                    }),
                _ => None,
            })
            .unwrap_or(Value::Null);
        vec![apply(
            m,
            vec![
                json!({"op": "fill.add", "params": {"fill": {"layer": "Bottom", "net": gnd, "region": "board", "name": "GND"}}}),
                json!({"op": "fill.refill", "params": {"target": {"fills": "all"}}}),
            ],
        )]
    }));
    // 14. The check.
    out.push(Box::new(|_| {
        vec![(
            "query",
            json!({"name": "project.issues", "params": {"scope": {"kind": "all"}}}),
        )]
    }));
    out
}

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
}

/// One run of the script on a fresh project under `policy`.
fn build(policy: Policy) -> Fallible<Built> {
    let library = ChatLibrary::locate()?;
    let mut config = ChatConfig::new(
        &library,
        crate::model::ModelProfile::anthropic("claude-opus-5-5"),
        unbounded(),
    )?;
    config.policy = policy;
    let mut session = fresh()?;
    let grant = library.reach.grant();
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let mut model = Scripted {
        steps: steps().into(),
        n: 0,
        sent: Vec::new(),
        cached: Vec::new(),
        last: String::new(),
    };
    let mut prompts = Vec::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut Vec::new(),
        vec![Part::text(
            "The window shows: {\"workspace\": \"schematic\"}\n\nBuild a small board: a 2-pin header powers a red 0603 LED through a 4.7k 0603 resistor. Put them on the schematic, update the PCB, place them close together, route every net, pour GND on the bottom and check it.",
        )],
        &mut tools,
        &mut |e| {
            if let Event::Request { prompt_tokens, .. } = e {
                prompts.push(prompt_tokens);
            }
        },
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    let refused = tools.refused;
    Ok(Built {
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
            .map(|(s, c)| s.saturating_sub(*c).div_ceil(3))
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

/// A fresh project with J1, R1 and D1 placed by the script's first six
/// steps, and the conversation that placed them.
fn placed() -> Fallible<(ChatConfig, Session, Vec<Message>)> {
    let library = ChatLibrary::locate()?;
    let config = ChatConfig::new(
        &library,
        crate::model::ModelProfile::anthropic("claude-opus-5-5"),
        unbounded(),
    )?;
    let mut session = fresh()?;
    let grant = library.reach.grant();
    let mut history = Vec::new();
    let mut model = Scripted {
        steps: steps().into_iter().take(6).collect(),
        n: 0,
        sent: Vec::new(),
        cached: Vec::new(),
        last: String::new(),
    };
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let outcome = turn(
        &config,
        &mut model,
        &mut history,
        vec![Part::text(
            "Place a 2-pin header, a 4.7k 0603 resistor and a red 0603 LED.",
        )],
        &mut tools,
        &mut |_| {},
        &AtomicBool::new(false),
    );
    assert_eq!(outcome.end, End::Answered);
    assert!(tools.refused.is_empty(), "{:#?}", tools.refused);
    Ok((config, session, history))
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

/// The terminal of the pin named `name` in a `schematic.pins` part.
fn pin(part: &Value, name: &str) -> Value {
    part["pins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row[2] == name)
        .map_or(Value::Null, |row| row[0].clone())
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
    // 3. One check.
    steps.push(Box::new(|_| {
        vec![(
            "query",
            json!({"name": "project.issues", "params": {"scope": {"kind": "all"}}}),
        )]
    }));
    let mut model = Scripted {
        steps: steps.into(),
        n: 0,
        sent: Vec::new(),
        cached: Vec::new(),
        last: String::new(),
    };
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
    // Three tool requests and the answer, in a small budget: about 80 000
    // tokens a request was re-sent before (E2E run 13).
    assert_eq!(outcome.usage.requests, 4);
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
    let mut model = Scripted {
        steps: steps.into(),
        n: 0,
        sent: Vec::new(),
        cached: Vec::new(),
        last: String::new(),
    };
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
