//! Nets, tracks and vias of Rust-built synthetic boards, through the
//! engine: labels join the pads' terminals into named nets, and tracks and
//! vias land on those nets and layers.
mod support;

use sparksmith_core::{CopperLayer, ViaSpan};
use sparksmith_kicad::report::Case;
use support::{Board, Footprint, Pad, import};

type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// Two resistors joined by a top track, a via and a bottom track, plus a
/// net with a space in its name, two names equal but for case, an arc
/// track and a track piece too short to import.
fn routed() -> Board {
    let pad = |number: &'static str, x: f64, net: (u32, &str)| {
        Pad::smd(number, "rect", [x, 0.0, 0.0], [0.8, 0.6])
            .with(&format!("(net {} \"{}\")", net.0, net.1))
    };
    let resistor = |reference: &str, x: f64, y: f64, nets: [(u32, &str); 2]| {
        Footprint::new(
            reference,
            [x, y, 0.0],
            false,
            vec![pad("1", -1.0, nets[0]), pad("2", 1.0, nets[1])],
        )
    };
    let items = "
  (net 1 \"SIG\") (net 2 \"my net\") (net 3 \"gnd\") (net 4 \"GND\")
  (gr_rect (start 0 0) (end 40 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))
  (segment (start 11 10) (end 20 10) (width 0.25) (layer \"F.Cu\") (net 1))
  (via (at 20 10) (size 0.6) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 1))
  (segment (start 20 10) (end 29 10) (width 0.25) (layer \"B.Cu\") (net 1))
  (segment (start 29 10) (end 29 10.004) (width 0.25) (layer \"B.Cu\") (net 1))
  (arc (start 9 10) (mid 8 9) (end 7 10) (width 0.2) (layer \"F.Cu\") (net 2))
  (segment (start 5 15) (end 6 15) (width 0.25) (layer \"F.Cu\") (net 9))
";
    Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![
            resistor("R1", 10.0, 10.0, [(2, "my net"), (1, "SIG")]),
            resistor("R2", 30.0, 10.0, [(1, "SIG"), (3, "gnd")]),
            resistor("R3", 20.0, 16.0, [(4, "GND"), (4, "GND")]),
        ],
        items: items.into(),
    }
}

#[test]
fn pads_join_into_named_nets_through_labels() -> Fallible<()> {
    let imported = import(routed().text().as_bytes())?;
    let project = imported.engine.project();
    let report = &imported.report;
    assert_eq!(report.counts.nets, 4);
    let name = |designator: &str, terminal: u32| -> Option<String> {
        let instance = project.instance_by_designator(designator)?;
        let net = project.net_of(sparksmith_core::Endpoint {
            instance: instance.id,
            terminal: sparksmith_core::TerminalId(terminal),
        })?;
        Some(project.net_name(net.id).to_owned())
    };
    // R1.2 and R2.1 share SIG.
    assert_eq!(name("R1", 2).as_deref(), Some("SIG"));
    assert_eq!(name("R2", 1).as_deref(), Some("SIG"));
    let sig = |d: &str, t: u32| {
        let i = project.instance_by_designator(d)?;
        project.net_of(sparksmith_core::Endpoint {
            instance: i.id,
            terminal: sparksmith_core::TerminalId(t),
        })
    };
    assert_eq!(sig("R1", 2).map(|n| n.id), sig("R2", 1).map(|n| n.id));
    // A space is no label character; "gnd" and "GND" stay two nets.
    assert_eq!(name("R1", 1).as_deref(), Some("my_net"));
    assert_ne!(sig("R2", 2).map(|n| n.id), sig("R3", 1).map(|n| n.id));
    assert_eq!(report.count(Case::NetRenamed), 2);
    Ok(())
}

#[test]
fn tracks_and_vias_land_on_their_nets_and_layers() -> Fallible<()> {
    let imported = import(routed().text().as_bytes())?;
    let project = imported.engine.project();
    let report = &imported.report;
    let r1 = project.instance_by_designator("R1").ok_or("R1")?;
    let sig = project
        .net_of(sparksmith_core::Endpoint {
            instance: r1.id,
            terminal: sparksmith_core::TerminalId(2),
        })
        .ok_or("SIG")?
        .id;
    let on = |layer: CopperLayer| {
        project
            .tracks
            .iter()
            .filter(|t| t.net == sig && t.layer == layer)
            .count()
    };
    assert_eq!((on(CopperLayer::Top), on(CopperLayer::Bottom)), (1, 1));
    let via = project.vias.first().ok_or("no via")?;
    assert_eq!((via.net, via.span), (sig, ViaSpan::Through));
    assert!((via.drill - 0.3).abs() < 1e-9 && (via.diameter - 0.6).abs() < 1e-9);
    // The outline's lower-left corner, KiCad's (0, 20), is our origin:
    // KiCad's (20, 10) is (20, 20 − 10) here.
    assert!((via.position[0] - 20.0).abs() < 1e-9 && (via.position[1] - 10.0).abs() < 1e-9);
    // The arc became chords from (9, 10) over (8, 11) to (7, 10).
    assert_eq!(report.count(Case::ArcTrack), 1);
    let chords: Vec<_> = project
        .tracks
        .iter()
        .filter(|t| t.net != sig && (t.width - 0.2).abs() < 1e-9)
        .collect();
    assert!(chords.len() > 8, "{}", chords.len());
    let top = chords
        .iter()
        .flat_map(|t| [t.start[1], t.end[1]])
        .fold(f64::MIN, f64::max);
    assert!((top - 11.0).abs() < 1e-3, "{top}");
    // The 4 µm piece is below the engine's least length and is cleaned
    // away; the track on a net without pads has nowhere to go. Both are
    // counted.
    assert_eq!(report.count(Case::TinyTrackCleaned), 1);
    assert_eq!(report.count(Case::TrackWithoutNet), 1);
    assert_eq!(report.count(Case::TrackRefused), 0);
    assert_eq!(report.counts.vias, 1);
    Ok(())
}

/// The board-check findings of an imported project, as (code, message).
fn findings(project: &sparksmith_core::Project) -> Fallible<Vec<(String, String)>> {
    let report = sparksmith_core::issues::report(
        project,
        &sparksmith_core::library::Drafts::default(),
        &sparksmith_core::issues::IssueScope::All,
        &Default::default(),
    )?;
    Ok(report
        .rows
        .iter()
        .flat_map(|r| std::iter::once(&r.finding).chain(&r.also))
        .map(|f| (f.code.as_str().to_owned(), f.message.clone()))
        .collect())
}

/// Under the shipped Standard profile a board drawn finer than it allows
/// (0.1 mm tracks, 0.15 mm gaps, a via of a 0.2 mm drill and a 0.1 mm ring)
/// imports whole: no track or via is left out, and the board checks list
/// what breaks the profile. The document opens again as it was imported.
#[test]
fn a_board_finer_than_the_profile_imports_whole_and_is_checked() -> Fallible<()> {
    let standard: sparksmith_core::rules::ProfileFile =
        serde_json::from_str(include_str!("../../../library/profiles/standard.json"))?;
    let mut board = routed();
    board.items.push_str(
        "  (segment (start 5 5) (end 15 5) (width 0.1) (layer \"F.Cu\") (net 1))
  (segment (start 5 5.25) (end 15 5.25) (width 0.1) (layer \"F.Cu\") (net 2))
  (via (at 20 5) (size 0.4) (drill 0.2) (layers \"F.Cu\" \"B.Cu\") (net 1))
  (segment (start 18 5.4) (end 22 5.4) (width 0.1) (layer \"F.Cu\") (net 2))\n",
    );
    let presets = sparksmith_core::StackupPresets::fixture()?;
    let imported = sparksmith_kicad::import(
        board.text().as_bytes(),
        &sparksmith_kicad::Setup {
            board: &sparksmith_core::NewBoard::fixture(),
            presets: &presets,
            profile: Some(&standard.profile),
            evaluators: Default::default(),
            project_file: None,
            schematic: None,
        },
    )?;
    let project = imported.engine.project();
    assert_eq!(imported.report.count(Case::TrackRefused), 0);
    assert_eq!(imported.report.count(Case::ViaRefused), 0);
    assert!(project.refused_import_tracks.is_empty());
    assert_eq!(imported.report.counts.vias, 2);
    assert_eq!(imported.report.counts.tracks, project.tracks.len());
    let fine = project
        .tracks
        .iter()
        .filter(|t| (t.width - 0.1).abs() < 1e-9)
        .count();
    assert_eq!(fine, 3);
    assert_eq!(
        project.rules.profile.as_ref().map(|p| p.id.as_str()),
        Some("standard")
    );
    let found = findings(project)?;
    let of = |code: &str| found.iter().filter(|(c, _)| c == code).count();
    assert_eq!(of("board.track_width"), 3, "{found:#?}");
    // Track to track and via to track.
    assert!(of("board.clearance") >= 2, "{found:#?}");
    assert!(of("board.via_drill_size") >= 1, "{found:#?}");
    assert!(of("board.via_annular_ring") >= 1, "{found:#?}");
    let again = sparksmith_core::Engine::from_file(imported.engine.to_file())?;
    assert_eq!(again.project().tracks, project.tracks);
    assert_eq!(again.project().vias, project.vias);
    Ok(())
}

/// A SIG track across R3's GND pads is copper the file has: it is imported
/// and reported as a short of two nets.
#[test]
fn a_track_over_another_nets_pad_is_imported_and_reported() -> Fallible<()> {
    let mut board = routed();
    board
        .items
        .push_str("  (segment (start 18 16) (end 22 16) (width 0.25) (layer \"F.Cu\") (net 1))\n");
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::TrackRefused), 0);
    let found = findings(imported.engine.project())?;
    assert!(
        found
            .iter()
            .any(|(code, text)| code == "board.overlap" && text.contains("R3")),
        "{found:#?}"
    );
    Ok(())
}

/// An imported short is neither waived nor exported: the waive command
/// refuses it, a waiver a file holds of it counts for nothing, and with
/// every other finding waived it alone blocks the export (which refuses
/// while anything blocks) until the track is gone.
#[test]
fn an_imported_short_blocks_until_it_is_fixed() -> Fallible<()> {
    use sparksmith_core::fabrication::FabricationCommand;
    use sparksmith_core::fabrication::preflight::PreflightReport;
    use sparksmith_core::layout::LayoutCommand;
    use sparksmith_core::{Operation, Query, QueryResult};
    let check = |engine: &sparksmith_core::Engine| -> Fallible<PreflightReport> {
        match engine.query(Query::Preflight { profile: None })? {
            QueryResult::Preflight(report) => Ok(*report),
            _ => Err("not a preflight report".into()),
        }
    };
    let mut board = routed();
    board
        .items
        .push_str("  (segment (start 18 16) (end 22 16) (width 0.25) (layer \"F.Cu\") (net 1))\n");
    let imported = import(board.text().as_bytes())?;
    let mut engine = imported.engine;
    let before = check(&engine)?;
    let shorts: Vec<String> = before
        .items
        .iter()
        .filter(|i| i.code == "board.overlap")
        .map(|i| i.key.clone())
        .collect();
    assert!(!shorts.is_empty(), "{before:?}");
    assert!(
        before
            .items
            .iter()
            .filter(|i| shorts.contains(&i.key))
            .all(|i| i.blocks() && !i.offers_waiver())
    );
    let revision = engine.revision();
    let refused = engine.apply(
        revision,
        Operation::Fabrication(FabricationCommand::Waive {
            keys: shorts.clone(),
            reason: "as imported".into(),
        }),
    );
    let message = refused.err().ok_or("the short was waived")?.to_string();
    assert!(message.contains("fabrication.not_waivable"), "{message}");
    assert_eq!(engine.revision(), revision);
    // Every finding waived in a file: those that may be are, the shorts not.
    let mut file = engine.to_file();
    let mut keys: Vec<String> = before.items.iter().map(|i| i.key.clone()).collect();
    keys.sort();
    keys.dedup();
    file.project.production.waivers = keys
        .into_iter()
        .map(|key| sparksmith_core::model::production::Waiver {
            key,
            reason: "as imported".into(),
        })
        .collect();
    engine.replace_document(file)?;
    let after = check(&engine)?;
    let blocks: Vec<&str> = after
        .items
        .iter()
        .filter(|i| i.blocks() && i.code == "board.overlap")
        .map(|i| i.key.as_str())
        .collect();
    assert_eq!(blocks.len(), shorts.len(), "{after:?}");
    assert!(after.blocking >= shorts.len());
    // Fixed: the track is deleted, and no short is left.
    let track = engine
        .project()
        .tracks
        .iter()
        .find(|t| (t.start[1] - 4.0).abs() < 1e-6 && (t.end[1] - 4.0).abs() < 1e-6)
        .ok_or("the track across R3")?
        .id;
    engine.apply(
        engine.revision(),
        Operation::Layout(LayoutCommand::DeleteTrack { id: track }),
    )?;
    let fixed = check(&engine)?;
    assert!(
        fixed.items.iter().all(|i| i.code != "board.overlap"),
        "{fixed:?}"
    );
    Ok(())
}

/// What a document cannot hold is left out and listed with the reason, and
/// the rest is imported: a track off the board, a via without a ring.
#[test]
fn copper_the_board_cannot_hold_is_listed_and_the_rest_imported() -> Fallible<()> {
    let mut board = routed();
    board.items.push_str(
        "  (segment (start 35 5) (end 45 5) (width 0.25) (layer \"F.Cu\") (net 1))
  (via (at 20 5) (size 0.3) (drill 0.3) (layers \"F.Cu\" \"B.Cu\") (net 1))\n",
    );
    let imported = import(board.text().as_bytes())?;
    let report = &imported.report;
    assert_eq!(report.count(Case::TrackRefused), 1);
    assert_eq!(report.count(Case::ViaRefused), 1);
    assert!(report.counts.tracks > 2);
    assert_eq!(report.counts.vias, 1);
    // E2E run 11, finding 4: the report names each refused piece, where it
    // ran (KiCad's (35, 5) is (35, 20 − 5) here) and why.
    for (case, says) in [
        (
            Case::TrackRefused,
            "Track on SIG (Top) from (35, 15) to (45, 15) mm: track copper must lie inside",
        ),
        (
            Case::ViaRefused,
            "Via on SIG at (20, 15) mm: via drill must be positive and smaller than its pad",
        ),
    ] {
        let lines: Vec<String> = report.refused_copper_of(case).map(|r| r.line()).collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains(says), "{lines:?}");
        assert!(
            report
                .summary_named()
                .contains(&format!("    {}\n", lines[0])),
            "{}",
            report.summary_named()
        );
    }
    Ok(())
}

/// A track from R2's SIG pad whose second piece runs off the board: that
/// piece cannot be held, and the first piece, now ending in nothing, says
/// in its finding that the import refused the rest and why.
#[test]
fn a_stub_a_refused_track_left_says_so() -> Fallible<()> {
    let mut board = routed();
    board.items.push_str(
        "  (segment (start 29 10) (end 29 16) (width 0.25) (layer \"F.Cu\") (net 1))
  (segment (start 29 16) (end 29 25) (width 0.25) (layer \"F.Cu\") (net 1))\n",
    );
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::TrackRefused), 1);
    let project = imported.engine.project();
    let refused = &project.refused_import_tracks;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(
        refused[0].reason,
        sparksmith_core::model::import_review::TrackRefusal::OutsideBoard
    );
    let found = findings(project)?;
    // KiCad's (29, 16) is (29, 20 − 16) here.
    assert!(
        found.iter().any(|(code, text)| code == "board.track_dangling"
            && text
                == "Track on SIG (Top) stops at (29, 4) mm: import refused the rest (outside the board)"),
        "{found:#?}"
    );
    Ok(())
}

#[test]
fn the_project_files_rules_become_the_projects_own_rule_set() -> Fallible<()> {
    // Original project rules: a default class, a Power class by pattern
    // and by explicit assignment, and board minimums, one looser than the
    // prototype profile's (the drill minimum, 0.05 < 0.1 mm).
    let project_file = br#"{"board":{"design_settings":{"rules":{
        "min_copper_edge_clearance":0.4,"min_through_hole_diameter":0.05}}},
        "net_settings":{"classes":[
          {"name":"Default","clearance":0.2,"track_width":0.25,"via_diameter":0.6,"via_drill":0.3},
          {"name":"Power","clearance":0.3,"track_width":0.5,"diff_pair_width":0.2,"diff_pair_gap":0.2}],
        "netclass_patterns":[{"netclass":"Power","pattern":"G*"}],
        "netclass_assignments":{"my net":"Power"}}}"#;
    let imported = support::import_with_project(routed().text().as_bytes(), Some(project_file))?;
    let project = imported.engine.project();
    let report = &imported.report;
    let rules = &project.rules;
    assert_eq!(rules.source.as_deref(), Some("From the KiCad project"));
    // Rules are set after the copper: no part of the import can be undone.
    assert!(!imported.engine.can_undo());
    assert_eq!(rules.net_classes.len(), 1);
    let power = &rules.net_classes[0];
    assert_eq!(power.name, "Power");
    assert_eq!(power.track_width, Some(0.5));
    assert_eq!(power.diff_pair_gap, Some(0.2));
    assert_eq!(report.count(Case::NetClass), 1);
    // The pattern carries over; "my net" is "my_net" here, so it is
    // assigned explicitly.
    assert_eq!(rules.patterns.len(), 1);
    assert_eq!(report.count(Case::NetClassAssigned), 1);
    let class = |name: &str| {
        project
            .nets
            .iter()
            .find(|n| n.name == name)
            .and_then(|n| rules.class_of(n.id, &n.name))
    };
    // "GND" is "GND~2" here ('gnd' took the name): the pattern still matches.
    assert_eq!(class("GND~2"), Some(power.id));
    assert_eq!(class("gnd"), None);
    assert_eq!(class("my_net"), Some(power.id));
    assert_eq!(class("SIG"), None);
    // The board minimums: the edge clearance is taken, the looser drill
    // minimum is not (the profile's stays).
    let effective = rules.effective();
    use sparksmith_core::rules::CopperObject::{BoardEdge, Track};
    assert!((effective.copper.clearance.get(Track, BoardEdge) - 0.4).abs() < 1e-9);
    assert!((effective.holes.drill_min - 0.1).abs() < 1e-9);
    assert!((effective.copper.track_width_default - 0.25).abs() < 1e-9);
    assert_eq!(report.count(Case::RuleLooserThanFab), 1);
    // The rule not taken is named with the KiCad and the profile's value,
    // in the summary for the person and in the detail file.
    let looser = report.looser_rules();
    assert_eq!(looser.len(), 1);
    assert_eq!(
        looser[0].line(),
        "min drill: KiCad 0.05 mm, fab profile 0.1 mm (min_through_hole_diameter)"
    );
    assert!(report.summary_named().contains(&looser[0].line()));
    assert_eq!(
        report.to_json()["looser_rules"][0]["kicad_setting"],
        "min_through_hole_diameter"
    );
    Ok(())
}

/// Run 7, finding 3: a class pattern with a power net's `+` (`+5*`) is
/// taken as a wildcard; a regular expression is refused and named with
/// its reason; KiCad's board `min_clearance`, looser than the profile,
/// is counted and named among the rules not taken.
#[test]
fn power_net_patterns_are_taken_and_refused_ones_named() -> Fallible<()> {
    let pad = |number: &'static str, x: f64, net: (u32, &str)| {
        Pad::smd(number, "rect", [x, 0.0, 0.0], [0.8, 0.6])
            .with(&format!("(net {} \"{}\")", net.0, net.1))
    };
    let board = Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![Footprint::new(
            "R1",
            [10.0, 10.0, 0.0],
            false,
            vec![pad("1", -1.0, (1, "+5V")), pad("2", 1.0, (2, "GND"))],
        )],
        items: "  (net 1 \"+5V\") (net 2 \"GND\")
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n"
            .into(),
    };
    let project_file = br#"{"board":{"design_settings":{"rules":{"min_clearance":0.01}}},
        "net_settings":{"classes":[
          {"name":"Default","clearance":0.2,"track_width":0.25},
          {"name":"Power","clearance":0.25,"track_width":0.4}],
        "netclass_patterns":[{"netclass":"Power","pattern":"+5*"},
          {"netclass":"Power","pattern":"^GND$"}]}}"#;
    let imported = support::import_with_project(board.text().as_bytes(), Some(project_file))?;
    let project = imported.engine.project();
    let report = &imported.report;
    let rules = &project.rules;
    let power = rules.net_classes.first().ok_or("Power")?;
    assert_eq!(
        rules
            .patterns
            .iter()
            .map(|p| p.pattern.as_str())
            .collect::<Vec<_>>(),
        ["+5*"]
    );
    let net = project.nets.iter().find(|n| n.name == "+5V").ok_or("+5V")?;
    assert_eq!(rules.class_of(net.id, &net.name), Some(power.id));
    assert_eq!(report.count(Case::NetClassAssigned), 0, "by its pattern");
    assert_eq!(report.count(Case::NetClassPatternRefused), 1);
    let refused = report.refused_patterns();
    assert_eq!(
        refused.iter().map(|p| p.line()).collect::<Vec<_>>(),
        ["^GND$: a regular expression; only * and ? wildcards are read"]
    );
    assert!(report.summary_named().contains(&refused[0].line()));
    // The board's least clearance, looser than the profile's: each copper
    // pair named.
    let looser: Vec<_> = report
        .looser_rules()
        .iter()
        .filter(|r| r.kicad_setting == "min_clearance")
        .collect();
    assert_eq!(looser.len(), 10, "{looser:?}");
    assert!(report.summary_named().contains("KiCad 0.01 mm"));
    Ok(())
}

/// E2E run 9, finding 6: a class whose only membership is a regular
/// expression (`V[A-Z]+|GND`) still has that pattern refused and named.
/// The same pattern on the Default class is not a refusal: a net no
/// class takes is the default class's already, so it is skipped as every
/// default-class pattern is (run 9's project had it on Default).
#[test]
fn a_regex_only_class_pattern_is_named_and_a_default_class_pattern_skipped() -> Fallible<()> {
    let board = Board {
        copper: 2,
        origin: None,
        setup: "(pad_to_mask_clearance 0)".into(),
        footprints: vec![Footprint::new(
            "R1",
            [10.0, 10.0, 0.0],
            false,
            vec![
                Pad::smd("1", "rect", [-1.0, 0.0, 0.0], [0.8, 0.6]).with("(net 1 \"VIN\")"),
                Pad::smd("2", "rect", [1.0, 0.0, 0.0], [0.8, 0.6]).with("(net 2 \"GND\")"),
            ],
        )],
        items: "  (net 1 \"VIN\") (net 2 \"GND\")
  (gr_rect (start 0 0) (end 20 20) (stroke (width 0.1) (type default)) (layer \"Edge.Cuts\"))\n"
            .into(),
    };
    let project = |class: &str| {
        let classes = r#"{"name":"Default","clearance":0.2,"track_width":0.25},
              {"name":"Power","clearance":0.25,"track_width":0.4}"#;
        let pattern = format!(r#"{{"netclass":"{class}","pattern":"V[A-Z]+|GND"}}"#);
        format!(r#"{{"net_settings":{{"classes":[{classes}],"netclass_patterns":[{pattern}]}}}}"#)
    };
    let power =
        support::import_with_project(board.text().as_bytes(), Some(project("Power").as_bytes()))?;
    let lines: Vec<String> = power
        .report
        .refused_patterns()
        .iter()
        .map(|p| p.line())
        .collect();
    assert_eq!(
        lines,
        ["V[A-Z]+|GND: a regular expression; only * and ? wildcards are read"]
    );
    assert!(power.report.summary_named().contains(&lines[0]));
    assert!(power.engine.project().rules.patterns.is_empty());
    let default =
        support::import_with_project(board.text().as_bytes(), Some(project("Default").as_bytes()))?;
    assert!(default.report.refused_patterns().is_empty());
    assert!(default.engine.project().rules.patterns.is_empty());
    Ok(())
}
