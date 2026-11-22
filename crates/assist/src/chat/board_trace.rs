//! A small generic board built through the six verbs only, as the chat
//! calls them (no model): a 2-pin header powers an LED through a 0603
//! resistor. The fab profile is chosen by id with an option. Parts are made with the shipped generators, placed on the
//! schematic inside the sheet frame, wired, synced to the board, placed,
//! routed and given a GND fill on the bottom, and the check passes.
//!
//! Every call is what a model would send; each later call takes its ids
//! from an earlier reply, named beside it. The size of each reply as the
//! chat hands it to the model is printed (`--nocapture`), with the sizes of
//! the exploratory calls a model typically makes.
use crate::chat::ChatLibrary;
use crate::verbs::Agent;
use serde_json::{Value, json};
use sparksmith_automation::front::NoOne;
use sparksmith_automation::{Grant, Session};

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A fresh project as File › New makes it when the person's default fab
/// profile is JLCPCB 2-layer (run 12's setup): the shipped new board, the
/// shipped standards and that profile.
pub(super) fn fresh() -> Fallible<Session> {
    let context = sparksmith_io::standards::shipped_context()?;
    let defaults = sparksmith_io::new_project::shipped()?;
    let shipped = sparksmith_io::shipped::ShippedLibrary::locate()?;
    let profile = sparksmith_io::rule_profiles::shipped_profiles(&shipped)?
        .into_iter()
        .find(|p| p.id == "jlcpcb-2-layer")
        .ok_or("no shipped JLCPCB 2-layer profile")?;
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

/// The calls made so far: verb, entry and the reply's size as the chat
/// hands it to the model (`result` or `error`, as JSON text).
struct Trace {
    session: Session,
    grant: Grant,
    rows: Vec<(String, String, usize)>,
}

impl Trace {
    fn new() -> Fallible<Self> {
        Ok(Self {
            session: fresh()?,
            grant: ChatLibrary::locate()?.reach.grant(),
            rows: Vec::new(),
        })
    }

    /// The entry a call names, for the table.
    fn entry(input: &Value) -> String {
        input["name"]
            .as_str()
            .or_else(|| input["operations"][0]["op"].as_str())
            .or_else(|| input["text"].as_str())
            .unwrap_or_default()
            .to_owned()
    }

    /// The reply to `verb` with `input`, as the chat's built-in agent runs
    /// it, and whether it succeeded.
    fn send(&mut self, verb: &str, input: Value) -> (bool, Value) {
        let entry = Self::entry(&input);
        let reply = Agent {
            session: &mut self.session,
            grant: &self.grant,
            turn: None,
            window: None,
            asker: &mut NoOne,
        }
        .verb(verb, input);
        let ok = reply["ok"] == true;
        let body = if ok {
            reply["result"].clone()
        } else {
            reply["error"].clone()
        };
        self.rows
            .push((verb.to_owned(), entry, body.to_string().len()));
        (ok, body)
    }

    /// [`Self::send`], a refused call failing the test.
    fn call(&mut self, verb: &str, input: Value) -> Fallible<Value> {
        let entry = Self::entry(&input);
        let (ok, body) = self.send(verb, input);
        if !ok {
            return Err(format!("{verb} {entry} refused: {body}").into());
        }
        Ok(body)
    }

    /// `apply` of one operation at `expected_revision`: a model takes it
    /// from the last reply's `revision` (every edit reply carries it).
    fn apply(&mut self, op: &str, params: Value) -> Fallible<Value> {
        let revision = self.session.engine().revision();
        self.call(
            "apply",
            json!({"expected_revision": revision, "operations": [{"op": op, "params": params}]}),
        )
    }

    fn print(&self, title: &str) {
        println!("{title}");
        let mut total = 0;
        for (n, (verb, entry, bytes)) in self.rows.iter().enumerate() {
            println!("{:>3} {verb:<8} {entry:<28} {bytes:>7} B", n + 1);
            total += bytes;
        }
        println!("    total {total} B over {} calls", self.rows.len());
    }
}

/// `library.generate_part` with `preset`, then `component.place` of what
/// it made at `at` (sheet mm, y up, inside A4's frame). The instance comes
/// back as the place reply's `outcomes[0].instance`.
fn make_and_place(t: &mut Trace, generator: &str, preset: &str, at: [f64; 2]) -> Fallible<u64> {
    let made = t.apply(
        "library.generate_part",
        json!({"generator": generator, "scope": "component", "preset": preset}),
    )?;
    // component.place takes these from `outcome.report.revisions`, by
    // `kind`: the component as {id, revision} (without `kind`, which it
    // refuses), the symbol's id as `representation`, the binding's and
    // footprint's ids as `physical`.
    let revisions = &made["outcome"]["report"]["revisions"];
    let of = |kind: &str| -> Fallible<Value> {
        revisions
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["kind"] == kind)
            .cloned()
            .ok_or_else(|| format!("no {kind} in {made}").into())
    };
    let component = of("component")?;
    let placed = t.apply(
        "component.place",
        json!({"component": {"id": component["id"], "revision": component["revision"]},
            "representation": of("symbol")?["id"], "unit": 1, "position": at,
            "physical": {"binding": of("binding")?["id"], "footprint": of("footprint")?["id"]}}),
    )?;
    placed["outcomes"][0]["instance"]
        .as_u64()
        .ok_or_else(|| format!("no instance in {placed}").into())
}

