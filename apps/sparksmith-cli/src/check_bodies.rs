//! `check-bodies FILE.sparksmith [--all]`: the board check's placed-body
//! rule (`board.body_off_pads`, `sparksmith_core::layout::bodies`) as a
//! report: how many placed bodies were checked, how many have a lead off
//! its pad, a histogram of each part's worst lead offset from its pad
//! centre, pin-1 agreement, and each failing part (every part with
//! `--all`) with its leads.
use serde_json::{Value, json};
use sparksmith_core::layout::bodies::{self, PinOne};
use sparksmith_core::model::Project;

/// Upper edges of the worst-offset histogram's bins, mm.
const BINS_MM: [f64; 7] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0];

/// The report as JSON.
pub fn report(project: &Project, all: bool) -> Value {
    let report = bodies::bodies(project);
    let mut histogram = vec![0usize; BINS_MM.len() + 1];
    for part in &report.parts {
        let worst = part.worst_off_centre_mm.unwrap_or(0.0);
        let bin = BINS_MM
            .iter()
            .position(|edge| worst <= *edge)
            .unwrap_or(BINS_MM.len());
        histogram[bin] += 1;
    }
    let labels: Vec<String> = BINS_MM
        .iter()
        .map(|e| format!("<= {e} mm"))
        .chain(std::iter::once(format!(
            "> {} mm",
            BINS_MM[BINS_MM.len() - 1]
        )))
        .collect();
    let pin_one =
        |want: fn(&PinOne) -> bool| report.parts.iter().filter(|p| want(&p.pin_one)).count();
    let parts: Vec<&bodies::PlacedBody> = report
        .parts
        .iter()
        .filter(|p| all || p.off_pads > 0 || matches!(p.pin_one, PinOne::Disagrees { .. }))
        .collect();
    json!({
        "checked": report.parts.len(),
        "off_pads": report.off_pads(),
        "without_leads": report.without_leads.len(),
        "envelopes": report.envelopes,
        "pin_one": {
            "agrees": pin_one(|p| matches!(p, PinOne::Agrees)),
            "disagrees": pin_one(|p| matches!(p, PinOne::Disagrees { .. })),
            "unknown": pin_one(|p| matches!(p, PinOne::Unknown)),
        },
        "worst_off_centre_histogram": labels
            .into_iter()
            .zip(histogram)
            .map(|(label, count)| json!({"bin": label, "parts": count}))
            .collect::<Vec<_>>(),
        "parts": parts,
    })
}
