//! Sparksmith desktop editor (formerly Pcbcraft).
mod app;
mod assistant;
#[cfg(test)]
mod bundle_tests;
mod commands;
mod context_menu;
mod fab_suggestions;
mod file_ui;
mod fill_worker;
mod generator_form;
mod generator_gallery;
mod generators;
mod issue_reports;
mod issues;
mod keymap;
mod keys;
mod kit;
mod layer_view;
mod library_sources;
mod lighting;
mod menu_bar;
mod model_export;
mod model_library;
mod model_scene;
#[cfg(target_os = "macos")]
mod native_menu;
mod navigation;
mod online_library;
mod open_documents;
mod palette;
mod part_chooser;
mod part_editor;
// Wired into the Place palette by the registry slice.
mod part_preview;
mod pcb_editor;
mod pin_mux;
mod place_palette;
mod recent_projects;
mod refusal;
mod resolve_parts;
mod reveal;
mod scene;
mod schematic_editor;
mod shell;
mod symbol_view;
mod update_preview;
mod variants;
mod viewport;
mod workflow_cards;

/// Tests: the fixture context (the repository's standards and materials)
/// made current on this test's thread for the rest of the test, as the
/// app makes the shipped one current on its window's thread; and its
/// material library.
#[cfg(test)]
pub(crate) fn test_materials()
-> Result<&'static sparksmith_core::materials::MaterialLibrary, String> {
    let context =
        sparksmith_core::library::standards::Context::fixture().map_err(|e| e.to_string())?;
    std::mem::forget(context.enter());
    context.materials().map_err(|e| e.to_string())
}

/// Tests: the stack of the default stackup with the material library
/// loaded.
#[cfg(test)]
pub(crate) fn test_stack() -> Result<sparksmith_solidcraft::stack::Stack, String> {
    test_materials()?;
    sparksmith_solidcraft::stack::Stack::standard().map_err(|e| e.to_string())
}

fn main() -> eframe::Result {
    let compact = std::env::args().any(|argument| argument == "--compact");
    // Files opened from Finder arrive as AppKit events, not arguments.
    #[cfg(target_os = "macos")]
    open_documents::install();
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(if compact {
            [900.0, 680.0]
        } else {
            [1380.0, 900.0]
        })
        .with_min_inner_size([760.0, 560.0])
        .with_title_shown(false)
        .with_titlebar_shown(false)
        .with_fullsize_content_view(true);
    // As the window was left: full screen, maximized or its frame.
    let kept = (!compact).then(app::window_state::kept).flatten();
    let options = eframe::NativeOptions {
        viewport: app::window_state::restore(viewport, kept),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        sparksmith_core::product::NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)?))),
    )
}