/// The net holding `instance`'s `terminal`: what a model reads with
/// `query project.inspect {"at": "/project/nets"}` (`/nets` is refused,
/// naming the snapshot's keys).
fn net_of(t: &mut Trace, instance: u64, terminal: u64) -> Fallible<u64> {
    let nets = t.call(
        "query",
        json!({"name": "project.inspect", "params": {"at": "/project/nets"}}),
    )?;
    let list = nets["value"]
        .as_array()
        .or_else(|| nets.as_array())
        .cloned()
        .unwrap_or_default();
    list.iter()
        .find(|net| {
            net["terminals"].as_array().is_some_and(|ends| {
                ends.iter()
                    .any(|e| e["instance"] == instance && e["terminal"] == terminal)
            })
        })
        .and_then(|net| net["id"].as_u64())
        .ok_or_else(|| format!("no net holds {instance}.{terminal}: {nets}").into())
}

#[test]
fn a_small_board_is_built_routed_and_checked_through_the_verbs() -> Fallible {
    let mut t = Trace::new()?;
    // The fab profile: the compact listing gives each profile's id and its
    // options with their choices; one step sets one by id with a choice.
    let listed = t.call("query", json!({"name": "rules.fab_profiles", "params": {}}))?;
    let profiles = listed["result"].as_array().cloned().unwrap_or_default();
    let pcbway = profiles
        .iter()
        .find(|p| p["id"] == "pcbway-2-layer")
        .ok_or_else(|| format!("no pcbway-2-layer in {listed}"))?;
    assert!(
        pcbway["options"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|o| o["id"] == "limits" && o["default"] == "normal"),
        "{pcbway}"
    );
    assert!(pcbway.get("rules").is_none(), "{pcbway}");
    // Its reply names the snapshot as one changed field.
    let set = t.apply(
        "rules.set_fab_profile",
        json!({"profile": "pcbway-2-layer", "options": {"limits": "minimum"}}),
    )?;
    assert_eq!(set["changes"]["fields"], json!(["rules.profile"]), "{set}");
    let rules = &t.session.engine().project().rules;
    let board = rules.profile.as_ref().ok_or("no profile")?;
    assert_eq!(board.id, "pcbway-2-layer");
    assert_eq!(
        board.chosen().get("limits").map(String::as_str),
        Some("minimum")
    );
    // Parts: a 1×2 header (power in), a 0603 LED and a 0603 resistor, each
    // made by a shipped generator preset (`library.generators {text}` lists
    // the keys) and placed on Sheet 1.
    let j1 = make_and_place(&mut t, "header", "header-1x2-2.54", [20.0, -60.0])?;
    let d1 = make_and_place(&mut t, "chip", "led-0603", [100.0, -60.0])?;
    let r1 = make_and_place(&mut t, "chip", "resistor-0603", [60.0, -40.0])?;
    // Wires, terminal to terminal. Terminal ids are in component.inspect's
    // `functions.active[].terminal` and its findings: header 1, 2; resistor
    // 1, 2; LED K = 1, A = 2.
    let end = |instance: u64, terminal: u64| json!({"instance": instance, "terminal": terminal});
    for (from, to) in [
        (end(j1, 1), end(r1, 1)), // VIN
        (end(r1, 2), end(d1, 2)), // resistor to anode
        (end(d1, 1), end(j1, 2)), // cathode to GND
    ] {
        t.apply("schematic.connect", json!({"from": from, "to": to}))?;
    }
    // The board: 30 × 20 mm, origin at its lower-left corner.
    t.apply("layout.set_board_size", json!({"size": [30.0, 20.0]}))?;
    // Update PCB: an operation without parameters, sent with `{}`.
    t.apply("layout.sync", json!({}))?;
    // Sync placed every part; layout.place would now refuse ("component is
    // already placed"), so layout.move puts them close together.
    for (instance, at, rotation) in [
        (j1, [5.0, 10.0], 90.0),
        (r1, [15.0, 6.0], 0.0),
        (d1, [24.0, 10.0], 270.0),
    ] {
        t.apply(
            "layout.move",
            json!({"instance": instance, "position": at, "rotation_deg": rotation, "layer": "Top"}),
        )?;
    }
    // Pad centres on the board: placement position and rotation applied to
    // the footprint's pads (header pitch 2.54 mm, 0603 resistor ±0.65 mm,
    // 0603 LED ±0.825 mm). No query lists them; checked here against the
    // engine.
    let pads: Vec<(u64, [f64; 2])> = sparksmith_core::layout::pads(t.session.engine().project())
        .iter()
        .map(|p| (p.net, p.position))
        .collect();
    // layout.pads lists the same centres by designator and pad number.
    let listed = t.call(
        "query",
        json!({"name": "layout.pads", "params": {"designators": ["j1"]}}),
    )?;
    let rows = listed["result"].as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 2, "{listed}");
    for row in &rows {
        assert_eq!(row["designator"], "J1", "{row}");
        let at = [
            row["position"][0].as_f64().unwrap_or(f64::NAN),
            row["position"][1].as_f64().unwrap_or(f64::NAN),
        ];
        assert!(
            pads.iter()
                .any(|(n, p)| row["net"] == *n && (p[0] - at[0]).hypot(p[1] - at[1]) < 1e-9),
            "{row}"
        );
    }
    let missing = t.send(
        "query",
        json!({"name": "layout.pads", "params": {"designators": ["U9"]}}),
    );
    assert!(
        !missing.0 && missing.1.to_string().contains("U9"),
        "{}",
        missing.1
    );
    let vin = net_of(&mut t, j1, 1)?;
    let gnd = net_of(&mut t, j1, 2)?;
    let led = net_of(&mut t, r1, 2)?;
    let routes = [
        (vin, vec![[5.0, 8.73], [5.0, 6.0], [14.35, 6.0]]),
        (led, vec![[15.65, 6.0], [24.0, 6.0], [24.0, 9.175]]),
        (
            gnd,
            vec![[5.0, 11.27], [5.0, 14.0], [24.0, 14.0], [24.0, 10.825]],
        ),
    ];
    for (net, path) in &routes {
        for point in [path[0], path[path.len() - 1]] {
            let near = |p: &[f64; 2]| (p[0] - point[0]).hypot(p[1] - point[1]) < 1e-6;
            assert!(
                pads.iter().any(|(n, p)| n == net && near(p)),
                "no pad of net {net} at {point:?}: {pads:?}"
            );
        }
        for leg in path.windows(2) {
            t.apply(
                "layout.route",
                json!({"request": {"net": net, "start": leg[0], "end": leg[1],
                    "width": 0.25, "layer": "Top"}}),
            )?;
        }
    }
    // A GND fill over the whole bottom; the header's plated pad joins it.
    t.apply(
        "fill.add",
        json!({"fill": {"layer": "Bottom", "net": gnd, "region": "board", "name": "GND"}}),
    )?;
    t.apply("fill.refill", json!({"target": {"fills": "all"}}))?;
    // The check: no errors, one warning (no fiducials) and nothing
    // unrouted. What is left is a to-do per part with no orderable Part
    // chosen (generic parts).
    let issues = t.call(
        "query",
        json!({"name": "project.issues", "params": {"scope": {"kind": "all"}}}),
    )?;
    let report = &issues["result"];
    assert_eq!(report["counts"]["error"], 0, "{report}");
    // One warning: the board has surface-mount parts and no fiducials,
    // which its PCBWay profile asks three of for assembly.
    assert_eq!(report["counts"]["warning"], 1, "{report}");
    let rows = report["rows"].as_array().cloned().unwrap_or_default();
    assert!(
        rows.iter()
            .all(|r| r["finding"]["code"] == "purchasing.no_part"
                || r["finding"]["code"] == "board.fiducials_too_few"),
        "{report}"
    );
    assert_eq!(t.session.engine().project().tracks.len(), 7);
    let preflight = t.call(
        "query",
        json!({"name": "fabrication.preflight", "params": {}}),
    )?;
    println!(
        "preflight: {}",
        preflight.to_string().chars().take(600).collect::<String>()
    );
    t.print("The build's calls, with each reply's size:");

    // What a model typically asks on this project.
    t.rows.clear();
    for (verb, input) in [
        ("query", json!({"name": "project.inspect", "params": {}})),
        (
            "query",
            json!({"name": "project.inspect", "params": {"at": "/revision"}}),
        ),
        (
            "query",
            json!({"name": "library.search", "params": {"query": "resistor 0603"}}),
        ),
        ("query", json!({"name": "library.generators", "params": {}})),
        ("query", json!({"name": "library.rescan", "params": {}})),
        ("query", json!({"name": "rules.fab_profiles", "params": {}})),
        (
            "query",
            json!({"name": "rules.fab_profiles", "params": {"id": "pcbway-2-layer"}}),
        ),
        (
            "query",
            json!({"name": "project.issues", "params": {"scope": {"kind": "all"}}}),
        ),
        ("describe", json!({"name": "component.place"})),
        ("describe", json!({"name": "layout.route"})),
        ("find", json!({"text": "place a component"})),
    ] {
        let (_, body) = t.send(verb, input);
        let text = body.to_string();
        if text.len() <= 600 {
            println!("{verb}: {text}");
        }
    }
    t.print("Exploratory calls on the built board:");
    let mut empty = Trace::new()?;
    empty.send("query", json!({"name": "project.inspect", "params": {}}));
    empty.print("project.inspect on the fresh project:");
    Ok(())
}
