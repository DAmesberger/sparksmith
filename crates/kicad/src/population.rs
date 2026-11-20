//! KiCad's ways of saying a part is not fitted, as Sparksmith's typed
//! default population ([`docs/assembly-variants.md`], Import): the
//! do-not-populate attribute, and a value that says "not fitted" in words
//! ("DNP", "10k DNP"). Exclude-from-BOM is not one: it marks test points,
//! mechanical items and logos, which are not left off.
//!
//! A value never carries the population in Sparksmith, so the marker is
//! taken out of the value: a value that is only the marker becomes empty,
//! a real value beside it is kept.
//!
//! The markers are import-translation logic, kept short and conservative:
//! a word that also means something else in a value ("NC": no-connect or
//! normally closed; "NP": non-polarised; "NF": nanofarad written apart) is
//! a marker only where the whole value is that word.

/// Why an imported part is not populated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Why {
    /// KiCad's do-not-populate attribute (`dnp`) on the footprint or the
    /// symbol.
    DoNotPopulate,
    /// A value that says "not fitted" ([`split_marker`]).
    MarkerValue,
}

impl Why {
    /// The report's fixed reason.
    pub fn text(self) -> &'static str {
        match self {
            Self::DoNotPopulate => "KiCad's do-not-populate attribute",
            Self::MarkerValue => "a not-fitted value marker (taken out of the value)",
        }
    }
}

/// Markers that say "not fitted" wherever they stand in a value, as
/// upper-case word sequences ("10k DNP", "DNP 10k", "10k (not fitted)").
const MARKERS: &[&[&str]] = &[
    &["DNP"],
    &["DNF"],
    &["DNI"],
    &["DNM"],
    &["NOFIT"],
    &["NOPOP"],
    &["NO", "FIT"],
    &["NO", "POP"],
    &["NOT", "FITTED"],
    &["NOT", "FIT"],
    &["NOT", "POPULATED"],
    &["NOT", "MOUNTED"],
    &["DO", "NOT", "POPULATE"],
    &["DO", "NOT", "FIT"],
    &["DO", "NOT", "MOUNT"],
];

/// Words that say "not fitted" only as the whole value (letters only, so
/// "N/F" and "N.F." are "NF"); beside a real value they mean something
/// else.
const WHOLE_MARKERS: &[&str] = &["NF", "NP"];

/// Characters that end a word of a value.
fn separator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '/' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '|' | '-' | '_'
        )
}

/// Characters trimmed off a word before it is compared.
fn punctuation(c: char) -> bool {
    matches!(c, '.' | '*' | '!' | '?' | '#' | '"' | '\'' | '~' | '=')
}

/// `value` without its not-fitted marker (empty when the value is only
/// the marker); `None` when it has none (the value is kept as it is).
pub fn split_marker(value: &str) -> Option<String> {
    let compact: String = value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if WHOLE_MARKERS.contains(&compact.as_str()) {
        return Some(String::new());
    }
    // The words, with their byte spans in `value`.
    let mut words: Vec<(usize, usize, String)> = Vec::new();
    let mut start = None;
    for (i, c) in value.char_indices().chain([(value.len(), ' ')]) {
        match (start, separator(c)) {
            (None, false) => start = Some(i),
            (Some(s), true) => {
                let word = value
                    .get(s..i)
                    .unwrap_or_default()
                    .trim_matches(punctuation)
                    .to_ascii_uppercase();
                if !word.is_empty() {
                    words.push((s, i, word));
                }
                start = None;
            }
            _ => {}
        }
    }
    let mut cut = vec![false; words.len()];
    let mut found = false;
    let mut k = 0;
    while k < words.len() {
        let hit = MARKERS.iter().find(|m| {
            words
                .get(k..k + m.len())
                .is_some_and(|w| w.iter().zip(m.iter()).all(|(w, m)| w.2 == *m))
        });
        match hit {
            Some(m) => {
                for c in cut.iter_mut().skip(k).take(m.len()) {
                    *c = true;
                }
                found = true;
                k += m.len();
            }
            None => k += 1,
        }
    }
    if !found {
        return None;
    }
    let mut rest = String::new();
    let mut from = 0;
    for ((s, e, _), cut) in words.iter().zip(&cut) {
        if *cut {
            rest.push_str(value.get(from..*s).unwrap_or_default());
            from = *e;
        }
    }
    rest.push_str(value.get(from..).unwrap_or_default());
    // Brackets the marker stood in, and separators left at the ends.
    let mut rest = rest.replace("()", "").replace("[]", "");
    loop {
        let trimmed = rest
            .trim_matches(|c: char| separator(c) || punctuation(c))
            .to_owned();
        let tidied = trimmed.replace("()", "").replace("[]", "");
        if tidied == rest {
            break;
        }
        rest = tidied;
    }
    let rest = rest.split_whitespace().collect::<Vec<_>>().join(" ");
    // "NC (not fitted)": the NC belongs to the marker.
    let rest_compact: String = rest
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if rest_compact == "NC" || WHOLE_MARKERS.contains(&rest_compact.as_str()) {
        return Some(String::new());
    }
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::split_marker;

    #[test]
    fn markers_alone_empty_the_value() {
        for value in [
            "DNP",
            "dnp",
            " DNP ",
            "(DNP)",
            "-DNP-",
            "*DNP*",
            "DNF",
            "NF",
            "N/F",
            "NP",
            "DNI",
            "NOFIT",
            "NO FIT",
            "Not Fitted",
            "NC (not fitted)",
            "do not populate",
        ] {
            assert_eq!(split_marker(value).as_deref(), Some(""), "{value}");
        }
    }

    #[test]
    fn a_real_value_beside_a_marker_is_kept() {
        for (value, kept) in [
            ("10k DNP", "10k"),
            ("10k/DNP", "10k"),
            ("DNP 10k", "10k"),
            ("10k (DNP)", "10k"),
            ("10k_DNP", "10k"),
            ("100n 16V DNF", "100n 16V"),
            ("10k, 1% not fitted", "10k, 1%"),
        ] {
            assert_eq!(split_marker(value).as_deref(), Some(kept), "{value}");
        }
    }

    #[test]
    fn values_without_a_marker_are_untouched() {
        for value in [
            "NC",
            "10k",
            "100 nF",
            "10u NP",
            "NPN",
            "DNPX",
            "LM317-ADJ",
            "",
            "~",
        ] {
            assert_eq!(split_marker(value), None, "{value}");
        }
    }
}
