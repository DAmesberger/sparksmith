//! Compact replies through the public JSON contract: what a mutation reports,
//! what `verbose` returns, and that the document ends up the same either way.
use super::*;
use crate::gate::Caller;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn add_via(session: &Session, position: [f64; 2]) -> Value {
    let revision = session.engine().revision();
    let net = session
        .engine()
        .project()
        .nets
        .first()
        .map_or(0, |net| net.id);
    json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Layout", "operation": {"command": "layout.add_via", "params": {
            "via": {"net": net, "position": position, "drill": 0.3, "diameter": 0.6, "span": "through", "tented": true}
        }}},
    }})
}

fn demo() -> std::result::Result<Session, Box<dyn std::error::Error>> {
    Ok(Session::fixture(sparksmith_core::Engine::demo()?))
}

fn ok(reply: Value) -> std::result::Result<Value, Box<dyn std::error::Error>> {
    if reply["ok"] != true {
        return Err(format!("{}", reply["error"]).into());
    }
    Ok(reply["result"].clone())
}

/// A placed via is reported by id, not by document; undo and redo report the
/// same id removed and added again. The whole reply is small.
#[test]
fn a_mutation_reports_its_ids_and_undo_reports_them_back() -> TestResult {
    let mut session = demo()?;
    let revision = session.engine().revision();
    let request = add_via(&session, [1.0, 1.0]);
    let reply = ok(session.respond(Caller::Operator, request))?;
    assert_eq!(reply["revision"], revision + 1);
    assert_eq!(reply["outcome"]["outcome"], "via_added", "{reply}");
    let via = reply["outcome"]["via"].clone();
    assert_eq!(reply["changes"]["added"]["vias"], json!([via]), "{reply}");
    for snapshot_key in ["project", "erc", "board_checks"] {
        assert!(reply.get(snapshot_key).is_none(), "{snapshot_key}");
    }
    assert_eq!(reply["can_undo"], true);

    let undone = ok(session.respond(Caller::Operator, json!({"command": "project.undo", "params": {"expected_revision": session.engine().revision()}})))?;
    assert_eq!(
        undone["changes"]["removed"]["vias"],
        json!([via]),
        "{undone}"
    );
    assert_eq!(undone["can_redo"], true);
    let redone = ok(session.respond(Caller::Operator, json!({"command": "project.redo", "params": {"expected_revision": session.engine().revision()}})))?;
    assert_eq!(redone["changes"]["added"]["vias"], json!([via]), "{redone}");
    Ok(())
}

/// `verbose` beside the command or inside the parameters returns what
/// `dispatch` returns: the snapshot with the outcome.
#[test]
fn verbose_returns_the_snapshot_in_either_place() -> TestResult {
    let mut session = demo()?;
    let mut request = add_via(&session, [1.0, 1.0]);
    request["verbose"] = json!(true);
    let beside = ok(session.respond(Caller::Operator, request))?;
    assert_eq!(beside["outcome"]["outcome"], "via_added");
    assert_eq!(
        beside["project"],
        serde_json::to_value(session.engine().project())?
    );
    assert!(beside["board_checks"].is_array());

    let mut request = add_via(&session, [2.0, 1.0]);
    request["params"]["verbose"] = json!(true);
    let inside = ok(session.respond(Caller::Operator, request))?;
    assert!(inside["project"].is_object());

    let mut request = add_via(&session, [3.0, 1.0]);
    request["verbose"] = json!(false);
    let compact = ok(session.respond(Caller::Operator, request))?;
    assert!(compact.get("project").is_none());

    let plain = ok(session.respond(
        Caller::Operator,
        json!({"command": "project.inspect", "params": {"verbose": true}}),
    ))?;
    assert_eq!(plain, session.snapshot());
    Ok(())
}

