//! A reader of plain PDF drawings: uncompressed content streams, the
//! cross-reference table, the page tree, and on each page the text runs,
//! circles and straight strokes. What it does not know it refuses.
use crate::{Fallible, Point};
use std::collections::BTreeMap;

/// Millimetres of one PDF point.
const MM: f64 = 25.4 / 72.0;
/// Pages one drawing takes.
const MAX_PAGES: usize = 1000;

/// What one page shows, mm from its lower-left corner, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    /// Each text run and where its baseline starts.
    pub texts: Vec<(String, Point)>,
    /// Each circle drawn as four curves: centre and radius.
    pub circles: Vec<(Point, f64)>,
    /// Each straight stroke, with its pen.
    pub lines: Vec<(Point, Point, f64)>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token<'a> {
    Number(f64),
    Text(Vec<u8>),
    Word(&'a [u8]),
}

fn find(data: &[u8], what: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(what.len())
        .position(|w| w == what)
        .map(|k| from + k)
}

fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 0 | 12)
}

fn delimiter(b: u8) -> bool {
    space(b) || matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'/' | b'%')
}

/// The tokens of a content stream.
fn tokens(data: &[u8]) -> Fallible<Vec<Token<'_>>> {
    let mut out = Vec::new();
    let mut k = 0;
    while let Some(&b) = data.get(k) {
        if space(b) {
            k += 1;
            continue;
        }
        match b {
            b'(' => {
                let (text, end) = literal(data, k + 1)?;
                out.push(Token::Text(text));
                k = end;
            }
            b'<' => {
                let end = find(data, b">", k).ok_or("pdf: an unclosed hexadecimal string")?;
                let digits: Vec<u8> = data[k + 1..end]
                    .iter()
                    .copied()
                    .filter(|b| !space(*b))
                    .collect();
                let mut text = Vec::new();
                for pair in digits.chunks(2) {
                    let pair = std::str::from_utf8(pair)?;
                    let pair = if pair.len() == 1 {
                        format!("{pair}0")
                    } else {
                        pair.to_owned()
                    };
                    text.push(u8::from_str_radix(&pair, 16)?);
                }
                out.push(Token::Text(text));
                k = end + 1;
            }
            _ => {
                let start = k;
                k += 1;
                while data.get(k).is_some_and(|b| !delimiter(*b)) {
                    k += 1;
                }
                let word = &data[start..k];
                match std::str::from_utf8(word).map(str::parse::<f64>) {
                    Ok(Ok(v)) if v.is_finite() => out.push(Token::Number(v)),
                    _ => out.push(Token::Word(word)),
                }
            }
        }
    }
    Ok(out)
}

/// The literal string whose text starts at `k`, and where it ends.
fn literal(data: &[u8], mut k: usize) -> Fallible<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    let mut depth = 1;
    while let Some(&b) = data.get(k) {
        k += 1;
        match b {
            b'\\' => {
                let e = *data.get(k).ok_or("pdf: an unclosed string")?;
                k += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'0'..=b'7' => {
                        let mut v = u32::from(e - b'0');
                        for _ in 0..2 {
                            match data.get(k) {
                                Some(d @ b'0'..=b'7') => {
                                    v = v * 8 + u32::from(d - b'0');
                                    k += 1;
                                }
                                _ => break,
                            }
                        }
                        out.push(u8::try_from(v & 0xFF)?);
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(b);
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((out, k));
                }
                out.push(b);
            }
            b => out.push(b),
        }
    }
    Err("pdf: an unclosed string".into())
}

/// Text of the WinAnsi encoding.
fn win_ansi(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| match b {
            0x85 => '\u{2026}',
            0x91 => '\u{2018}',
            0x92 => '\u{2019}',
            0x93 => '\u{201C}',
            0x94 => '\u{201D}',
            0x95 => '\u{2022}',
            0x96 => '\u{2013}',
            0x97 => '\u{2014}',
            b => char::from(*b),
        })
        .collect()
}

