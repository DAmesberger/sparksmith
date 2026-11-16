//! Test fixtures, never in the product: an invented radio module, EXR-22,
//! its datasheet as a small text PDF, and the answers a model gives the
//! datasheet workflow's three judgement steps for it. The desktop's and the
//! CLI's tests run the workflow on these (feature `test-fixtures`).
use serde_json::{Value, json};

/// A valid PDF, written by hand, with one page per text and a known size
/// (612 × 792 points); a page with empty text has none.
pub fn text_pdf(pages: &[&str]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let n = pages.len();
    let kids: Vec<String> = (0..n).map(|k| format!("{} 0 R", 4 + 2 * k)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {n} >>",
        kids.join(" ")
    ));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for (k, text) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
            5 + 2 * k
        ));
        let stream = if text.is_empty() {
            // A filled square, so a drawing-only page renders something.
            "0 0 0 rg 100 100 200 200 re f".to_owned()
        } else {
            format!("BT /F1 12 Tf 72 700 Td ({text}) Tj ET")
        };
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (k, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", k + 1).as_bytes());
    }
    let xref = out.len();
    let count = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {count}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {count} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    out
}

/// EXR-22: 8 + 6 + 8 castellated pads at 1.27 mm and a 2 × 2 ground array
/// (terminal 23). Invented for the tests.
pub fn pinout() -> Value {
    let mut pins = Vec::new();
    for n in 1..=22u32 {
        let (name, functions, default) = match n {
            1 | 22 => (
                format!("GND{n}"),
                json!([{"label": "gnd", "direction": "power_input", "signal": "power"}]),
                "gnd",
            ),
            2 => (
                "VCC".to_owned(),
                json!([{"label": "vcc", "direction": "power_input", "signal": "power"}]),
                "vcc",
            ),
            _ => {
                let g = n - 3;
                let mut f = vec![
                    json!({"label": "gpio_in", "direction": "input", "signal": "digital"}),
                    json!({"label": "gpio_out", "direction": "output", "signal": "digital", "drive": "push_pull"}),
                ];
                if g == 0 {
                    f.push(json!({"label": "uart0_tx", "direction": "output", "signal": "digital", "drive": "push_pull", "peripheral": "UART0", "role": "tx", "mux": "F1"}));
                }
                (format!("IO{g}"), json!(f), "gpio_in")
            }
        };
        pins.push(
            json!({"row": format!("r{n}"), "number": {"MOD": n.to_string()}, "name": name,
            "class": "electrical", "functions": functions, "default": default,
            "source": {"page": 3, "table": "Table 1"}}),
        );
    }
    pins.push(
        json!({"row": "r23", "number": {"MOD": "23"}, "name": "EPAD", "class": "electrical",
        "functions": [{"label": "gnd", "direction": "power_input", "signal": "power"}],
        "default": "gnd", "source": {"page": 9, "table": "Figure 4"}}),
    );
    json!({
        "format": 1,
        "device": {"name": "EXR-22 radio module", "reference": {"title": "EXR-22 datasheet (fictitious)", "document_revision": "1"}},
        "variants": [{"name": "MOD", "terminal_count": 23}],
        "peripherals": [{"label": "UART0", "kind": "uart", "roles": [{"label": "tx", "signal": "TX", "required_when_used": true}]}],
        "pins": pins,
        "rules": [{"label": "io0_strap", "script": "if active(\"IO0\", \"uart0_tx\") { warn(\"IO0 selects the boot mode at reset\", [\"IO0\"]) } else { ok() }", "source": {"page": 5}}],
    })
}

pub fn module_values() -> Value {
    json!({"family": "module", "width": 14.0, "length": 20.0, "height": 2.8, "pcb_thickness": 0.8,
        "antenna_length": 5.0, "can_size": [12.0, 12.4], "can_centre": [0.0, -2.2],
        "pitch": 1.27, "pad_size": [1.0, 0.8],
        "rows": [
            {"side": "left", "count": 8, "first": [-6.5, 2.2]},
            {"side": "bottom", "count": 6, "first": [-3.175, -9.5]},
            {"side": "right", "count": 8, "first": [6.5, 2.2 - 7.0 * 1.27]},
        ],
        "castellation": 0.5, "land_width": 0.8, "land_extend": [0.4, 0.0],
        "ground": {"rows": 2, "cols": 2, "pitch": [2.0, 2.0], "size": [1.2, 1.2], "centre": [0.0, -3.0], "number": "23"},
        "keepout": {"beyond": [0.0, 0.0, 0.0]}})
}

/// EXR-22's datasheet: a pin table on page 1 and a drawing on page 2, at
/// `path`.
pub fn exr22_datasheet(path: &std::path::Path) -> std::io::Result<()> {
    let table = format!(
        "EXR-22 PIN TABLE {}",
        "PIN 1 GND PIN 2 VCC PIN 3 IO0 ".repeat(12)
    );
    std::fs::write(path, text_pdf(&[&table, ""]))
}

/// The answers to the datasheet workflow's judgement steps for EXR-22, by
/// step: the package plan, the pinout document and the review report.
pub fn exr22_answers() -> [(&'static str, Value); 3] {
    [
        (
            "plan",
            json!({"generator": "module", "params": crate::legacy::module_params(&module_values()),
                "why": "EXR-22 module, drawing on page 9",
                "pages": "exr22.pdf: pinout 1, drawing 2, land pattern 2"}),
        ),
        ("pinout", json!({"document": pinout()})),
        (
            "report",
            json!({"report": {
                "part": "EXR-22", "summary": "Synthetic module.",
                "sources": [{"name": "DS", "title": "EXR-22 datasheet", "revision": "1"}],
                "values": [{"item": "pitch", "value": "1.27 mm",
                    "source": {"document": "DS", "page": 9, "location": "Figure 4"}, "confidence": "high"}],
                "open_questions": ["Ground array optional?"]}}),
        ),
    ]
}