/// Refusals are the same envelope and change nothing, and a stale revision
/// or a malformed request is still refused.
#[test]
fn a_refused_request_answers_with_its_diagnostic_and_changes_nothing() -> TestResult {
    let mut session = demo()?;
    let before = session.engine().document().clone();
    let revision = session.engine().revision();
    let mut stale = add_via(&session, [1.0, 1.0]);
    stale["params"]["expected_revision"] = json!(revision + 5);
    let reply = session.respond(Caller::Operator, stale);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "stale_revision");
    assert_eq!(session.engine().document(), &before);

    let reply = session.respond(
        Caller::Operator,
        json!({"command": "project.apply", "params": {"nonsense": 1}}),
    );
    assert_eq!(reply["error"]["code"], "malformed_request");
    let reply = session.respond(
        Caller::Operator,
        json!({"command": "project.apply", "params": {"verbose": true, "nonsense": 1}}),
    );
    assert_eq!(reply["error"]["code"], "malformed_request");
    assert_eq!(session.engine().document(), &before);
    assert_eq!(session.engine().revision(), revision);
    Ok(())
}

/// A via touching no copper is a deliberate `board.via_dangling`: the reply
/// lists that finding against the new via, the open total moves by what
/// appeared and went, and undo counts it resolved. A finding that was already
/// open is not repeated.
#[test]
fn findings_report_what_appeared_and_what_went() -> TestResult {
    let mut session = demo()?;
    // Open findings as Issues lists them: every row and what it merges.
    let open = |session: &Session| match session.engine().query(Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    }) {
        Ok(QueryResult::Issues(issues)) => issues.rows.iter().map(|r| r.findings()).sum(),
        _ => usize::MAX,
    };
    let resolved = |findings: &Value| -> usize {
        findings["resolved"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, count)| count.as_u64())
            .sum::<u64>() as usize
    };
    let open_before = open(&session);
    let request = add_via(&session, [1.0, 1.0]);
    let reply = ok(session.respond(Caller::Operator, request))?;
    let findings = &reply["findings"];
    let new = findings["new"].as_array().ok_or("new findings")?;
    let about_via = json!({"kind": "via", "id": reply["outcome"]["via"]});
    assert!(
        new.iter().any(|f| f["code"] == "board.via_dangling"
            && f["objects"]
                .as_array()
                .is_some_and(|objects| objects.contains(&about_via))),
        "{reply}"
    );
    assert!(findings.get("omitted").is_none(), "{reply}");
    assert_eq!(
        open_before + new.len() - resolved(findings),
        open(&session),
        "{reply}"
    );
    assert_eq!(findings["open"], open(&session));

    let undone = ok(session.respond(Caller::Operator, json!({"command": "project.undo", "params": {"expected_revision": session.engine().revision()}})))?;
    assert_eq!(
        undone["findings"]["resolved"]["board.via_dangling"], 1,
        "{undone}"
    );
    assert_eq!(undone["findings"]["open"], open_before, "{undone}");
    Ok(())
}

/// A changed draft is listed with its findings; other drafts are not. A
/// draft-only edit runs no design check, so it reports no design findings.
#[test]
fn only_the_changed_draft_is_reported() -> TestResult {
    let mut session = demo()?;
    let revision = session.engine().revision();
    let count = session.engine().drafts().items.len();
    assert!(count > 0, "the demo has drafts");
    let existing = serde_json::to_value(session.engine().drafts())?;
    let symbol = existing["items"]
        .as_array()
        .and_then(|items| items.iter().find(|d| d["definition"]["kind"] == "symbol"))
        .ok_or("the demo has a symbol draft")?;
    let mut meta = symbol["meta"].clone();
    meta["name"] = json!("Only this");
    let reply = ok(session.respond(Caller::Operator, json!({"command": "project.apply", "params": {
        "expected_revision": revision,
        "operation": {"editor": "Library", "operation": {"command": "library.new_draft", "params": {
            "meta": meta,
            "definition": symbol["definition"],
        }}},
    }})))?;
    let id = reply["outcome"]["id"].clone();
    assert_eq!(
        reply["changes"]["added"]["drafts.items"],
        json!([id]),
        "{reply}"
    );
    let drafts = reply["drafts"].as_array().ok_or("drafts")?;
    assert_eq!(drafts.len(), 1, "{reply}");
    assert_eq!(drafts[0]["id"], id);
    assert_eq!(drafts[0]["name"], "Only this");
    assert_eq!(drafts[0]["kind"], "symbol");
    assert!(drafts[0]["findings"].is_array());
    assert!(reply.get("findings").is_none(), "{reply}");
    Ok(())
}

