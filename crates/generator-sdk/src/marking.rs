//! A package's default top marking as parameters: lines of text the
//! presets give (data), returned with the generation as its `marking`.
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use sparksmith_generator_abi::v1::{self, Param, ParamType, ParamValue, Problem, limits};

use crate::args::Args;

/// The lines parameter's key.
pub const KEY: &str = "marking";
/// The pin-1 dot parameter's key.
pub const DOT_KEY: &str = "marking_pin1_dot";

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

/// The marking the parameters ask for: the non-blank lines of `marking`
/// and, where the generator declares it, `marking_pin1_dot`; `ink` when
/// the part's marking is printed rather than laser-etched. `None` when
/// there is nothing to mark.
pub fn of(args: &Args<'_>, ink: bool) -> Result<Option<v1::Marking>, Problem> {
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
    }))
}
