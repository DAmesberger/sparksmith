//! Artwork templates as drawn: the values the core bound for one placed
//! part ([`sparksmith_core::library::marking::artwork`]) put into the SVG
//! before it is painted.
//!
//! - An element whose `data-if` condition is not bound to `"1"` is left
//!   out with everything in it, so groups show, hide or swap by the part.
//! - In `text` and `tspan` content, each `{…}` placeholder becomes its
//!   bound value (XML-escaped), or nothing when none is bound: a template
//!   never draws another part's text or its own placeholder. `{{` and `}}`
//!   are literal braces. Other text (a style sheet) is kept as written.
//! - `<use data-art="ID" x y width height/>` becomes the installed
//!   artwork `ID`, fitted into the box (centred, its aspect kept) and
//!   painted in the marking's finish (`fill="currentColor"` on its group,
//!   so a monochrome mark follows the etch or ink). Artwork that is not
//!   installed draws nothing there, quietly: the rest of the template is
//!   drawn.
//! - `<use data-code="datamatrix" data-payload="…" x y width height/>`
//!   becomes the generated 2D code of its payload (placeholders filled as
//!   in text), its dark modules a square centred in the box, painted in
//!   the marking's finish; an empty payload, or one the code cannot hold,
//!   draws nothing.
//!
//! The pass copies the source between the events it keeps, so what it
//! does not touch reaches the painter byte for byte.
use crate::{Error, Result};
use quick_xml::events::{BytesStart, Event};
use sparksmith_core::library::marking::artwork::{
    ART_ATTRIBUTE, CODE_ATTRIBUTE, CONDITION_ATTRIBUTE, PAYLOAD_ATTRIBUTE, code_symbology,
    condition_key, is_template,
};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Installed artwork by id.
pub type ArtSource<'a> = &'a dyn Fn(&str) -> Option<std::sync::Arc<str>>;

/// The artwork installed with the library (the current context's).
pub fn installed_art(id: &str) -> Option<std::sync::Arc<str>> {
    sparksmith_core::library::standards::Context::current()
        .ok()?
        .artwork(id)
        .cloned()
}

/// Further artwork one template may fit in.
const MAX_ART: usize = 8;

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(format!("marking artwork: {}", message.into()))
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        (a.key.local_name().as_ref() == key.as_bytes()).then(|| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map(|v| v.to_string())
                .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).to_string())
        })
    })
}

