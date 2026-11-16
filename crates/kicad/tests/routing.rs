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

#[test]
fn a_track_the_checks_refuse_is_listed_and_the_rest_imported() -> Fallible<()> {
    let mut board = routed();
    // A SIG track across R3's GND pads: the clearance check refuses it.
    board
        .items
        .push_str("  (segment (start 18 16) (end 22 16) (width 0.25) (layer \"F.Cu\") (net 1))\n");
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::TrackRefused), 1);
    assert!(imported.report.counts.tracks > 2);
    // E2E run 11, finding 4: the report names each refused piece, where it
    // ran (KiCad's (18, 16) is (18, 20 − 16) here) and the check's own
    // sentence: what it came too close to.
    let refused: Vec<String> = imported
        .report
        .refused_copper_of(Case::TrackRefused)
        .map(|r| r.line())
        .collect();
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0]
            .starts_with("Track on SIG (Top) from (18, 4) to (22, 4) mm: Too close to R3 pad"),
        "{refused:?}"
    );
    assert!(
        imported
            .report
            .summary_named()
            .contains(&format!("    {}\n", refused[0])),
        "{}",
        imported.report.summary_named()
    );
    Ok(())
}

/// A track from R2's SIG pad whose second piece runs onto R3's GND pad:
/// that piece is refused, and the first piece, now ending in nothing, says
/// in its finding that the import refused the rest and why.
#[test]
fn a_stub_a_refused_track_left_says_so() -> Fallible<()> {
    let mut board = routed();
    board.items.push_str(
        "  (segment (start 29 10) (end 29 16) (width 0.25) (layer \"F.Cu\") (net 1))
  (segment (start 29 16) (end 21 16) (width 0.25) (layer \"F.Cu\") (net 1))\n",
    );
    let imported = import(board.text().as_bytes())?;
    assert_eq!(imported.report.count(Case::TrackRefused), 1);
    let project = imported.engine.project();
    let refused = &project.refused_import_tracks;
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert_eq!(
        refused[0].reason,
        sparksmith_core::model::import_review::TrackRefusal::Clearance
    );
    let report = sparksmith_core::issues::report(
        project,
        &sparksmith_core::library::Drafts::default(),
        &sparksmith_core::issues::IssueScope::All,
        &Default::default(),
    )?;
    let stubs: Vec<&str> = report
        .rows
        .iter()
        .filter(|r| r.finding.code == "board.track_dangling")
        .map(|r| r.finding.message.as_str())
        .collect();
    // KiCad's (29, 16) is (29, 20 − 16) here.
    assert!(
        stubs.contains(
            &"Track on SIG (Top) stops at (29, 4) mm: import refused the rest (clearance)"
        ),
        "{stubs:?}"
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
