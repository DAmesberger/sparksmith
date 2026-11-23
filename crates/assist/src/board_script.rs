//! Shared offline model scripts for the developer bench and chat regression tests.
use crate::Result;
use crate::model::{Message, ModelBackend, ModelReply, ModelRequest, Part, StopReason};
use crate::usage::Usage;
use serde_json::{Value, json};
use sparksmith_automation::Session;

/// The reply to call `id` in the last message, read as JSON.
pub(crate) fn reply_to(messages: &[Message], id: &str) -> Value {
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
pub(crate) fn revision(messages: &[Message]) -> Value {
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

pub(crate) type Step = Box<dyn FnMut(&[Message]) -> Vec<(&'static str, Value)>>;

/// A model that answers from steps, each reading the replies it is sent.
pub struct Scripted {
    steps: std::collections::VecDeque<Step>,
    n: u32,
    /// Characters of each request: system, tools and messages.
    pub sent: Vec<usize>,
    /// Of those, the characters equal to the start of the request before
    /// it: what a prompt cache can serve (a prefix match, as a provider's
    /// cache is).
    pub cached: Vec<usize>,
    last: String,
    answer: fn(&[Message]) -> Result<String>,
}

impl ModelBackend for Scripted {
    fn label(&self) -> String {
        "scripted board".into()
    }
    fn send(&mut self, request: &ModelRequest<'_>) -> Result<ModelReply> {
        let whole = format!(
            "{}{}{}",
            serde_json::to_string(request.tools)?,
            request.system,
            serde_json::to_string(request.messages)?
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
            input_tokens: crate::usage::estimated_tokens(chars as u64),
            output_tokens: 200,
            ..Usage::default()
        };
        let Some(mut step) = self.steps.pop_front() else {
            return Ok(ModelReply {
                model: "scripted".into(),
                content: vec![Part::text((self.answer)(request.messages)?)],
                stop: StopReason::EndTurn,
                usage,
                usage_estimated: true,
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
            usage_estimated: true,
        })
    }
    fn count_prompt_tokens(&mut self, _: &ModelRequest<'_>) -> Result<Option<u64>> {
        Ok(None)
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
pub(crate) fn steps() -> Vec<Step> {
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
    out
}

/// The terminal of the pin named `name` in a `schematic.pins` part.
pub(crate) fn pin(part: &Value, name: &str) -> Value {
    part["pins"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|row| row[2] == name)
        .map_or(Value::Null, |row| row[0].clone())
}

impl Scripted {
    pub(crate) fn from_steps(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            n: 0,
            sent: Vec::new(),
            cached: Vec::new(),
            last: String::new(),
            answer: |_| Ok("The board is built, routed and checked.".into()),
        }
    }

    pub fn board_build() -> Self {
        Self::from_checked_steps(steps())
    }

    fn last_result(messages: &[Message]) -> Result<Value> {
        let text = messages
            .last()
            .into_iter()
            .flat_map(|m| m.content.iter().rev())
            .find_map(|part| match part {
                Part::ToolResult { text, .. } => Some(text),
                _ => None,
            })
            .ok_or_else(|| crate::Error::Protocol("missing final tool result".into()))?;
        Ok(serde_json::from_str(text)?)
    }

    /// Report the all-scope check the final edit actually returned.
    fn check_after_edit(messages: &[Message]) -> Result<String> {
        let reply = Self::last_result(messages)?;
        let findings = &reply["findings"];
        let counts: sparksmith_core::issues::IssueCounts =
            serde_json::from_value(findings["issues"].clone())?;
        let open = findings["open"]
            .as_u64()
            .ok_or_else(|| crate::Error::Protocol("missing count of open findings".into()))?;
        Ok(format!(
            "Checked the board. Open findings {open} (errors {}, warnings {}, to-dos {}, informational {}).",
            counts.error, counts.warning, counts.to_do, counts.info
        ))
    }

    /// The wiring-only regression also ends with an edit that reports its check.
    pub(crate) fn from_checked_steps(steps: Vec<Step>) -> Self {
        let mut script = Self::from_steps(steps);
        script.answer = Self::check_after_edit;
        script
    }

    /// Both variants read the same starting issues and require the same
    /// successful repair before saving. The comparison adds one issues query.
    pub fn drc_fix(code: sparksmith_core::diagnostics::FindingCode, recheck: bool) -> Self {
        let inspect = || {
            (
                "query",
                json!({"name":"project.issues","params":{"scope":{"kind":"all"}}}),
            )
        };
        let save = || ("act", json!({"name":"project.save","params":{}}));
        let mut script: Vec<Step> = vec![
            Box::new(move |_| {
                vec![
                    inspect(),
                    (
                        "query",
                        json!({"name":"project.inspect","params":{"at":"/revision"}}),
                    ),
                ]
            }),
            Box::new(move |m| {
                let report = reply_to(m, "c1_0");
                let id = report["result"]["rows"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .flat_map(|row| {
                        std::iter::once(&row["finding"])
                            .chain(row["also"].as_array().into_iter().flatten())
                    })
                    .find(|finding| finding["code"] == code.as_str())
                    .and_then(|finding| finding["objects"].as_array())
                    .and_then(|objects| objects.iter().find(|object| object["kind"] == "via"))
                    .map(|object| object["id"].clone());
                vec![(
                    "apply",
                    json!({"expected_revision":revision(m),"operations":[{"op":"layout.via_delete","params":{"id":id}}]}),
                )]
            }),
            Box::new(move |m| {
                let result = reply_to(m, "c2_0");
                let findings = &result["findings"];
                if findings["resolved"][code.as_str()] != 1
                    || findings["resolved_total"] != 1
                    || findings["new"] != json!([])
                {
                    return Vec::new();
                }
                vec![if recheck { inspect() } else { save() }]
            }),
        ];
        if recheck {
            script.push(Box::new(move |m| {
                let report = reply_to(m, "c3_0");
                if report["result"]["rows"].is_array() {
                    vec![save()]
                } else {
                    Vec::new()
                }
            }));
        }
        let mut result = Self::from_steps(script);
        result.answer = |messages| {
            let saved = Self::last_result(messages)?;
            if saved.get("saved_path").is_some() {
                Ok("The dangling via was removed and the checked repair was saved.".into())
            } else {
                Err(crate::Error::Protocol(
                    "the repair was not confirmed and saved".into(),
                ))
            }
        };
        result
    }

    /// Each track ID and the guarded revision come from the actual query replies.
    pub fn two_header() -> Self {
        let script: Vec<Step> = vec![
            Box::new(|_| {
                vec![
                    ("describe", json!({"name":"layout.track_set"})),
                    ("describe", json!({"name":"layout.set_board_size"})),
                    (
                        "query",
                        json!({"name":"project.inspect","params":{"at":"/project/tracks"}}),
                    ),
                ]
            }),
            Box::new(|messages| {
                let tracks = reply_to(messages, "c1_2");
                let mut operations: Vec<_> = tracks["value"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|t| json!({"op":"layout.track_set","params":{"id":t["id"],"width":0.5}}))
                    .collect();
                operations.push(json!({"op":"layout.set_board_size","params":{"size":[24,20]}}));
                vec![(
                    "apply",
                    json!({"expected_revision":revision(messages),"operations":operations}),
                )]
            }),
            Box::new(|_| {
                vec![
                    ("act", json!({"name":"project.save","params":{}})),
                    ("query", json!({"name":"fabrication.preflight"})),
                ]
            }),
        ];
        Self::from_steps(script)
    }
}

/// A fresh project as File › New makes it when the person's default fab
/// profile is JLCPCB 2-layer (run 12's setup): the shipped new board, the
/// shipped standards and that profile.
pub fn fresh() -> std::result::Result<Session, Box<dyn std::error::Error>> {
    fresh_with_profile("jlcpcb-2-layer")
}

/// The production new-project data and the task's selected fabrication profile.
pub fn fresh_with_profile(
    profile_id: &str,
) -> std::result::Result<Session, Box<dyn std::error::Error>> {
    let context = sparksmith_io::standards::shipped_context()?;
    let defaults = sparksmith_io::new_project::shipped()?;
    let shipped = sparksmith_io::shipped::ShippedLibrary::locate()?;
    let profile = sparksmith_io::rule_profiles::shipped_profiles(&shipped)?
        .into_iter()
        .find(|p| p.id == profile_id)
        .ok_or_else(|| format!("missing shipped fabrication profile {profile_id}"))?;
    let mut project = sparksmith_core::Project::new(&defaults.board);
    project.rules.profile = Some(profile);
    let mut engine = sparksmith_core::Engine::from_file_with(
        sparksmith_core::format::EditorFile {
            version: sparksmith_core::format::FILE_VERSION,
            project,
            drafts: Default::default(),
        },
        &defaults.presets.glass_dk,
    )?;
    engine.set_context(context);
    Ok(Session::fixture(engine))
}
