//! Marking text templates: parsing and rendering.
//!
//! Grammar: literal text with `{…}` placeholders; `{{` and `}}` are literal
//! braces and a line break starts a new marking line. Placeholders:
//! `{mpn}`, `{manufacturer}`, `{designator}`, `{code}` (the top-mark code
//! the spec's code table gives the placed part's MPN), `{var:<name>}` and
//! `{field:<lineage>}` with an optional formatter
//! `{field:<lineage>|<format>}`. The lineage is written `D12` or `12`.
//! Formats: the codes `code3`, `code4`, `eia96`, `cap3` and
//! `letter:<table>`, and `num` / `num:<SI prefix>`, the field's number
//! without its unit (`{field:D3|num:µ}` writes 100 µF as `100`).
//!
//! **Alternatives and conditions.** A placeholder may list alternatives
//! separated by `;`: the first that renders is drawn (`{mpn;field:D9}`).
//! An alternative may carry a condition after `?` on a field's number:
//! `field:<lineage><=<number>` or `>=`, the number optionally in percent
//! (`{field:D7|code4?field:D8<=1%;field:D7|code3}` marks a 1 % resistor
//! with four digits and any other with three). A condition on a field
//! without a value does not hold. Nothing renders when no alternative
//! does.
use super::codes::{self, CodeScheme, TableRef};
use super::{MAX_MARKING_VAR, MarkingInputs};
use crate::ids::{DefinitionId, Id};

/// One parsed piece of a template.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Piece {
    Literal(String),
    Mpn,
    /// The code table's entry for the placed part's MPN.
    Code,
    Manufacturer,
    Designator,
    Var(String),
    Field(DefinitionId, Option<Format>),
    /// The first alternative whose condition holds and which renders.
    First(Vec<(Piece, Option<Condition>)>),
}

/// How a field's value is written.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Format {
    Code(CodeScheme),
    /// The SI value divided by `10^exponent`, shortest decimal form.
    Number {
        exponent: i32,
    },
}

/// A condition: on a field's number (`field:D8<=1%`), on a text the part
/// gives (`mpn^=ESP32-S3`), or that a text is there at all (`code`); a
/// leading `!` negates it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Condition {
    test: Test,
    negated: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Test {
    Number {
        field: DefinitionId,
        at_most: bool,
        value: f64,
    },
    /// The subject's text compared as part numbers are (upper case,
    /// spaces ignored).
    Text {
        subject: Subject,
        op: TextOp,
        text: String,
    },
    /// The subject has a text that is not blank.
    Present(Subject),
}

