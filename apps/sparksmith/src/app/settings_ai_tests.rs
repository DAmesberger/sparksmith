//! Settings › AI, headless: the whole app driven through its
//! accessibility tree at a small and a large window, and the page's
//! saves on their own. No test reaches the system keychain or a network:
//! the page's keyring is [`test_keyring::MEMORY`] and Test connection's
//! request is [`test_keyring::check`] under test.
use super::*;
use crate::app::ax_harness::{Ax, Fallible};
use crate::assistant::State as Chat;
use crate::shell::Workspace;
use egui::accesskit::Role;

const SMALL: egui::Vec2 = egui::vec2(1024.0, 640.0);
const LARGE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// What a key pasted in the tests looks like.
const PASTED: &str = "sk-ant-pasted-secret";

/// The app at `size`, settled.
fn app_at(size: egui::Vec2) -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Schematic)?;
    ax.window = size;
    ax.settle();
    Ok(ax)
}

/// The app at `size` with Settings open, settled.
fn settings_at(size: egui::Vec2) -> Fallible<Ax> {
    let mut ax = app_at(size)?;
    ax.app.settings_window.toggle();
    ax.settle();
    Ok(ax)
}

/// Scroll the Settings body until the node of `role` named `name` is well
/// inside the window; the node.
pub(crate) fn scroll_to(
    ax: &mut Ax,
    role: Role,
    name: &str,
) -> Fallible<crate::app::ax_harness::Node> {
    scroll_to_nth(ax, role, name, false)
}

/// [`scroll_to`] the last node of that name (a form's Save below
/// Connection's).
fn scroll_to_nth(
    ax: &mut Ax,
    role: Role,
    name: &str,
    last: bool,
) -> Fallible<crate::app::ax_harness::Node> {
    for _ in 0..240 {
        let all = ax.all(role, name);
        let node = if last { all.last() } else { all.first() }
            .map(|n| (*n).clone())
            .ok_or_else(|| format!("no {role:?} {name:?} in:\n{}", ax.dump()))?;
        let room = ax.window.y;
        if node.rect.top() > 70.0 && node.rect.bottom() < room - 90.0 {
            return Ok(node);
        }
        let down = node.rect.bottom() >= room - 90.0;
        let at = egui::pos2(ax.window.x / 2.0, room / 2.0);
        ax.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, if down { -40.0 } else { 40.0 }),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        ax.settle();
    }
    Err(format!("{name:?} never came into view").into())
}

/// Scroll to the button `name` and click it.
fn press(ax: &mut Ax, name: &str) -> Fallible {
    let node = scroll_to(ax, Role::Button, name)?;
    if node.disabled {
        return Err(format!("{name:?} is disabled:\n{}", ax.dump()).into());
    }
    ax.click_at(node.center());
    Ok(())
}

/// Open the folded section `title` (its caption as written).
fn unfold(ax: &mut Ax, title: &str) -> Fallible {
    let node = scroll_to(ax, Role::DisclosureTriangle, title)?;
    if node.toggled != Some(true) {
        ax.click_at(node.center());
    }
    Ok(())
}

fn page(ax: &mut Ax) -> Fallible<&mut State> {
    ax.app
        .settings_window
        .ai
        .state
        .as_mut()
        .ok_or_else(|| "the page was not read".into())
}

fn key_focused(ax: &Ax) -> bool {
    ax.ctx.memory(|m| m.has_focus(key_field_id()))
}

/// Every Connection button shown lies inside the Settings card, so none is
/// cut off or unreachable at a small window.
fn buttons_inside_card(ax: &Ax) -> Fallible {
    let card = ax
        .ctx
        .memory(|m| m.area_rect(egui::Id::unique("app-settings")))
        .ok_or("no Settings card")?;
    for name in [
        "Paste",
        "Show",
        "Hide",
        "Replace",
        "Remove",
        "Test connection",
        "Save",
        "Keep the stored key",
    ] {
        for node in ax.all(Role::Button, name) {
            assert!(
                node.rect.right() <= card.right() + 0.5 && node.rect.left() >= card.left() - 0.5,
                "{name} at {:?} leaves the card {card:?}",
                node.rect
            );
        }
    }
    Ok(())
}

fn keychain_has(base: &str) -> bool {
    test_keyring::KEYS.with(|k| k.borrow().contains_key(base))
}

