//! Geometry smoke test of a board-edge pass-through: the board features a
//! thin 4-layer board with a straddle-mount plug and a mid-mount receptacle
//! depends on, built and checked end to end through the command engine
//! (`Session::respond`, which answers each line of `sparksmith stdio
//! --operator`; MCP's verbs run the same entries) on an original, generic
//! board. No real part or vendor data.
//!
//! The board: 14 × 14 mm, 0.8 mm, four layers, on the shipped
//! `jlcpcb-4-layer` profile held to its "Absolute minimum" values.
//!
//! - J1, a 24-contact mid-mount receptacle in a notch of the bottom edge:
//!   both 0.5 mm pitch rows on the top face, the B row in front by the
//!   notch, the A row behind it.
//! - J2, a 24-contact straddle-mount plug on the top edge: its A row on the
//!   top face and its B row on the bottom face, B12 under A1.
//! - One differential pair from J1's B2/B3 through a 0.15/0.25 mm via in
//!   each pad to the bottom layer and on to J2's B2/B3 on the bottom face,
//!   over a ground fill on the inner layer above it.
//!
//! Expected positions are derived from the board parameters and the pad
//! tables below, never read back from the document. The fabrication files
//! are read back with `sparksmith-fabcheck`, the independent Gerber and
//! Excellon reader; connectivity is taken from their copper and drills.
//!
//! Known gaps are asserted as they are, each under a `KNOWN GAP` comment,
//! so a fix has to update this test.
use serde_json::{Value, json};
use sparksmith_automation::{Caller, Session};
use sparksmith_fabcheck::polygons::{self, Shape, Shapes};
use sparksmith_fabcheck::{excellon, gerber};
use sparksmith_io::shipped::ShippedLibrary;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Board width and height, mm.
const WIDTH: f64 = 14.0;
const HEIGHT: f64 = 14.0;
/// The connector notch in the bottom edge: its width, the depth it is
/// drawn with and the depth its parameter is then set to.
const NOTCH_WIDTH: f64 = 7.0;
const NOTCH_DRAWN: f64 = 2.0;
const NOTCH_DEPTH: f64 = 2.5;
/// Contact pitch, pad size and the pads' distance from the board edge.
const PITCH: f64 = 0.5;
const PAD: [f64; 2] = [0.3, 1.0];
const SETBACK: f64 = 0.25;
/// Between the two rows of the mid-mount receptacle.
const ROW_GAP: f64 = 0.6;
/// The fabricator's smallest via: drill and pad diameter.
const VIA: [f64; 2] = [0.15, 0.25];
/// The stackup entered: outer and inner foils (µm), prepreg and core (mm).
const FOILS_UM: [f64; 4] = [35.0, 17.5, 17.5, 35.0];
const DIELECTRICS_MM: [f64; 3] = [0.1, 0.5, 0.1];

/// Contact `i` (1–12) of a row, along the row from its middle.
fn column(i: u32) -> f64 {
    (f64::from(i) - 6.5) * PITCH
}

