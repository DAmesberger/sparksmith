//! Preflight for a live native-export question, on the board held by the
//! approval. A draft model may still be exported with production findings.
use crate::kit;
use sparksmith_automation::live::NativeExportReview;
use sparksmith_core::fabrication::PreflightReport;
use std::sync::{Arc, mpsc};

type Checked = Result<PreflightReport, String>;

pub(super) enum Preflight {
    Checking(mpsc::Receiver<Checked>),
    Done(Checked),
}

impl Preflight {
    pub fn start(review: Arc<NativeExportReview>, ctx: &egui::Context) -> Self {
        let (send, receive) = mpsc::channel();
        let wake = ctx.clone();
        match std::thread::Builder::new()
            .name("live-export-preflight".into())
            .spawn(move || {
                let report = review
                    .preflight()
                    .map_err(|e| crate::refusal::shown_error(&e));
                let _ = send.send(report);
                wake.request_repaint();
            }) {
            Ok(_) => Self::Checking(receive),
            Err(error) => Self::Done(Err(error.to_string())),
        }
    }

    pub fn finished(&self) -> bool {
        matches!(self, Self::Done(_))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, review: &NativeExportReview) {
        if let Self::Checking(receive) = self {
            match receive.try_recv() {
                Ok(report) => *self = Self::Done(report),
                Err(mpsc::TryRecvError::Disconnected) => {
                    *self = Self::Done(Err("The production check stopped unexpectedly.".into()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        kit::value_row(ui, "Board revision", &review.revision().to_string());
        kit::value_row(
            ui,
            "Geometry tier",
            crate::model_export::tier_name(review.params().tier),
        );
        if review.params().write_anyway {
            kit::value_row(ui, "Geometry check", "Write even if geometry differs");
        }
        match self {
            Self::Checking(_) => kit::value_row(ui, "Preflight", "Checking this board…"),
            Self::Done(Err(reason)) => {
                kit::value_row(ui, "Preflight", &format!("Unavailable. {reason}"));
            }
            Self::Done(Ok(report)) => {
                kit::value_row(ui, "Preflight", &crate::pcb_editor::report_summary(report));
                if !report.items.is_empty() {
                    kit::table(
                        ui,
                        &[kit::TableColumn {
                            title: "Findings",
                            weight: 1.0,
                        }],
                        report.items.len(),
                        |ui, row, _| {
                            let Some(item) = report.items.get(row) else {
                                return;
                            };
                            if item.waiver.is_some() {
                                kit::summary_row(ui, &format!("Waived · {}", item.title), None);
                            } else {
                                kit::summary_row(ui, &item.title, None);
                            }
                        },
                        None,
                    );
                }
            }
        }
    }
}
