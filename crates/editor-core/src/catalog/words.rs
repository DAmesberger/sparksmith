//! How a word of a search names text, the one rule every part search
//! follows (the catalog's index, the generators' presets): a word matches
//! where a word of the text starts, never inside one ("fuse" does not find
//! "Littelfuse"), and a word also means the words the search vocabulary
//! gives it (`generic-families.json`'s `synonyms`: "button" also asks for
//! "tactile"). The vocabulary is library data; nothing here names a word.
use crate::library::standards;

/// Whether `needle` (lower case) starts at a word boundary of `hay`
/// (lower case): at its start, after a character that is not a letter or
/// digit, or where letters turn to digits or digits to letters ("0603" in
/// "r0603", "k7" in "4k7").
pub fn starts_word(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let mut previous: Option<char> = None;
    for (at, c) in hay.char_indices() {
        let boundary = match previous {
            None => true,
            Some(p) => {
                !p.is_alphanumeric()
                    || (p.is_alphabetic() && c.is_numeric())
                    || (p.is_numeric() && c.is_alphabetic())
            }
        };
        if boundary && hay[at..].starts_with(needle) {
            return true;
        }
        previous = Some(c);
    }
    false
}

/// `word` (lower case) and every word the search vocabulary says it also
/// means, `word` first; just `word` without the shipped standards.
pub fn meanings(word: &str) -> Vec<String> {
    let mut out = vec![word.to_owned()];
    if let Ok(standards) = standards::get() {
        for synonym in &standards.generic.synonyms {
            if synonym.word.eq_ignore_ascii_case(word) {
                out.extend(synonym.means.iter().map(|m| m.to_lowercase()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_matches_where_a_word_starts() {
        assert!(starts_word("littelfuse mdsm-4", "mdsm"));
        assert!(!starts_word("littelfuse mdsm-4", "fuse"));
        assert!(starts_word("fuse 1206", "fuse"));
        assert!(starts_word("r0603", "0603"));
        assert!(starts_word("4k7", "k7"));
        assert!(starts_word("sma (do-214ac)", "do-214"));
        assert!(!starts_word("tactile", "act"));
    }
}