/// What a text condition reads.
#[derive(Clone, Debug, PartialEq)]
enum Subject {
    Mpn,
    Code,
    Manufacturer,
    Field(DefinitionId),
    Var(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TextOp {
    /// `=`
    Equals,
    /// `^=`
    Starts,
    /// `$=`
    Ends,
    /// `*=`
    Contains,
}

/// Longest literal a text condition compares with.
const MAX_CONDITION_TEXT: usize = 64;

/// Alternatives one placeholder may list.
const MAX_ALTERNATIVES: usize = 4;

/// SI prefixes a `num` format may name, with their power of ten.
const PREFIXES: [(&str, i32); 10] = [
    ("p", -12),
    ("n", -9),
    ("µ", -6),
    ("u", -6),
    ("m", -3),
    ("", 0),
    ("k", 3),
    ("M", 6),
    ("G", 9),
    ("%", -2),
];

/// A variable name: 1–32 bytes of `a-z`, `0-9` and `_`.
pub(super) fn var_name_valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_MARKING_VAR
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn lineage(text: &str) -> Option<DefinitionId> {
    let digits = text.strip_prefix('D').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<Id>().ok().map(DefinitionId)
}

fn formatter(text: &str) -> Result<Format, String> {
    if text == "num" {
        return Ok(Format::Number { exponent: 0 });
    }
    if let Some(prefix) = text.strip_prefix("num:") {
        return PREFIXES
            .iter()
            .find(|(p, _)| *p == prefix)
            .map(|(_, exponent)| Format::Number {
                exponent: *exponent,
            })
            .ok_or_else(|| format!("unknown SI prefix \"{prefix}\""));
    }
    code(text).map(Format::Code)
}

fn code(text: &str) -> Result<CodeScheme, String> {
    Ok(match text {
        "code3" => CodeScheme::Resistor3Digit,
        "code4" => CodeScheme::Resistor4Digit,
        "eia96" => CodeScheme::Eia96,
        "cap3" => CodeScheme::CapacitorPf3Digit,
        other => match other.strip_prefix("letter:") {
            Some(table) => {
                let scheme = CodeScheme::VoltageLetter {
                    table: TableRef(table.into()),
                };
                if let Some(problem) = scheme.problem() {
                    return Err(problem);
                }
                scheme
            }
            None => return Err(format!("unknown code \"{other}\"")),
        },
    })
}

fn subject(text: &str) -> Option<Subject> {
    Some(match text {
        "mpn" => Subject::Mpn,
        "code" => Subject::Code,
        "manufacturer" => Subject::Manufacturer,
        _ => {
            if let Some(name) = text.strip_prefix("var:") {
                return var_name_valid(name).then(|| Subject::Var(name.into()));
            }
            return lineage(text.strip_prefix("field:")?).map(Subject::Field);
        }
    })
}

/// Parse a condition, or say why it is not one.
pub(super) fn condition(text: &str) -> Result<Condition, String> {
    let bad = || {
        format!(
            "\"{text}\" is not a condition (field:<lineage><=<number> or >=; mpn, code, manufacturer, field:<lineage> or var:<name> with =, ^= starts, $= ends or *= contains a text, or alone: it has one; ! negates)"
        )
    };
    let (negated, rest) = match text.trim().strip_prefix('!') {
        Some(rest) => (true, rest.trim()),
        None => (false, text.trim()),
    };
    let number = |id: &str, at_most: bool, number: &str| -> Result<Test, String> {
        let field = id
            .trim()
            .strip_prefix("field:")
            .and_then(lineage)
            .ok_or_else(bad)?;
        let number = number.trim();
        let (number, scale) = match number.strip_suffix('%') {
            Some(n) => (n.trim(), 0.01),
            None => (number, 1.0),
        };
        let value = number
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(bad)?
            * scale;
        Ok(Test::Number {
            field,
            at_most,
            value,
        })
    };
    let test = if let Some((id, n)) = rest.split_once("<=") {
        number(id, true, n)?
    } else if let Some((id, n)) = rest.split_once(">=") {
        number(id, false, n)?
    } else if let Some((left, right)) = rest.split_once('=') {
        let (left, op) = if let Some(left) = left.strip_suffix('^') {
            (left, TextOp::Starts)
        } else if let Some(left) = left.strip_suffix('$') {
            (left, TextOp::Ends)
        } else if let Some(left) = left.strip_suffix('*') {
            (left, TextOp::Contains)
        } else {
            (left, TextOp::Equals)
        };
        let text = super::code_key(right);
        if text.is_empty() || text.len() > MAX_CONDITION_TEXT || text.chars().any(char::is_control)
        {
            return Err(bad());
        }
        Test::Text {
            subject: subject(left.trim()).ok_or_else(bad)?,
            op,
            text,
        }
    } else {
        Test::Present(subject(rest).ok_or_else(bad)?)
    };
    Ok(Condition { test, negated })
}

fn placeholder(body: &str) -> Result<Piece, String> {
    if body.contains(';') || body.contains('?') {
        let options: Vec<&str> = body.split(';').map(str::trim).collect();
        if options.len() > MAX_ALTERNATIVES {
            return Err(format!(
                "more than {MAX_ALTERNATIVES} alternatives in {{{body}}}"
            ));
        }
        let mut first = Vec::with_capacity(options.len());
        for option in options {
            let (what, when) = match option.split_once('?') {
                Some((what, when)) => (what.trim(), Some(condition(when.trim())?)),
                None => (option, None),
            };
            if what.is_empty() || what.contains('?') {
                return Err(format!("an empty alternative in {{{body}}}"));
            }
            first.push((single(what)?, when));
        }
        return Ok(Piece::First(first));
    }
    single(body)
}

fn single(body: &str) -> Result<Piece, String> {
    match body {
        "mpn" => return Ok(Piece::Mpn),
        "code" => return Ok(Piece::Code),
        "manufacturer" => return Ok(Piece::Manufacturer),
        "designator" => return Ok(Piece::Designator),
        _ => {}
    }
    if let Some(name) = body.strip_prefix("var:") {
        return if var_name_valid(name) {
            Ok(Piece::Var(name.into()))
        } else {
            Err(format!("\"{name}\" is not a variable name (a-z, 0-9, _)"))
        };
    }
    if let Some(rest) = body.strip_prefix("field:") {
        let (id, code) = match rest.split_once('|') {
            Some((id, code)) => (id, Some(formatter(code)?)),
            None => (rest, None),
        };
        let id = lineage(id).ok_or_else(|| format!("\"{id}\" is not a field lineage"))?;
        return Ok(Piece::Field(id, code));
    }
    Err(format!("unknown placeholder {{{body}}}"))
}

/// Parse a template, or say why it is not one.
pub(super) fn parse(template: &str) -> Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '}' => return Err("an unmatched \"}\" (write \"}}\")".into()),
            '{' => {
                let mut body = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some('{') | None => {
                            return Err("an unclosed placeholder (write \"{{\")".into());
                        }
                        Some(c) => body.push(c),
                    }
                }
                if !literal.is_empty() {
                    pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                }
                pieces.push(placeholder(body.trim())?);
            }
            '\n' => literal.push('\n'),
            c if c.is_control() => return Err("a control character".into()),
            c => literal.push(c),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Literal(literal));
    }
    Ok(pieces)
}

