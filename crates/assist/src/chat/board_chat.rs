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
}

impl ModelBackend for Scripted {
    fn label(&self) -> String {
        "scripted board".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let chars = request.system.len()
            + serde_json::to_string(request.tools).map_or(0, |t| t.len())
            + serde_json::to_string(request.messages).map_or(0, |t| t.len());
        self.sent.push(chars);
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
    ("resistor 0603 1k", "R1"),
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
            // The fab rules: a 135 KB listing, like the bulky replies run
            // 12's model read.
            ("query", json!({"name": "rules.fab_profiles", "params": {}})),
            ("describe", json!({"name": "component.place"})),
            (
                "find",
                json!({"text": "place a component on the schematic"}),
            ),
        ]
    }));
    // 2. Search for the parts.
    out.push(Box::new(|_| PARTS.iter().map(|(w, _)| search(w)).collect()));
    // 3. Make the generic parts the search offers: each make goes alone,
    // one revision after the other.
    out.push(Box::new(move |m| {
        let start = revision(m).as_u64().unwrap_or_default();
        (0..PARTS.len())
            .map(|k| {
                let make = reply_to(m, &format!("c2_{k}"))["generic"][0]["make"].clone();
                (
                    "apply",
                    json!({"expected_revision": start + k as u64, "operations": [make]}),
                )
            })
            .collect()
    }));
    // 4. Search again: now held, with their place data.
    out.push(Box::new(|_| PARTS.iter().map(|(w, _)| search(w)).collect()));
    // 5. Place them; position left to the sheet.
    out.push(Box::new(move |m| {
        let ops = PARTS
            .iter()
            .enumerate()
            .map(|(k, (_, designator))| {
                let hint = &reply_to(m, &format!("c4_{k}"))["generic"][0];
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
    // 6. Read the instances' terminals.
    let l = learnt.clone();
    out.push(Box::new(move |m| {
        let placed = reply_to(m, "c5_0");
        let instances: Vec<Value> = placed["outcomes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|o| o["instance"].clone())
            .collect();
        l.borrow_mut()["instances"] = json!(instances);
        instances
            .iter()
            .map(|i| {
                (
                    "query",
                    json!({"name": "component.inspect", "params": {"instance": i}}),
                )
            })
            .collect()
    }));
    // 7. Wire: J1.1 to R1.1 (VIN), R1.2 to D1 anode, D1 cathode to J1.2.
    let l = learnt.clone();
    out.push(Box::new(move |m| {
        let ids = l.borrow()["instances"].clone();
        let terminal = |k: usize, label: &str| -> Value {
            let report = reply_to(m, &format!("c6_{k}"));
            let functions = report["result"]["functions"]["active"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            functions
                .iter()
                .find(|f| f["label"] == label || f["name"] == label)
                .map(|f| f["terminal"].clone())
                .unwrap_or_else(|| json!(if label == "A" || label == "2" { 2 } else { 1 }))
        };
        let end = |k: usize, t: Value| json!({"instance": ids[k], "terminal": t});
        let wires = [
            (end(0, json!(1)), end(1, json!(1))),
            (end(1, json!(2)), end(2, terminal(2, "A"))),
            (end(2, terminal(2, "K")), end(0, json!(2))),
        ];
        let ops = wires
            .into_iter()
            .map(
                |(from, to)| json!({"op": "schematic.connect", "params": {"from": from, "to": to}}),
            )
            .collect();
        vec![apply(m, ops)]
    }));
    // 8. The board and Update PCB.
    out.push(Box::new(move |m| {
        vec![apply(
            m,
            vec![
                json!({"op": "layout.set_board_size", "params": {"size": [30.0, 20.0]}}),
                json!({"op": "layout.sync", "params": {}}),
            ],
        )]
    }));
    // 9. Place the parts close together.
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
    // 10. Where the pads are.
    out.push(Box::new(|_| {
        vec![("query", json!({"name": "layout.pads", "params": {}}))]
    }));
    // 11. Route each net with L-shaped legs between its pads.
    out.push(Box::new(move |m| {
        let pads = reply_to(m, "c10_0");
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
    // 12. A GND fill on the bottom, on the net of J1's pad 2 (from the
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
    // 13. The check.
    out.push(Box::new(|_| {
        vec![(
            "query",
            json!({"name": "project.issues", "params": {"scope": {"kind": "all"}}}),
        )]
    }));
    out
}

/// A build: estimated prompts, characters sent, the session, refused calls.
type Built = (Vec<u64>, Vec<usize>, Session, Vec<String>);

/// One run of the script on a fresh project under `policy`.
fn build(policy: Policy) -> Fallible<Built> {
    let library = ChatLibrary::locate()?;
    let mut config = ChatConfig::new(
        &library,
        crate::model::ModelProfile::anthropic("claude-opus-5-5"),
        unbounded(),
    );
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
    };
    let mut prompts = Vec::new();
    let outcome = turn(
        &config,
        &mut model,
        &mut Vec::new(),
        vec![Part::text(
            "The window shows: {\"workspace\": \"schematic\"}\n\nBuild a small board: a 2-pin header powers a red 0603 LED through a 1k 0603 resistor. Put them on the schematic, update the PCB, place them close together, route every net, pour GND on the bottom and check it.",
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
    Ok((prompts, model.sent, session, refused))
}

#[test]
fn the_chat_builds_a_small_board_routed_and_checked_in_small_requests() -> Fallible {
    let (prompts, sent, session, refused) = build(Policy::CHAT)?;
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
    assert_eq!(report["result"]["counts"]["warning"], 0, "{report}");
    let rows = report["result"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        rows.iter()
            .all(|r| r["finding"]["code"] == "purchasing.no_part"),
        "{report}"
    );
    let (before, sent_before, _, _) = build(Policy::KEEP_ALL)?;
    println!("request  estimated prompt tokens: K1 policy | kept whole (before)");
    for (n, (now, then)) in prompts.iter().zip(&before).enumerate() {
        println!("{:>7}  {now:>8} | {then:>8}", n + 1);
    }
    let total = |v: &[u64]| v.iter().sum::<u64>();
    println!(
        "  total  {:>8} | {:>8}   (largest {} | {})",
        total(&prompts),
        total(&before),
        prompts.iter().max().unwrap_or(&0),
        before.iter().max().unwrap_or(&0)
    );
    println!(
        "characters sent: {} | {}",
        sent.iter().sum::<usize>(),
        sent_before.iter().sum::<usize>()
    );
    // The target: a request of a board build stays under about 25 k tokens.
    assert!(prompts.iter().all(|p| *p < 25_000), "{prompts:?}");
    Ok(())
}
