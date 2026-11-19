//! A generator's body for an imported part: fitted to the footprint the
//! part keeps, every part of the package moved in one undo step, a body
//! whose leads miss the pads refused with nothing changed.
use super::Session;
use super::body::fixture_engine;
use serde_json::{Value, json};
use sparksmith_core::library::PackageModel;

type Fallible = Result<(), Box<dyn std::error::Error>>;

fn session() -> Result<Session, Box<dyn std::error::Error>> {
    Ok(Session::fixture(fixture_engine()?))
}

fn call(session: &mut Session, command: &str, mut params: Value) -> Value {
    if command == "library.make_body" {
        params["expected_revision"] = json!(session.engine().revision());
    }
    session.dispatch(json!({"command": command, "params": params}))
}

fn component_of(session: &Session, designator: &str) -> Option<(u64, u32)> {
    session
        .engine()
        .project()
        .instances
        .iter()
        .find(|i| i.designator == designator)
        .map(|i| (i.component.id.0, i.component.revision.0))
}

fn body(generator: &str, preset: &str) -> Value {
    json!({"designator": "R1", "generator": generator, "preset": preset})
}

#[test]
fn a_generated_body_fits_the_kept_footprint_and_every_part_moves_in_one_step() -> Fallible {
    let mut session = session()?;
    let target = call(
        &mut session,
        "library.body_fit",
        json!({"designator": "R1"}),
    );
    assert_eq!(target["ok"], true, "{target}");
    let target = &target["result"]["target"];
    assert_eq!(target["parts"], json!(["R1", "R2"]), "{target}");
    assert_eq!(
        target["footprint"]["pads"].as_array().map(Vec::len),
        Some(2)
    );
    let package = sparksmith_core::commands::body::package_of(session.engine().document(), "R1")?;
    let footprint_pads = |session: &Session| {
        let library = &session.engine().project().library;
        library
            .footprints
            .iter()
            .filter(|f| f.definition.package.id == package.id)
            .map(|f| f.definition.pads.clone())
            .collect::<Vec<_>>()
    };
    let pads_before = footprint_pads(&session);
    let u1 = component_of(&session, "U1");

    let plan = call(
        &mut session,
        "library.body_fit",
        body("chip", "resistor-0603"),
    );
    assert_eq!(plan["ok"], true, "{plan}");
    let plan = &plan["result"];
    assert_eq!(plan["accepted"], true, "{plan}");
    assert_eq!(plan["parts"], json!(["R1", "R2"]));
    assert_eq!(plan["leads"].as_array().map(Vec::len), Some(2), "{plan}");
    assert!(
        plan["leads"][0]["miss_mm"]
            .as_f64()
            .is_some_and(|m| m <= 0.0)
    );
    assert_eq!(plan["change_counts"]["placed part"], 2);
    assert_eq!(plan["change_counts"]["package revision"], 1);
    // Nothing changed by the fit.
    let before = session.engine().revision();
    assert_eq!(component_of(&session, "R1").map(|c| c.1), Some(1));

    let made = call(
        &mut session,
        "library.make_body",
        json!({"designator": "R1", "generator": "chip", "preset": "resistor-0603",
               "sources": "r0603.pdf: drawing 2"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let after = session.engine().revision();
    assert_ne!(after, before);
    // Both parts of the package moved to its next revisions; U1 did not.
    assert_eq!(component_of(&session, "R1").map(|c| c.1), Some(2));
    assert_eq!(component_of(&session, "R2").map(|c| c.1), Some(2));
    assert_eq!(component_of(&session, "U1"), u1);
    let library = &session.engine().project().library;
    let new = sparksmith_core::commands::body::package_of(session.engine().document(), "R2")?;
    assert_eq!(new.id, package.id);
    assert_eq!(new.revision.0, 2);
    let published = library.package(new).ok_or("the new package")?;
    let PackageModel::PartFile(part) = &published.definition.model else {
        return Err(format!("a part file: {:?}", published.definition.model).into());
    };
    assert!(part.derived.bodies >= 3, "a body and two terminations");
    assert_eq!(published.definition.leads.len(), 2);
    assert!(
        published
            .meta
            .provenance
            .contains("recreated by AI from r0603.pdf: drawing 2")
    );
    assert!(published.meta.provenance.contains("Reviewed: no"));
    // The footprint kept its pads (a new revision pinning the package).
    let pads_after = footprint_pads(&session);
    assert_eq!(pads_after.len(), 2);
    assert!(pads_after.iter().all(|p| *p == pads_before[0]));
    // The review report is held against the body.
    let report = json!({"part": "R0603", "summary": "Chip body.",
        "sources": [{"name": "DS", "title": "R0603", "revision": "1"}],
        "values": [{"item": "length", "value": "1.6 mm",
            "source": {"document": "DS", "page": 2, "location": "Figure 1"}, "confidence": "high"}]});
    let checked = call(
        &mut session,
        "library.review_check",
        json!({"report": report, "body": "R1"}),
    );
    assert_eq!(checked["result"]["accepted"], true, "{checked}");
    let unmade = call(
        &mut session,
        "library.review_check",
        json!({"report": report, "body": "U1"}),
    );
    assert_eq!(unmade["result"]["accepted"], false, "{unmade}");
    // One undo step takes it all back.
    let undo = session
        .dispatch(json!({"command": "project.undo", "params": {"expected_revision": after}}));
    assert_eq!(undo["ok"], true, "{undo}");
    assert_eq!(component_of(&session, "R1").map(|c| c.1), Some(1));
    assert_eq!(component_of(&session, "R2").map(|c| c.1), Some(1));
    let library = &session.engine().project().library;
    assert!(library.package(new).is_none());
    Ok(())
}

#[test]
fn a_body_whose_leads_miss_the_pads_is_refused_and_changes_nothing() -> Fallible {
    let mut session = session()?;
    let before = session.engine().revision();
    let plan = call(
        &mut session,
        "library.body_fit",
        body("chip", "resistor-2512"),
    );
    assert_eq!(plan["ok"], true, "{plan}");
    let plan = &plan["result"];
    assert_eq!(plan["accepted"], false, "{plan}");
    let findings = plan["findings"].as_array().ok_or("findings")?;
    assert!(
        findings
            .iter()
            .any(|f| f["code"] == "body.lead_off_land" && f["blocks"] == true),
        "{plan}"
    );
    let refused = call(
        &mut session,
        "library.make_body",
        json!({"designator": "R1", "generator": "chip", "preset": "resistor-2512",
               "sources": "r2512.pdf: drawing 1"}),
    );
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["rule"], "body.misfit", "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("Lead 1 misses pad 1")),
        "{refused}"
    );
    assert_eq!(session.engine().revision(), before);
    // A lead the body does not have, a reason instead of a generator and
    // an unknown part are refused too.
    let mut unknown = body("chip", "resistor-0603");
    unknown["pins"] = json!([{"lead": "9", "pad": "1"}]);
    let unknown = call(&mut session, "library.body_fit", unknown);
    assert_eq!(unknown["ok"], false, "{unknown}");
    let missing = call(
        &mut session,
        "library.body_fit",
        json!({"designator": "U1", "missing": "no switch family"}),
    );
    assert!(
        missing["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains("no switch family")),
        "{missing}"
    );
    let nobody = call(
        &mut session,
        "library.body_fit",
        json!({"designator": "X9"}),
    );
    assert_eq!(nobody["ok"], false, "{nobody}");
    assert_eq!(session.engine().revision(), before);
    Ok(())
}

/// Friction #11: each lead says where its contact sits on its land: the
/// contact and the land's centre in the datum frame, and its offset along
/// the lead and across it, with the land's size.
#[test]
fn each_lead_reports_where_it_touches_its_land() -> Fallible {
    let mut session = session()?;
    let plan = call(
        &mut session,
        "library.body_fit",
        body("chip", "resistor-0603"),
    );
    assert_eq!(plan["ok"], true, "{plan}");
    let leads = plan["result"]["leads"].as_array().ok_or("leads")?;
    assert_eq!(leads.len(), 2);
    let f = |v: &Value| v.as_f64().ok_or("a number");
    for lead in leads {
        let on = &lead["on_land"];
        let (along, across) = (f(&on["along_mm"])?, f(&on["across_mm"])?);
        // The pads are 0.95 mm along the lead (x) and 1.0 mm across it.
        assert!((f(&on["length_mm"])? - 0.95).abs() < 1e-6, "{lead}");
        assert!((f(&on["width_mm"])? - 1.0).abs() < 1e-6, "{lead}");
        assert!(along.abs() <= 0.475 && across.abs() <= 0.5, "{lead}");
        // The offset is the contact's distance from the land's centre.
        let off = f(&lead["off_centre_mm"])?;
        assert!((along.hypot(across) - off).abs() < 2e-3, "{lead}");
        let (contact, centre) = (&on["contact_mm"], &on["centre_mm"]);
        let dx = f(&contact[0])? - f(&centre[0])?;
        let dy = f(&contact[1])? - f(&centre[1])?;
        assert!((dx.hypot(dy) - off).abs() < 2e-3, "{lead}");
        assert!(
            on["summary"]
                .as_str()
                .is_some_and(|s| s.contains("along the lead")),
            "{lead}"
        );
    }
    // Pad 1 lies at −x: outward is −x, so a contact at smaller x than the
    // pad's centre is outward of it, and the other way round.
    let lead1 = leads.iter().find(|l| l["pad"] == "1").ok_or("lead 1")?;
    let on = &lead1["on_land"];
    let dx = f(&on["contact_mm"][0])? - f(&on["centre_mm"][0])?;
    assert!((f(&on["along_mm"])? + dx).abs() < 2e-3, "{lead1}");
    Ok(())
}

/// Friction #12: a lead lands an unnumbered non-plated hole, by its id or
/// by where it falls; the hole never gets a terminal or a net.
#[test]
fn a_mechanical_lead_lands_an_unnumbered_hole() -> Fallible {
    let mut session = session()?;
    let target = call(
        &mut session,
        "library.body_fit",
        json!({"designator": "J1"}),
    );
    let pads = target["result"]["target"]["footprint"]["pads"]
        .as_array()
        .ok_or("pads")?
        .clone();
    let hole = pads
        .iter()
        .find(|p| p["kind"] == "non_plated_hole")
        .ok_or_else(|| format!("no hole: {target}"))?;
    assert_eq!(hole["number"], "");
    let pad1 = pads.iter().find(|p| p["number"] == "1").ok_or("pad 1")?;
    let j1 = |pins: Value| json!({"designator": "J1", "generator": "chip", "preset": "resistor-0603", "pins": pins});
    // By position: lead 2 has no pad 2, and falls in the hole.
    let auto = call(&mut session, "library.body_fit", j1(json!([])));
    assert_eq!(auto["result"]["accepted"], true, "{auto}");
    let pegs: Vec<&Value> = auto["result"]["leads"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["hole"] == hole["id"])
        .collect();
    assert_eq!(pegs.len(), 1, "{auto}");
    assert_eq!(pegs[0]["lead"], "2");
    // By id.
    let named = call(
        &mut session,
        "library.body_fit",
        j1(json!([{"lead": "2", "hole": hole["id"]}])),
    );
    assert_eq!(named["result"]["accepted"], true, "{named}");
    // A copper pad is no hole; a pad and a hole together are refused.
    let before = session.engine().revision();
    for pins in [
        json!([{"lead": "2", "hole": pad1["id"]}]),
        json!([{"lead": "2", "pad": "1", "hole": hole["id"]}]),
    ] {
        let refused = call(&mut session, "library.body_fit", j1(pins));
        assert_eq!(refused["ok"], false, "{refused}");
    }
    assert_eq!(session.engine().revision(), before);
    // Applied: the package keeps one lead contact, on its one terminal;
    // the hole keeps no terminal.
    let made = call(
        &mut session,
        "library.make_body",
        json!({"designator": "J1", "generator": "chip", "preset": "resistor-0603",
               "pins": [{"lead": "2", "hole": hole["id"]}], "sources": "peg.pdf: drawing 1"}),
    );
    assert_eq!(made["ok"], true, "{made}");
    let package = sparksmith_core::commands::body::package_of(session.engine().document(), "J1")?;
    let library = &session.engine().project().library;
    let published = library.package(package).ok_or("package")?;
    assert_eq!(published.definition.leads.len(), 1);
    assert_eq!(published.definition.terminals.len(), 1);
    let holes: Vec<_> = library
        .footprints
        .iter()
        .filter(|f| f.definition.package == package)
        .flat_map(|f| f.definition.pads.iter())
        .filter(|p| p.kind == sparksmith_core::library::PadKind::NonPlatedHole)
        .collect();
    assert!(!holes.is_empty() && holes.iter().all(|p| p.terminal.is_none()));
    Ok(())
}
