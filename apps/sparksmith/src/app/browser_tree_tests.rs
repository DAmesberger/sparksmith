//! The Browser's Components folder grouped by category, on the whole app
//! at both window sizes: an imported-board-like project (invented parts
//! that carry nothing but their designator prefix) lands in the category
//! tree of `catalog/categories.json` with counts, folds are remembered, a
//! filter opens the categories that hold its matches, and Settings ›
//! Browser turns the grouping off.
use crate::app::ax_harness::{Ax, Fallible};
use crate::shell::Workspace;
use egui::Role;
use sparksmith_core::Engine;

const SIZES: [egui::Vec2; 2] = [egui::vec2(1024.0, 640.0), egui::vec2(1920.0, 1080.0)];

/// Invented parts named as an imported board names them, with the
/// designator prefixes such a board uses; the last one's prefix maps to
/// no category.
const PARTS: [(&str, &str); 12] = [
    ("R_0603_THICK", "R"),
    ("C_0805_X7R", "C"),
    ("J_HDR_1x02", "J"),
    ("R_0402_THICK", "R"),
    ("L_4030_POWER", "L"),
    ("SW_TACT_6MM", "SW"),
    ("U_LDO_SOT23", "U"),
    ("U_MCU_QFN48", "U"),
    ("D_SMA_RECT", "D"),
    ("H_M3_HOLE", "H"),
    ("LS_BUZZER_12MM", "LS"),
    ("ZQ_ODDITY", "ZQ"),
];

fn imported() -> Fallible<Engine> {
    let mut project = sparksmith_core::catalog::synthetic::project(
        PARTS.len(),
        &sparksmith_core::NewBoard::fixture(),
    )?;
    let mut components: Vec<_> = project.library.components.iter().map(|c| c.id).collect();
    components.sort();
    for (id, (name, prefix)) in components.iter().zip(PARTS) {
        for c in project
            .library
            .components
            .iter_mut()
            .filter(|c| c.id == *id)
        {
            c.meta.name = name.into();
            c.definition.designator_prefix = prefix.into();
        }
    }
    Ok(Engine::from_file(sparksmith_core::format::EditorFile {
        version: sparksmith_core::format::FILE_VERSION,
        project,
        drafts: Default::default(),
    })?)
}

fn browser(size: egui::Vec2) -> Fallible<Ax> {
    let mut ax = Ax::with_engine(imported()?, Workspace::Schematic)?;
    ax.window = size;
    ax.app.browser_visible = true;
    ax.settle();
    Ok(ax)
}

/// The dim count painted on `row`'s line.
fn count(ax: &Ax, row: &str) -> Fallible<String> {
    let node = ax.find(Role::Button, row)?;
    ax.seen
        .iter()
        .find(|(text, rect)| {
            text != row
                && !text.is_empty()
                && text.chars().all(|c| c.is_ascii_digit())
                && rect.center().y > node.rect.top()
                && rect.center().y < node.rect.bottom()
                && rect.left() > node.rect.center().x
                && rect.right() <= node.rect.right()
        })
        .map(|(text, _)| text.clone())
        .ok_or_else(|| format!("no count on {row}: {}", ax.dump()).into())
}

fn shown(ax: &Ax, row: &str) -> bool {
    ax.has(Role::Button, row)
}