/// The first screen is Connection alone: what to use, one API key field
/// with Paste and Show, the model and Test connection; everything else is
/// folded, and no jargon shows. Opened, the folded parts are all there.
#[test]
fn the_first_screen_is_connection_and_the_rest_is_folded() -> Fallible {
    for size in [SMALL, LARGE] {
        let mut ax = settings_at(size)?;
        for caption in [
            "AI",
            "CONNECTION",
            "ADVANCED",
            "PERMISSIONS",
            "RUN LIMITS",
            "PROFILES",
            "WORKFLOWS",
        ] {
            assert!(
                ax.texts.iter().any(|t| t == caption),
                "{caption} at {size:?}: {:?}",
                ax.texts
            );
        }
        scroll_to(&mut ax, Role::Label, "Use")?;
        assert!(ax.texts.iter().any(|t| t == "Anthropic"), "{:?}", ax.texts);
        assert!(ax.has(Role::PasswordInput, "API key"), "{}", ax.dump());
        for button in ["Paste", "Show", "Test connection", "Save"] {
            assert!(ax.has(Role::Button, button), "{button}:\n{}", ax.dump());
        }
        // Nothing to save or test before a key is pasted, and it says why.
        assert!(ax.find(Role::Button, "Save")?.disabled);
        assert!(ax.find(Role::Button, "Test connection")?.disabled);
        // The model starts from the shipped default.
        let state = page(&mut ax)?;
        let default = state
            .library
            .as_ref()
            .map(|l| l.default_model.clone())
            .map_err(Clone::clone)?;
        let quick = state.quick.as_ref().ok_or("Connection's form")?;
        assert_eq!(quick.draft.model, default);
        // Folded: none of Advanced's, the permissions' or the limits' rows,
        // and no word of jargon on the screen.
        for hidden in ["Edit the design", "Limit model runs", "Add a model"] {
            assert!(!ax.has(Role::Label, hidden), "{hidden} shows at {size:?}");
        }
        for word in [
            "endpoint", "Endpoint", "preset", "ACP", "Base URL", "base URL",
        ] {
            assert!(
                !ax.texts.iter().any(|t| t.contains(word)),
                "{word:?} shows at {size:?}: {:?}",
                ax.texts
            );
        }
        // Opened, each part is there.
        unfold(&mut ax, "Permissions")?;
        for label in [
            "Edit the design",
            "Read or write files",
            "Publish to a library",
            "Move my view",
        ] {
            assert!(ax.has(Role::Label, label), "{label} at {size:?}");
        }
        assert!(ax.texts.iter().any(|t| t == "Always allow"));
        assert!(ax.texts.iter().any(|t| t == "Ask every time"));
        assert!(!ax.has(Role::Label, "Edits") && !ax.has(Role::Label, "Calls"));
        // Model runs are not limited by default; turned on, the limits
        // start from the shipped values (45 minutes, Q7) and are kept.
        unfold(&mut ax, "Run limits")?;
        let check = scroll_to(&mut ax, Role::CheckBox, "Limit model runs")?;
        assert_eq!(check.toggled, Some(false));
        ax.click_at(check.center());
        ax.settle();
        assert!(ax.has(Role::Label, "Model requests"), "{}", ax.dump());
        let saved = stored::read(&stored::user_file(&ax.dir.path().join("config")))?;
        assert_eq!(saved.limits.map(|l| l.minutes), Some(45));
        unfold(&mut ax, "Workflows")?;
        assert!(
            ax.texts.iter().any(|t| {
                t.starts_with("Datasheet → component v") || t == "datasheet-to-component"
            }),
            "{:?}",
            ax.texts
        );
        unfold(&mut ax, "Profiles")?;
        assert!(ax.texts.iter().any(|t| t.starts_with("default · built in")));
        for (text, rect) in &ax.seen {
            assert!(
                rect.right() <= size.x + 0.5,
                "{text:?} runs off the window at {size:?}: {rect:?}"
            );
        }
        // No prices in the product.
        assert!(
            !ax.texts
                .iter()
                .any(|t| t.contains('$') || t.contains("USD") || t.contains("Cost"))
        );
    }
    Ok(())
}

