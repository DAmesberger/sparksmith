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
