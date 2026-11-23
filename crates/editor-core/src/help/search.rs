//! The palette's matcher, shared by the offline help readers.
use super::{Facts, HelpEntry, HelpKind, Index};

pub const WORD_MATCH: u32 = 300;

pub fn match_score(query: &str, title: &str, id: &str, keywords: &str, workspace: bool) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let title_words = title.to_lowercase();
    let words = title_words.split(|c: char| !c.is_alphanumeric());
    let keywords = keywords.to_lowercase();
    let other = keywords.split(|c: char| !c.is_alphanumeric());
    let all = query
        .split_whitespace()
        .all(|q| words.clone().chain(other.clone()).any(|w| w.starts_with(q)));
    let exact = query
        .split_whitespace()
        .all(|q| words.clone().any(|w| w == q));
    let whole = if !exact {
        0
    } else if workspace {
        900
    } else {
        850
    };
    score(query, title)
        .max(if all { 700 } else { 0 })
        .max(score(query, id) / 2)
        .max(whole)
}

pub fn search<'a>(
    index: &'a Index,
    text: &str,
    kinds: &[HelpKind],
    limit: usize,
) -> Vec<&'a HelpEntry> {
    let query = text.trim().to_lowercase();
    let mut hits: Vec<_> = index
        .values()
        .filter(|e| kinds.is_empty() || kinds.contains(&e.id.kind()))
        .filter_map(|entry| {
            let mut keywords = entry.keywords.join(" ");
            keywords.push(' ');
            keywords.push_str(&entry.summary);
            if let Facts::Term { avoid, .. } = &entry.facts {
                keywords.push(' ');
                keywords.push_str(&avoid.join(" "));
            }
            let weight = if entry.id.as_str() == query {
                u32::MAX
            } else {
                match_score(&query, &entry.title, entry.id.as_str(), &keywords, false)
            };
            (weight > 0).then_some((weight, entry))
        })
        .collect();
    if !query.is_empty() && hits.iter().any(|(s, _)| *s >= WORD_MATCH) {
        hits.retain(|(s, _)| *s >= WORD_MATCH);
    }
    hits.sort_by_key(|(weight, entry)| (std::cmp::Reverse(*weight), &entry.title, &entry.id));
    hits.into_iter().take(limit).map(|(_, e)| e).collect()
}

/// How well `q` matches `s` (0 = not at all): a prefix beats a word start beats a substring
/// beats the letters in order with gaps.
pub fn score(q: &str, s: &str) -> u32 {
    if q.is_empty() {
        return 1;
    }
    let s = s.to_lowercase();
    if s.starts_with(q) {
        return 1000 - s.len().min(500) as u32;
    }
    if s.split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(q))
    {
        return 800;
    }
    if s.contains(q) {
        return 600;
    }
    // Letters in order: fewer gaps score higher.
    let mut gaps = 0u32;
    let mut it = s.chars();
    let mut last_hit = true;
    for qc in q.chars() {
        let mut found = false;
        for c in it.by_ref() {
            if c == qc {
                found = true;
                break;
            }
            if last_hit {
                gaps += 1;
            }
            last_hit = false;
        }
        if !found {
            return 0;
        }
        last_hit = true;
    }
    300u32.saturating_sub(gaps.saturating_mul(20)).max(10)
}
