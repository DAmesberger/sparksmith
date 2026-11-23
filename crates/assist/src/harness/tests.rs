use super::*;
use serde_json::{Value, json};

type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn profile_errors_identify_the_file_and_field() -> Fallible {
    let shipped = ShippedLibrary::locate()?;
    let text = shipped.read(Shelf::Assist, "harness/compact.json", MAX_FILE_BYTES)?;
    let source: Value = serde_json::from_slice(&text)?;
    for (path, invalid, field) in [
        (
            "/surface_view/strategy",
            json!("hidden"),
            "surface_view.strategy",
        ),
        (
            "/condense/keep_rounds",
            json!("two"),
            "condense.keep_rounds",
        ),
        (
            "/reply_shape/reply_chars",
            json!("small"),
            "reply_shape.reply_chars",
        ),
        ("/condense/keep_rounds", json!(0), "condense.keep_rounds"),
        (
            "/condense/strategy",
            json!("unregistered"),
            "condense.strategy",
        ),
        (
            "/reply_shape/strategy",
            json!("unregistered"),
            "reply_shape.strategy",
        ),
        (
            "/surface_view/full_on_request",
            json!(false),
            "surface_view.full_on_request",
        ),
        ("/condense/trigger", json!(1.1), "condense.trigger"),
        ("/condense/target", json!(0.9), "condense.target"),
        ("/recover/repeat_limit", json!(0), "recover.repeat_limit"),
        (
            "/reply_shape/reply_chars",
            json!(0),
            "reply_shape.reply_chars",
        ),
        ("/nudge/after_tokens", json!("never"), "nudge.after_tokens"),
    ] {
        let mut broken = source.clone();
        *broken.pointer_mut(path).ok_or("fixture field missing")? = invalid;
        let problem = HarnessProfile::parse(&broken.to_string(), "harness/test.json")
            .err()
            .ok_or("broken profile accepted")?
            .to_string();
        assert!(
            problem.contains("harness/test.json") && problem.contains(field),
            "{problem}"
        );
    }
    for slot in [
        "surface_view",
        "reply_shape",
        "condense",
        "nudge",
        "recover",
    ] {
        let mut broken = source.clone();
        broken[slot]["disable_approval"] = json!(true);
        let why = HarnessProfile::parse(&broken.to_string(), "harness/test.json")
            .err()
            .ok_or("unknown nested field accepted")?
            .to_string();
        assert!(
            why.contains("harness/test.json")
                && why.contains(slot)
                && why.contains("disable_approval"),
            "{why}"
        );
    }
    let mut fast = source;
    fast["condense"]["keep_rounds"] = json!(1);
    fast["nudge"]["after_tokens"] = json!(0);
    let policy = HarnessProfile::parse(&fast.to_string(), "harness/test.json")?.policy();
    assert_eq!((policy.keep_rounds, policy.nudge_tokens), (1, 0));
    Ok(())
}

#[test]
fn strategies_are_chosen_only_from_explicit_profile_names() {
    let names = HarnessSelection {
        model: Some("model"),
        workflow: Some("workflow"),
        step: Some("step"),
        connection: Some("person"),
    };
    assert_eq!(names.choose("fallback"), "person");
    let names = HarnessSelection {
        connection: None,
        ..names
    };
    assert_eq!(names.choose("fallback"), "step");
    let names = HarnessSelection {
        step: None,
        ..names
    };
    assert_eq!(names.choose("fallback"), "workflow");
    let names = HarnessSelection {
        workflow: None,
        ..names
    };
    assert_eq!(names.choose("fallback"), "model");
    assert_eq!(HarnessSelection::default().choose("fallback"), "fallback");
}

