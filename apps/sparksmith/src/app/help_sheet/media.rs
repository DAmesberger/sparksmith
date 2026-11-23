//! Decode a shipped screenshot off the UI thread, then reuse its texture.
use egui::{ColorImage, TextureHandle, Ui};
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, Receiver, TryRecvError},
};

enum Image {
    Reading(Receiver<Result<ColorImage, String>>),
    Ready(TextureHandle),
    Failed(String),
}

#[derive(Default)]
pub(super) struct Images(BTreeMap<String, Image>);

impl Images {
    pub(super) fn show(&mut self, ui: &mut Ui, file: &str, alt: &str) {
        let state = self.0.entry(file.to_owned()).or_insert_with(|| {
            let (tx, rx) = mpsc::channel();
            let ctx = ui.ctx().clone();
            let file = file.to_owned();
            match std::thread::Builder::new()
                .name("help screenshot".into())
                .spawn(move || {
                    let _ = tx.send(read(&file).map_err(|e| format!("{file}: {e}")));
                    ctx.request_repaint();
                }) {
                Ok(_) => Image::Reading(rx),
                Err(e) => Image::Failed(e.to_string()),
            }
        });
        if let Image::Reading(rx) = state {
            match rx.try_recv() {
                Ok(Ok(pixels)) => {
                    *state = Image::Ready(ui.ctx().load_texture(file, pixels, Default::default()))
                }
                Ok(Err(e)) => *state = Image::Failed(e),
                Err(TryRecvError::Disconnected) => {
                    *state = Image::Failed(format!("Could not load {file}"))
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        match state {
            Image::Ready(texture) => {
                let image = egui::Image::new(&*texture).max_width(ui.available_width());
                let response = ui.add(image);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, alt));
            }
            Image::Reading(_) => {
                ui.label(alt);
            }
            Image::Failed(e) => {
                ui.label(alt);
                crate::kit::refusal_wrapped(ui, e);
            }
        }
    }
}

fn read(file: &str) -> Result<ColorImage, Box<dyn std::error::Error>> {
    let bytes =
        ShippedLibrary::locate()?.read(Shelf::Help, &format!("media/{file}"), 16 * 1024 * 1024)?;
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode()?.into_rgba8();
    Ok(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}