#[test]
fn the_netlist_keeps_ordinary_nets_when_there_are_no_ties() -> TestResult {
    let mut session = demo()?;
    let nets = json!(session.engine().project().nets);
    let result = ok(session.respond(
        Caller::Operator,
        json!({"command":"project.inspect","params":{"at":"/netlist"}}),
    ))?;
    assert_eq!(result["value"], json!({"nets":nets,"ties":[]}));
    Ok(())
}

/// `project.inspect` answers with counts that match the document; `at` reads
/// one part of the snapshot; a pointer that names nothing is refused.
#[test]
fn inspect_summarizes_and_reads_one_part() -> TestResult {
    let mut session = demo()?;
    let reply = ok(session.respond(
        Caller::Operator,
        json!({"command": "project.inspect", "params": {}}),
    ))?;
    let project = session.engine().project();
    let counts = &reply["summary"]["counts"];
    assert_eq!(counts["tracks"], project.tracks.len());
    assert_eq!(counts["instances"], project.instances.len());
    assert_eq!(
        counts["library.components"],
        project.library.components.len()
    );
    assert_eq!(
        reply["summary"]["drafts"].as_array().map(Vec::len),
        Some(session.engine().drafts().items.len())
    );
    let QueryResult::Issues(issues) = session.engine().query(Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    })?
    else {
        return Err("project.issues gave another report".into());
    };
    assert_eq!(
        reply["summary"]["findings"]["open"],
        issues.rows.iter().map(|r| r.findings()).sum::<usize>()
    );
    assert!(reply.get("project").is_none());

    let nets = ok(session.respond(
        Caller::Operator,
        json!({"command": "project.inspect", "params": {"at": "/project/nets"}}),
    ))?;
    assert_eq!(
        nets["value"],
        serde_json::to_value(&session.engine().project().nets)?
    );
    assert_eq!(nets["revision"], session.engine().revision());
    // The in-process path reads the same part.
    let direct =
        ok(session
            .dispatch(json!({"command": "project.inspect", "params": {"at": "/project/nets"}})))?;
    assert_eq!(direct, nets);

    let missing = session.respond(
        Caller::Operator,
        json!({"command": "project.inspect", "params": {"at": "/project/nothing"}}),
    );
    assert_eq!(missing["error"]["code"], "not_found");
    // The refusal names what is valid where the pointer stopped matching:
    // the keys of an object, the positions of a list, and the pointer form.
    fn message(session: &mut Session, at: &str) -> String {
        session.respond(
            Caller::Operator,
            json!({"command": "project.inspect", "params": {"at": at}}),
        )["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }
    let text = message(&mut session, "/project/nothing");
    assert!(
        text.contains("nothing is not in /project") && text.contains("nets"),
        "{text}"
    );
    let drafts = session.engine().drafts().items.len();
    assert!(drafts > 0, "the demo has drafts");
    let text = message(&mut session, &format!("/drafts/{drafts}/draft"));
    assert!(
        text.contains(&format!("from 0 to {} (not an id)", drafts - 1)),
        "{text}"
    );
    let text = message(&mut session, "drafts");
    assert!(
        text.contains("not a JSON pointer") && text.contains("drafts, "),
        "{text}"
    );
    Ok(())
}

/// Opening or starting a project replaces the document: a summary, not a
/// list of everything that differs.
#[test]
fn replacing_the_document_answers_with_a_summary() -> TestResult {
    let mut session = demo()?;
    let profile = sparksmith_core::rules::fixtures::prototype()?;
    let reply = ok(session.respond(Caller::Operator, json!({"command": "project.new", "params": {"expected_revision": session.engine().revision(), "profile": profile}})))?;
    assert_eq!(reply["summary"]["counts"]["tracks"], 0);
    // The new board pins the profile it was given.
    assert_eq!(session.engine().project().rules.profile, Some(profile));
    assert!(reply.get("changes").is_none(), "{reply}");
    Ok(())
}

/// A file effect answers as before, without document changes.
#[test]
fn a_request_that_changes_nothing_reports_no_changes() -> TestResult {
    let mut session = demo()?;
    let reply = ok(session.respond(
        Caller::Operator,
        json!({"command": "library.sources", "params": {}}),
    ))?;
    assert_eq!(reply, json!({"sources": []}));
    Ok(())
}

/// The ESP32 board's whole replay through `respond` builds the file the same
/// replay builds through `dispatch`, byte for byte, with no request refused,
/// and no reply carries the document. Saving is left out: the test writes
/// nothing into the example.
#[test]
fn the_board_replay_builds_the_same_document_with_compact_replies() -> TestResult {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let replay = std::fs::read_to_string(
        root.join("examples/esp32-s31-devboard/esp32-s31-devboard.replay.jsonl"),
    )?;
    let mut compact = Session::default();
    let mut full = Session::default();
    let mut largest = 0;
    for line in replay.lines() {
        let mut request: Value = serde_json::from_str(line)?;
        if request["command"] == "project.save" {
            continue;
        }
        // Paths the replay reads are relative to the repository root.
        if request["command"] == "project.open" || request["command"] == "library.use_part_file" {
            let path = request["params"]["path"].as_str().ok_or("path")?;
            request["params"]["path"] = json!(root.join(path).display().to_string());
        }
        let reply = compact.respond(Caller::Operator, request.clone());
        assert_eq!(reply["ok"], true, "{line}: {reply}");
        assert!(reply["result"].get("project").is_none(), "{line}");
        largest = largest.max(reply.to_string().len());
        let reply = full.dispatch(request);
        assert_eq!(reply["ok"], true, "{line}: {reply}");
    }
    assert_eq!(compact.engine().revision(), full.engine().revision());
    assert_eq!(
        serde_json::to_vec(&compact.engine().to_file())?,
        serde_json::to_vec(&full.engine().to_file())?
    );
    // Measured 9 243 bytes (docs/e2e-esp32-devboard.md).
    assert!(largest < 16 * 1024, "largest reply {largest} bytes");
    Ok(())
}

mod diff {
    use super::super::reply::tests_support::{changes_of, delta};
    use serde_json::json;
    use sparksmith_core::Finding;

    #[test]
    fn collections_are_diffed_by_identity() {
        let before = json!({
            "next_id": 5,
            "tracks": [{"id": 1, "w": 1}, {"id": 2, "w": 1}, {"id": 3, "w": 1}],
            "placements": [{"instance": 7, "x": 0}],
            "library": {"parts": [{"id": 4, "revision": 1, "m": "a"}]},
            "board_size": [10, 10],
        });
        let after = json!({
            "next_id": 9,
            "tracks": [{"id": 1, "w": 2}, {"id": 2, "w": 1}, {"id": 8, "w": 1}],
            "placements": [{"instance": 7, "x": 3}],
            "library": {"parts": [{"id": 4, "revision": 1, "m": "a"}, {"id": 4, "revision": 2, "m": "b"}]},
            "board_size": [20, 10],
            "fills": [{"id": 6}],
        });
        assert_eq!(
            changes_of(&before, &after),
            json!({
                "added": {"tracks": [8], "fills": [6], "library.parts": [{"id": 4, "revision": 2}]},
                "changed": {"tracks": [1], "placements": [7]},
                "removed": {"tracks": [3]},
                "fields": ["board_size"],
            })
        );
    }

    /// An emptied collection is left out when serialized; a reorder has no
    /// element to name, so it is reported as the collection's field.
    #[test]
    fn emptied_and_reordered_collections_are_still_reported() {
        let before = json!({"fills": [{"id": 6}], "sheets": [{"id": 1}, {"id": 2}]});
        let after = json!({"sheets": [{"id": 2}, {"id": 1}]});
        assert_eq!(
            changes_of(&before, &after),
            json!({"removed": {"fills": [6]}, "fields": ["sheets"]})
        );
        assert_eq!(changes_of(&before, &before), serde_json::Value::Null);
    }

    #[test]
    fn long_lists_are_cut_and_counted() {
        let old: Vec<_> = (0..1000).map(|i| json!({"id": i, "w": 1})).collect();
        let new: Vec<_> = (0..1000).map(|i| json!({"id": i, "w": 2})).collect();
        let changes = changes_of(&json!({"tracks": old}), &json!({"tracks": new}));
        assert_eq!(
            changes["changed"]["tracks"].as_array().map(Vec::len),
            Some(200)
        );
        assert_eq!(changes["omitted"], 800);
    }

    #[test]
    fn repeated_findings_count_each_time_and_long_lists_are_cut() {
        let finding = |code: &str, text: &str| Finding::new(code, text.to_owned(), vec![]);
        let old = [finding("a", "x"), finding("a", "x"), finding("b", "y")];
        let new = [finding("a", "x"), finding("c", "z")];
        let out = delta(&old, &new);
        assert_eq!(
            out["new"],
            json!([{"code": "c", "message": "z", "objects": []}])
        );
        assert_eq!(out["resolved"], json!({"a": 1, "b": 1}));
        assert_eq!(out["open"], 2);
        assert!(out.get("omitted").is_none());

        let many: Vec<_> = (0..100).map(|i| finding("c", &format!("{i}"))).collect();
        let out = delta(&[], &many);
        assert_eq!(out["new"].as_array().map(Vec::len), Some(20));
        assert_eq!(out["omitted"], 80);
        assert_eq!(out["open"], 100);
    }
}

/// The chat's turn summary: what changed between the document before a
/// turn and after it, by id with the names a person reads, and the
/// findings that came and went.
#[test]
fn the_difference_between_two_documents_names_what_changed() -> TestResult {
    let mut session = demo()?;
    let before = session.engine().document().clone();
    let settings = *session.engine().check_settings();
    assert_eq!(difference(&before, &before, &settings)?, json!({}));
    let reply = ok(session.respond(Caller::Operator, add_via(&session, [1.0, 1.0])))?;
    let via = reply["outcome"]["via"].clone();
    let after = session.engine().document().clone();
    let summary = difference(&before, &after, &settings)?;
    assert_eq!(
        summary["changes"]["added"]["vias"],
        json!([via]),
        "{summary}"
    );
    assert!(summary["findings"]["open"].is_u64(), "{summary}");
    let mut names = serde_json::Map::new();
    let root = json!({"layout": {"parts": [{"id": 7, "designator": "U1"}, {"id": 8}]}});
    let listed =
        std::collections::BTreeMap::from([("layout.parts".to_owned(), vec![json!(7), json!(8)])]);
    super::reply::tests_support::name_listed(&root, &listed, &mut names);
    assert_eq!(Value::Object(names), json!({"7": "U1"}));
    Ok(())
}

/// A via edit on a project that packs asset bytes reports that via as
/// changed and nothing else: no unrelated collection, no field, and the
/// asset bytes are never part of the reply.
#[test]
fn a_local_edit_reports_only_the_objects_it_wrote() -> TestResult {
    use sparksmith_core::library::bundle::{AssetBytes, PackedAsset, asset_hash};
    let mut engine = sparksmith_core::Engine::showcase()?;
    let bytes = vec![7u8; 4096];
    let mut file = engine.to_file();
    let mut assets: Vec<PackedAsset> = file.project.library.assets.iter().cloned().collect();
    assets.push(PackedAsset {
        hash: asset_hash(&bytes),
        media_type: "model/step".into(),
        data: AssetBytes::new(bytes),
    });
    file.project.library.assets = sparksmith_core::Shared::new(assets);
    engine = sparksmith_core::Engine::from_file(file)?;
    let via = engine.project().vias.first().ok_or("a via")?.clone();
    let mut session = Session::fixture(engine);
    let reply = ok(session.respond(
        Caller::Operator,
        json!({"command": "project.apply", "params": {
            "expected_revision": session.engine().revision(),
            "operation": {"editor": "Layout", "operation": {"command": "layout.via_tent", "params": {
                "id": via.id, "tented": !via.tented
            }}},
        }}),
    ))?;
    assert_eq!(
        reply["changes"],
        json!({"changed": {"vias": [via.id]}}),
        "{reply}"
    );
    Ok(())
}

/// The sections both states share are left out of the diff, and a shared
/// section that changed is still reported by id: a packed asset added to
/// the library is listed by its hash.
#[test]
fn a_changed_library_section_is_reported_by_id() -> TestResult {
    use sparksmith_core::library::bundle::{AssetBytes, PackedAsset, asset_hash};
    let session = demo()?;
    let before = session.engine().document().clone();
    let mut after = before.clone();
    let bytes = vec![3u8; 64];
    let hash = asset_hash(&bytes);
    let project = std::sync::Arc::make_mut(&mut after.project);
    let mut assets: Vec<PackedAsset> = project.library.assets.iter().cloned().collect();
    assets.push(PackedAsset {
        hash,
        media_type: "model/step".into(),
        data: AssetBytes::new(bytes),
    });
    project.library.assets = sparksmith_core::Shared::new(assets);
    let summary = difference(&before, &after, session.engine().check_settings())?;
    assert_eq!(
        summary["changes"],
        json!({"added": {"library.assets": [serde_json::to_value(hash)?]}}),
        "{summary}"
    );
    Ok(())
}

/// Review finding: the inspect summary said 0 open findings on the
/// showcase while Issues and fabrication listed its purchasing to-dos. The
/// summary counts from Issues' own rows: the same findings, the same
/// counts by severity.
#[test]
fn the_summary_counts_what_issues_lists() -> TestResult {
    let mut session = Session::new(Engine::showcase()?);
    let reply = ok(session.respond(
        Caller::Operator,
        json!({"command": "project.inspect", "params": {}}),
    ))?;
    let QueryResult::Issues(issues) = session.engine().query(Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    })?
    else {
        return Err("project.issues gave another report".into());
    };
    assert!(
        issues.counts.to_do > 0,
        "the showcase's parts have no Part chosen"
    );
    let findings = &reply["summary"]["findings"];
    assert_eq!(findings["issues"], serde_json::to_value(issues.counts)?);
    let open: usize = issues.rows.iter().map(|r| r.findings()).sum();
    assert_eq!(findings["open"], open);
    assert!(
        findings["by_code"]
            .get("purchasing.no_part")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0),
        "{findings}"
    );
    // A change reply's `open` is the same count.
    let changed = ok(session.respond(Caller::Operator, add_via(&session, [1.0, 1.0])))?;
    let QueryResult::Issues(after) = session.engine().query(Query::Issues {
        scope: sparksmith_core::issues::IssueScope::All,
    })?
    else {
        return Err("project.issues gave another report".into());
    };
    assert_eq!(
        changed["findings"]["open"],
        after.rows.iter().map(|r| r.findings()).sum::<usize>()
    );
    Ok(())
}

