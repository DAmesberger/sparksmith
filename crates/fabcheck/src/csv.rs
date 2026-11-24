//! Comma-separated tables read back (RFC 4180): quoted fields, doubled
//! quotes, CR LF or LF line ends.
use crate::Fallible;

/// Records one table takes.
pub const MAX_RECORDS: usize = 1_000_000;

/// The records of `source`, the header first. Every record has as many
/// fields as the header.
pub fn records(source: &str) -> Fallible<Vec<Vec<String>>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // Inside a quoted field.
    let mut inside = false;
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        if inside {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => inside = false,
                c => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                quoted = true;
                inside = true;
            }
            '"' => return Err("csv: a quote inside an unquoted field".into()),
            ',' => {
                record.push(std::mem::take(&mut field));
                quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' => {
                record.push(std::mem::take(&mut field));
                quoted = false;
                out.push(std::mem::take(&mut record));
                if out.len() > MAX_RECORDS {
                    return Err(format!("csv: more than {MAX_RECORDS} records").into());
                }
            }
            _ if quoted => return Err("csv: text after a quoted field".into()),
            c => field.push(c),
        }
    }
    if inside {
        return Err("csv: a quoted field is not closed".into());
    }
    if !field.is_empty() || !record.is_empty() || quoted {
        return Err("csv: the last line has no line end".into());
    }
    let width = out.first().map_or(0, Vec::len);
    if let Some(k) = out.iter().position(|r| r.len() != width) {
        return Err(format!(
            "csv: line {} has {} fields, the header {width}",
            k + 1,
            out[k].len()
        )
        .into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_fields_hold_commas_quotes_and_line_ends() -> Fallible<()> {
        assert_eq!(
            records("a,b\n\"x, y\",\"say \"\"z\"\"\nand\"\n")?,
            [
                vec!["a".to_owned(), "b".into()],
                vec!["x, y".into(), "say \"z\"\nand".into()]
            ]
        );
        for broken in ["a,b\nx\n", "a\n\"x", "a\nx", "a\nx\"y\n", "a\n\"x\"y\n"] {
            assert!(records(broken).is_err(), "{broken:?}");
        }
        Ok(())
    }
}
