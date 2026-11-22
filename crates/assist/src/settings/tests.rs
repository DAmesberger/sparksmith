use super::*;

type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

fn local(name: &str) -> EndpointSetting {
    EndpointSetting {
        name: name.into(),
        provider: Wire::Openai,
        base_url: "http://localhost:11434/v1".into(),
        model: "qwen3.8:27b".into(),
        timeout_minutes: 90,
        capabilities: Some(Declared {
            tool_calling: true,
            vision: true,
            native_pdf: false,
            context_tokens: 65_536,
            output_tokens: None,
        }),
    }
}

fn agent(name: &str) -> AgentSetting {
    AgentSetting {
        name: name.into(),
        command: "opencode".into(),
        args: vec!["acp".into()],
        env: BTreeMap::new(),
    }
}

/// The shipped defaults read and check: the chat's limits sized from E2E
/// run 13 (a turn far below the 4 000 000 tokens it once allowed, above
/// the chat's 250 000-token idle stop, and a conversation of several
/// turns), 45 minutes a turn (Q7), edits
/// without asking, files and publishing to a library asked, and every
/// preset a valid starting point.
#[test]
fn the_shipped_settings_load() -> Fallible {
    let shipped = Shipped::locate()?;
    let limits = shipped.limits;
    assert_eq!(limits.minutes, 45);
    assert_eq!(limits.total_tokens, 400_000);
    assert_eq!(limits.requests, 20);
    assert_eq!(limits.output_tokens, 100_000);
    assert_eq!(limits.conversation_tokens, 2_000_000);
    assert!(limits.total_tokens > crate::chat::Policy::CHAT.idle_tokens);
    assert!(limits.conversation_tokens >= 5 * limits.total_tokens);
    assert_eq!(shipped.permissions.edits, Rule::Always);
    assert_eq!(shipped.permissions.files, Rule::Ask);
    assert_eq!(shipped.permissions.publish_library, Rule::Ask);
    assert!(
        shipped
            .endpoint_presets
            .iter()
            .any(|p| p.endpoint.provider == Wire::Anthropic)
    );
    assert!(
        shipped
            .endpoint_presets
            .iter()
            .any(|p| p.endpoint.provider == Wire::Openai)
    );
    assert_eq!(shipped.agent_presets.len(), 4);
    Ok(())
}

#[test]
fn settings_round_trip_and_a_missing_file_is_empty() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = user_file(dir.path());
    assert_eq!(read(&path)?, AssistSettings::default());
    let settings = AssistSettings {
        endpoints: vec![local("Local")],
        agents: vec![agent("OpenCode")],
        connection: Some("OpenCode".into()),
        permissions: Some(Permissions {
            edits: Rule::Ask,
            files: Rule::Session,
            publish_library: Rule::Deny,
            view: Rule::Always,
        }),
        limits: Some(Shipped::locate()?.limits),
        ..AssistSettings::default()
    };
    write(&path, &settings)?;
    assert_eq!(read(&path)?, settings);
    // The assistant uses an agent; a model run takes the first endpoint.
    assert_eq!(
        settings.run_endpoint().map(|e| e.name.as_str()),
        Some("Local")
    );
    Ok(())
}