/// A content stream's page.
fn page(stream: &[u8]) -> Fallible<Page> {
    let mut page = Page::default();
    let mut operands: Vec<f64> = Vec::new();
    let mut text: Option<Vec<u8>> = None;
    let mut pen = 0.0;
    let mut at: Point = [0.0; 2];
    // The path: its points, and how many curves it has.
    let mut path: Vec<Point> = Vec::new();
    let mut subpaths: Vec<(Vec<Point>, usize, bool)> = Vec::new();
    let mut curves = 0;
    let mm = |v: &[f64], k: usize| -> Fallible<Point> {
        match (v.get(k), v.get(k + 1)) {
            (Some(x), Some(y)) => Ok([x * MM, y * MM]),
            _ => Err("pdf: an operator lacks a number".into()),
        }
    };
    for token in tokens(stream)? {
        let word = match token {
            Token::Number(v) => {
                operands.push(v);
                continue;
            }
            Token::Text(t) => {
                text = Some(t);
                continue;
            }
            Token::Word(word) => word,
        };
        match word {
            b"w" => pen = operands.first().copied().unwrap_or(0.0) * MM,
            b"m" => {
                if !path.is_empty() {
                    subpaths.push((std::mem::take(&mut path), curves, false));
                }
                curves = 0;
                path.push(mm(&operands, 0)?);
            }
            b"l" => path.push(mm(&operands, 0)?),
            b"c" => {
                curves += 1;
                path.push(mm(&operands, 4)?);
            }
            b"h" => subpaths.push((std::mem::take(&mut path), curves, true)),
            b"S" | b"s" | b"f" | b"f*" | b"F" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                if !path.is_empty() {
                    subpaths.push((std::mem::take(&mut path), curves, false));
                }
                for (points, curves, closed) in std::mem::take(&mut subpaths) {
                    match (points.as_slice(), curves) {
                        ([a, b, c, d, e], 4) if closed && a == e => {
                            let center = [(a[0] + c[0]) / 2.0, (a[1] + c[1]) / 2.0];
                            let other = [(b[0] + d[0]) / 2.0, (b[1] + d[1]) / 2.0];
                            let radius = (a[0] - c[0]).hypot(a[1] - c[1]) / 2.0;
                            if (center[0] - other[0]).hypot(center[1] - other[1]) > 1e-3 {
                                return Err("pdf: a curve that is no circle".into());
                            }
                            page.circles.push((center, radius));
                        }
                        (points, 0) if matches!(word, b"S" | b"s") => {
                            page.lines
                                .extend(points.windows(2).map(|w| (w[0], w[1], pen)));
                            if let (true, Some(a), Some(b)) =
                                (closed, points.last(), points.first())
                            {
                                page.lines.push((*a, *b, pen));
                            }
                        }
                        (_, 0) => {}
                        _ => return Err("pdf: a curve that is no circle".into()),
                    }
                }
                curves = 0;
            }
            b"Tm" => at = mm(&operands, 4)?,
            b"Tj" => {
                let shown = text.take().ok_or("pdf: Tj without a string")?;
                page.texts.push((win_ansi(&shown), at));
            }
            b"BT" | b"ET" | b"Tf" | b"J" | b"j" | b"g" | b"G" | b"rg" | b"RG" => {}
            word if word.starts_with(b"/") => continue,
            other => {
                return Err(format!(
                    "pdf: unsupported operator {:?}",
                    String::from_utf8_lossy(other)
                )
                .into());
            }
        }
        operands.clear();
        text = None;
    }
    Ok(page)
}

/// The object `number`'s bytes between `obj` and `endobj`.
struct Objects<'a> {
    data: &'a [u8],
    at: BTreeMap<usize, usize>,
}