/// A reply leaves out the tracks and vias both states hold unchanged, and
/// still reports exactly what diffing the whole documents reports: changed,
/// added and removed ones by id, and an order change alone as the field.
#[test]
fn trimmed_copper_reports_what_whole_documents_report() -> TestResult {
    use super::reply::tests_support::{changes_between, changes_of};
    let engine = sparksmith_core::Engine::showcase()?;
    let before = engine.project().clone();
    let whole = |a: &Project, b: &Project| -> Result<Value> {
        Ok(changes_of(
            &serde_json::to_value(a)?,
            &serde_json::to_value(b)?,
        ))
    };
    let mut edits: Vec<Project> = Vec::new();
    // One track changed, one via removed, one track added.
    let mut p = before.clone();
    p.tracks[1].width += 0.05;
    p.vias.remove(0);
    let mut t = p.tracks[2].clone();
    t.id = p.next_id;
    p.next_id += 1;
    p.tracks.push(t);
    edits.push(p);
    // Only the order of the tracks.
    let mut p = before.clone();
    p.tracks.swap(0, 1);
    edits.push(p);
    // The order and one track.
    let mut p = before.clone();
    p.tracks.swap(0, 1);
    p.tracks[3].width += 0.05;
    edits.push(p);
    for after in &edits {
        let reported = changes_between(&before, after)?;
        assert_eq!(reported, whole(&before, after)?);
        assert_ne!(reported, Value::Null);
    }
    Ok(())
}