/// A broken file is refused with its path, never read as empty, so no
/// save replaces it by mistake; unknown fields are refused by name.
#[test]
fn a_broken_or_unknown_file_is_refused() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = user_file(dir.path());
    std::fs::create_dir_all(path.parent().ok_or("parent")?)?;
    std::fs::write(&path, r#"{"format":1,"api_key":"sk-x"}"#)?;
    let why = read(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(
        why.contains("api_key") && why.contains("settings.json"),
        "{why}"
    );
    std::fs::write(&path, r#"{"format":2}"#)?;
    let why = read(&path).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(why.contains("format 2"), "{why}");
    Ok(())
}

#[test]
fn every_rule_is_checked_with_its_reason() {
    let base = AssistSettings {
        endpoints: vec![local("Local")],
        agents: vec![agent("OpenCode")],
        ..AssistSettings::default()
    };
    assert_eq!(base.check(), Ok(()));
    let refused = |change: &dyn Fn(&mut AssistSettings), word: &str| {
        let mut s = base.clone();
        change(&mut s);
        let why = s.check().err().unwrap_or_default();
        assert!(why.contains(word), "{word}: {why}");
    };
    refused(&|s| s.agents[0].name = "Local".into(), "two connections");
    refused(&|s| s.connection = Some("Gone".into()), "not set up");
    refused(&|s| s.endpoints[0].base_url = "ftp://x".into(), "http");
    refused(&|s| s.endpoints[0].model = " ".into(), "model id");
    refused(&|s| s.endpoints[0].timeout_minutes = 0, "timeout");
    refused(&|s| s.endpoints[0].capabilities = None, "declare");
    refused(
        &|s| {
            if let Some(c) = &mut s.endpoints[0].capabilities {
                c.context_tokens = 10;
            }
        },
        "context",
    );
    refused(&|s| s.agents[0].command = String::new(), "command");
    refused(
        &|s| {
            s.agents[0].env.insert("1BAD".into(), "x".into());
        },
        "variable name",
    );
    // An agent signs in itself: no key goes into the file.
    refused(
        &|s| {
            s.agents[0]
                .env
                .insert("ANTHROPIC_API_KEY".into(), "sk".into());
        },
        "sign in with the agent",
    );
    refused(
        &|s| {
            s.limits = Some(Limits {
                requests: 1,
                output_tokens: 1,
                total_tokens: 1,
                minutes: 0,
                conversation_tokens: 1,
            });
        },
        "minutes",
    );
    refused(
        &|s| {
            s.limits = Some(Limits {
                requests: 1,
                output_tokens: 1,
                total_tokens: 1,
                minutes: 1,
                conversation_tokens: 0,
            });
        },
        "tokens of a conversation",
    );
    // The Messages API declares its own capabilities.
    let mut anthropic = base.clone();
    anthropic.endpoints[0].provider = Wire::Anthropic;
    anthropic.endpoints[0].base_url = "https://api.anthropic.com".into();
    anthropic.endpoints[0].capabilities = None;
    assert_eq!(anthropic.check(), Ok(()));
}

#[test]
fn an_endpoint_gives_the_model_as_declared() -> Fallible {
    let mut e = local("Local");
    let p = e.model_profile()?;
    assert_eq!(p.provider, Provider::OpenAiCompatible);
    assert_eq!(p.capabilities.context_tokens, 65_536);
    assert!(p.capabilities.vision && !p.capabilities.native_pdf);
    assert_eq!(p.request_timeout, Duration::from_secs(90 * 60));
    if let Some(c) = &mut e.capabilities {
        c.output_tokens = Some(4_000);
    }
    assert_eq!(e.model_profile()?.output.per_request, 4_000);
    assert!(e.summary().contains("this computer"));
    Ok(())
}

/// The chat runs within the shipped limits until the person sets their
/// own; a turn's tokens are held within what is left of the
/// conversation's, and a Continue past the conversation's limit raises it.
#[test]
fn a_turn_budget_is_the_turns_limits_within_the_conversations() -> Fallible {
    let shipped = Shipped::locate()?;
    let none = AssistSettings::default();
    assert_eq!(none.limits_or(&shipped), shipped.limits);
    let mut own = shipped.limits;
    own.total_tokens = 1_000;
    own.conversation_tokens = 2_500;
    own.minutes = 10;
    let settings = AssistSettings {
        limits: Some(own),
        ..AssistSettings::default()
    };
    let limits = settings.limits_or(&shipped);
    let (fresh, bound) = limits.turn_budget(0, 0);
    assert_eq!(bound, Bound::Turn);
    assert_eq!(fresh.max_total_tokens, 1_000);
    assert_eq!(fresh.max_requests, own.requests);
    assert_eq!(fresh.max_output_tokens, own.output_tokens);
    assert_eq!(fresh.max_duration, Duration::from_secs(10 * 60));
    let (late, bound) = limits.turn_budget(2_000, 0);
    assert_eq!(bound, Bound::Conversation);
    assert_eq!(late.max_total_tokens, 500);
    let (spent, _) = limits.turn_budget(3_000, 0);
    assert_eq!(spent.max_total_tokens, 0, "nothing is left to send");
    let (raised, bound) = limits.turn_budget(3_000, 1_000);
    assert_eq!(bound, Bound::Conversation);
    assert_eq!(raised.max_total_tokens, 500);
    Ok(())
}

#[test]
fn a_free_name_does_not_clash() {
    let s = AssistSettings {
        endpoints: vec![local("Local"), local("Local 2")],
        ..AssistSettings::default()
    };
    assert_eq!(s.free_name("Local"), "Local 3");
    assert_eq!(s.free_name("Other"), "Other");
}

/// Copy, edit with a check on save, remove: a text that fails the check
/// leaves the file as it was, and the built-in comes back on removal.
#[test]
fn a_user_profile_is_copied_checked_on_save_and_removed() -> Fallible {
    let shipped = AssistLibrary::locate()?;
    let dir = tempfile::tempdir()?;
    let folder = crate::profile::user_folder(dir.path());
    let listed = profiles(&shipped, Some(&folder));
    assert!(
        listed
            .iter()
            .all(|p| p.source == ProfileSource::BuiltIn && p.profile.is_ok())
    );
    let path = shipped.copy_to_user("default", &folder)?;
    let listed = profiles(&shipped, Some(&folder));
    let default = listed
        .iter()
        .find(|p| p.name == "default")
        .ok_or("listed")?;
    assert_eq!(default.source, ProfileSource::User);
    assert_eq!(listed.iter().filter(|p| p.name == "default").count(), 1);

    let before = std::fs::read_to_string(&path)?;
    let bad = before.replace("\"base\"", "\"sometimes\"");
    assert!(save_profile(&folder, "default", &bad).is_err());
    let renamed = before.replace("\"default\"", "\"other\"");
    let why = save_profile(&folder, "default", &renamed)
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(why.contains("file's name"), "{why}");
    assert_eq!(std::fs::read_to_string(&path)?, before, "left as it was");

    let good = r#"{"format": 1, "name": "default", "system": {"add": ["Be brief."]}}"#;
    let saved = save_profile(&folder, "default", good)?;
    assert_eq!(
        saved.system,
        crate::profile::SystemPrompt::Add(vec!["Be brief.".into()])
    );

    // A broken file is listed with its reason, never used.
    std::fs::write(folder.join("broken.json"), "{")?;
    let listed = profiles(&shipped, Some(&folder));
    let broken = listed.iter().find(|p| p.name == "broken").ok_or("listed")?;
    assert!(broken.profile.is_err());

    remove_profile(&folder, "default")?;
    let listed = profiles(&shipped, Some(&folder));
    let default = listed
        .iter()
        .find(|p| p.name == "default")
        .ok_or("listed")?;
    assert_eq!(default.source, ProfileSource::BuiltIn);
    Ok(())
}