impl<'a> Objects<'a> {
    /// Read through the cross-reference table the file's end names.
    fn of(data: &'a [u8]) -> Fallible<(Self, &'a [u8])> {
        if !data.starts_with(b"%PDF-") {
            return Err("pdf: no PDF header".into());
        }
        let tail = data.trim_ascii_end();
        if !tail.ends_with(b"%%EOF") {
            return Err("pdf: the file does not end with %%EOF".into());
        }
        let start = data
            .windows(9)
            .rposition(|w| w == b"startxref")
            .ok_or("pdf: no startxref")?;
        let numbers = std::str::from_utf8(&data[start + 9..tail.len() - 5])?;
        let xref: usize = numbers.trim().parse()?;
        let trailer = find(data, b"trailer", xref).ok_or("pdf: no trailer")?;
        let table = data
            .get(xref..trailer)
            .and_then(|t| t.strip_prefix(b"xref"))
            .ok_or("pdf: the cross-reference table is not where it is said")?;
        let mut words = std::str::from_utf8(table)?.split_ascii_whitespace();
        let mut at = BTreeMap::new();
        while let Some(first) = words.next() {
            let first: usize = first.parse()?;
            let count: usize = words
                .next()
                .ok_or("pdf: a cross-reference table")?
                .parse()?;
            for k in 0..count {
                let offset: usize = words
                    .next()
                    .ok_or("pdf: a cross-reference table")?
                    .parse()?;
                let _generation = words.next();
                if words.next() == Some("n") {
                    at.insert(first.saturating_add(k), offset);
                }
            }
        }
        Ok((Self { data, at }, &data[trailer..start]))
    }

    fn get(&self, number: usize) -> Fallible<&'a [u8]> {
        let offset = *self
            .at
            .get(&number)
            .ok_or_else(|| format!("pdf: no object {number}"))?;
        let head = format!("{number} 0 obj");
        let body = self
            .data
            .get(offset..)
            .and_then(|d| d.strip_prefix(head.as_bytes()))
            .ok_or_else(|| format!("pdf: object {number} is not where it is said"))?;
        Ok(body)
    }

    /// The dictionary of an object (up to its stream or its end).
    fn dictionary(&self, number: usize) -> Fallible<&'a str> {
        let body = self.get(number)?;
        let end = find(body, b"endobj", 0).ok_or("pdf: an object without its end")?;
        let end = find(&body[..end], b"stream", 0).unwrap_or(end);
        Ok(std::str::from_utf8(&body[..end])?)
    }

    fn stream(&self, number: usize) -> Fallible<&'a [u8]> {
        let dictionary = self.dictionary(number)?;
        if dictionary.contains("/Filter") {
            return Err("pdf: a compressed stream".into());
        }
        let length: usize = after(dictionary, "/Length")
            .and_then(|r| r.split_ascii_whitespace().next())
            .ok_or("pdf: a stream without its length")?
            .trim_end_matches('>')
            .parse()?;
        let body = self.get(number)?;
        let start = dictionary.len() + b"stream".len();
        let start = match body.get(start..) {
            Some([b'\r', b'\n', ..]) => start + 2,
            Some([b'\n', ..]) => start + 1,
            _ => return Err("pdf: a stream".into()),
        };
        body.get(start..start.saturating_add(length))
            .filter(|_| {
                body.get(start + length..)
                    .is_some_and(|r| r.trim_ascii_start().starts_with(b"endstream"))
            })
            .ok_or_else(|| "pdf: a stream is not as long as it says".into())
    }
}

/// What follows the key `name` in a dictionary.
fn after<'a>(dictionary: &'a str, name: &str) -> Option<&'a str> {
    dictionary.match_indices(name).find_map(|(k, _)| {
        let rest = &dictionary[k + name.len()..];
        (!rest.starts_with(|c: char| c.is_ascii_alphanumeric())).then_some(rest)
    })
}

/// The object a key refers to (`/Contents 8 0 R`).
fn reference(dictionary: &str, name: &str) -> Fallible<usize> {
    let mut words = after(dictionary, name)
        .ok_or_else(|| format!("pdf: no {name}"))?
        .split_ascii_whitespace();
    Ok(words.next().ok_or("pdf: a reference")?.parse()?)
}