/// The first run, by clicking: the chat says no model is connected and
/// offers one button; it opens Connection with the key field focused;
/// Paste puts the key in, masked; Test connection answers; Save keeps the
/// key in the keychain only, and the chat is ready.
#[test]
fn the_first_run_connects_a_model_from_the_chat() -> Fallible {
    for size in [SMALL, LARGE] {
        test_keyring::KEYS.with(|k| k.borrow_mut().clear());
        let mut ax = app_at(size)?;
        let ctx = ax.ctx.clone();
        ax.app.toggle_assistant(&ctx);
        ax.settle();
        assert!(
            matches!(&ax.app.assistant.conversation.state, Chat::NotSetUp(why) if why == NOT_CONNECTED),
            "{:?}",
            ax.app.assistant.conversation.state
        );
        assert!(ax.texts.iter().any(|t| t == "No model connected"));
        assert!(
            !ax.texts.iter().any(|t| t.contains("ANTHROPIC_API_KEY")),
            "no variable names in the chat"
        );
        ax.button(crate::assistant::CONNECT)?;
        ax.settle();
        assert!(ax.app.settings_window.is_open());
        // Connection is in view, its key field focused.
        let field = ax.find(Role::PasswordInput, "API key")?;
        assert!(
            field.rect.top() > 0.0 && field.rect.bottom() < size.y,
            "the key field is in view at {size:?}: {:?}",
            field.rect
        );
        assert!(key_focused(&ax), "the key field has the keyboard");
        buttons_inside_card(&ax)?;
        // Paste: the clipboard's text arrives as a paste into the field.
        ax.button("Paste")?;
        assert!(key_focused(&ax), "Paste keeps the keyboard on the field");
        ax.frame(vec![egui::Event::Paste(PASTED.into())]);
        ax.settle();
        assert_eq!(
            page(&mut ax)?.quick.as_ref().map(|q| q.key.as_str()),
            Some(PASTED)
        );
        assert!(
            !ax.texts.iter().any(|t| t.contains(PASTED)),
            "the key is masked"
        );
        // Show shows it while typing; Hide masks it again.
        ax.button("Show")?;
        assert!(ax.texts.iter().any(|t| t.contains(PASTED)));
        ax.button("Hide")?;
        assert!(!ax.texts.iter().any(|t| t.contains(PASTED)));
        // Test connection: one short request, off the UI thread.
        press(&mut ax, "Test connection")?;
        let model = page(&mut ax)?
            .quick
            .as_ref()
            .map(|q| q.draft.model.clone())
            .unwrap_or_default();
        let passed = format!("Connected: {model} answered.");
        ax.until("the test's answer", |ax| ax.texts.contains(&passed))?;
        let config = ax.dir.path().join("config");
        assert!(
            !stored::user_file(&config).exists(),
            "nothing is written before Save"
        );
        press(&mut ax, "Save")?;
        let text = std::fs::read_to_string(stored::user_file(&config))?;
        assert!(!text.contains(PASTED), "no key in the file: {text}");
        let saved = stored::read(&stored::user_file(&config))?;
        assert_eq!(saved.connection.as_deref(), Some("Anthropic"));
        assert!(keychain_has("https://api.anthropic.com"));
        assert!(ax.texts.iter().any(|t| t == STORED), "{:?}", ax.texts);
        assert!(!ax.has(Role::PasswordInput, "API key"), "the key is gone");
        buttons_inside_card(&ax)?;
        assert_eq!(
            page(&mut ax)?.quick.as_ref().map(|q| q.key.as_str()),
            Some("")
        );
        // The chat is ready, without its button.
        ax.settle();
        assert_eq!(ax.app.assistant.conversation.state, Chat::Idle);
        assert!(!ax.has(Role::Button, crate::assistant::CONNECT));
    }
    Ok(())
}

