//! Settings › AI, headless: the whole app driven through its
//! accessibility tree at a small and a large window, and the page's
//! saves on their own. No test reaches the system keychain: the page's
//! keyring is [`test_keyring::MEMORY`] under test.
use super::*;
use crate::app::ax_harness::{Ax, Fallible};
use crate::shell::Workspace;
use egui::accesskit::Role;

const SMALL: egui::Vec2 = egui::vec2(1024.0, 640.0);
const LARGE: egui::Vec2 = egui::vec2(1920.0, 1080.0);

/// The app at `size` with Settings open, settled.
fn settings_at(size: egui::Vec2) -> Fallible<Ax> {
    let mut ax = Ax::new(Workspace::Schematic)?;
    ax.window = size;
    ax.app.settings_window.toggle();
    ax.settle();
    Ok(ax)
}

/// Scroll the Settings body until the node of `role` named `name` is well
/// inside the window; the node.
fn scroll_to(ax: &mut Ax, role: Role, name: &str) -> Fallible<crate::app::ax_harness::Node> {
    for _ in 0..80 {
        let node = ax
            .all(role, name)
            .first()
            .map(|n| (*n).clone())
            .ok_or_else(|| format!("no {role:?} {name:?} in:\n{}", ax.dump()))?;
        let room = ax.window.y;
        if node.rect.top() > 60.0 && node.rect.bottom() < room - 60.0 {
            return Ok(node);
        }
        let down = node.rect.bottom() >= room - 60.0;
        let at = egui::pos2(ax.window.x / 2.0, room / 2.0);
        ax.frame(vec![
            egui::Event::PointerMoved(at),
            egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, if down { -120.0 } else { 120.0 }),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        ax.settle();
    }
    Err(format!("{name:?} never came into view").into())
}

fn page(ax: &mut Ax) -> Fallible<&mut State> {
    ax.app
        .settings_window
        .ai
        .state
        .as_mut()
        .ok_or_else(|| "the page was not read".into())
}

/// Every part of Settings › AI shows at both sizes, nothing runs off the
/// window's side, and the shipped workflow is listed with its availability.
#[test]
fn the_page_shows_every_part_at_both_sizes() -> Fallible {
    for size in [SMALL, LARGE] {
        let mut ax = settings_at(size)?;
        for caption in [
            "AI",
            "CONNECTIONS",
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
        for label in [
            "Edit the design",
            "Read or write files",
            "Publish to a library",
            "Move my view",
            "Model requests",
            "Time",
        ] {
            assert!(
                ax.has(Role::Label, label),
                "{label} at {size:?}:\n{}",
                ax.dump()
            );
        }
        // The choices for asking, and no per-turn caps.
        assert!(ax.texts.iter().any(|t| t == "Always allow"));
        assert!(ax.texts.iter().any(|t| t == "Ask every time"));
        assert!(!ax.has(Role::Label, "Edits") && !ax.has(Role::Label, "Calls"));
        // The run limits start from the shipped values (45 minutes, Q7).
        let state = page(&mut ax)?;
        let shown = match (&state.settings, &state.shipped) {
            (Ok(s), Ok(shipped)) => s.limits_or(shipped),
            _ => return Err("the page read its settings".into()),
        };
        assert_eq!(shown.minutes, 45);
        // The shipped datasheet workflow, with where it comes from and
        // whether it can run here (or why not); its title once it loads,
        // else its folder's name.
        assert!(
            ax.texts.iter().any(|t| {
                t.starts_with("Datasheet → component v") || t == "datasheet-to-component"
            }),
            "{:?}",
            ax.texts
        );
        assert!(
            ax.texts.iter().any(|t| t.starts_with("Shipped · available")
                || t.starts_with("Shipped · unavailable: ")),
            "{:?}",
            ax.texts
        );
        // Built-in profiles, each offered to copy.
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

/// Adding an endpoint from a preset by clicking: the form opens with the
/// preset's values, a typed key is masked, Save writes the endpoint (never
/// the key) and the key goes to the keychain only.
#[test]
fn an_endpoint_is_added_from_a_preset_and_its_key_kept_out_of_files() -> Fallible {
    test_keyring::KEYS.with(|k| k.borrow_mut().clear());
    for size in [LARGE, SMALL] {
        let mut ax = settings_at(size)?;
        let config = ax.dir.path().join("config");
        // Add endpoint's preset menu sits in the control column of its row,
        // level with the label, where the number fields are.
        let row = scroll_to(&mut ax, Role::Label, "Add endpoint")?;
        let column = ax.find(Role::TextInput, "Model requests")?.center().x;
        ax.click_at(egui::pos2(column, row.center().y));
        let preset = [
            Role::Button,
            Role::ListBoxOption,
            Role::MenuItem,
            Role::Label,
        ]
        .into_iter()
        .find_map(|role| {
            ax.all_prefix(role, "OpenAI-compatible")
                .first()
                .map(|n| (*n).clone())
        })
        .ok_or_else(|| format!("no preset in:\n{}", ax.dump()))?;
        ax.click_at(preset.center());
        assert!(
            ax.texts.iter().any(|t| t == "NEW ENDPOINT"),
            "{:?}",
            ax.texts
        );
        let local = config.join("assist").join("settings.json");
        assert!(!local.exists(), "nothing is written before Save");
        // Type a model and a key, then save.
        let state = page(&mut ax)?;
        let Some(Editing::Endpoint(edit)) = &mut state.editing else {
            return Err("the endpoint form".into());
        };
        edit.draft.model = "qwen3.8:27b".into();
        edit.key = "sk-typed-secret".into();
        ax.settle();
        assert!(
            !ax.texts.iter().any(|t| t.contains("sk-typed-secret")),
            "the key field is masked"
        );
        let save = scroll_to(&mut ax, Role::Button, "Save")?;
        ax.click_at(save.center());
        let text = std::fs::read_to_string(&local)?;
        assert!(text.contains("qwen3.8:27b"), "{text}");
        assert!(!text.contains("sk-typed"), "no key in the file: {text}");
        assert!(
            test_keyring::KEYS.with(|k| k.borrow().contains_key("http://localhost:11434/v1")),
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
        .find(|p| p.endpoint.provider == Wire::Openai)
        .map(|p| p.endpoint.clone())
        .ok_or("an OpenAI-compatible preset")?;
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
    assert!(!save_endpoint(state, &settings, keyring, &mut edit));
    assert!(edit.key.is_empty(), "the key leaves the form");
    let why = state.refusal.clone().unwrap_or_default();
    assert!(
        why.contains("plain HTTP") && !why.contains("sk-typed"),
        "{why}"
    );
    assert!(!stored::user_file(dir.path()).exists(), "nothing saved");
    assert!(test_keyring::KEYS.with(|k| k.borrow().is_empty()));
    // Without a key, a server on another computer is fine.
    assert!(save_endpoint(state, &settings, keyring, &mut edit));
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

/// Limits and permissions store only what differs from the shipped
/// values, so a changed default reaches everyone who kept it.
#[test]
fn limits_and_permissions_keep_only_what_differs() -> Fallible {
    let dir = tempfile::tempdir()?;
    let mut page = read_page(dir.path());
    let state = page.state();
    let settings = state.settings.clone()?;
    let shipped = Shipped::locate()?;
    let mut next = settings.clone();
    let mut limits = shipped.limits;
    limits.minutes = 90;
    next.limits = Some(limits);
    assert!(state.commit(next));
    let saved = stored::read(&stored::user_file(dir.path()))?;
    assert_eq!(saved.limits_or(&shipped).minutes, 90);
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
    let save = scroll_to(&mut ax, Role::Button, "Save")?;
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
