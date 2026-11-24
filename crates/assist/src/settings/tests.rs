use super::*;

type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

fn local(name: &str) -> EndpointSetting {
    EndpointSetting {
        harness_profile: None,
        timeout_minutes: None,
        name: name.into(),
        provider: Wire::Openai,
        base_url: "http://localhost:11434/v1".into(),
        model: "qwen3.8:27b".into(),
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

/// The shipped defaults read and check: 45 minutes a run (Q7), edits
/// without asking, files and publishing to a library asked, and every
/// preset a valid starting point.
#[test]
fn the_shipped_settings_load() -> Fallible {
    let shipped = Shipped::locate()?;
    assert_eq!(shipped.limits_start.minutes, 45);
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
        limits: Some(Shipped::locate()?.limits_start),
        ..AssistSettings::default()
    };
    write(&path, &settings)?;
    let kept = read(&path)?;
    assert_eq!(kept.endpoints[0].timeout_minutes, Some(Wait::Limit(90)));
    let shipped = Shipped::locate()?;
    let wait = |e: &EndpointSetting| {
        e.waiting_profile(&shipped, &shipped.network)
            .map(|p| p.silence.after)
    };
    let minutes = |n: u64| Some(Duration::from_secs(n * 60));
    assert_eq!(wait(&kept.endpoints[0])?, minutes(90));

    value["endpoints"][0]["timeout_minutes"] = "no_limit".into();
    std::fs::write(&path, value.to_string())?;
    let unlimited = read(&path)?;
    assert_eq!(wait(&unlimited.endpoints[0])?, None);
    write(&path, &unlimited)?;
    assert_eq!(read(&path)?, unlimited);

    // Where nothing is said, as the presets say: here, and elsewhere.
    let mut silent = local("Local");
    assert_eq!(wait(&silent)?, minutes(90));
    silent.base_url = "https://example.com/v1".into();
    assert_eq!(wait(&silent)?, minutes(30));

    value["endpoints"][0]["timeout_minutes"] = 0.into();
    std::fs::write(&path, value.to_string())?;
    assert!(read(&path).is_err_and(|e| e.to_string().contains("minutes")));
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
    refused(
        &|s| s.endpoints[0].harness_profile = Some("../test".into()),
        "harness_profile",
    );
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
            });
        },
        "minutes",
    );
    // The Messages API declares its own capabilities.
    let mut anthropic = base.clone();
    anthropic.endpoints[0].provider = Wire::Anthropic;
    anthropic.endpoints[0].base_url = "https://api.anthropic.com".into();
    anthropic.endpoints[0].capabilities = None;
    assert_eq!(anthropic.check(), Ok(()));
}

/// A file of an earlier version, which says how long a reply may take, keeps
/// its value; `no_limit` is a choice, and a file without it has the shipped
/// wait of such a connection.
#[test]
fn a_connections_wait_is_read_and_defaults_to_the_shipped_one() -> Fallible {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("settings.json");
    let settings = AssistSettings {
        endpoints: vec![local("Local")],
        ..AssistSettings::default()
    };
    let mut value = serde_json::to_value(&settings)?;
    value["endpoints"][0]["timeout_minutes"] = 90.into();
    std::fs::write(&path, value.to_string())?;
    assert_eq!(read(&path)?, settings);
    Ok(())
}

#[test]
fn an_endpoint_gives_the_model_as_declared() -> Fallible {
    let mut e = local("Local");
    let p = e.model_profile()?;
    assert_eq!(p.provider, Provider::OpenAiCompatible);
    assert_eq!(p.capabilities.context_tokens, 65_536);
    assert!(p.capabilities.vision && !p.capabilities.native_pdf);
    if let Some(c) = &mut e.capabilities {
        c.output_tokens = Some(4_000);
    }
    assert_eq!(e.model_profile()?.output.per_request, 4_000);
    assert!(e.summary().contains("this computer"));
    Ok(())
}

/// A run is unlimited unless the person sets limits, and then has each of
/// theirs.
#[test]
fn a_runs_limits_are_the_persons_or_none() -> Fallible {
    let none = run_budget(None);
    assert_eq!(
        (none.max_requests, none.max_output_tokens),
        (None, None),
        "nothing is capped"
    );
    assert_eq!((none.max_total_tokens, none.max_duration), (None, None));
    let mut limits = Shipped::locate()?.limits_start;
    limits.minutes = 10;
    let own = run_budget(Some(&limits));
    assert_eq!(own.max_requests, Some(limits.requests));
    assert_eq!(own.max_output_tokens, Some(limits.output_tokens));
    assert_eq!(own.max_total_tokens, Some(limits.total_tokens));
    assert_eq!(own.max_duration, Some(Duration::from_secs(10 * 60)));
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

/// The person's network waits replace the shipped ones; a broken file
/// leaves the shipped ones.
#[test]
fn network_waits_are_the_persons_or_the_shipped_ones() -> Fallible {
    let shipped = Shipped::locate()?.network;
    let dir = tempfile::tempdir()?;
    assert_eq!(network(Some(dir.path()))?, shipped);
    let own = Network {
        search: Wait::NO_LIMIT,
        download: Wait::Limit(300),
        ..shipped
    };
    let settings = AssistSettings {
        network: Some(own),
        ..AssistSettings::default()
    };
    let path = user_file(dir.path());
    write(&path, &settings)?;
    assert_eq!(network(Some(dir.path()))?, own);
    assert_eq!(Network::silence(own.search).after, None);
    assert_eq!(
        Network::silence(own.download).after,
        Some(Duration::from_secs(300))
    );
    let mut bad = settings;
    bad.network = Some(Network {
        index: Wait::Limit(0),
        ..own
    });
    assert!(bad.check().is_err_and(|why| why.contains("index")));
    std::fs::write(&path, "not settings")?;
    assert_eq!(network(Some(dir.path()))?, shipped);
    Ok(())
}
