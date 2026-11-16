//! The main window's state across launches: full screen, maximized, or
//! its frame, kept per user (`UserSettings::window`) once it has settled,
//! and given to the window when the app starts again.
use super::App;
use sparksmith_io::settings::{self, UserDirs, WindowState};

/// Seconds a changed window state must hold before it is kept, so a resize
/// drag or a full-screen animation saves once, at its end.
const SETTLE_S: f64 = 1.0;

/// The kept window state, read before the window opens. Any problem with
/// the settings gives the default frame; the app reports the settings
/// problem itself once it runs.
pub(crate) fn kept() -> Option<WindowState> {
    let dirs = UserDirs::from_env().ok()?;
    settings::read_settings(&dirs.settings_path())
        .ok()?
        .window
        .filter(|w| w.check().is_ok())
}

/// `builder` opening as `window` was left: its frame, then maximized or in
/// full screen when it was.
pub(crate) fn restore(
    builder: egui::ViewportBuilder,
    window: Option<WindowState>,
) -> egui::ViewportBuilder {
    let Some(window) = window else {
        return builder;
    };
    let mut builder = builder.with_inner_size(window.size);
    if let Some(position) = window.position {
        builder = builder.with_position(position);
    }
    builder
        .with_maximized(window.maximized)
        .with_fullscreen(window.fullscreen)
}

/// The window's state from what the system reports. Full screen and
/// maximized keep `before`'s frame (the frame to return to); a minimized
/// window, or one whose frame is unknown, gives nothing new.
pub(crate) fn observed(
    info: &egui::ViewportInfo,
    before: Option<WindowState>,
) -> Option<WindowState> {
    if info.minimized == Some(true) {
        return None;
    }
    let fullscreen = info.fullscreen.unwrap_or(false);
    let maximized = info.maximized.unwrap_or(false);
    let (position, size) = match (before, fullscreen || maximized) {
        (Some(before), true) => (before.position, before.size),
        _ => {
            let inner = info.inner_rect?;
            (
                info.outer_rect.map(|r| [r.min.x, r.min.y]),
                [inner.width(), inner.height()],
            )
        }
    };
    let state = WindowState {
        fullscreen,
        maximized,
        position,
        size,
    };
    state.check().is_ok().then_some(state)
}

impl App {
    /// Keep the window's state once it has held for [`SETTLE_S`].
    pub(super) fn window_state(&mut self, ctx: &egui::Context) {
        let kept = self.sources.window();
        let Some(now) = ctx.input(|i| observed(i.viewport(), kept)) else {
            return;
        };
        if Some(now) == kept {
            self.window_seen = None;
            return;
        }
        match self.window_seen {
            Some((seen, since)) if seen == now => {
                if self.now - since >= SETTLE_S {
                    self.sources.set_window(now);
                    self.window_seen = None;
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_S));
                }
            }
            _ => {
                self.window_seen = Some((now, self.now));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(SETTLE_S));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    fn info(fullscreen: bool, at: [f32; 2], size: [f32; 2]) -> egui::ViewportInfo {
        let inner = Rect::from_min_size(pos2(at[0], at[1] + 28.0), vec2(size[0], size[1]));
        egui::ViewportInfo {
            fullscreen: Some(fullscreen),
            maximized: Some(false),
            minimized: Some(false),
            inner_rect: Some(inner),
            outer_rect: Some(Rect::from_min_max(pos2(at[0], at[1]), inner.max)),
            ..Default::default()
        }
    }

    /// E2E run 11, finding 7: a window quit in full screen opens in full
    /// screen again, and leaving it returns to the frame it had before.
    #[test]
    fn full_screen_is_kept_with_the_frame_to_return_to() {
        let framed = observed(&info(false, [100.0, 50.0], [1380.0, 900.0]), None);
        assert_eq!(
            framed,
            Some(WindowState {
                fullscreen: false,
                maximized: false,
                position: Some([100.0, 50.0]),
                size: [1380.0, 900.0],
            })
        );
        let full = observed(&info(true, [0.0, 0.0], [1920.0, 1080.0]), framed);
        assert_eq!(
            full,
            Some(WindowState {
                fullscreen: true,
                maximized: false,
                position: Some([100.0, 50.0]),
                size: [1380.0, 900.0],
            })
        );
        let builder = restore(egui::ViewportBuilder::default(), full);
        assert_eq!(builder.fullscreen, Some(true));
        assert_eq!(builder.inner_size, Some(vec2(1380.0, 900.0)));
        assert_eq!(builder.position, Some(pos2(100.0, 50.0)));
        // A minimized window says nothing new.
        let mut minimized = info(false, [0.0, 0.0], [10.0, 10.0]);
        minimized.minimized = Some(true);
        assert_eq!(observed(&minimized, full), None);
    }
}