/// A reply's findings are those its edit changed: a check settings change
/// at the same revision (an impedance tolerance set in Settings) changes
/// findings no edit made, so the next edit does not report them. The
/// showcase's first routed net takes a 50 Ω target: with no tolerance it
/// is `impedance.no_tolerance`, with one it is checked.
#[test]
fn a_check_settings_change_is_not_reported_as_the_next_edits_findings() -> TestResult {
    use sparksmith_core::layout::CheckSettings;
    use sparksmith_core::rules::{RulesCommand, netclass::NetClassSpec, signal::SignalRules};
    let mut session = Session::fixture(sparksmith_core::Engine::showcase()?);
    let net = session
        .engine()
        .project()
        .tracks
        .first()
        .ok_or("a track")?
        .net;
    let engine = session.engine_mut();
    engine.set_check_settings(CheckSettings::default())?;
    let spec: NetClassSpec = serde_json::from_value(json!({"name": "Z50"}))?;
    let spec = NetClassSpec {
        signal: SignalRules {
            impedance: Some(50.0),
            ..Default::default()
        },
        ..spec
    };
    let class = match engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::AddNetClass { class: spec }),
    )? {
        sparksmith_core::Outcome::NetClassAdded { class } => class,
        other => return Err(format!("{other:?}").into()),
    };
    engine.apply(
        engine.revision(),
        Operation::Rules(RulesCommand::AssignNets {
            nets: vec![net],
            class: Some(class),
        }),
    )?;
    let codes = |findings: &Value| -> Vec<String> {
        let new = findings["new"].as_array().into_iter().flatten();
        let resolved = findings["resolved"].as_object().into_iter().flatten();
        new.filter_map(|f| f["code"].as_str().map(str::to_owned))
            .chain(resolved.map(|(code, _)| code.clone()))
            .filter(|code| code.starts_with("impedance."))
            .collect()
    };
    let unchecked = |session: &Session| -> std::result::Result<usize, Box<dyn std::error::Error>> {
        let report = session
            .engine()
            .issue_report_of(session.engine().document())?;
        Ok(report
            .rows
            .iter()
            .filter(|r| r.finding.code == "impedance.no_tolerance")
            .count())
    };
    assert!(unchecked(&session)? > 0, "the target is not checked");
    // An edit sets the reply's baseline under no tolerance.
    let first = ok(session.respond(Caller::Operator, add_via(&session, [1.0, 1.0])))?;
    assert_eq!(codes(&first["findings"]), Vec::<String>::new(), "{first}");
    // Settings change, at the same revision.
    let revision = session.engine().revision();
    session.engine_mut().set_check_settings(CheckSettings {
        impedance_tolerance: Some(50.0),
    })?;
    assert_eq!(session.engine().revision(), revision);
    assert_eq!(unchecked(&session)?, 0, "the target is checked now");
    let second = ok(session.respond(Caller::Operator, add_via(&session, [2.0, 1.0])))?;
    assert_eq!(codes(&second["findings"]), Vec::<String>::new(), "{second}");
    Ok(())
}