#[test]
fn components_are_grouped_by_category_with_counts() -> Fallible {
    for size in SIZES {
        let mut ax = browser(size)?;
        // Each top-level category that holds a part, with its count;
        // categories start folded, Other last.
        for (row, n) in [
            ("Passives", "3"),
            ("Magnetics", "1"),
            ("Discretes", "1"),
            ("ICs", "2"),
            ("Electromechanical", "2"),
            ("Connectors and cables", "1"),
            ("Board features", "1"),
            ("Other", "1"),
        ] {
            assert_eq!(count(&ax, row)?, n, "{row} at {size:?}");
        }
        for empty in ["Sensors", "Protection", "Optoelectronics"] {
            assert!(!shown(&ax, empty), "{empty} holds nothing");
        }
        assert!(!shown(&ax, "R_0603_THICK"), "folded by default");
        let passives = ax.find(Role::Button, "Passives")?.rect;
        let other = ax.find(Role::Button, "Other")?.rect;
        assert!(other.top() > passives.top(), "Other is last");

        // Opening Passives shows its subcategories, then Resistors its
        // parts; the folds are remembered from frame to frame.
        ax.click(Role::Button, "Passives")?;
        assert_eq!(count(&ax, "Resistors")?, "2");
        assert_eq!(count(&ax, "Capacitors")?, "1");
        ax.click(Role::Button, "Resistors")?;
        ax.settle();
        assert!(shown(&ax, "R_0603_THICK") && shown(&ax, "R_0402_THICK"));
        assert!(!shown(&ax, "C_0805_X7R"), "Capacitors stays folded");
        // Folding Passives again hides everything under it; opening it
        // shows Resistors still open.
        ax.click(Role::Button, "Passives")?;
        assert!(!shown(&ax, "Resistors") && !shown(&ax, "R_0603_THICK"));
        ax.click(Role::Button, "Passives")?;
        assert!(shown(&ax, "R_0603_THICK"), "the Resistors fold is kept");
        ax.click(Role::Button, "Passives")?;
        // A prefix the data does not map is filed under Other.
        ax.click(Role::Button, "Other")?;
        assert!(shown(&ax, "ZQ_ODDITY"));
        ax.click(Role::Button, "Other")?;
        // The ICs have no subcategory from a prefix: listed in ICs itself.
        ax.click(Role::Button, "ICs")?;
        assert!(shown(&ax, "U_LDO_SOT23") && shown(&ax, "U_MCU_QFN48"));
    }
    Ok(())
}

#[test]
fn a_filter_opens_the_categories_that_hold_its_matches() -> Fallible {
    for size in SIZES {
        let mut ax = browser(size)?;
        ax.ctx.data_mut(|d| {
            d.insert_temp(
                egui::Id::unique("browser-component-filter"),
                "BUZZER".to_owned(),
            )
        });
        ax.settle();
        // Folded categories open for the match, and only categories that
        // hold one are listed.
        assert_eq!(count(&ax, "Electromechanical")?, "1");
        assert_eq!(count(&ax, "Buzzers and speakers")?, "1");
        assert!(shown(&ax, "LS_BUZZER_12MM"), "{}", ax.dump());
        for gone in ["Passives", "ICs", "Other"] {
            assert!(!shown(&ax, gone), "{gone} holds no match");
        }
        // Cleared, the categories fold back as they were.
        ax.ctx.data_mut(|d| {
            d.insert_temp(egui::Id::unique("browser-component-filter"), String::new())
        });
        ax.settle();
        assert!(shown(&ax, "Passives"));
        assert!(!shown(&ax, "LS_BUZZER_12MM"));
    }
    Ok(())
}

#[test]
fn settings_browser_lists_components_flat() -> Fallible {
    for size in SIZES {
        let mut ax = browser(size)?;
        ax.app.settings_window.open();
        ax.settle();
        let check = crate::app::settings_ai::tests::scroll_to(
            &mut ax,
            Role::CheckBox,
            "Group components by category",
        )?;
        assert_eq!(check.toggled, Some(true), "on by default");
        ax.click_at(check.center());
        ax.settle();
        assert!(ax.app.sources.list_settings().flat_components);
        ax.app.settings_window.close();
        ax.settle();
        // Flat, as before: every component at once, no category rows.
        for (name, _) in &PARTS[..6] {
            assert!(shown(&ax, name), "{name} at {size:?}");
        }
        assert!(!shown(&ax, "Passives"));
    }
    Ok(())
}

/// Category and component rows draw their category's icon, as the data
/// names it, each with a master of its own; a part no category takes
/// shows Other's.
#[test]
fn rows_show_their_categorys_icon() -> Fallible {
    use sparksmith_ui::icon_svg::audit;
    for size in SIZES {
        let mut ax = browser(size)?;
        ax.click(Role::Button, "Other")?;
        audit::start();
        ax.settle();
        let drawn = audit::finish();
        assert!(shown(&ax, "ZQ_ODDITY"), "{}", ax.dump());
        for icon in [
            "part_resistor",
            "part_inductor",
            "part_diode",
            "part_ic",
            "part_switch",
            "part_connector",
            "part_fiducial",
            "part_other",
        ] {
            // The rows asked for it, and the shipped set (which the test
            // contexts do not install) draws it from its own master.
            assert!(
                drawn.iter().any(|r| r.name == icon),
                "{icon} at {size:?}: {drawn:?}"
            );
            let master = sparksmith_ui::icon_svg::resolve(icon);
            assert!(!master.placeholder && master.master == icon, "{icon}");
        }
    }
    Ok(())
}