/// Where the board puts a pad, from the parameters alone: `row` is `'A'`
/// or `'B'`, `notch` the notch depth in force.
fn pad_at(designator: &str, row: char, i: u32, notch: f64) -> [f64; 2] {
    let across = if row == 'A' { column(i) } else { -column(i) };
    let centre = WIDTH / 2.0;
    match (designator, row) {
        // J1's origin lies on the notch's floor; its B row is in front.
        ("J1", 'B') => [centre + across, notch + SETBACK + PAD[1] / 2.0],
        ("J1", _) => [
            centre + across,
            notch + SETBACK + PAD[1] + ROW_GAP + PAD[1] / 2.0,
        ],
        // J2's origin lies on the top edge, turned half round: both rows
        // at one distance from the edge, the B row on the bottom face.
        _ => [centre + across, HEIGHT - SETBACK - PAD[1] / 2.0],
    }
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

fn point(value: &Value) -> Fallible<[f64; 2]> {
    match (value[0].as_f64(), value[1].as_f64()) {
        (Some(x), Some(y)) => Ok([x, y]),
        _ => Err(format!("not a point: {value}").into()),
    }
}

fn id(value: &Value) -> Fallible<u64> {
    value
        .as_u64()
        .ok_or_else(|| format!("not an id: {value}").into())
}

fn rows(value: &Value) -> Fallible<&Vec<Value>> {
    value
        .as_array()
        .ok_or_else(|| format!("not a list: {value}").into())
}

/// One row of `project.issues`.
#[derive(Clone, Debug, PartialEq)]
struct Issue {
    severity: String,
    code: String,
    message: String,
}

/// A session driven by JSON requests only, at the revision of its last
/// reply.
struct Board {
    session: Session,
    revision: u64,
}

impl Board {
    fn new() -> Fallible<Self> {
        let library = ShippedLibrary::for_checkout()?;
        let context = sparksmith_io::standards::context(&library)?;
        let mut session = Session::new(sparksmith_io::new_project::engine(&context)?);
        session.set_shipped(library);
        Ok(Self {
            session,
            revision: 0,
        })
    }

    /// The reply to `request`, whatever it is: one line of
    /// `sparksmith stdio --operator`.
    fn send(&mut self, request: Value) -> Value {
        let reply = self.session.respond(Caller::Operator, request);
        if let Some(revision) = reply["result"]["revision"].as_u64() {
            self.revision = revision;
        }
        reply
    }

    /// The result of an accepted `request`.
    fn ok(&mut self, request: Value) -> Fallible<Value> {
        let text = request.to_string();
        let mut reply = self.send(request);
        if reply["ok"] != true {
            let shown: String = text.chars().take(600).collect();
            return Err(format!("refused: {shown}\n{reply}").into());
        }
        Ok(reply["result"].take())
    }

    fn operation(&self, editor: &str, command: &str, params: Value) -> Value {
        let operation = if params.is_null() {
            json!({"command": command})
        } else {
            json!({"command": command, "params": params})
        };
        json!({"command": "project.apply", "params": {
            "expected_revision": self.revision,
            "operation": {"editor": editor, "operation": operation},
        }})
    }

    /// One document operation, accepted.
    fn apply(&mut self, editor: &str, command: &str, params: Value) -> Fallible<Value> {
        let request = self.operation(editor, command, params);
        self.ok(request)
    }

    fn query(&mut self, query: &str, params: Value) -> Fallible<Value> {
        let mut result = self.ok(json!({"command": "project.query", "params": {
            "query": query, "params": params,
        }}))?;
        Ok(result["result"].take())
    }

    /// The whole reply of a verbose `project.inspect`.
    fn inspect(&mut self) -> Fallible<Value> {
        self.ok(json!({"command": "project.inspect", "params": {}, "verbose": true}))
    }

    fn project(&mut self) -> Fallible<Value> {
        Ok(self.inspect()?["project"].take())
    }

    fn undo(&mut self) -> Fallible {
        let revision = self.revision;
        self.ok(json!({"command": "project.undo", "params": {"expected_revision": revision}}))?;
        Ok(())
    }

    fn issues(&mut self) -> Fallible<Vec<Issue>> {
        let report = self.query("project.issues", json!({"scope": {"kind": "all"}}))?;
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        Ok(rows(&report["rows"])?
            .iter()
            .map(|row| Issue {
                severity: text(&row["severity"]),
                code: text(&row["finding"]["code"]),
                message: text(&row["finding"]["message"]),
            })
            .collect())
    }

    /// The issues of `code`.
    fn found(&mut self, code: &str) -> Fallible<Vec<Issue>> {
        Ok(self
            .issues()?
            .into_iter()
            .filter(|i| i.code == code)
            .collect())
    }

    fn errors(&mut self) -> Fallible<Vec<Issue>> {
        Ok(self
            .issues()?
            .into_iter()
            .filter(|i| i.severity == "error")
            .collect())
    }

    fn net(&mut self, name: &str) -> Fallible<u64> {
        let project = self.project()?;
        rows(&project["nets"])?
            .iter()
            .find(|n| n["name"] == name)
            .map(|n| id(&n["id"]))
            .ok_or_else(|| format!("no net {name}"))?
    }

    /// `layout.pads`: where the board says pad `number` of `designator`
    /// is, its layer and its net's name.
    fn pad(&mut self, designator: &str, number: &str) -> Fallible<([f64; 2], String, String)> {
        let pads = self.query("layout.pads", json!({}))?;
        let pad = rows(&pads)?
            .iter()
            .find(|p| p["designator"] == designator && p["pad"] == number)
            .ok_or_else(|| format!("no pad {designator}.{number}"))?;
        let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
        Ok((
            point(&pad["position"])?,
            text(&pad["layer"]),
            text(&pad["net_name"]),
        ))
    }

    fn placement(&mut self, instance: u64) -> Fallible<([f64; 2], f64)> {
        let project = self.project()?;
        let placement = rows(&project["placements"])?
            .iter()
            .find(|p| p["instance"] == instance)
            .ok_or("not placed")?;
        Ok((
            point(&placement["position"])?,
            placement["rotation_deg"].as_f64().ok_or("no rotation")?,
        ))
    }

    fn via(&mut self, net: u64, at: [f64; 2]) -> Value {
        let request = self.operation(
            "Layout",
            "layout.add_via",
            json!({"via": {
                "net": net, "position": at, "drill": VIA[0], "diameter": VIA[1],
                "span": "through", "tented": true,
            }}),
        );
        self.send(request)
    }

    fn set_parameter(&mut self, name: &str, mm: f64, comment: &str) -> Fallible<Value> {
        self.apply(
            "Board",
            "board.edit",
            json!({"edits": [{"op": "set_parameter", "param": {
                "name": name, "expr": format!("{mm} mm"), "unit": "mm", "comment": comment,
            }}]}),
        )
    }
}

/// How far [`build`] goes.
#[derive(Clone, Copy, PartialEq, PartialOrd)]
enum Stage {
    /// The parts placed, held by dimensions, and the pair defined.
    Paired,
    /// Vias, the routed pair, the ground vias and the plane.
    Routed,
}

struct Built {
    board: Board,
    /// Instances.
    j1: u64,
    j2: u64,
    /// Nets `[positive, negative]` of the pair.
    lane: [u64; 2],
}

/// Step 1: the profile at its absolute minimum values and a 0.8 mm,
/// 4-layer stackup.
fn start(board: &mut Board) -> Fallible {
    let reply = board.apply(
        "Rules",
        "rules.set_fab_profile",
        json!({"profile": "jlcpcb-4-layer", "options": {"limits": "minimum"}}),
    )?;
    // A new project has two layers: the profile says so until the stackup
    // is entered.
    assert_eq!(
        reply["findings"]["new"][0]["code"], "board.profile_layers",
        "{reply}"
    );
    let foil = |um: f64, plane: bool| json!({"thickness_um": um, "plane": plane});
    let reply = board.apply(
        "Layout",
        "layout.set_stackup",
        json!({
            "copper": [
                foil(FOILS_UM[0], false), foil(FOILS_UM[1], true),
                foil(FOILS_UM[2], true), foil(FOILS_UM[3], false),
            ],
            "dielectrics": [
                {"kind": "prepreg", "thickness_mm": DIELECTRICS_MM[0], "glass": "style2116", "dk": 4.16},
                {"kind": "core", "thickness_mm": DIELECTRICS_MM[1], "glass": "style7628", "dk": 4.6},
                {"kind": "prepreg", "thickness_mm": DIELECTRICS_MM[2], "glass": "style2116", "dk": 4.16},
            ],
            "mask_top": true, "mask_bottom": true,
            "silkscreen_top": true, "silkscreen_bottom": true,
        }),
    )?;
    assert_eq!(
        reply["findings"]["resolved"]["board.profile_layers"], 1,
        "{reply}"
    );
    assert_eq!(board.errors()?, vec![]);
    // The option is also set on its own, as a board that already has the
    // profile sets it.
    board.apply(
        "Rules",
        "rules.set_fab_options",
        json!({"options": {"limits": "minimum"}}),
    )?;
    Ok(())
}

/// The laminate and copper of the stackup entered, mm.
fn thickness() -> f64 {
    DIELECTRICS_MM.iter().sum::<f64>() + FOILS_UM.iter().sum::<f64>() / 1000.0
}

/// The sketch curves of the named edges a part is held to.
struct Edges {
    notch: String,
    left: String,
    top: String,
}

/// Step 3, the shape: a notch (mid-mount cutout) drawn in the board sketch
/// and held by parameters. The rectangle `board.start_sketch` draws is
/// replaced by a closed polyline, since `board.edit` cannot split an edge.
fn outline(board: &mut Board) -> Fallible<Edges> {
    board.apply(
        "Board",
        "board.set_rectangle",
        json!({"size": [WIDTH, HEIGHT]}),
    )?;
    board.apply("Board", "board.start_sketch", Value::Null)?;
    let sketch = board.project()?["board_shape"]["sketch"].take();
    let mut gone: Vec<Value> = rows(&sketch["sketch"]["curves"])?
        .iter()
        .map(|c| json!({"curve": c["id"]}))
        .collect();
    gone.extend(
        rows(&sketch["sketch"]["points"])?
            .iter()
            .filter(|p| p["id"] != "origin")
            .map(|p| json!({"point": p["id"]})),
    );
    let mut edits: Vec<Value> = rows(&sketch["edges"])?
        .iter()
        .map(|e| json!({"op": "unname_edge", "name": e["name"]}))
        .collect();
    let (left, right) = ((WIDTH - NOTCH_WIDTH) / 2.0, (WIDTH + NOTCH_WIDTH) / 2.0);
    // Counter-clockwise from the origin: a-b, the notch b-c-d-e, e-f, then
    // the right, top and left edges.
    let corners = [
        [0.0, 0.0],
        [left, 0.0],
        [left, NOTCH_DRAWN],
        [right, NOTCH_DRAWN],
        [right, 0.0],
        [WIDTH, 0.0],
        [WIDTH, HEIGHT],
        [0.0, HEIGHT],
    ];
    edits.push(json!({"op": "sketch", "edit": {"delete": {"ids": gone}}}));
    edits.push(json!({"op": "sketch", "edit": {"add_polyline": {
        "points": corners, "closed": true, "construction": false,
    }}}));
    let reply = board.apply("Board", "board.edit", json!({"edits": edits}))?;
    let created = rows(&reply["outcome"]["created"])?;
    let named = |key: &str| -> Vec<String> {
        created
            .iter()
            .filter_map(|c| c[key].as_str().map(str::to_owned))
            .collect()
    };
    let (points, lines) = (named("point"), named("curve"));
    let ([a, b, _, _, e, f, _, h], [ab, bc, cd, _, _, fg, gh, ha]) = (&points[..], &lines[..])
    else {
        return Err(format!("the polyline made {points:?} and {lines:?}").into());
    };
    let param = |name: &str, expr: String, comment: &str| {
        json!({"op": "set_parameter", "param": {
            "name": name, "expr": expr, "unit": "mm", "comment": comment,
        }})
    };
    let constraint = |kind: &str, refs: Value| json!({"op": "sketch", "edit": {"add_constraint": {"kind": kind, "refs": refs}}});
    let dimension = |kind: &str, name: &str, refs: Value| {
        json!({"op": "sketch", "edit": {"add_dimension": {
            "kind": kind, "refs": refs, "value": {"param": name},
        }}})
    };
    let mut edits = vec![
        param(
            "notch_w",
            format!("{NOTCH_WIDTH} mm"),
            "Width of the connector notch",
        ),
        param(
            "notch_d",
            format!("{NOTCH_DRAWN} mm"),
            "Depth of the connector notch",
        ),
        // A sketch dimension takes a number or a parameter, not an
        // expression: the derived value is a parameter of its own.
        param(
            "notch_x",
            "(width - notch_w) / 2".into(),
            "Notch side from the nearer board side",
        ),
    ];
    edits.extend(lines.iter().enumerate().map(|(n, line)| {
        constraint(
            if n % 2 == 0 { "horizontal" } else { "vertical" },
            json!([{"curve": line}]),
        )
    }));
    edits.extend([
        constraint("coincident", json!([{"point": "origin"}, {"point": a}])),
        dimension(
            "horizontal_distance",
            "width",
            json!([{"point": a}, {"point": f}]),
        ),
        dimension(
            "vertical_distance",
            "height",
            json!([{"point": a}, {"point": h}]),
        ),
        dimension("vertical_distance", "notch_d", json!([{"curve": bc}])),
        // Both shoulders of the notch on one line, the notch centred.
        constraint("point_on_curve", json!([{"point": e}, {"curve": ab}])),
        dimension("distance", "notch_x", json!([{"point": b}, {"curve": ha}])),
        dimension("distance", "notch_x", json!([{"point": e}, {"curve": fg}])),
    ]);
    for (name, curve) in [
        ("bottom", ab),
        ("right", fg),
        ("top", gh),
        ("left", ha),
        ("notch", cd),
    ] {
        edits.push(json!({"op": "name_edge", "name": name, "curve": curve}));
    }
    board.apply("Board", "board.edit", json!({"edits": edits}))?;
    let solve = board.project()?["board_shape"]["sketch"]["solve"].take();
    assert_eq!(
        (&solve["status"], &solve["dof"]),
        (&json!("solved"), &json!(0)),
        "the notched outline is fully defined by its parameters: {solve}"
    );
    assert_eq!(outer(board)?, notched(NOTCH_DRAWN));
    Ok(Edges {
        notch: cd.clone(),
        left: ha.clone(),
        top: gh.clone(),
    })
}

/// A point in whole micrometres, for comparing outlines.
fn um(p: [f64; 2]) -> (i64, i64) {
    (
        (p[0] * 1000.0).round() as i64,
        (p[1] * 1000.0).round() as i64,
    )
}

/// The corners of the notched outline for a notch `depth` deep.
fn notched(depth: f64) -> BTreeSet<(i64, i64)> {
    let (left, right) = ((WIDTH - NOTCH_WIDTH) / 2.0, (WIDTH + NOTCH_WIDTH) / 2.0);
    [
        [0.0, 0.0],
        [left, 0.0],
        [left, depth],
        [right, depth],
        [right, 0.0],
        [WIDTH, 0.0],
        [WIDTH, HEIGHT],
        [0.0, HEIGHT],
    ]
    .into_iter()
    .map(um)
    .collect()
}

/// `board.shape`: the corners of the solved outer outline.
fn outer(board: &mut Board) -> Fallible<BTreeSet<(i64, i64)>> {
    let shape = board.query("board.shape", json!({}))?;
    let mut corners = BTreeSet::from([um(point(&shape["outer"]["start"])?)]);
    for segment in rows(&shape["outer"]["segments"])? {
        corners.insert(um(point(&segment["line"]["to"])?));
    }
    Ok(corners)
}

/// The pad table of one footprint: `(number, at, bottom face)`.
fn pad_rows(straddle: bool) -> Vec<Value> {
    let front = SETBACK + PAD[1] / 2.0;
    let behind = SETBACK + PAD[1] + ROW_GAP + PAD[1] / 2.0;
    let mut pads = Vec::new();
    for (row, bottom, y, sign) in if straddle {
        // B12 lies under A1.
        [('A', false, front, -1.0), ('B', true, front, 1.0)]
    } else {
        [('A', false, behind, 1.0), ('B', false, front, -1.0)]
    } {
        for i in 1..=12 {
            let mut pad = json!({
                "number": format!("{row}{i}"), "at": [sign * column(i), y], "size": PAD,
            });
            if bottom {
                pad["side"] = json!("bottom");
            }
            pads.push(pad);
        }
    }
    pads
}

/// The library ids of the connector: component, symbol, binding and its
/// two footprints.
struct Connector {
    component: u64,
    symbol: u64,
    binding: u64,
    mid_mount: u64,
    straddle: u64,
    terminals: BTreeMap<String, u64>,
}

/// Step 2: one generic 24-contact connector with two footprints made by
/// `library.footprint_from_pads`, one of them with a pad row on each face.
fn connector(board: &mut Board) -> Fallible<Connector> {
    let name = "EDGE24";
    let numbers: Vec<String> = ['A', 'B']
        .iter()
        .flat_map(|row| (1..=12).map(move |i| format!("{row}{i}")))
        .collect();
    let pins: Vec<Value> = numbers
        .iter()
        .map(|n| {
            let label = n.to_lowercase();
            json!({
                "row": label, "number": {"P24": n}, "name": n, "class": "electrical",
                "source": {"page": 1, "table": "Contact table"},
                "functions": [{"label": label, "direction": "passive", "signal": "unspecified"}],
                "default": label,
            })
        })
        .collect();
    let target = json!({"target": "new", "name": name});
    let document = json!({
        "format": 1,
        "device": {"name": name, "reference": {
            "title": "Generic 24-contact board-edge connector (original test data)",
            "document_revision": "1",
            "url": "https://example.com/generic-24-contact-connector",
        }},
        "variants": [{"name": "P24", "terminal_count": 24}],
        "peripherals": [], "pins": pins, "rules": [],
    });
    let plan = board.query(
        "library.pinout_preview",
        json!({"target": target, "document": document}),
    )?;
    let reply = board.apply(
        "Library",
        "library.pinout_apply",
        json!({"target": target, "document": document, "base": plan["base"]}),
    )?;
    let made: Vec<u64> = rows(&reply["outcome"]["report"]["drafts"])?
        .iter()
        .filter_map(Value::as_u64)
        .collect();
    let [component, symbol, package, binding] = made[..] else {
        return Err(format!("drafts made: {made:?}").into());
    };
    let draft = |board: &mut Board, wanted: u64| -> Fallible<Value> {
        let mut inspected = board.inspect()?;
        let at = rows(&inspected["drafts"])?
            .iter()
            .position(|d| d["draft"]["id"] == wanted)
            .ok_or_else(|| format!("no draft {wanted}"))?;
        Ok(inspected["drafts"][at]["draft"].take())
    };
    // The package's envelope, then the two footprints.
    let mut edited = draft(board, package)?;
    edited["definition"]["body"]["envelope_nm"] = json!([7_000_000, 4_000_000, 3_000_000]);
    board.apply(
        "Library",
        "library.edit",
        json!({"edits": [{"id": package, "meta": edited["meta"], "definition": edited["definition"]}]}),
    )?;
    let mut footprints = Vec::new();
    for (label, straddle) in [("mid-mount", false), ("straddle", true)] {
        let reply = board.apply(
            "Library",
            "library.new_draft",
            json!({
                "meta": {
                    "name": format!("{name} {label}"), "description": "",
                    "provenance": "Original test data",
                },
                "definition": {"kind": "footprint", "body": {
                    "package": {"id": package, "revision": 1}, "next_local": 1,
                    "source": null, "pads": [], "graphics": [], "polarity": null,
                    "datum": {"offset": {"x_nm": 0, "y_nm": 0}, "rotation_mdeg": 0},
                    "defaults": {"mask_expansion_nm": 0, "paste_reduction_nm": 0},
                }},
            }),
        )?;
        let footprint = id(&reply["outcome"]["id"])?;
        let reply = board.apply(
            "Library",
            "library.footprint_from_pads",
            json!({"draft": footprint, "pads": pad_rows(straddle), "assign_by_number": true}),
        )?;
        let report = &reply["outcome"]["report"];
        assert_eq!(rows(&report["pads"])?.len(), 24, "{report}");
        assert_eq!(
            (&report["unassigned"], &report["unlanded"]),
            (&json!([]), &json!([])),
            "every pad lands its terminal by its number: {report}"
        );
        footprints.push(footprint);
    }
    let [mid_mount, straddle] = footprints[..] else {
        return Err("two footprints".into());
    };
    let mut edited = draft(board, binding)?;
    edited["definition"]["body"]["footprints"] = json!([
        {"id": mid_mount, "revision": 1}, {"id": straddle, "revision": 1},
    ]);
    board.apply(
        "Library",
        "library.edit",
        json!({"edits": [{"id": binding, "meta": edited["meta"], "definition": edited["definition"]}]}),
    )?;
    board.apply(
        "Library",
        "library.publish",
        json!({"drafts": [component, symbol, package, binding, mid_mount, straddle]}),
    )?;
    let project = board.project()?;
    let terminals = rows(&project["library"]["components"][0]["definition"]["terminals"])?
        .iter()
        .filter_map(|t| Some((t["name"].as_str()?.to_owned(), t["id"].as_u64()?)))
        .collect();
    Ok(Connector {
        component,
        symbol,
        binding,
        mid_mount,
        straddle,
        terminals,
    })
}

/// The nets of the contacts that are connected; the rest are marked
/// no-connect.
const NETS: [(&str, &str); 6] = [
    ("B2", "LANE_P"),
    ("B3", "LANE_N"),
    ("A1", "GND"),
    ("A12", "GND"),
    ("B1", "GND"),
    ("B12", "GND"),
];

/// J1 and J2 on the schematic, their six connected contacts on three nets.
fn schematic(board: &mut Board, connector: &Connector) -> Fallible<[u64; 2]> {
    let mut instances = Vec::new();
    for (designator, footprint, x) in [
        ("J1", connector.mid_mount, 0.0),
        ("J2", connector.straddle, 120.0),
    ] {
        let reply = board.apply(
            "Instance",
            "component.place",
            json!({
                "component": {"id": connector.component, "revision": 1},
                "designator": designator, "properties": [],
                "representation": connector.symbol, "unit": 1, "position": [x, 0.0],
                "physical": {"binding": connector.binding, "footprint": footprint},
                "sheet": 1,
            }),
        )?;
        instances.push(id(&reply["outcome"]["instance"])?);
    }
    let terminal = |name: &str| -> Fallible<u64> {
        connector
            .terminals
            .get(name)
            .copied()
            .ok_or_else(|| format!("no terminal {name}").into())
    };
    let mut row = 0.0;
    for (instance, x) in [(instances[0], -60.0), (instances[1], 60.0)] {
        for (contact, net) in NETS {
            row -= 5.0;
            board.apply(
                "Schematic",
                "label.place",
                json!({
                    "sheet": 1, "name": net, "kind": {"type": "local"}, "position": [x, row],
                    "attach": {"instance": instance, "terminal": terminal(contact)?},
                }),
            )?;
        }
        let unused: Vec<u64> = connector
            .terminals
            .iter()
            .filter(|(name, _)| NETS.iter().all(|(contact, _)| *contact != name.as_str()))
            .map(|(_, id)| *id)
            .collect();
        board.apply(
            "Instance",
            "instance.set_no_connect",
            json!({"instance": instance, "terminals": unused, "no_connect": true}),
        )?;
    }
    Ok([instances[0], instances[1]])
}

/// The profile's pad to board edge clearance and where it comes from.
const EDGE_RULE: &str = "needs 0.2 mm (JLCPCB 4-layer: pad to board edge clearance)";

/// Steps 2 and 3, the parts: placed at the board edges (a pad nearer the
/// edge than the fabricator's 0.2 mm is reported, on either face), held to
/// the notch and the top edge by dimensions, and following the notch's
/// parameter.
fn place(board: &mut Board, edges: &Edges, [j1, j2]: [u64; 2]) -> Fallible {
    let centre = WIDTH / 2.0;
    let put = |at: [f64; 2], instance: u64, turn: f64| json!({"instance": instance, "position": at, "rotation_deg": turn, "layer": "Top"});
    // 0.1 mm too far out: every pad of both faces is 0.15 mm from the edge.
    board.apply(
        "Layout",
        "layout.place",
        put([centre, HEIGHT + 0.1], j2, 180.0),
    )?;
    let close = board.found("board.edge_clearance")?;
    assert_eq!(close.len(), 24, "{close:?}");
    for row in ['A', 'B'] {
        let message = format!("J2 pad {row}7 is 0.15 mm from the board edge; {EDGE_RULE}");
        assert!(
            close
                .iter()
                .any(|i| i.severity == "error" && i.message == message),
            "{close:?}"
        );
    }
    board.apply("Layout", "layout.move", put([centre, HEIGHT], j2, 180.0))?;
    // 0.1 mm into the notch: the front row is 0.15 mm from the notch's
    // floor, which is board edge like any other.
    board.apply(
        "Layout",
        "layout.place",
        put([centre, NOTCH_DRAWN - 0.1], j1, 0.0),
    )?;
    let close = board.found("board.edge_clearance")?;
    assert_eq!(close.len(), 12, "{close:?}");
    assert!(
        close
            .iter()
            .all(|i| i.message.starts_with("J1 pad B") && i.message.ends_with(EDGE_RULE)),
        "{close:?}"
    );
    board.apply("Layout", "layout.move", put([centre, NOTCH_DRAWN], j1, 0.0))?;
    assert_eq!(board.found("board.edge_clearance")?, vec![]);
    // Held by dimensions, not coordinates: the origin on the notch's floor
    // (or the top edge) and on the board's centre line.
    for (instance, edge) in [(j1, &edges.notch), (j2, &edges.top)] {
        let origin =
            json!({"part": {"feature": "origin", "target": {"kind": "placement", "id": instance}}});
        for (curve, value) in [
            (edge, json!({"fixed": 0.0})),
            (&edges.left, json!({"expr": "width / 2"})),
        ] {
            board.apply(
                "Dimension",
                "layout.dimension_add",
                json!({"dimension": {
                    "kind": {"dim": "distance"},
                    "refs": [origin.clone(), {"board": {"curve": curve}}],
                    "value": value,
                }}),
            )?;
        }
    }
    assert!(near(board.placement(j1)?.0, [centre, NOTCH_DRAWN]));
    // The driving parameter: a deeper notch, and J1 follows its floor.
    board.set_parameter("notch_d", NOTCH_DEPTH, "Depth of the connector notch")?;
    assert_eq!(outer(board)?, notched(NOTCH_DEPTH));
    assert!(near(board.placement(j1)?.0, [centre, NOTCH_DEPTH]));
    let (at, turn) = board.placement(j2)?;
    assert!(near(at, [centre, HEIGHT]) && (turn - 180.0).abs() < 1e-9);
    // Every pad where the tables and the parameters put it; J2's B row on
    // the bottom face, B12 under A1.
    for designator in ["J1", "J2"] {
        for row in ['A', 'B'] {
            for i in 1..=12 {
                let (at, layer, _) = board.pad(designator, &format!("{row}{i}"))?;
                let expected = pad_at(designator, row, i, NOTCH_DEPTH);
                assert!(near(at, expected), "{designator}.{row}{i}: {at:?}");
                let bottom = designator == "J2" && row == 'B';
                assert_eq!(layer, if bottom { "Bottom" } else { "Top" });
            }
        }
    }
    assert!(near(board.pad("J2", "B12")?.0, board.pad("J2", "A1")?.0));
    assert_eq!(board.errors()?, vec![]);
    Ok(())
}

/// Step 4, the rules: an 85 Ω pair class on the smallest via and the pair.
fn pair(board: &mut Board) -> Fallible<[u64; 2]> {
    let lane = [board.net("LANE_P")?, board.net("LANE_N")?];
    let reply = board.apply(
        "Rules",
        "net_class.add",
        json!({"class": {
            "name": "Lane", "via_drill": VIA[0], "via_diameter": VIA[1],
            "signal": {"diff_impedance": 85.0, "impedance_tolerance": 10.0, "pair_max_skew": 0.1},
        }}),
    )?;
    let class = id(&reply["outcome"]["class"])?;
    board.apply(
        "Rules",
        "net_class.assign",
        json!({"class": class, "nets": lane}),
    )?;
    let reply = board.apply(
        "Rules",
        "diff_pair.add",
        json!({"pair": {"name": "LANE", "positive": lane[0], "negative": lane[1]}}),
    )?;
    assert_eq!(reply["outcome"]["pair"], 0, "{reply}");
    Ok(lane)
}

/// The centre line of the pair between the two connectors, mm from each
/// row of pads.
const FAN: f64 = 1.0;

/// `layout.route_pair` for the pair on the bottom layer, from J1's B2/B3
/// to J2's.
fn route(notch: f64) -> Value {
    let start = [pad_at("J1", 'B', 2, notch), pad_at("J1", 'B', 3, notch)];
    let end = [pad_at("J2", 'B', 2, notch), pad_at("J2", 'B', 3, notch)];
    let x = (start[0][0] + start[1][0]) / 2.0;
    json!({"request": {
        "pair": 0, "start": start, "end": end,
        "legs": [{"layer": "Bottom", "points": [[x, start[0][1] + FAN], [x, end[0][1] - FAN]]}],
    }})
}

/// Step 4, the copper, and step 5's plane: a via in each of J1's B2 and
/// B3 pads, the pair on the bottom layer from those vias, a ground via in
/// every ground pad and a ground fill on the inner layer over the pair.
fn copper(board: &mut Board, lane: [u64; 2]) -> Fallible {
    for (net, contact) in [(lane[0], 2), (lane[1], 3)] {
        let reply = board.via(net, pad_at("J1", 'B', contact, NOTCH_DEPTH));
        assert_eq!(reply["ok"], true, "{reply}");
        // In the pad of its own net, 0.5 mm from the next via: under the
        // absolute minimum values nothing but its open hole under the paste.
        let new: Vec<&str> = rows(&reply["result"]["findings"]["new"])?
            .iter()
            .filter_map(|f| f["code"].as_str())
            .collect();
        assert_eq!(new, ["board.via_in_pad_open"], "{reply}");
    }
    let reply = board.apply("Layout", "layout.route_pair", route(NOTCH_DEPTH))?;
    let new: Vec<&str> = rows(&reply["findings"]["new"])?
        .iter()
        .filter_map(|f| f["code"].as_str())
        .collect();
    assert_eq!(new, ["impedance.uncoupled"], "{reply}");
    let ground = board.net("GND")?;
    for (designator, row, contact) in [
        ("J1", 'A', 1),
        ("J1", 'A', 12),
        ("J1", 'B', 1),
        ("J1", 'B', 12),
        // Through J2's A1 and A12 into B12 and B1 under them.
        ("J2", 'A', 1),
        ("J2", 'A', 12),
    ] {
        let reply = board.via(ground, pad_at(designator, row, contact, NOTCH_DEPTH));
        assert_eq!(reply["ok"], true, "{reply}");
    }
    board.apply(
        "Fill",
        "fill.add",
        json!({"fill": {"name": "GND plane", "net": ground, "layer": {"Inner": 2}, "region": "board"}}),
    )?;
    let reply = board.apply("Fill", "fill.refill", json!({"target": {"fills": "all"}}))?;
    // The plane joins the eight ground pads through their vias. Before it,
    // six of them are named unrouted: board.unrouted names the pads the
    // airwires join (A21, 9a6f4cd8), and J2's B12 and B1 are already joined
    // to its A1 and A12 above them through their vias, so no airwire ends
    // there. Named before the plane, measured: J1 A1, A12, B1, B12 and J2
    // A1, A12. All six are resolved; with every pad of the split net named
    // (before A21) it was eight.
    assert_eq!(
        reply["findings"]["resolved"]["board.unrouted"], 6,
        "{reply}"
    );
    Ok(())
}

fn build(stage: Stage) -> Fallible<Built> {
    let mut board = Board::new()?;
    start(&mut board)?;
    let edges = outline(&mut board)?;
    let connector = connector(&mut board)?;
    let [j1, j2] = schematic(&mut board, &connector)?;
    place(&mut board, &edges, [j1, j2])?;
    let lane = pair(&mut board)?;
    if stage >= Stage::Routed {
        copper(&mut board, lane)?;
    }
    Ok(Built {
        board,
        j1,
        j2,
        lane,
    })
}

/// The codes of the production check's items, sorted.
fn preflight_codes(report: &Value) -> Fallible<Vec<String>> {
    let mut codes: Vec<String> = rows(&report["items"])?
        .iter()
        .filter_map(|i| i["code"].as_str().map(str::to_owned))
        .collect();
    codes.sort();
    Ok(codes)
}

/// Steps 1 to 5 as a whole: the finished board has no error, the pair is
/// what was asked for, and the production check passes with the warnings
/// a bare two-connector board has.
#[test]
fn the_finished_board_passes_the_production_check() -> Fallible {
    let Built { mut board, .. } = build(Stage::Routed)?;
    assert_eq!(board.errors()?, vec![]);
    // 0.8 mm: the laminate and copper entered.
    let stackup = board.project()?["stackup"].take();
    let entered = rows(&stackup["dielectrics"])?
        .iter()
        .filter_map(|d| d["thickness_mm"].as_f64())
        .sum::<f64>()
        + rows(&stackup["copper"])?
            .iter()
            .filter_map(|c| c["thickness_um"].as_f64())
            .sum::<f64>()
            / 1000.0;
    assert!((entered - thickness()).abs() < 1e-9 && (thickness() - 0.805).abs() < 1e-9);
    // Each half of the pair changes layer once, in its pad, and the two
    // halves are the same length; only the fan-outs run uncoupled.
    let report = board.query("layout.length_report", json!({}))?;
    for name in ["LANE_P", "LANE_N"] {
        let row = rows(&report["nets"])?
            .iter()
            .find(|n| n["name"] == name)
            .ok_or("no length row")?;
        assert_eq!(row["length"]["via_count"], 1, "{row}");
    }
    let lane = &report["pairs"][0];
    assert!(
        lane["skew"].as_f64().is_some_and(|skew| skew.abs() < 1e-9),
        "{lane}"
    );
    let uncoupled = lane["uncoupled"].as_f64().ok_or("no uncoupled length")?;
    assert!(uncoupled > 0.0 && uncoupled < 0.5, "{lane}");
    let preflight = board.query("fabrication.preflight", json!({}))?;
    assert_eq!(preflight["profile"], "JLCPCB 4-layer");
    assert_eq!(
        (&preflight["errors"], &preflight["blocking"]),
        (&json!(0), &json!(0))
    );
    assert_eq!(
        preflight_codes(&preflight)?,
        [
            // Both faces carry surface-mount pads, so both want fiducials:
            // the plug's bottom row counts as an assembled side.
            "board.fiducials_too_few",
            "board.fiducials_too_few",
            // Every via is an open hole in a pad: the pair's two, J1's four
            // ground vias, and J2's two, each in a pad of either face.
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "board.via_in_pad_open",
            "fabrication.envelope_model",
            "fabrication.envelope_model",
            "impedance.uncoupled",
            "purchasing.no_part",
            "purchasing.no_part",
        ]
    );
    Ok(())
}

/// The fabricator's limits doing their job on this geometry: the small
/// vias only pass under the absolute minimum values, and a via through a
/// straddle part's top pad is refused for the other net's pad under it.
#[test]
fn the_fabricator_limits_are_reported_not_passed() -> Fallible {
    let Built { mut board, .. } = build(Stage::Routed)?;
    // "Preferred Min. Via hole size: 0.2mm" under the recommended values.
    board.apply(
        "Rules",
        "rules.set_fab_options",
        json!({"options": {"limits": "recommended"}}),
    )?;
    let small = board.found("board.via_drill_size")?;
    assert_eq!(small.len(), 8, "{small:?}");
    assert!(
        small.iter().all(|i| i.severity == "error"
            && i.message.ends_with(
                "drill 0.15 mm is under the minimum; needs 0.2 mm (JLCPCB 4-layer: min drill)"
            )),
        "{small:?}"
    );
    // The rules moved, so the plane is stale: filled again, the vias are
    // all that blocks production.
    board.apply("Fill", "fill.refill", json!({"target": {"fills": "all"}}))?;
    let preflight = board.query("fabrication.preflight", json!({}))?;
    assert_eq!(preflight["blocking"], 8, "{preflight}");
    let blocking: BTreeSet<&str> = rows(&preflight["items"])?
        .iter()
        .filter(|i| i["severity"] == "error")
        .filter_map(|i| i["code"].as_str())
        .collect();
    assert_eq!(blocking, BTreeSet::from(["board.via_drill_size"]));
    board.undo()?;
    board.undo()?;
    assert_eq!(board.errors()?, vec![]);
    // J2's A2 is on the top face, B11 on the bottom face under it: a
    // through via in the one pad lands in the other.
    let net = board.pad("J2", "A2")?.2;
    let net = board.net(&net)?;
    let revision = board.revision;
    let reply = board.via(net, pad_at("J2", 'A', 2, NOTCH_DEPTH));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(
        reply["error"]["message"],
        "Too close to J2 pad B11: needs 0.09 mm (JLCPCB 4-layer)"
    );
    assert_eq!(board.session.engine().revision(), revision);
    Ok(())
}

/// A board parameter changed after routing: the connectors move with the
/// board, the vias stay in their pads, the pair stays connected, and one
/// refill brings the plane back.
#[test]
fn a_parameter_change_carries_the_routed_connectors() -> Fallible {
    let Built {
        mut board,
        j1,
        j2,
        lane,
        ..
    } = build(Stage::Routed)?;
    const WIDER: f64 = 2.0;
    board.set_parameter("width", WIDTH + WIDER, "Board width")?;
    let moved = |p: [f64; 2]| [p[0] + WIDER / 2.0, p[1]];
    assert!(near(
        board.placement(j1)?.0,
        moved([WIDTH / 2.0, NOTCH_DEPTH])
    ));
    assert!(near(board.placement(j2)?.0, moved([WIDTH / 2.0, HEIGHT])));
    let project = board.project()?;
    for (net, contact) in [(lane[0], 2), (lane[1], 3)] {
        let at = moved(pad_at("J1", 'B', contact, NOTCH_DEPTH));
        assert!(
            rows(&project["vias"])?
                .iter()
                .any(|v| v["net"] == net && point(&v["position"]).is_ok_and(|p| near(p, at))),
            "the via of contact B{contact} at {at:?}"
        );
    }
    // The stale plane is the only thing to mend.
    let codes: BTreeSet<String> = board.errors()?.into_iter().map(|i| i.code).collect();
    assert!(
        codes
            .iter()
            .all(|c| ["fill.clearance", "fill.overlap", "fill.board_edge"].contains(&c.as_str())),
        "{codes:?}"
    );
    board.apply("Fill", "fill.refill", json!({"target": {"fills": "all"}}))?;
    assert_eq!(board.errors()?, vec![]);
    assert_eq!(board.found("board.unrouted")?, vec![]);
    let report = board.query("layout.length_report", json!({}))?;
    let skew = report["pairs"][0]["skew"].as_f64();
    assert!(skew.is_some_and(|skew| skew.abs() < 1e-9), "{report}");
    Ok(())
}

/// One fabrication file read back.
struct Film {
    file: gerber::GerberFile,
    /// The whole image: disjoint pieces of copper (or openings).
    image: Shapes,
}

impl Film {
    fn read(dir: &Path, base: &str, suffix: &str) -> Fallible<Self> {
        let file = gerber::parse(&std::fs::read_to_string(
            dir.join(format!("{base}-{suffix}.gbr")),
        )?)?;
        let image = polygons::render(&file, |_| true)?;
        Ok(Self { file, image })
    }

    /// The piece of the image that covers `at`.
    fn piece(&self, at: [f64; 2]) -> Option<usize> {
        self.image.iter().position(|shape| covers(shape, at))
    }

    /// The flashes that carry pad attribute `.P` of `designator`:
    /// `(pad number, at, net)`.
    fn pads(&self, designator: &str) -> Vec<(String, [f64; 2], String)> {
        self.file
            .flashes()
            .filter_map(|o| {
                let pad = o.object_attributes.get(".P")?;
                let gerber::Kind::Flash { at } = &o.kind else {
                    return None;
                };
                (pad.first()? == designator).then(|| {
                    (
                        pad.get(1).cloned().unwrap_or_default(),
                        *at,
                        net_of(o).unwrap_or_default(),
                    )
                })
            })
            .collect()
    }

    fn flashes_at(&self) -> Vec<[f64; 2]> {
        self.file
            .flashes()
            .filter_map(|o| match &o.kind {
                gerber::Kind::Flash { at } => Some(*at),
                _ => None,
            })
            .collect()
    }
}

/// The net attribute `.N` of an object.
fn net_of(object: &gerber::Object) -> Option<String> {
    object.object_attributes.get(".N")?.first().cloned()
}

/// A point of an object's copper: a flash's centre or a stroke's start. A
/// region has none: every region here is the fill, whose net the vias on
/// it name.
fn anchor(object: &gerber::Object) -> Option<[f64; 2]> {
    match &object.kind {
        gerber::Kind::Flash { at } => Some(*at),
        gerber::Kind::Draw { from, .. } | gerber::Kind::Arc { from, .. } => Some(*from),
        gerber::Kind::Region { .. } => None,
    }
}

/// Whether `shape` (an outer ring and its holes) covers `at`: even-odd
/// over its rings.
fn covers(shape: &Shape, at: [f64; 2]) -> bool {
    let (x, y) = (at[0] * 1e6, at[1] * 1e6);
    let mut inside = false;
    for ring in shape {
        for (k, a) in ring.iter().enumerate() {
            let b = &ring[(k + 1) % ring.len()];
            let (ax, ay, bx, by) = (
                f64::from(a.x),
                f64::from(a.y),
                f64::from(b.x),
                f64::from(b.y),
            );
            if (ay > y) != (by > y) && x < ax + (y - ay) * (bx - ax) / (by - ay) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Copper joined across the four copper files: pieces of one layer's image
/// are joined by the plated holes that pass through them.
struct Copper {
    layers: Vec<Film>,
    /// For each layer and piece, the set it belongs to (the smallest
    /// `(layer, piece)` joined to it).
    joined: BTreeMap<(usize, usize), (usize, usize)>,
}

impl Copper {
    fn read(dir: &Path, base: &str, holes: &[[f64; 2]]) -> Fallible<Self> {
        let layers = [
            "copper-l1-top",
            "copper-l2-inner1",
            "copper-l3-inner2",
            "copper-l4-bottom",
        ]
        .iter()
        .map(|suffix| Film::read(dir, base, suffix))
        .collect::<Fallible<Vec<_>>>()?;
        let mut joined = BTreeMap::new();
        for (l, film) in layers.iter().enumerate() {
            for piece in 0..film.image.len() {
                joined.insert((l, piece), (l, piece));
            }
        }
        let mut copper = Self { layers, joined };
        for hole in holes {
            let through: Vec<(usize, usize)> = copper
                .layers
                .iter()
                .enumerate()
                .filter_map(|(l, film)| Some((l, film.piece(*hole)?)))
                .collect();
            assert_eq!(
                through.len(),
                4,
                "a plated hole at {hole:?} has a pad on every layer"
            );
            for pair in through.windows(2) {
                copper.join(pair[0], pair[1]);
            }
        }
        Ok(copper)
    }

    fn set(&self, mut node: (usize, usize)) -> (usize, usize) {
        while let Some(next) = self.joined.get(&node).filter(|n| **n != node) {
            node = *next;
        }
        node
    }

    fn join(&mut self, a: (usize, usize), b: (usize, usize)) {
        let (a, b) = (self.set(a), self.set(b));
        self.joined.insert(a.max(b), a.min(b));
    }

    /// The set of copper that covers `at` on `layer` (0 is the top).
    fn at(&self, layer: usize, at: [f64; 2]) -> Fallible<(usize, usize)> {
        let piece = self.layers[layer]
            .piece(at)
            .ok_or_else(|| format!("no copper at {at:?} on layer {}", layer + 1))?;
        Ok(self.set((layer, piece)))
    }

    /// The net names the objects of each joined set carry.
    fn nets(&self) -> Fallible<BTreeMap<(usize, usize), BTreeSet<String>>> {
        let mut nets: BTreeMap<(usize, usize), BTreeSet<String>> = BTreeMap::new();
        for (l, film) in self.layers.iter().enumerate() {
            for object in &film.file.objects {
                let (Some(net), Some(at)) = (net_of(object), anchor(object)) else {
                    continue;
                };
                nets.entry(self.at(l, at)?).or_default().insert(net);
            }
        }
        Ok(nets)
    }
}

/// Step 5, the output: `project.export_fabrication` writes the board, and
/// the files, read back, are the board: the bottom-face pads on the bottom
/// copper, mask and paste and not on the top, the notch in the profile,
/// the small vias in the plated drill file, one net on every joined piece
/// of copper, and the pair continuous from J1's pads through the vias to
/// the bottom tracks and J2's bottom pads.
#[test]
fn the_fabrication_files_read_back_as_the_board() -> Fallible {
    let Built { mut board, .. } = build(Stage::Routed)?;
    let dir = tempfile::tempdir()?;
    let base = "edge";
    // Saved first: a project never saved has no identity of its own yet,
    // and its files then carry the release's digest as `.ProjectId`.
    let saved = dir.path().join("edge.sparksmith");
    board.ok(json!({"command": "project.save", "params": {"path": saved}}))?;
    let export = |board: &mut Board, folder: &str| {
        board.ok(json!({"command": "project.export_fabrication", "params": {
            "output": dir.path().join(folder), "options": {"base_name": base},
        }}))
    };
    let written = export(&mut board, "fab")?;
    assert_eq!(written["preflight"]["blocking"], 0, "{written}");
    let fab = dir.path().join("fab");
    // Without a creation date the set is the same bytes each time: this is
    // the request `sparksmith fab --date none` sends for the saved file.
    export(&mut board, "again")?;
    for file in rows(&written["files"])? {
        let name = file["name"].as_str().ok_or("a file name")?;
        assert!(
            std::fs::read(fab.join(name))? == std::fs::read(dir.path().join("again").join(name))?,
            "{name} differs between two exports"
        );
    }

    // The drill file: one plated tool of 0.15 mm for all eight vias.
    let drill = excellon::parse(&std::fs::read_to_string(
        fab.join(format!("{base}-plated.drl")),
    )?)?;
    assert_eq!(
        drill.attribute(".FileFunction"),
        Some(vec!["Plated".into(), "1".into(), "4".into(), "PTH".into()])
    );
    let tools: Vec<(u32, f64)> = drill.tools.iter().map(|(t, d)| (*t, *d)).collect();
    assert_eq!(tools, [(1, VIA[0])]);
    // KNOWN GAP (via fill): the vias in pads are plain via drills. Nothing
    // in the drill file, the copper or the drawings says they are filled
    // and capped; a via has no such attribute to state.
    assert_eq!(
        drill.tool_functions.get(&1).map(Vec::as_slice),
        Some(&["ViaDrill".to_owned()][..])
    );
    let holes: Vec<[f64; 2]> = drill
        .hits
        .iter()
        .filter_map(|hit| match *hit {
            excellon::Hit::Round { at, .. } => Some(at),
            excellon::Hit::Slot { .. } => None,
        })
        .collect();
    assert_eq!(holes.len(), 8);
    let lane_vias = [
        pad_at("J1", 'B', 2, NOTCH_DEPTH),
        pad_at("J1", 'B', 3, NOTCH_DEPTH),
    ];
    for at in lane_vias {
        assert!(holes.iter().any(|h| near(*h, at)), "a hole at {at:?}");
    }
    assert!(!fab.join(format!("{base}-non-plated.drl")).exists());

    // The copper: pads by their `.P` attribute, each where the tables put
    // it, on the face they are on, with their net.
    let copper = Copper::read(&fab, base, &holes)?;
    let (top, bottom) = (&copper.layers[0], &copper.layers[3]);
    assert_eq!(
        bottom.file.file_attribute(".FileFunction"),
        Some(&["Copper".to_owned(), "L4".to_owned(), "Bot".to_owned()][..])
    );
    let expected = |designator: &str, row: char| -> BTreeMap<String, ((i64, i64), String)> {
        (1..=12)
            .map(|i| {
                let number = format!("{row}{i}");
                let net = NETS
                    .iter()
                    .find(|(contact, _)| *contact == number)
                    .map(|(_, net)| (*net).to_owned());
                (
                    number,
                    (
                        um(pad_at(designator, row, i, NOTCH_DEPTH)),
                        net.unwrap_or_default(),
                    ),
                )
            })
            .collect()
    };
    let read = |film: &Film, designator: &str| -> BTreeMap<String, ((i64, i64), String)> {
        film.pads(designator)
            .into_iter()
            .map(|(number, at, net)| {
                // An unconnected contact's net is its own, named by the tool.
                let named = NETS.iter().any(|(_, n)| *n == net);
                (number, (um(at), if named { net } else { String::new() }))
            })
            .collect()
    };
    assert_eq!(read(bottom, "J2"), expected("J2", 'B'));
    assert_eq!(read(bottom, "J1"), BTreeMap::new());
    assert_eq!(read(top, "J2"), expected("J2", 'A'));
    let mut j1 = expected("J1", 'A');
    j1.extend(expected("J1", 'B'));
    assert_eq!(read(top, "J1"), j1);
    for inner in &copper.layers[1..3] {
        assert_eq!(inner.pads("J1").len() + inner.pads("J2").len(), 0);
    }

    // Mask and paste: an opening for each pad on its own face, 36 on the
    // top and J2's 12 B pads on the bottom, each the pad's size.
    let behind: BTreeSet<(i64, i64)> = (1..=12)
        .map(|i| um(pad_at("J2", 'B', i, NOTCH_DEPTH)))
        .collect();
    for kind in ["mask", "paste"] {
        let under = Film::read(&fab, base, &format!("{kind}-bottom"))?;
        let at: BTreeSet<(i64, i64)> = under.flashes_at().into_iter().map(um).collect();
        assert_eq!(under.file.objects.len(), 12, "{kind}");
        assert_eq!(at, behind, "{kind}");
        assert!((polygons::area_mm2(&under.image) - 12.0 * PAD[0] * PAD[1]).abs() < 1e-6);
        let over = Film::read(&fab, base, &format!("{kind}-top"))?;
        assert_eq!(over.file.objects.len(), 36, "{kind}");
        assert!((polygons::area_mm2(&over.image) - 36.0 * PAD[0] * PAD[1]).abs() < 1e-6);
        // KNOWN GAP (via fill): the paste opening of a pad with a via in
        // it is the whole pad, over the hole, as if the via were filled
        // and capped; the tool neither knows nor asks.
        for at in lane_vias {
            assert!(over.piece(at).is_some(), "{kind} over the via at {at:?}");
        }
    }

    // The profile: the notch is part of the one outline.
    let profile = Film::read(&fab, base, "profile")?;
    assert_eq!(
        profile.file.file_attribute(".FileFunction"),
        Some(&["Profile".to_owned(), "NP".to_owned()][..])
    );
    let mut strokes = BTreeSet::new();
    for object in &profile.file.objects {
        let gerber::Kind::Draw { from, to } = &object.kind else {
            return Err(format!("the profile holds {:?}", object.kind).into());
        };
        strokes.insert((um(*from).min(um(*to)), um(*from).max(um(*to))));
    }
    let corners: BTreeSet<(i64, i64)> = strokes.iter().flat_map(|(a, b)| [*a, *b]).collect();
    assert_eq!(corners, notched(NOTCH_DEPTH));
    assert_eq!(strokes.len(), 8);
    let (left, right) = ((WIDTH - NOTCH_WIDTH) / 2.0, (WIDTH + NOTCH_WIDTH) / 2.0);
    for (a, b) in [
        ([left, 0.0], [left, NOTCH_DEPTH]),
        ([left, NOTCH_DEPTH], [right, NOTCH_DEPTH]),
        ([right, 0.0], [right, NOTCH_DEPTH]),
    ] {
        assert!(strokes.contains(&(um(a).min(um(b)), um(a).max(um(b)))));
    }

    // Net attributes agree with the copper: every joined piece of copper
    // carries one net name, on pads, vias, tracks and the fill alike.
    let nets = copper.nets()?;
    for (set, names) in &nets {
        assert_eq!(names.len(), 1, "copper {set:?} carries {names:?}");
    }
    // The pair, from the output alone: J1's top pad, the via in it, the
    // via's pads on both inner layers, the bottom track and J2's bottom
    // pad are one piece of copper, named for the net; the two halves and
    // ground are three different pieces.
    let mut halves = Vec::new();
    for (contact, name) in [(2, "LANE_P"), (3, "LANE_N")] {
        let from = pad_at("J1", 'B', contact, NOTCH_DEPTH);
        let to = pad_at("J2", 'B', contact, NOTCH_DEPTH);
        let set = copper.at(0, from)?;
        for layer in 1..4 {
            assert_eq!(copper.at(layer, from)?, set, "{name} through its via");
        }
        assert_eq!(copper.at(3, to)?, set, "{name} to J2 on the bottom");
        // Midway, the track itself.
        let x = (pad_at("J1", 'B', 2, NOTCH_DEPTH)[0] + pad_at("J1", 'B', 3, NOTCH_DEPTH)[0]) / 2.0;
        let side = if from[0] > x { 1.0 } else { -1.0 };
        let track = (0..400)
            .map(|k| [x + side * f64::from(k) * 0.001, HEIGHT / 2.0])
            .find_map(|p| copper.layers[3].piece(p));
        assert_eq!(track.map(|piece| copper.set((3, piece))), Some(set));
        assert_eq!(nets.get(&set), Some(&BTreeSet::from([name.to_owned()])));
        // Nothing of J2's top face is on it: the via is J1's.
        assert!(
            copper.layers[0]
                .piece(to)
                .is_none_or(|p| copper.set((0, p)) != set)
        );
        halves.push(set);
    }
    let ground = copper.at(0, pad_at("J1", 'B', 1, NOTCH_DEPTH))?;
    for (designator, row, contact, layer) in [
        ("J1", 'A', 1, 0),
        ("J1", 'A', 12, 0),
        ("J1", 'B', 12, 0),
        ("J2", 'A', 1, 0),
        ("J2", 'A', 12, 0),
        ("J2", 'B', 1, 3),
        ("J2", 'B', 12, 3),
    ] {
        let at = pad_at(designator, row, contact, NOTCH_DEPTH);
        assert_eq!(copper.at(layer, at)?, ground, "{designator}.{row}{contact}");
    }
    // The plane: the largest piece of the third layer, and ground.
    let plane = copper.layers[2]
        .image
        .iter()
        .enumerate()
        .max_by(|a, b| {
            polygons::area_mm2(std::slice::from_ref(a.1))
                .total_cmp(&polygons::area_mm2(std::slice::from_ref(b.1)))
        })
        .map(|(piece, _)| copper.set((2, piece)));
    assert_eq!(plane, Some(ground));
    assert!(halves[0] != halves[1] && halves[0] != ground && halves[1] != ground);
    Ok(())
}

/// Step 6: the native 3D export builds every body the board plan
/// describes, and the plan holds the notch in every laminate slab, J2's B
/// row under the board and a barrel through the board for each via.
#[test]
fn the_native_model_has_the_notch_and_the_bottom_face_pads() -> Fallible {
    use sparksmith_core::CopperLayer;
    let Built {
        mut board, j1, j2, ..
    } = build(Stage::Routed)?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("edge.solidcraft");
    let exported = board.ok(json!({"command": "project.export_native", "params": {
        "path": path, "tier": "full",
    }}))?;
    let check = &exported["check"];
    assert_eq!(check["differs"], false, "{exported}");
    assert_eq!(check["difference_count"], 0, "{exported}");
    assert_eq!(check["planned"], check["built"], "{exported}");
    assert!(std::fs::metadata(&path)?.len() > 0);

    // What was planned, and so built: the plan the export checked against.
    let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
    let plan = sparksmith_solidcraft::project::plan::of(board.session.engine().project())?;
    assert_eq!(check["planned"], plan.bodies().count(), "{exported}");
    let area = WIDTH * HEIGHT - NOTCH_WIDTH * NOTCH_DEPTH;
    assert!(!plan.laminate.is_empty());
    for slab in &plan.laminate {
        let loops = slab.board_loops();
        let outline = loops.first().ok_or("a slab without an outline")?;
        assert!((outline.area() - area).abs() < 1e-6, "{}", slab.name);
        let (low, high) = slab.bounds()?;
        assert!(near([low[0], low[1]], [0.0, 0.0]) && near([high[0], high[1]], [WIDTH, HEIGHT]));
    }
    let top_of_board = plan
        .laminate
        .iter()
        .map(|slab| slab.z[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let lands = |instance: u64, layer: CopperLayer| -> Vec<[f64; 2]> {
        plan.pads
            .iter()
            .filter(|p| p.instance == instance)
            .flat_map(|p| &p.bodies)
            .filter(|b| b.layer == Some(layer))
            .map(|b| b.z)
            .collect()
    };
    // J2's B row hangs under the laminate (z = 0 at its bottom face); the
    // other 36 pads stand on it.
    let under = lands(j2, CopperLayer::Bottom);
    assert_eq!(under.len(), 12);
    assert!(
        under.iter().all(|z| z[1].abs() < 1e-9 && z[0] < 0.0),
        "{under:?}"
    );
    let over = [lands(j2, CopperLayer::Top), lands(j1, CopperLayer::Top)].concat();
    assert_eq!(over.len(), 36);
    assert!(
        over.iter()
            .all(|z| (z[0] - top_of_board).abs() < 1e-9 && z[1] > top_of_board),
        "{over:?}"
    );
    assert_eq!(lands(j1, CopperLayer::Bottom).len(), 0);
    // Eight vias, each one barrel through the whole board.
    use sparksmith_solidcraft::project::plan::{Origin, ViaPart};
    assert_eq!(plan.vias.len(), 8);
    for via in &plan.vias {
        let barrels: Vec<[f64; 2]> = via
            .bodies
            .iter()
            .filter(|b| {
                matches!(
                    b.source,
                    Origin::Via {
                        part: ViaPart::Barrel,
                        ..
                    }
                )
            })
            .map(|b| b.z)
            .collect();
        let [z] = barrels[..] else {
            return Err(format!("via {} has {} barrels", via.via, barrels.len()).into());
        };
        assert!(z[0] <= 1e-9 && z[1] >= top_of_board - 1e-9, "{z:?}");
    }
    Ok(())
}

/// `layout.route_pair` with a layer change at J1's B row: one request
/// takes the pair from the pads through a via pair to the bottom layer and
/// on to J2. The via pair is spaced by the rules, not by the pads.
#[test]
fn a_pair_layer_change_at_the_pads_spaces_its_vias_by_the_rules() -> Fallible {
    let Built {
        mut board, lane, ..
    } = build(Stage::Paired)?;
    let start = [
        pad_at("J1", 'B', 2, NOTCH_DEPTH),
        pad_at("J1", 'B', 3, NOTCH_DEPTH),
    ];
    let end = [
        pad_at("J2", 'B', 2, NOTCH_DEPTH),
        pad_at("J2", 'B', 3, NOTCH_DEPTH),
    ];
    let centre = [(start[0][0] + start[1][0]) / 2.0, start[0][1]];
    // The layer changes where the top leg ends, here at the pads
    // themselves: the bottom leg starts at the shared point.
    board.apply(
        "Layout",
        "layout.route_pair",
        json!({"request": {
            "pair": 0, "start": start, "end": end,
            "legs": [
                {"layer": "Top", "points": [centre]},
                {"layer": "Bottom", "points": [centre, [centre[0], end[0][1] - FAN]]},
            ],
        }}),
    )?;
    assert_eq!(board.errors()?, vec![]);
    let open = board.found("board.unrouted")?;
    assert!(
        open.iter().all(|i| i.message.ends_with("(GND)")),
        "{open:?}"
    );
    // The pitch of the via pair under this profile: each half's 0.154 mm
    // track, 0.09 mm from the other half's, passes the other half's
    // 0.15 mm hole at the 0.2 mm "via hole to track" row, so
    // 0.15 + 2 × 0.2 − 0.09 = 0.46 mm, and 5 µm to spare rounded up to
    // 0.47 mm.
    //
    // KNOWN GAP (pair vias on pad centres): 0.47 mm is not the pads'
    // 0.5 mm, so each via sits 0.015 mm beside its pad's centre. It is
    // still inside its pad. `layout.add_via` at each pad centre and
    // `layout.route_pair` on the bottom layer from there puts them on the
    // centres ([`copper`]).
    const PAIR_PITCH: f64 = 0.47;
    let project = board.project()?;
    let vias = rows(&project["vias"])?;
    assert_eq!(vias.len(), 2, "{vias:?}");
    for (net, pad, side) in [(lane[0], start[0], 1.0), (lane[1], start[1], -1.0)] {
        let via = vias
            .iter()
            .find(|v| v["net"] == net)
            .ok_or("a via per half")?;
        let at = point(&via["position"])?;
        assert!(
            near(at, [centre[0] + side * PAIR_PITCH / 2.0, pad[1]]),
            "{at:?}"
        );
        assert!(
            (at[0] - pad[0]).abs() + VIA[1] / 2.0 <= PAD[0] / 2.0,
            "{at:?} in its pad"
        );
        assert_eq!(
            (&via["drill"], &via["diameter"]),
            (&json!(VIA[0]), &json!(VIA[1]))
        );
    }
    let report = board.query("layout.length_report", json!({}))?;
    for row in rows(&report["nets"])? {
        assert_eq!(row["length"]["via_count"], 1, "{row}");
    }
    Ok(())
}
