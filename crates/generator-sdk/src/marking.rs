//! A package's default top marking as parameters: lines of text the
//! presets give (data), returned with the generation as its `marking`.
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use sparksmith_generator_abi::v1::{self, Param, ParamType, ParamValue, Problem, limits};

use crate::args::Args;

/// The lines parameter's key.
pub const KEY: &str = "marking";
/// The pin-1 dot parameter's key.
pub const DOT_KEY: &str = "marking_pin1_dot";
/// The text height parameter's key.
pub const HEIGHT_KEY: &str = "marking_height";

/// The tallest marking text height a part may ask for: 5 mm, beyond any
/// component's print.
pub const MAX_HEIGHT_NM: i64 = 5_000_000;

/// Most characters in one line: a top mark of a few short codes.
pub const LINE_CHARS: u32 = 32;

/// The `marking` parameter: up to [`limits::MARKING_LINES`] lines, none by
/// default. Placeholders (`{mpn}`, `{field:Resistance|code3}`, …) are
/// filled per placed part by the host.
pub fn param() -> Param {
    Param {
        key: KEY.into(),
        label: "Top marking".into(),
        help: Some(
            "Lines marked on the package top, top to bottom: literal text and placeholders the host fills per placed part ({mpn}, {manufacturer}, {designator}, {field:<standard field>} with an optional code such as |code3). None by default."
                .into(),
        ),
        ty: ParamType::List {
            of: Box::new(ParamType::Text {
                max_chars: LINE_CHARS,
            }),
            max_len: limits::MARKING_LINES as u32,
        },
        default: ParamValue::List(Vec::new()),
        group: Some("marking".into()),
        context: None,
    }
}

/// The `marking_pin1_dot` parameter: a dot at the marking area's pin-1
/// corner, for packages whose moulding has no pin-1 feature.
pub fn dot_param() -> Param {
    Param {
        key: DOT_KEY.into(),
        label: "Pin-1 dot in the marking".into(),
        help: Some(
            "A dot at the pin-1 corner of the top marking, as on packages whose moulding has no pin-1 feature."
                .into(),
        ),
        ty: ParamType::Bool,
        default: ParamValue::Bool(false),
        group: Some("marking".into()),
        context: None,
    }
}

/// The `marking_height` parameter: the capital height of the marking's
/// text as the part carries it. The presets give it per package (data:
/// estimates from manufacturers' photographs and marking drawings); 0, the
/// default, fits the text to the marking area.
pub fn height_param() -> Param {
    Param {
        key: HEIGHT_KEY.into(),
        label: "Marking text height".into(),
        help: Some(
            "The capital height of the marking's text as the part carries it; a line too long for the marking area is set smaller. 0 fits the text to the area."
                .into(),
        ),
        ty: ParamType::Length {
            min_nm: 0,
            max_nm: MAX_HEIGHT_NM,
            step_nm: None,
        },
        default: ParamValue::Int(0),
        group: Some("marking".into()),
        context: None,
    }
}

/// The code table parameter's key.
pub const CODES_KEY: &str = "marking_codes";

/// The `marking_codes` parameter: the manufacturer's top-mark code of each
/// orderable part number, `MPN=CODE` per entry (a package marking table,
/// data of the presets); a line's `{code}` draws the placed part's. None
/// by default.
pub fn codes_param() -> Param {
    Param {
        key: CODES_KEY.into(),
        label: "Marking codes by part number".into(),
        help: Some(
            "The top-mark code each orderable part number carries, one MPN=CODE per entry, from the manufacturer's package marking table; {code} in a marking line draws the placed part's, and nothing for a part number not listed."
                .into(),
        ),
        ty: ParamType::List {
            of: Box::new(ParamType::Text {
                max_chars: 2 * limits::MARKING_CODE_BYTES as u32 + 1,
            }),
            max_len: limits::MARKING_CODES as u32,
        },
        default: ParamValue::List(Vec::new()),
        group: Some("marking".into()),
        context: None,
    }
}

/// The code table `marking_codes` gives: each `MPN=CODE` entry, both sides
/// trimmed. An entry without `=` or with an empty side is refused.
pub fn codes(args: &Args<'_>) -> Result<BTreeMap<String, String>, Problem> {
    let mut out = BTreeMap::new();
    for value in args.list(CODES_KEY)? {
        let ParamValue::Text(entry) = value else {
            continue;
        };
        let pair = entry
            .split_once('=')
            .map(|(mpn, code)| (mpn.trim(), code.trim()))
            .filter(|(mpn, code)| !mpn.is_empty() && !code.is_empty());
        let Some((mpn, code)) = pair else {
            return Err(Problem {
                code: "param.marking_codes".into(),
                field: Some(CODES_KEY.into()),
                message: alloc::format!(
                    "\"{entry}\" is not a marking code entry: write the part number, \"=\" and its code."
                ),
            });
        };
        out.insert(String::from(mpn), String::from(code));
    }
    Ok(out)
}

/// The text height `marking_height` asks for: `None` at 0 (fit the area).
pub fn height(args: &Args<'_>) -> Result<Option<i64>, Problem> {
    Ok(Some(args.int(HEIGHT_KEY)?).filter(|h| *h > 0))
}