/// The pages of a drawing, in order.
pub fn pages(data: &[u8]) -> Fallible<Vec<Page>> {
    let (objects, trailer) = Objects::of(data)?;
    let root = reference(std::str::from_utf8(trailer)?, "/Root")?;
    let tree = reference(objects.dictionary(root)?, "/Pages")?;
    let tree = objects.dictionary(tree)?;
    let kids = after(tree, "/Kids")
        .and_then(|k| k.trim_start().strip_prefix('['))
        .and_then(|k| k.split(']').next())
        .ok_or("pdf: no pages")?;
    let kids: Vec<&str> = kids.split_ascii_whitespace().collect();
    if kids.len() / 3 > MAX_PAGES {
        return Err(format!("pdf: more than {MAX_PAGES} pages").into());
    }
    kids.chunks(3)
        .map(|kid| {
            let number: usize = kid.first().ok_or("pdf: a page")?.parse()?;
            let contents = reference(objects.dictionary(number)?, "/Contents")?;
            page(objects.stream(contents)?)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page file around a content stream.
    fn file(stream: &str) -> Vec<u8> {
        let bodies = [
            "<</Type/Catalog/Pages 2 0 R>>".to_owned(),
            "<</Type/Pages/Count 1/Kids[3 0 R]>>".to_owned(),
            "<</Type/Page/Parent 2 0 R/Contents 4 0 R>>".to_owned(),
            format!("<</Length {}>>\nstream\n{stream}\nendstream", stream.len()),
        ];
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut at = Vec::new();
        for (k, body) in bodies.iter().enumerate() {
            at.push(out.len());
            out.extend(format!("{} 0 obj\n{body}\nendobj\n", k + 1).bytes());
        }
        let xref = out.len();
        out.extend(b"xref\n0 5\n0000000000 65535 f\r\n");
        for a in at {
            out.extend(format!("{a:010} 00000 n\r\n").bytes());
        }
        out.extend(format!("trailer\n<</Size 5/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF").bytes());
        out
    }

    #[test]
    fn texts_circles_and_strokes_are_read_in_millimetres() -> Fallible<()> {
        let pages = pages(&file(
            "72 w\n0 0 m\n72 0 l\nS\nBT\n/F1 8 Tf\n1 0 0 1 144 72 Tm\n(R1 \\(x\\)) Tj\nET\n\
             172 100 m\n172 139 139 172 100 172 c\n61 172 28 139 28 100 c\n\
             28 61 61 28 100 28 c\n139 28 172 61 172 100 c\nh\nS",
        ))?;
        let [page] = pages.as_slice() else {
            return Err("one page".into());
        };
        let near = |a: Point, b: Point| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-9;
        let [(text, at)] = page.texts.as_slice() else {
            return Err("one text".into());
        };
        assert!(text == "R1 (x)" && near(*at, [50.8, 25.4]));
        let [(a, b, pen)] = page.lines.as_slice() else {
            return Err("one stroke".into());
        };
        assert!(near(*a, [0.0; 2]) && near(*b, [25.4, 0.0]) && (pen - 25.4).abs() < 1e-9);
        let [(c, r)] = page.circles.as_slice() else {
            return Err("one circle".into());
        };
        assert!((c[0] - 100.0 * MM).abs() < 1e-9 && (r - 25.4).abs() < 1e-9);
        Ok(())
    }

    #[test]
    fn a_cut_or_moved_file_is_refused() {
        let whole = file("0 0 m\n1 1 l\nS");
        assert!(pages(&whole).is_ok());
        assert!(pages(&whole[..whole.len() - 4]).is_err());
        let mut moved = whole.clone();
        moved.insert(9, b'\n');
        assert!(pages(&moved).is_err());
        assert!(pages(&file("0 0 m\n1 1 l\n5 sh")).is_err());
    }
}
