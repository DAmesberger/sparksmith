//! The generic placed subcircuit and the scripted wiring turn used by the bench.
use super::{Scripted, Step, fresh, pin, reply_to, steps};
use crate::chat::{self, ChatConfig, ChatLibrary, End, Tools};
use crate::model::{Message, ModelProfile, Part};
use crate::verbs::Agent;
use serde_json::{Value, json};
use sparksmith_automation::{Grant, Session, front::NoOne};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

pub(crate) struct Verbs<'a> {
    pub(crate) session: &'a mut Session,
    pub(crate) grant: &'a Grant,
    pub(crate) refused: Vec<String>,
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

type Placed = (ChatConfig, Session, Vec<Message>);

/// Create a header, resistor and LED and place them with the board-build
/// script. This preparation is outside the measured wiring conversation.
pub fn placed() -> std::result::Result<Placed, Box<dyn std::error::Error>> {
    let library = ChatLibrary::locate()?;
    let config = ChatConfig::new(
        &library,
        ModelProfile::anthropic("claude-opus-5-5"),
        chat::unbounded(),
    )?;
    let mut session = fresh()?;
    let grant = library.reach.grant();
    let mut history = Vec::new();
    let mut model = Scripted::from_steps(steps().into_iter().take(6));
    let mut tools = Verbs {
        session: &mut session,
        grant: &grant,
        refused: Vec::new(),
    };
    let outcome = chat::turn(
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
    if outcome.end != End::Answered || !tools.refused.is_empty() {
        return Err(format!(
            "subcircuit setup ended with {}: {:?}",
            outcome.end, tools.refused
        )
        .into());
    }
    Ok((config, session, history))
}

/// Read designator/pin pairs from the task, resolve their stable IDs using
/// the first query, then use the intent check's actual revision and operations.
pub fn wire(nets: BTreeMap<String, Vec<[String; 2]>>) -> Scripted {
    let designators: BTreeSet<String> = nets
        .values()
        .flatten()
        .map(|pair| pair[0].clone())
        .collect();
    let steps: Vec<Step> = vec![
        Box::new(move |_| {
            vec![(
                "query",
                json!({"name":"schematic.pins","params":{"designators":designators}}),
            )]
        }),
        Box::new(move |messages| {
            let pins = reply_to(messages, "c1_0");
            let mut intent = BTreeMap::new();
            for (name, pairs) in &nets {
                let endpoints: Vec<_> = pairs
                    .iter()
                    .map(|[designator, label]| {
                        let part = pins["result"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .find(|part| part["designator"].as_str() == Some(designator.as_str()));
                        part.map(|p| json!({"instance":p["instance"],"terminal":pin(p,label)}))
                            .unwrap_or(Value::Null)
                    })
                    .collect();
                intent.insert(name, endpoints);
            }
            vec![(
                "query",
                json!({"name":"schematic.check_intent","params":{"nets":intent}}),
            )]
        }),
        Box::new(|messages| {
            let plan = reply_to(messages, "c2_0");
            if plan["result"]["blocked"].is_string() {
                return Vec::new();
            }
            vec![(
                "apply",
                json!({"expected_revision":plan["revision"],"operations":plan["result"]["operations"]}),
            )]
        }),
    ];
    Scripted::from_checked_steps(steps)
}