/// A stored key: Replace shows the field again (Keep the stored key goes
/// back), a new key replaces it, and Remove takes it out of the keychain;
/// the chat is then not connected again.
#[test]
fn a_stored_key_is_replaced_and_removed() -> Fallible {
    for size in [SMALL, LARGE] {
        test_keyring::KEYS.with(|k| k.borrow_mut().clear());
        let mut ax = app_at(size)?;
        let config = ax.dir.path().join("config");
        // Connected before: Anthropic with its key in the keychain.
        let shipped = Shipped::locate()?;
        let mut anthropic = shipped
            .endpoint_presets
            .iter()
            .find(|p| p.endpoint.provider == Wire::Anthropic)
            .map(|p| p.endpoint.clone())
            .ok_or("an Anthropic choice")?;
        anthropic.model = "a-model".into();
        stored::write(
            &stored::user_file(&config),
            &AssistSettings {
                endpoints: vec![anthropic],
                connection: Some("Anthropic".into()),
                ..AssistSettings::default()
            },
        )?;
        test_keyring::KEYS.with(|k| {
            k.borrow_mut()
                .insert("https://api.anthropic.com".into(), "stored".into())
        });
        let ctx = ax.ctx.clone();
        ax.app.toggle_assistant(&ctx);
        ax.settle();
        assert_eq!(ax.app.assistant.conversation.state, Chat::Idle);
        ax.app.settings_window.toggle();
        ax.settle();
        scroll_to(&mut ax, Role::Button, "Replace")?;
        assert!(ax.texts.iter().any(|t| t == STORED));
        assert!(ax.find(Role::Button, "Save")?.disabled, "nothing to save");
        // The status line under the form's buttons: egui paints only what
        // the scroll area shows, and where Replace lands depends on the
        // sections above, so bring the buttons into view first.
        scroll_to(&mut ax, Role::Button, "Test connection")?;
        assert!(
            ax.texts.iter().any(|t| t == "Set: the chat uses a-model."),
            "{:?}",
            ax.texts
        );
        // Replace, then Keep the stored key: back as it was.
        press(&mut ax, "Replace")?;
        assert!(ax.has(Role::PasswordInput, "API key"));
        assert!(key_focused(&ax));
        buttons_inside_card(&ax)?;
        press(&mut ax, "Keep the stored key")?;
        buttons_inside_card(&ax)?;
        assert!(!ax.has(Role::PasswordInput, "API key"));
        // Replace with a new key and save.
        press(&mut ax, "Replace")?;
        ax.frame(vec![egui::Event::Paste(PASTED.into())]);
        ax.settle();
        press(&mut ax, "Save")?;
        assert!(keychain_has("https://api.anthropic.com"));
        assert!(!ax.has(Role::PasswordInput, "API key"));
        let text = std::fs::read_to_string(stored::user_file(&config))?;
        assert!(!text.contains(PASTED), "{text}");
        // Remove: out of the keychain, the field back, the chat not
        // connected.
        press(&mut ax, "Remove")?;
        assert!(!keychain_has("https://api.anthropic.com"));
        assert!(ax.has(Role::PasswordInput, "API key"), "{}", ax.dump());
        ax.settle();
        assert!(
            matches!(ax.app.assistant.conversation.state, Chat::NotSetUp(_)),
            "{:?}",
            ax.app.assistant.conversation.state
        );
    }
    Ok(())
}

/// An agent is one click: Use Claude Code, with no command, arguments or
/// key on the screen; it is kept as the connection.
#[test]
fn an_agent_is_chosen_in_one_click() -> Fallible {
    let mut ax = settings_at(LARGE)?;
    let state = page(&mut ax)?;
    let settings = state.settings.clone()?;
    let shipped = Shipped::locate()?;
    let index = shipped
        .agent_presets
        .iter()
        .position(|p| p.title == "Claude Code")
        .ok_or("Claude Code")?;
    // As the dropdown sets it.
    let quick = Quick::open(
        Choice::Agent(index),
        state,
        &settings,
        &shipped,
        test_keyring::MEMORY,
    );
    state.quick = Some(quick);
    ax.settle();
    assert!(!ax.has(Role::PasswordInput, "API key"));
    for hidden in ["Command", "Arguments", "Environment", "Model"] {
        assert!(!ax.has(Role::Label, hidden), "{hidden} shows");
    }
    press(&mut ax, "Use Claude Code")?;
    let saved = stored::read(&stored::user_file(&ax.dir.path().join("config")))?;
    assert_eq!(saved.connection.as_deref(), Some("Claude Code"));
    assert_eq!(saved.agents.len(), 1);
    assert!(ax.find(Role::Button, "Use Claude Code")?.disabled);
    Ok(())
}