fn number(e: &BytesStart, key: &str) -> Option<f64> {
    attr(e, key)?
        .trim()
        .trim_end_matches("px")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

/// Whether `e` is shown: no condition, or one bound to `"1"`.
fn shown(e: &BytesStart, bound: &BTreeMap<String, String>) -> bool {
    attr(e, CONDITION_ATTRIBUTE)
        .is_none_or(|c| bound.get(&condition_key(&c)).is_some_and(|v| v == "1"))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `raw` text content with its placeholders replaced by their values.
fn substitute(raw: &str, bound: &BTreeMap<String, String>) -> String {
    fill(raw, bound, escape)
}

/// `raw` with its placeholders replaced by their values, each passed
/// through `put`.
fn fill(raw: &str, bound: &BTreeMap<String, String>, put: fn(&str) -> String) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
            continue;
        }
        // A lone closing brace is dropped.
        if let Some(after) = tail.strip_prefix('}') {
            rest = after;
            continue;
        }
        match tail.find('}') {
            Some(end) => {
                let body = tail[1..end].trim();
                if let Some(value) = bound.get(body) {
                    out.push_str(&put(value));
                }
                rest = &tail[end + 1..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// The view box and the content of an artwork's root `<svg>`.
fn art_inner(svg: &str) -> Option<([f64; 4], &str)> {
    let mut reader = quick_xml::Reader::from_str(svg);
    let mut depth = 0usize;
    let mut found: Option<([f64; 4], usize)> = None;
    loop {
        let before = usize::try_from(reader.buffer_position()).ok()?;
        let event = reader.read_event().ok()?;
        let after = usize::try_from(reader.buffer_position()).ok()?;
        match event {
            Event::Start(e) => {
                if found.is_none() {
                    if e.local_name().as_ref() != b"svg" {
                        return None;
                    }
                    let view = attr(&e, "viewBox")
                        .map(|v| {
                            v.split([' ', ','])
                                .filter(|s| !s.is_empty())
                                .filter_map(|s| s.parse::<f64>().ok())
                                .collect::<Vec<_>>()
                        })
                        .and_then(|v| <[f64; 4]>::try_from(v).ok())
                        .or_else(|| {
                            Some([0.0, 0.0, number(&e, "width")?, number(&e, "height")?])
                        })?;
                    if !view.iter().all(|v| v.is_finite()) || view[2] <= 0.0 || view[3] <= 0.0 {
                        return None;
                    }
                    found = Some((view, after));
                } else {
                    depth += 1;
                }
            }
            Event::End(_) if found.is_some() => {
                if depth == 0 {
                    let (view, start) = found?;
                    return Some((view, svg.get(start..before)?));
                }
                depth -= 1;
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// The installed artwork `use` names, fitted into its box, or nothing.
fn fitted_art(e: &BytesStart, art: ArtSource<'_>) -> Option<String> {
    let id = attr(e, ART_ATTRIBUTE)?;
    let source = art(id.trim())?;
    let (view, inner) = art_inner(&source)?;
    let (x, y) = (number(e, "x").unwrap_or(0.0), number(e, "y").unwrap_or(0.0));
    let (w, h) = (number(e, "width")?, number(e, "height")?);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let s = (w / view[2]).min(h / view[3]);
    let tx = x + (w - view[2] * s) / 2.0 - view[0] * s;
    let ty = y + (h - view[3] * s) / 2.0 - view[1] * s;
    Some(format!(
        "<g fill=\"currentColor\" transform=\"translate({tx} {ty}) scale({s})\">{inner}</g>"
    ))
}

/// Codes one template may draw.
const MAX_CODES: usize = 4;

/// The generated 2D code a `use data-code` names, as a path of its dark
/// modules (one rectangle per run of a row) centred in its box, or
/// nothing.
fn code_modules(e: &BytesStart, bound: &BTreeMap<String, String>) -> Option<String> {
    use sparksmith_core::library::marking::matrix::{Ecc, encode};
    let symbology = code_symbology(&attr(e, CODE_ATTRIBUTE)?)?;
    let payload = fill(&attr(e, PAYLOAD_ATTRIBUTE)?, bound, str::to_owned);
    let payload = payload.trim();
    if payload.is_empty() {
        return None;
    }
    let symbol = encode(symbology, payload, Ecc::Medium).ok()?;
    let (x, y) = (number(e, "x").unwrap_or(0.0), number(e, "y").unwrap_or(0.0));
    let (w, h) = (number(e, "width")?, number(e, "height")?);
    if w <= 0.0 || h <= 0.0 || symbol.size == 0 {
        return None;
    }
    let side = w.min(h);
    let module = side / f64::from(symbol.size);
    let (x0, y0) = (x + (w - side) / 2.0, y + (h - side) / 2.0);
    let mut d = String::new();
    for (row, bits) in symbol.rows.iter().enumerate() {
        let bits = bits.as_bytes();
        let mut col = 0usize;
        while col < bits.len() {
            if bits[col] != b'1' {
                col += 1;
                continue;
            }
            let start = col;
            while col < bits.len() && bits[col] == b'1' {
                col += 1;
            }
            let (left, top) = (x0 + start as f64 * module, y0 + row as f64 * module);
            let run = (col - start) as f64 * module;
            d.push_str(&format!(
                "M{left:.4} {top:.4}h{run:.4}v{module:.4}h{:.4}z",
                -run
            ));
        }
    }
    (!d.is_empty()).then(|| format!("<path fill=\"currentColor\" d=\"{d}\"/>"))
}

/// `svg` with `bound`'s values put in and installed artwork fitted, or
/// `svg` itself when it is no template (nothing bound, no further art).
pub fn apply<'s>(
    svg: &'s str,
    bound: &BTreeMap<String, String>,
    art: ArtSource<'_>,
) -> Result<Cow<'s, str>> {
    if bound.is_empty()
        && !is_template(svg)
        && !svg.contains(ART_ATTRIBUTE)
        && !svg.contains(CODE_ATTRIBUTE)
    {
        return Ok(Cow::Borrowed(svg));
    }
    let mut out = String::with_capacity(svg.len());
    let mut reader = quick_xml::Reader::from_str(svg);
    let position = |r: &quick_xml::Reader<&[u8]>| {
        usize::try_from(r.buffer_position()).map_err(|_| invalid("too large"))
    };
    // Depth inside a left-out element, and the open elements' names.
    let mut skip = 0usize;
    let mut open: Vec<Vec<u8>> = Vec::new();
    let mut fitted = 0usize;
    let mut codes = 0usize;
    loop {
        let start = position(&reader)?;
        let event = reader
            .read_event()
            .map_err(|e| invalid(format!("the SVG is not well-formed XML: {e}")))?;
        let end = position(&reader)?;
        let raw = svg.get(start..end).unwrap_or_default();
        if skip > 0 {
            match event {
                Event::Start(_) => skip += 1,
                Event::End(_) => skip -= 1,
                Event::Eof => break,
                _ => {}
            }
            continue;
        }
        match event {
            Event::Start(e) => {
                if shown(&e, bound) {
                    open.push(e.local_name().as_ref().to_vec());
                    out.push_str(raw);
                } else {
                    skip = 1;
                }
            }
            Event::Empty(e) => {
                if !shown(&e, bound) {
                    continue;
                }
                if e.local_name().as_ref() == b"use" && attr(&e, ART_ATTRIBUTE).is_some() {
                    if fitted < MAX_ART
                        && let Some(group) = fitted_art(&e, art)
                    {
                        fitted += 1;
                        out.push_str(&group);
                    }
                    continue;
                }
                if e.local_name().as_ref() == b"use" && attr(&e, CODE_ATTRIBUTE).is_some() {
                    if codes < MAX_CODES
                        && let Some(path) = code_modules(&e, bound)
                    {
                        codes += 1;
                        out.push_str(&path);
                    }
                    continue;
                }
                out.push_str(raw);
            }
            Event::End(_) => {
                open.pop();
                out.push_str(raw);
            }
            Event::Text(_)
                if open
                    .last()
                    .is_some_and(|n| matches!(n.as_slice(), b"text" | b"tspan")) =>
            {
                out.push_str(&substitute(raw, bound));
            }
            Event::Eof => break,
            _ => out.push_str(raw),
        }
    }
    Ok(Cow::Owned(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// An original synthetic label and an original synthetic mark.
    const LABEL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><style>.a{fill:#000}</style><use data-art="maker-mark" x="0" y="0" width="6" height="3"/><text x="10" y="5">{mpn} &amp; {{x}}</text><g data-if="code"><text>{code}</text></g><g data-if="!code"><rect width="1" height="1"/><g><circle r="1"/></g></g></svg>"#;
    const MARK: &str = r#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 1"><path d="M0 0H4V1H0Z"/></svg>"#;

    fn bound(code: Option<&str>) -> BTreeMap<String, String> {
        let mut b = BTreeMap::new();
        b.insert("mpn".into(), "ACME<1>".into());
        b.insert("code".into(), code.unwrap_or_default().into());
        b.insert("?code".into(), if code.is_some() { "1" } else { "" }.into());
        b.insert(
            "?!code".into(),
            if code.is_some() { "" } else { "1" }.into(),
        );
        b
    }

    fn art(id: &str) -> Option<Arc<str>> {
        (id == "maker-mark").then(|| Arc::from(MARK))
    }

    #[test]
    fn values_go_into_text_and_conditions_choose_groups() -> Result<()> {
        let with = apply(LABEL, &bound(Some("N16R8")), &art)?;
        assert!(with.contains(">ACME&lt;1&gt; &amp; {x}</text>"), "{with}");
        assert!(with.contains("<g data-if=\"code\"><text>N16R8</text></g>"));
        assert!(!with.contains("<rect"), "{with}");
        // The style sheet keeps its braces.
        assert!(with.contains(".a{fill:#000}"));
        // The mark fitted into its 6 × 3 box: scale 1.5, centred in y.
        assert!(
            with.contains("<g fill=\"currentColor\" transform=\"translate(0 0.75) scale(1.5)\"><path d=\"M0 0H4V1H0Z\"/></g>"),
            "{with}"
        );
        let without = apply(LABEL, &bound(None), &art)?;
        assert!(!without.contains("N16R8"));
        assert!(without.contains("<rect width=\"1\" height=\"1\"/><g><circle r=\"1\"/></g></g>"));
        Ok(())
    }

    #[test]
    fn missing_art_and_unbound_placeholders_draw_nothing() -> Result<()> {
        let none = |_: &str| None;
        let out = apply(LABEL, &BTreeMap::new(), &none)?;
        assert!(!out.contains("data-art"), "{out}");
        assert!(!out.contains("{mpn}"));
        // Unbound conditions do not hold: neither group is drawn.
        assert!(!out.contains("<rect") && !out.contains("<text>"), "{out}");
        Ok(())
    }

    /// A `use data-code` becomes its code's dark modules in the box, from
    /// the bound payload; an unbound or empty payload draws nothing.
    #[test]
    fn a_code_element_draws_the_bound_payloads_modules() -> Result<()> {
        use sparksmith_core::library::marking::matrix::{Ecc, Symbology, encode};
        let label = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><use data-code="datamatrix" data-payload="{var:serial}" x="2" y="4" width="16" height="12"/></svg>"#;
        let none = |_: &str| None;
        let mut bound = BTreeMap::new();
        bound.insert("var:serial".to_string(), "SN&1".to_string());
        let out = apply(label, &bound, &none)?;
        let symbol =
            encode(Symbology::DataMatrix, "SN&1", Ecc::Medium).map_err(crate::Error::Invalid)?;
        // A 12 mm square centred in the 16 × 12 box: from x 4, y 4.
        let module = 12.0 / f64::from(symbol.size);
        assert!(
            out.contains("<path fill=\"currentColor\" d=\"M4.0000 4.0000h"),
            "{out}"
        );
        // One rectangle per run of dark modules.
        let runs: usize = symbol
            .rows
            .iter()
            .map(|r| r.split('0').filter(|s| !s.is_empty()).count())
            .sum();
        assert_eq!(out.matches('z').count(), runs);
        assert!(out.contains(&format!("v{module:.4}")));
        assert!(!out.contains("data-code"));
        // No payload bound: nothing drawn.
        let empty = apply(label, &BTreeMap::new(), &none)?;
        assert!(
            !empty.contains("<path") && !empty.contains("data-code"),
            "{empty}"
        );
        Ok(())
    }

    #[test]
    fn plain_artwork_is_passed_through() -> Result<()> {
        let none = |_: &str| None;
        assert!(matches!(
            apply(MARK, &BTreeMap::new(), &none)?,
            Cow::Borrowed(_)
        ));
        Ok(())
    }
}