/// Why a template renders nothing.
#[derive(Debug, PartialEq)]
pub(super) enum Missing {
    /// A value the text needs is absent (no Part, an unset field).
    Value(String),
    /// A code cannot represent the value.
    Code(String),
}

/// Render `pieces` for `inputs`. An unset variable renders as nothing (a
/// date code not yet known); any other missing value makes the whole
/// template unrepresentable.
pub(super) fn render(pieces: &[Piece], inputs: &MarkingInputs) -> Result<String, Missing> {
    let mut out = String::new();
    for piece in pieces {
        one(piece, inputs, &mut out)?;
    }
    // Values never inject control characters into the drawn text.
    Ok(out
        .chars()
        .filter(|c| *c == '\n' || !c.is_control())
        .collect())
}

/// Whether `condition` holds for `inputs`. A condition on a value the part
/// does not have does not hold (its negation does).
pub(super) fn holds(condition: &Condition, inputs: &MarkingInputs) -> bool {
    let held = match &condition.test {
        Test::Number {
            field,
            at_most,
            value,
        } => {
            // Within a part per million of the bound: a parsed "1 %" is
            // 0.01 within binary rounding.
            let slack = value.abs() * 1e-6;
            match inputs.fields.get(field).and_then(|(_, si)| *si) {
                Some(v) if *at_most => v <= value + slack,
                Some(v) => v >= value - slack,
                None => false,
            }
        }
        Test::Text { subject, op, text } => subject_text(subject, inputs).is_some_and(|have| {
            let have = super::code_key(&have);
            match op {
                TextOp::Equals => have == *text,
                TextOp::Starts => have.starts_with(text.as_str()),
                TextOp::Ends => have.ends_with(text.as_str()),
                TextOp::Contains => have.contains(text.as_str()),
            }
        }),
        Test::Present(subject) => {
            subject_text(subject, inputs).is_some_and(|t| !t.trim().is_empty())
        }
    };
    held != condition.negated
}

fn subject_text(subject: &Subject, inputs: &MarkingInputs) -> Option<String> {
    match subject {
        Subject::Mpn => inputs.mpn.clone(),
        Subject::Manufacturer => inputs.manufacturer.clone(),
        Subject::Code => inputs
            .mpn
            .as_deref()
            .and_then(|mpn| inputs.codes.get(&super::code_key(mpn)).cloned()),
        Subject::Field(id) => inputs.fields.get(id).map(|(text, _)| text.clone()),
        Subject::Var(name) => inputs.vars.get(name).cloned(),
    }
}

/// `value / 10^exponent` in its shortest decimal form (at most six
/// significant digits): 100e-6 at µ is `100`, 4.7e-6 `4.7`.
fn number(value: f64, exponent: i32) -> Option<String> {
    let scaled = value / 10f64.powi(exponent);
    if !scaled.is_finite() || scaled.abs() >= 1e15 {
        return None;
    }
    let text = format!("{:.6}", scaled);
    let text = text.trim_end_matches('0').trim_end_matches('.');
    Some(if text == "-0" {
        "0".into()
    } else {
        text.into()
    })
}