/// A model server on this computer asks for its address and no key, and
/// a server the person moved (LM Studio's port) is still that choice.
#[test]
fn a_local_server_asks_for_its_address_and_no_key() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let state = page.state();
    let settings = state.settings.clone()?;
    let shipped = Shipped::locate()?;
    let index = shipped
        .endpoint_presets
        .iter()
        .position(|p| p.endpoint.endpoint().is_ok_and(|e| e.is_loopback()))
        .ok_or("a local choice")?;
    let quick = Quick::open(
        Choice::Model(index),
        state,
        &settings,
        &shipped,
        test_keyring::MEMORY,
    );
    assert!(quick.local());
    let mut moved = quick.draft.clone();
    moved.base_url = "http://127.0.0.1:1234/v1".into();
    moved.model = "m".into();
    let saved = AssistSettings {
        endpoints: vec![moved],
        connection: Some(quick.draft.name.clone()),
        ..AssistSettings::default()
    };
    assert!(state.commit(saved.clone()));
    let read = Quick::read(state, &saved, &shipped, test_keyring::MEMORY);
    assert_eq!(read.choice, Choice::Model(index));
    // Ready without a key: the server is on this computer.
    assert_eq!(readiness(&saved, test_keyring::MEMORY), Ok(()));
    Ok(())
}

/// Adding a connection under Advanced by clicking: the form opens with the
/// choice's values, a typed key is masked, Save writes the connection
/// (never the key) and the key goes to the keychain only.
#[test]
fn a_connection_is_added_under_advanced_and_its_key_kept_out_of_files() -> Fallible {
    test_keyring::KEYS.with(|k| k.borrow_mut().clear());
    for size in [LARGE, SMALL] {
        let mut ax = settings_at(size)?;
        let config = ax.dir.path().join("config");
        unfold(&mut ax, "Advanced")?;
        let row = scroll_to(&mut ax, Role::Label, "Add a model")?;
        let column = ax.find(Role::TextInput, "Impedance tolerance")?.center().x;
        ax.click_at(egui::pos2(column, row.center().y));
        let preset = [
            Role::Button,
            Role::ListBoxOption,
            Role::MenuItem,
            Role::Label,
        ]
        .into_iter()
        .find_map(|role| {
            ax.all_prefix(role, "A model on this computer")
                .last()
                .map(|n| (*n).clone())
        })
        .ok_or_else(|| format!("no choice in:\n{}", ax.dump()))?;
        ax.click_at(preset.center());
        assert!(
            ax.texts.iter().any(|t| t == "NEW MODEL CONNECTION"),
            "{:?}",
            ax.texts
        );
        let local = config.join("assist").join("settings.json");
        assert!(!local.exists(), "nothing is written before Save");
        let state = page(&mut ax)?;
        let Some(Editing::Endpoint(edit)) = &mut state.editing else {
            return Err("the connection form".into());
        };
        edit.draft.model = "qwen3.8:27b".into();
        edit.key = "sk-typed-secret".into();
        ax.settle();
        assert!(
            !ax.texts.iter().any(|t| t.contains("sk-typed-secret")),
            "the key field is masked"
        );
        let harness = scroll_to(&mut ax, Role::TextInput, "Harness profile")?;
        ax.click_at(harness.center());
        ax.type_text("compact");
        // The form's Save, below Connection's.
        let save = scroll_to_nth(&mut ax, Role::Button, "Save", true)?;
        ax.click_at(save.center());
        let text = std::fs::read_to_string(&local)?;
        let settings = sparksmith_assist::settings::read(&local)?;
        let saved = settings.run_endpoint().ok_or("missing saved connection")?;
        assert_eq!(
            saved.model_profile()?.harness_profile.as_deref(),
            Some("compact")
        );
        assert!(text.contains("qwen3.8:27b"), "{text}");
        assert!(!text.contains("sk-typed"), "no key in the file: {text}");
        assert!(
            keychain_has("http://localhost:11434/v1"),
            "the key went to the keychain"
        );
        assert!(
            ax.texts
                .iter()
                .any(|t| t.starts_with("Local model — qwen3.8:27b")),
            "{:?}",
            ax.texts
        );
        test_keyring::KEYS.with(|k| k.borrow_mut().clear());
    }
    Ok(())
}

/// A page read once over `config`, as the window shows it.
fn read_page(config: &std::path::Path) -> AiPage {
    let mut page = AiPage::at(config.to_path_buf());
    let _ = page.state();
    page
}