/// The marking the parameters ask for: the non-blank lines of `marking`
/// at `marking_height` and, where the generator declares it,
/// `marking_pin1_dot`; `ink` when the part's marking is printed rather
/// than laser-etched. `None` when there is nothing to mark. A generator
/// calling it declares [`param`] and [`height_param`].
pub fn of(args: &Args<'_>, ink: bool) -> Result<Option<v1::Marking>, Problem> {
    let text_height = height(args)?;
    let pin1_dot = args.flag(DOT_KEY).unwrap_or(false);
    let lines: Vec<String> = args
        .list(KEY)?
        .iter()
        .filter_map(|v| match v {
            ParamValue::Text(t) if !t.trim().is_empty() => Some(String::from(t.trim())),
            _ => None,
        })
        .collect();
    Ok((!lines.is_empty() || pin1_dot).then_some(v1::Marking {
        lines,
        ink,
        pin1_dot,
        text_height,
        ..v1::Marking::default()
    }))
}

/// The print on an aluminium electrolytic's dark sleeve: a pale grey ink
/// (est.: the common light print on a black or dark blue sleeve).
pub const SLEEVE_INK: [u8; 3] = [0xd8, 0xdc, 0xe0];

/// The print on an aluminium can's top: black ink (est.).
pub const CAN_TOP_INK: [u8; 3] = [0x18, 0x18, 0x1a];

/// The angle a negative stripe spans round its sleeve, millidegrees (est.:
/// sleeves print it over a fifth to a quarter of the circumference).
pub const STRIPE_SPAN_MDEG: i64 = 80_000;

/// The angle the value text spans round the sleeve, centred opposite the
/// stripe, millidegrees (est.).
pub const SLEEVE_TEXT_SPAN_MDEG: i64 = 160_000;

/// An aluminium electrolytic's value lines: the capacitance in µF without
/// its unit and the rated voltage (`100` over `16V`), from each placed
/// part's Capacitance and Rated voltage.
pub fn electrolytic_lines() -> Vec<String> {
    alloc::vec![
        String::from("{field:Capacitance|num:µ}"),
        String::from("{field:Rated voltage|num}V"),
    ]
}

/// The negative stripe as SVG artwork `width_nm` round by `height_nm`
/// along: a pale band with a column of minus signs left open in it (the
/// sleeve shows through), each two fifths of the band wide, one every
/// three fifths of its width (est., as sleeves print them).
pub fn stripe_svg(width_nm: i64, height_nm: i64) -> String {
    // In micrometres.
    let (w, h) = ((width_nm / 1_000).max(1), (height_nm / 1_000).max(1));
    let [r, g, b] = SLEEVE_INK;
    let mut path = alloc::format!("M0 0H{w}V{h}H0Z");
    let (sign_w, sign_t, pitch) = (w * 2 / 5, (w / 12).max(1), (w * 3 / 5).max(1));
    let count = (h / pitch).max(1);
    let top = (h - count * pitch) / 2;
    for k in 0..count {
        let cy = top + k * pitch + pitch / 2;
        let (x0, y0) = ((w - sign_w) / 2, cy - sign_t / 2);
        path.push_str(&alloc::format!(
            "M{x0} {y0}H{}V{}H{x0}Z",
            x0 + sign_w,
            y0 + sign_t
        ));
    }
    alloc::format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\"><path fill=\"#{r:02x}{g:02x}{b:02x}\" fill-rule=\"evenodd\" d=\"{path}\"/></svg>"
    )
}

/// A polarised can's sleeve marking on the part file's `sleeve` cylinder
/// of `radius_nm`, `along_nm` long: the value lines round the side
/// opposite the negative lead, and the negative stripe facing it (angle 0)
/// as SVG artwork; the value lines at `text_height` ([`height`]).
pub fn electrolytic_sleeve(radius_nm: i64, along_nm: i64, text_height: Option<i64>) -> v1::Marking {
    // The stripe's arc: radius × span in radians (π / 180 000 per
    // millidegree, as 3 141 593 / 180 000 000 000).
    let round = radius_nm.saturating_mul(STRIPE_SPAN_MDEG) / 1_000 * 3_141_593 / 180_000_000;
    // The host keeps a twentieth of the region's smaller side clear round
    // its edge (the marking area margin); the stripe fills the rest.
    let margin = round.min(along_nm) / 20 + 1_000;
    let (width, height) = (round - 2 * margin, along_nm - 2 * margin);
    let stripe = v1::Marking {
        lines: Vec::new(),
        colour: Some(SLEEVE_INK),
        surface: Some(String::from("sleeve")),
        wrap: Some([-STRIPE_SPAN_MDEG / 2, STRIPE_SPAN_MDEG]),
        artwork: alloc::vec![v1::Artwork {
            svg: stripe_svg(width, height),
            size: [width.max(1), height.max(1)],
            at: v1::MarkPlace::Centre,
        }],
        ..v1::Marking::default()
    };
    v1::Marking {
        lines: electrolytic_lines(),
        text_height,
        colour: Some(SLEEVE_INK),
        surface: Some(String::from("sleeve")),
        wrap: Some([180_000 - SLEEVE_TEXT_SPAN_MDEG / 2, SLEEVE_TEXT_SPAN_MDEG]),
        decals: alloc::vec![stripe],
        ..v1::Marking::default()
    }
}