fn one(piece: &Piece, inputs: &MarkingInputs, out: &mut String) -> Result<(), Missing> {
    match piece {
        Piece::Literal(text) => out.push_str(text),
        Piece::Designator => out.push_str(&inputs.designator),
        Piece::Mpn => out.push_str(
            inputs
                .mpn
                .as_deref()
                .ok_or_else(|| Missing::Value("no manufacturer part number".into()))?,
        ),
        Piece::Code => {
            let mpn = inputs
                .mpn
                .as_deref()
                .ok_or_else(|| Missing::Value("no manufacturer part number".into()))?;
            let code = inputs.codes.get(&super::code_key(mpn)).ok_or_else(|| {
                Missing::Value(format!("no marking code for the part number {mpn}"))
            })?;
            out.push_str(code);
        }
        Piece::Manufacturer => out.push_str(
            inputs
                .manufacturer
                .as_deref()
                .ok_or_else(|| Missing::Value("no manufacturer".into()))?,
        ),
        Piece::Var(name) => {
            if let Some(value) = inputs.vars.get(name) {
                out.push_str(value);
            }
        }
        Piece::First(options) => {
            let mut last = Missing::Value("no alternative applies".into());
            for (option, when) in options {
                if when.as_ref().is_some_and(|c| !holds(c, inputs)) {
                    continue;
                }
                let mut text = String::new();
                match one(option, inputs, &mut text) {
                    Ok(()) => {
                        out.push_str(&text);
                        return Ok(());
                    }
                    Err(missing) => last = missing,
                }
            }
            return Err(last);
        }
        Piece::Field(id, format) => {
            let (text, si) = inputs
                .fields
                .get(id)
                .ok_or_else(|| Missing::Value(format!("field {id} has no value")))?;
            match format {
                None => out.push_str(text),
                Some(Format::Number { exponent }) => {
                    let written = si
                        .and_then(|v| number(v, *exponent))
                        .ok_or_else(|| Missing::Code(format!("{text} is not a number")))?;
                    out.push_str(&written);
                }
                Some(Format::Code(scheme)) => {
                    let value = si.ok_or_else(|| {
                        Missing::Code(format!("{text} is not a number for a {}", scheme.label()))
                    })?;
                    match codes::encode(scheme, value) {
                        Ok(Some(code)) => out.push_str(&code),
                        Ok(None) => {
                            return Err(Missing::Code(format!(
                                "No {} code for {text}",
                                scheme.label()
                            )));
                        }
                        Err(error) => return Err(Missing::Code(error.to_string())),
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_every_placeholder() {
        let _context = crate::library::standards::Context::fixture().map(|c| c.enter());
        assert_eq!(
            parse("{mpn} {{x}}\n{var:date_code}{field:D7|eia96}{field:9}"),
            Ok(vec![
                Piece::Mpn,
                Piece::Literal(" {x}\n".into()),
                Piece::Var("date_code".into()),
                Piece::Field(DefinitionId(7), Some(Format::Code(CodeScheme::Eia96))),
                Piece::Field(DefinitionId(9), None),
            ])
        );
        assert!(parse("{field:3|letter:tantalum}").is_ok());
    }

    #[test]
    fn refuses_malformed_templates() {
        for bad in [
            "{mpn",
            "mpn}",
            "{part}",
            "{var:Date}",
            "{field:x}",
            "{field:3|code5}",
            "{field:3|letter:vendor}",
            "a\tb",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn renders_and_reports_what_is_missing() {
        let mut inputs = MarkingInputs {
            designator: "R1".into(),
            ..Default::default()
        };
        inputs
            .fields
            .insert(DefinitionId(7), ("10.3 kΩ".into(), Some(10.3e3)));
        let pieces = parse("{designator}:{field:7|code4} {var:lot}").unwrap_or_default();
        assert_eq!(render(&pieces, &inputs), Ok("R1:1032 ".into()));
        let eia = parse("{field:7|eia96}").unwrap_or_default();
        assert!(matches!(render(&eia, &inputs), Err(Missing::Code(_))));
        let mpn = parse("{mpn}").unwrap_or_default();
        assert!(matches!(render(&mpn, &inputs), Err(Missing::Value(_))));
    }

    fn resistor(ohms: f64, tolerance: Option<f64>) -> MarkingInputs {
        let mut inputs = MarkingInputs::default();
        inputs
            .fields
            .insert(DefinitionId(7), (format!("{ohms} Ω"), Some(ohms)));
        if let Some(t) = tolerance {
            inputs
                .fields
                .insert(DefinitionId(8), (format!("{} %", t * 100.0), Some(t)));
        }
        inputs
    }

    #[test]
    fn the_first_alternative_whose_condition_holds_renders() {
        let _context = crate::library::standards::Context::fixture().map(|c| c.enter());
        let four = parse("{field:7|code4?field:8<=1%;field:7|code3}").unwrap_or_default();
        let eia = parse("{field:7|eia96?field:8<=1%;field:7|code3}").unwrap_or_default();
        // Hand-derived: IEC 60062 digits and multiplier, EIA-96 index.
        for (ohms, tol, four_code, eia_code) in [
            (10e3, Some(0.01), Some("1002"), Some("01C")),
            (10e3, Some(0.05), Some("103"), Some("103")),
            (10e3, None, Some("103"), Some("103")),
            (4.7e3, Some(0.01), Some("4701"), Some("472")),
            (4.99e3, Some(0.01), Some("4991"), Some("68B")),
            (4.7, Some(0.05), Some("4R7"), Some("4R7")),
            (4.7, Some(0.01), Some("4R70"), Some("4R7")),
            (0.0, None, Some("0"), Some("0")),
            (100.0, Some(0.01), Some("1000"), Some("01A")),
            (220.0, Some(0.05), Some("221"), Some("221")),
            (1e6, Some(0.05), Some("105"), Some("105")),
            (1e6, Some(0.01), Some("1004"), Some("01E")),
            (49.9, Some(0.01), Some("49R9"), Some("68X")),
            (0.1, Some(0.01), Some("R100"), Some("01Z")),
            (0.47, Some(0.05), Some("R47"), Some("R47")),
            (12.4e3, Some(0.01), Some("1242"), Some("10C")),
            (4.75, Some(0.05), None, None),
        ] {
            let inputs = resistor(ohms, tol);
            assert_eq!(
                render(&four, &inputs).ok().as_deref(),
                four_code,
                "{ohms} Ω {tol:?} four-digit rule"
            );
            assert_eq!(
                render(&eia, &inputs).ok().as_deref(),
                eia_code,
                "{ohms} Ω {tol:?} EIA-96 rule"
            );
        }
        let mpn = parse("{mpn;field:7}").unwrap_or_default();
        assert_eq!(
            render(&mpn, &resistor(10.0, None)).ok().as_deref(),
            Some("10 Ω")
        );
        let with = MarkingInputs {
            mpn: Some("LM358".into()),
            ..resistor(10.0, None)
        };
        assert_eq!(render(&mpn, &with).ok().as_deref(), Some("LM358"));
    }

    /// `{code}` draws the code table's entry for the placed part's MPN
    /// (upper case, spaces ignored) and nothing for a part number it does
    /// not list or a part without one: no fake code.
    #[test]
    fn a_code_is_the_tables_entry_for_the_part_number() {
        let pieces = parse("{code}").unwrap_or_default();
        let mut codes = BTreeMap::new();
        codes.insert(super::super::code_key("LMQ66430MC3RXBRQ1"), "3MC3Q".into());
        codes.insert(super::super::code_key("LMQ66420MA5RXBRQ1"), "2MA5Q".into());
        let with = |mpn: Option<&str>| MarkingInputs {
            mpn: mpn.map(str::to_owned),
            codes: codes.clone(),
            ..Default::default()
        };
        for (mpn, code) in [
            (Some("LMQ66430MC3RXBRQ1"), Some("3MC3Q")),
            (Some("lmq66420ma5rxbrq1 "), Some("2MA5Q")),
            (Some("LMQ66430MC5RXBRQ1"), None),
            (None, None),
        ] {
            assert_eq!(render(&pieces, &with(mpn)).ok().as_deref(), code, "{mpn:?}");
        }
    }

    #[test]
    fn numbers_print_without_their_unit() {
        let pieces = parse("{field:7|num:µ} {field:8|num}V").unwrap_or_default();
        let mut inputs = MarkingInputs::default();
        for (farads, volts, text) in [
            (100e-6, 16.0, "100 16V"),
            (4.7e-6, 6.3, "4.7 6.3V"),
            (1000e-6, 35.0, "1000 35V"),
            (22e-6, 50.0, "22 50V"),
        ] {
            inputs
                .fields
                .insert(DefinitionId(7), ("x".into(), Some(farads)));
            inputs
                .fields
                .insert(DefinitionId(8), ("y".into(), Some(volts)));
            assert_eq!(render(&pieces, &inputs).ok().as_deref(), Some(text));
        }
    }

    #[test]
    fn refuses_malformed_alternatives() {
        for bad in [
            "{mpn;}",
            "{mpn?part}",
            "{mpn?mpn=}",
            "{mpn?colour^=red}",
            "{mpn?field:x<=1}",
            "{mpn?field:3<=a}",
            "{field:3|num:q}",
            "{mpn;mpn;mpn;mpn;mpn}",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