fn local_draft() -> Fallible<EndpointSetting> {
    let shipped = Shipped::locate()?;
    let mut draft = shipped
        .endpoint_presets
        .iter()
        .find(|p| p.endpoint.endpoint().is_ok_and(|e| e.is_loopback()))
        .map(|p| p.endpoint.clone())
        .ok_or("a local choice")?;
    draft.model = "m".into();
    Ok(draft)
}

/// A key for plain HTTP to another computer is refused before it is
/// stored, leaves the form at once and is named nowhere; the endpoint is
/// not saved either.
#[test]
fn a_key_for_plain_http_to_another_computer_is_refused_and_never_kept() -> Fallible {
    test_keyring::KEYS.with(|k| k.borrow_mut().clear());
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let keyring = page.keyring;
    let state = page.state();
    let settings = state.settings.clone()?;
    let mut draft = local_draft()?;
    draft.base_url = "http://192.168.1.20:11434/v1".into();
    let mut edit = EndpointEdit {
        at: None,
        draft,
        key: "sk-typed".into(),
        stored: false,
    };
    assert!(!save_endpoint(state, &settings, keyring, &mut edit, true));
    assert!(edit.key.is_empty(), "the key leaves the form");
    let why = state.refusal.clone().unwrap_or_default();
    assert!(
        why.contains("plain HTTP") && !why.contains("sk-typed"),
        "{why}"
    );
    assert!(!stored::user_file(dir.path()).exists(), "nothing saved");
    assert!(test_keyring::KEYS.with(|k| k.borrow().is_empty()));
    // Without a key, a server on another computer is fine.
    assert!(save_endpoint(state, &settings, keyring, &mut edit, true));
    Ok(())
}

/// An agent is configuration only: saved with its arguments; a variable
/// that would carry a key is refused, naming it.
#[test]
fn an_agent_is_saved_and_a_key_variable_refused() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let state = page.state();
    let settings = state.settings.clone()?;
    let shipped = Shipped::locate()?;
    let preset = shipped
        .agent_presets
        .iter()
        .find(|p| !p.agent.args.is_empty())
        .ok_or("a preset with arguments")?;
    let mut edit = agent_edit(None, preset.agent.clone());
    edit.env = "OPENAI_API_KEY=sk-x".into();
    assert!(!save_agent(state, &settings, &mut edit));
    let why = state.refusal.clone().unwrap_or_default();
    assert!(why.contains("OPENAI_API_KEY"), "{why}");
    assert!(!stored::user_file(dir.path()).exists());
    edit.env = "OLLAMA_HOST=127.0.0.1".into();
    assert!(save_agent(state, &settings, &mut edit));
    let saved = stored::read(&stored::user_file(dir.path()))?;
    assert_eq!(saved.agents.len(), 1);
    assert_eq!(saved.agents[0].args, preset.agent.args);
    Ok(())
}

/// Limits the person sets are kept and checked; permissions left alone
/// stay unset, so a changed shipped default reaches everyone.
#[test]
fn limits_are_kept_and_checked_and_permissions_unset_stay_shipped() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let state = page.state();
    let settings = state.settings.clone()?;
    let shipped = Shipped::locate()?;
    let mut next = settings.clone();
    let mut limits = shipped.limits_start;
    limits.minutes = 90;
    next.limits = Some(limits);
    assert!(state.commit(next));
    let saved = stored::read(&stored::user_file(dir.path()))?;
    assert_eq!(saved.limits.map(|l| l.minutes), Some(90));
    assert_eq!(saved.permissions, None);
    // A limit out of bounds is refused and nothing changes.
    let mut over = saved.clone();
    limits.minutes = 0;
    over.limits = Some(limits);
    assert!(!state.commit(over));
    assert!(
        state
            .refusal
            .as_deref()
            .is_some_and(|w| w.contains("minutes"))
    );
    assert_eq!(stored::read(&stored::user_file(dir.path()))?, saved);
    Ok(())
}