#[test]
fn users_can_override_a_named_profile_and_recover_from_a_broken_copy() -> Fallible {
    let lib = ShippedLibrary::locate()?;
    let directory = tempfile::tempdir()?;
    let text = lib.read(Shelf::Assist, "harness/compact.json", MAX_FILE_BYTES)?;
    let mut changed: Value = serde_json::from_slice(&text)?;
    changed["condense"]["keep_rounds"] = json!(7);
    changed["surface_view"]["strategy"] = json!("full");
    let path = directory.path().join("compact.json");
    std::fs::write(&path, serde_json::to_vec(&changed)?)?;
    let own = HarnessProfile::load(&lib, Some(directory.path()), "compact")?;
    assert_eq!(own.policy().keep_rounds, 7);
    let mut args = json!({"name":"project.undo"});
    own.surface_view.apply("describe", &mut args);
    assert_eq!(args["detail"], "full");
    args["detail"] = json!("input");
    own.surface_view.apply("describe", &mut args);
    assert_eq!(args["detail"], "input");
    let mut query = json!({"name":"project.schema","params":{"name":"project.undo"}});
    own.surface_view.apply("query", &mut query);
    assert_eq!(query["params"]["detail"], "full");
    query["params"]["detail"] = json!("input");
    own.surface_view.apply("query", &mut query);
    assert_eq!(query["params"]["detail"], "input");
    let mut index = json!({"name":"project.schema"});
    own.surface_view.apply("query", &mut index);
    assert_eq!(
        index,
        json!({"name":"project.schema","params":{"detail":"full"}})
    );
    let mut malformed = json!({"name":"project.schema","params":"bad"});
    own.surface_view.apply("query", &mut malformed);
    assert_eq!(malformed["params"], "bad");
    let mut other = json!({"name":"project.inspect"});
    own.surface_view.apply("query", &mut other);
    assert!(other.get("params").is_none());
    let mut edit = json!({"operations":[]});
    own.surface_view.apply("apply", &mut edit);
    assert_eq!(edit, json!({"operations":[]}));
    changed["reach"] = json!(["files"]);
    std::fs::write(&path, serde_json::to_vec(&changed)?)?;
    let why = HarnessProfile::load(&lib, Some(directory.path()), "compact")
        .err()
        .ok_or("profile accepted reach")?
        .to_string();
    assert!(
        why.contains("reach") && why.contains(&path.display().to_string()),
        "{why}"
    );
    std::fs::remove_file(path)?;
    assert_eq!(
        HarnessProfile::load(&lib, Some(directory.path()), "compact")?,
        HarnessProfile::load(&lib, None, "compact")?
    );
    Ok(())
}

#[test]
fn task_and_person_profiles_replace_all_model_harness_settings() -> Fallible {
    use crate::{AssistLibrary, RunConfig, model::ModelProfile};
    let lib = ShippedLibrary::locate()?;
    let root = tempfile::tempdir()?;
    let run_dir = crate::profile::user_folder(root.path());
    std::fs::create_dir_all(&run_dir)?;
    let harness_dir = root.path().join("assist-harness");
    std::fs::create_dir_all(&harness_dir)?;
    let compact: Value = serde_json::from_slice(&lib.read(
        Shelf::Assist,
        "harness/compact.json",
        MAX_FILE_BYTES,
    )?)?;
    for (name, repeat) in [("workflow", 5), ("step", 6), ("person", 7)] {
        let mut body = compact.clone();
        body["name"] = json!(name);
        if name == "person" {
            body["reply_shape"]["reply_chars"] = json!(73);
            body["surface_view"]["strategy"] = json!("full");
        }
        body["recover"]["repeat_limit"] = json!(repeat);
        std::fs::write(
            harness_dir.join(format!("{name}.json")),
            serde_json::to_vec(&body)?,
        )?;
    }
    let run_profile = json!({"format":1,"name":"default","harness":{"repeat_limit":4},"harness_profile":"compact"});
    std::fs::write(
        run_dir.join("default.json"),
        serde_json::to_vec(&run_profile)?,
    )?;
    let mut config = RunConfig::new(
        &AssistLibrary::load(&lib)?,
        ModelProfile::openai_compatible("scripted", 32_768),
        None,
        Some(&run_dir),
    )?;
    assert_eq!(config.harness_for(None, None)?.recover.repeat_limit, 4);
    assert_eq!(
        config
            .harness_for(Some("workflow"), None)?
            .recover
            .repeat_limit,
        5
    );
    assert_eq!(
        config
            .harness_for(Some("workflow"), Some("step"))?
            .recover
            .repeat_limit,
        6
    );
    config.profile.harness_profile = Some("person".into());
    assert_eq!(
        config
            .harness_for(Some("missing"), Some("also-missing"))?
            .recover
            .repeat_limit,
        7
    );
    let chat = crate::chat::ChatConfig::with_profiles(
        &crate::chat::ChatLibrary::load(&lib)?,
        config.profile,
        crate::chat::unbounded(),
        Some(&run_dir),
    )?;
    assert_eq!(chat.policy.reply_chars, 73);
    assert_eq!(chat.surface_view.strategy, SchemaDetail::Full);
    Ok(())
}

#[test]
fn a_lines_first_strategy_honours_explicit_details() {
    let view = SurfaceView {
        strategy: SchemaDetail::Line,
        full_on_request: true,
    };
    let mut describe = json!({"name":"project.inspect"});
    view.apply("describe", &mut describe);
    assert_eq!(describe["detail"], "line");
    let mut query =
        json!({"name":"project.schema","params":{"name":"project.inspect","detail":"full"}});
    view.apply("query", &mut query);
    assert_eq!(query["params"]["detail"], "full");
}
