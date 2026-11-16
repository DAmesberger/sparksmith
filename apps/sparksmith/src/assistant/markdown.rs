//! The bounded Markdown a reply is shown in (`docs/assistant-panel.md`
//! §7): headings, paragraphs, bullet and numbered lists, fenced code, rules,
//! and inline `code`, **bold** and *italic*. Anything else is its text.
//! A reply past [`MAX_CHARS`] or [`MAX_BLOCKS`] is cut, saying so; a model
//! never makes the chat lay out an unbounded text.

/// Characters of one reply laid out.
pub const MAX_CHARS: usize = 20_000;
/// Blocks of one reply laid out.
pub const MAX_BLOCKS: usize = 400;

/// A run of text with one look.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

/// One block of a reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// `#` to `######`: the level, 1 to 6.
    Heading(u8, Vec<Span>),
    Paragraph(Vec<Span>),
    /// A list item: its marker ("•" or "3.") and how deep it is indented.
    Item {
        depth: u8,
        marker: String,
        spans: Vec<Span>,
    },
    /// A fenced code block, as written.
    Code(String),
    Rule,
}

/// The blocks of `text`, and whether it was cut at a bound.
pub fn parse(text: &str) -> (Vec<Block>, bool) {
    let mut cut = false;
    let text = if text.len() > MAX_CHARS {
        cut = true;
        let mut end = MAX_CHARS;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        &text[..end]
    } else {
        text
    };
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    let mut code: Option<Vec<&str>> = None;
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(spans(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    for line in text.lines() {
        if let Some(lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                blocks.push(Block::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut paragraph, &mut blocks);
            code = Some(Vec::new());
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut paragraph, &mut blocks);
            continue;
        }
        if let Some((level, rest)) = heading(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Heading(level, spans(rest)));
            continue;
        }
        if trimmed.len() >= 3 && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_') {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let depth = u8::try_from((indent / 2).min(4)).unwrap_or(4);
        if let Some(rest) = ["- ", "* ", "+ "]
            .iter()
            .find_map(|m| trimmed.strip_prefix(m))
        {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item {
                depth,
                marker: "•".into(),
                spans: spans(rest),
            });
            continue;
        }
        if let Some((number, rest)) = numbered(trimmed) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item {
                depth,
                marker: format!("{number}."),
                spans: spans(rest),
            });
            continue;
        }
        paragraph.push(trimmed);
        if blocks.len() >= MAX_BLOCKS {
            break;
        }
    }
    if let Some(lines) = code {
        blocks.push(Block::Code(lines.join("\n")));
    }
    flush(&mut paragraph, &mut blocks);
    if blocks.len() > MAX_BLOCKS {
        blocks.truncate(MAX_BLOCKS);
        cut = true;
    }
    (blocks, cut)
}

fn heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    let rest = line.get(hashes..)?;
    (1..=6)
        .contains(&hashes)
        .then_some(())
        .and(rest.strip_prefix(' '))
        .map(|rest| (u8::try_from(hashes).unwrap_or(6), rest.trim()))
}

fn numbered(line: &str) -> Option<(u32, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 4 {
        return None;
    }
    let rest = line.get(digits..)?;
    let rest = rest
        .strip_prefix(". ")
        .or_else(|| rest.strip_prefix(") "))?;
    Some((line.get(..digits)?.parse().ok()?, rest))
}

/// Inline spans: `code`, **bold**, *italic* and _italic_. An unclosed
/// marker is its text.
pub fn spans(text: &str) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut current = Span::default();
    let mut bold = false;
    let mut italic = false;
    let mut rest = text;
    let push = |out: &mut Vec<Span>, current: &mut Span, bold: bool, italic: bool| {
        if !current.text.is_empty() {
            out.push(std::mem::take(current));
        }
        current.bold = bold;
        current.italic = italic;
    };
    while let Some(c) = rest.chars().next() {
        if c == '`'
            && let Some(end) = rest[1..].find('`')
        {
            push(&mut out, &mut current, bold, italic);
            out.push(Span {
                text: rest[1..1 + end].to_owned(),
                code: true,
                ..Span::default()
            });
            rest = &rest[end + 2..];
            continue;
        }
        if rest.starts_with("**") && (bold || rest[2..].contains("**")) {
            push(&mut out, &mut current, !bold, italic);
            bold = !bold;
            current.bold = bold;
            rest = &rest[2..];
            continue;
        }
        if (c == '*' || c == '_')
            && (italic || rest[1..].contains(c))
            && !(c == '_' && !italic && current.text.ends_with(char::is_alphanumeric))
        {
            push(&mut out, &mut current, bold, !italic);
            italic = !italic;
            current.italic = italic;
            rest = &rest[1..];
            continue;
        }
        current.text.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if !current.text.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span {
            text: text.into(),
            ..Span::default()
        }
    }

    #[test]
    fn blocks_are_read_as_written() {
        let (blocks, cut) = parse(
            "# Board\nTwo vias were\nadded.\n\n- U1 moved\n  - by 1 mm\n2. Done\n```\nquery x\n```\n---",
        );
        assert!(!cut);
        assert_eq!(
            blocks,
            [
                Block::Heading(1, vec![plain("Board")]),
                Block::Paragraph(vec![plain("Two vias were added.")]),
                Block::Item {
                    depth: 0,
                    marker: "•".into(),
                    spans: vec![plain("U1 moved")]
                },
                Block::Item {
                    depth: 1,
                    marker: "•".into(),
                    spans: vec![plain("by 1 mm")]
                },
                Block::Item {
                    depth: 0,
                    marker: "2.".into(),
                    spans: vec![plain("Done")]
                },
                Block::Code("query x".into()),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn inline_marks_and_unclosed_markers() {
        assert_eq!(
            spans("Net **GND** has `3` _open_ ends"),
            [
                plain("Net "),
                Span {
                    text: "GND".into(),
                    bold: true,
                    ..Span::default()
                },
                plain(" has "),
                Span {
                    text: "3".into(),
                    code: true,
                    ..Span::default()
                },
                plain(" "),
                Span {
                    text: "open".into(),
                    italic: true,
                    ..Span::default()
                },
                plain(" ends"),
            ]
        );
        assert_eq!(spans("2 * 3 = 6 and snake_case"), [plain("2 * 3 = 6 and snake_case")]);
    }

    #[test]
    fn a_long_reply_is_cut_and_says_so() {
        let long = "word ".repeat(MAX_CHARS);
        let (blocks, cut) = parse(&long);
        assert!(cut);
        assert_eq!(blocks.len(), 1);
        let many = "- x\n".repeat(MAX_BLOCKS * 2);
        let (blocks, cut) = parse(&many);
        assert!(cut);
        assert_eq!(blocks.len(), MAX_BLOCKS);
    }
}