/// Copy to edit by clicking: the built-in profile becomes the person's
/// file, opened as text; a broken text is refused on save.
#[test]
fn a_built_in_profile_is_copied_to_edit_and_checked_on_save() -> Fallible {
    let mut ax = settings_at(LARGE)?;
    unfold(&mut ax, "Profiles")?;
    let copy = scroll_to(&mut ax, Role::Button, "Copy to edit")?;
    ax.click_at(copy.center());
    let folder = ax.dir.path().join("config").join("assist-profiles");
    let state = page(&mut ax)?;
    let (name, _) = state.profile.clone().ok_or("the profile editor")?;
    assert!(folder.join(format!("{name}.json")).exists());
    if let Some((_, text)) = &mut state.profile {
        *text = "{".into();
    }
    ax.settle();
    assert!(
        ax.texts
            .iter()
            .any(|t| t == &format!("PROFILE {}", name.to_uppercase()))
    );
    let save = scroll_to_nth(&mut ax, Role::Button, "Save", true)?;
    ax.click_at(save.center());
    let state = page(&mut ax)?;
    assert!(state.profile.is_some(), "the editor stays open");
    assert!(
        state
            .refusal
            .as_deref()
            .is_some_and(|w| w.contains("run profile"))
    );
    Ok(())
}

#[test]
fn only_the_last_owner_of_a_normalized_endpoint_releases_its_key() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let ring = page.keyring;
    let state = page.state();
    let mut settings = state.settings.clone()?;
    let mut a = local_draft()?;
    a.base_url = "https://shared.example/v1/".into();
    let mut b = a.clone();
    b.name = "Second model".into();
    b.base_url = "https://shared.example/v1".into();
    let old = b.endpoint()?;
    (ring.store)(&old, &Secret::new("fixture-key").ok_or("missing key")?)?;
    settings.endpoints = vec![a, b.clone()];
    assert!(state.commit(settings.clone()));
    assert!(remove_endpoint(state, &settings, ring, 0));
    assert!(ring.has(&old));
    let settings = state.settings.clone()?;
    b.base_url = "https://changed.example/v1".into();
    let mut edit = EndpointEdit {
        at: Some(0),
        draft: b,
        key: String::new(),
        stored: false,
    };
    assert!(save_endpoint(state, &settings, ring, &mut edit, false));
    assert!(!ring.has(&old));
    Ok(())
}

#[test]
fn orphan_key_cleanup_failure_names_the_saved_change_and_address() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let ring = Keyring {
        delete: |_| Err("keychain is locked".into()),
        ..page.keyring
    };
    let state = page.state();
    let mut settings = state.settings.clone()?;
    let draft = local_draft()?;
    let url = draft.endpoint()?;
    settings.endpoints = vec![draft];
    assert!(state.commit(settings.clone()));
    assert!(remove_endpoint(state, &settings, ring, 0));
    let why = state.refusal.as_deref().ok_or("missing cleanup failure")?;
    assert!(
        why.contains("saved") && why.contains(url.base()) && why.contains("locked"),
        "{why}"
    );
    assert!(
        stored::read(&stored::user_file(dir.path()))?
            .endpoints
            .is_empty()
    );
    Ok(())
}

#[test]
fn after_a_keychain_refusal_save_retries_the_saved_connection() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let ring = page.keyring;
    let locked = Keyring {
        store: |_, _| Err("locked".into()),
        ..ring
    };
    let state = page.state();
    let settings = state.settings.clone()?;
    let mut draft = local_draft()?;
    draft.base_url = "https://retry.example/v1".into();
    let endpoint = draft.endpoint()?;
    let mut edit = EndpointEdit {
        at: None,
        draft: draft.clone(),
        stored: false,
        key: "inert-retry".into(),
    };
    assert!(!save_endpoint(state, &settings, locked, &mut edit, true));
    let saved = stored::read(&stored::user_file(dir.path()))?;
    assert_eq!(saved.connection.as_deref(), Some(draft.name.as_str()));
    assert_eq!(saved.endpoints, [draft.clone()]);
    assert_eq!(state.settings.as_ref().ok(), Some(&saved));
    assert_eq!(edit.at, Some(0));
    assert_eq!(edit.key, "inert-retry");
    assert!(!edit.stored && !ring.has(&endpoint));
    assert!(
        state
            .refusal
            .as_deref()
            .is_some_and(|why| why.contains("saved") && why.contains("key"))
    );
    assert!(save_endpoint(state, &saved, ring, &mut edit, true));
    assert!(edit.key.is_empty() && ring.has(&endpoint));
    assert_eq!(
        stored::read(&stored::user_file(dir.path()))?.endpoints,
        [draft]
    );
    Ok(())
}
