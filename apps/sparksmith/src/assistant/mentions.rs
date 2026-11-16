//! The composer's files (`docs/assistant-panel.md` §7): `@` completes a
//! file name, fuzzily, from the files the conversation knows (attached or
//! used before in this session, then the project's folder and its
//! subfolders), and a path typed or pasted becomes the same chip. Each chip
//! goes with the message as an attachment.
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Files the project's folder lists at most, and entries it visits.
const MAX_PROJECT_FILES: usize = 4_000;
const MAX_VISITED: usize = 20_000;
/// Folders deep the project's folder is read.
const MAX_DEPTH: usize = 8;
/// Rows the `@` list shows.
pub const MAX_ROWS: usize = 8;
/// The longest `@` query read, in characters.
const MAX_QUERY: usize = 128;

/// A file `@` may complete to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub name: String,
    /// Where it is: its folder in the project, or the folder it was
    /// attached from.
    pub detail: String,
    /// Attached or used before in this session: ranked first.
    pub known: bool,
}

/// The candidates: `known` files (most recent first), then the files of
/// the project's `folder`, each once.
pub fn candidates(known: &[PathBuf], folder: Option<&Path>) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    let detail = |path: &Path| {
        let parent = path.parent().unwrap_or(Path::new(""));
        match folder.and_then(|f| parent.strip_prefix(f).ok()) {
            Some(inside) if inside.as_os_str().is_empty() => "Project folder".to_owned(),
            Some(inside) => inside.display().to_string(),
            None => parent.display().to_string(),
        }
    };
    for path in known.iter().rev() {
        if out.iter().any(|c| &c.path == path) {
            continue;
        }
        out.push(Candidate {
            name: file_name(path),
            detail: detail(path),
            path: path.clone(),
            known: true,
        });
    }
    if let Some(folder) = folder {
        for path in project_files(folder) {
            if out.iter().any(|c| c.path == path) {
                continue;
            }
            out.push(Candidate {
                name: file_name(&path),
                detail: detail(&path),
                path,
                known: false,
            });
        }
    }
    out
}

/// The files in `folder` and its subfolders, bounded: hidden entries and
/// `target` folders are left out, links are not followed.
pub fn project_files(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut visited = 0usize;
    let mut stack = vec![(folder.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<std::fs::DirEntry> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            visited += 1;
            if visited > MAX_VISITED || out.len() >= MAX_PROJECT_FILES {
                out.sort();
                return out;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name == "target" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < MAX_DEPTH {
                stack.push((entry.path(), depth + 1));
            } else if kind.is_file() {
                out.push(entry.path());
            }
        }
    }
    out.sort();
    out
}

/// The candidates `query` matches, best first, at most [`MAX_ROWS`]: by
/// name, else by the path in its folder.
pub fn rank<'a>(query: &str, candidates: &'a [Candidate]) -> Vec<&'a Candidate> {
    let mut scored: Vec<(i64, usize, &Candidate)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            let by_name = fuzzy(query, &c.name);
            let score = by_name.or_else(|| {
                fuzzy(query, &format!("{}/{}", c.detail, c.name)).map(|s| s - 20)
            })?;
            Some((score + if c.known { 25 } else { 0 }, i, c))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().take(MAX_ROWS).map(|(_, _, c)| c).collect()
}

/// How well `query` matches `text`: its characters in order, case aside;
/// each one found scores, more when it follows the last or starts a word,
/// less for the characters skipped. `None` when they are not all there.
pub fn fuzzy(query: &str, text: &str) -> Option<i64> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0i64;
    let mut at = 0usize;
    let mut last: Option<usize> = None;
    for q in query.to_lowercase().chars().take(MAX_QUERY) {
        let found = (at..text.len()).find(|&i| text[i] == q)?;
        score += 10;
        if last.is_some_and(|l| l + 1 == found) {
            score += 15;
        }
        if found == 0 || text.get(found - 1).is_some_and(|c| !c.is_alphanumeric()) {
            score += 10;
        }
        score -= i64::try_from(found - at).unwrap_or(i64::MAX / 4).min(1_000);
        last = Some(found);
        at = found + 1;
    }
    Some(score - i64::try_from(text.len() / 4).unwrap_or(0))
}

/// The `@` being typed before the cursor (a character index): the byte
/// where the `@` starts and the query after it. An `@` counts at the start
/// or after a space, with no space between it and the cursor.
pub fn active(text: &str, cursor: usize) -> Option<(usize, String)> {
    let end = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte);
    let before = text.get(..end)?;
    let at = before.rfind('@')?;
    let query = before.get(at + 1..)?;
    if query.chars().any(char::is_whitespace) || query.chars().count() > MAX_QUERY {
        return None;
    }
    let starts_word = before
        .get(..at)
        .is_none_or(|b| b.chars().next_back().is_none_or(char::is_whitespace));
    starts_word.then(|| (at, query.to_owned()))
}

/// Paths typed or pasted in `text` that name a file on this computer: a
/// word starting with `/`, `~/` or a drive (`C:\`), quotes and closing
/// punctuation aside. With `all` false only a finished word counts (a
/// space follows it), so a path being typed is left alone.
pub fn typed_paths(text: &str, all: bool) -> Vec<(Range<usize>, PathBuf)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices().chain(std::iter::once((text.len(), ' '))) {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                start = None;
                let finished = i < text.len();
                if !(all || finished) {
                    continue;
                }
                let Some(word) = text.get(s..i) else {
                    continue;
                };
                let core = word.trim_start_matches(['"', '\'', '(']);
                let lead = word.len() - core.len();
                let core = core.trim_end_matches(['"', '\'', ')', ',', ';', ':', '.']);
                if let Some(path) = as_path(core)
                    && path.is_file()
                {
                    out.push((s + lead..s + lead + core.len(), path));
                }
            }
            _ => {}
        }
    }
    out
}

fn as_path(word: &str) -> Option<PathBuf> {
    if let Some(rest) = word.strip_prefix("~/") {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
        return Some(Path::new(&home).join(rest));
    }
    let drive = word.len() > 3
        && word.as_bytes()[0].is_ascii_alphabetic()
        && word.get(1..3).is_some_and(|s| s == ":\\" || s == ":/");
    (word.starts_with('/') || drive).then(|| PathBuf::from(word))
}

/// A path's file name, or the path when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_at_counts_at_a_word_start_up_to_the_cursor() {
        assert_eq!(active("make @tlv", 9), Some((5, "tlv".into())));
        assert_eq!(active("@", 1), Some((0, String::new())));
        assert_eq!(active("make @tlv757p.pdf it", 20), None, "a space ends it");
        assert_eq!(active("me@example", 10), None, "inside a word");
        assert_eq!(active("make @tlv", 7), Some((5, "t".into())));
        assert_eq!(active("ä @ö", 4), Some((3, "ö".into())));
    }

    #[test]
    fn fuzzy_matches_in_order_and_ranks_word_starts_and_runs() {
        assert!(fuzzy("tlv", "tlv757p.pdf").is_some());
        assert!(fuzzy("t7p", "tlv757p.pdf").is_some());
        assert!(fuzzy("pdt", "tlv757p.pdf").is_none());
        let start = fuzzy("tlv", "tlv757p.pdf").unwrap_or(i64::MIN);
        let inside = fuzzy("tlv", "notes-atlv.pdf").unwrap_or(i64::MIN);
        assert!(start > inside, "{start} {inside}");
    }

    #[test]
    fn known_files_rank_first_and_the_list_is_bounded() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir_all(dir.path().join("datasheets"))?;
        std::fs::create_dir_all(dir.path().join(".git"))?;
        for name in ["datasheets/tlv757p.pdf", "board.sparksmith", ".git/tlv.pdf"] {
            std::fs::write(dir.path().join(name), b"x")?;
        }
        for n in 0..20 {
            std::fs::write(dir.path().join(format!("tlv-{n}.txt")), b"x")?;
        }
        let elsewhere = PathBuf::from("/elsewhere/tlv757-errata.pdf");
        let all = candidates(std::slice::from_ref(&elsewhere), Some(dir.path()));
        assert!(!all.iter().any(|c| c.path.starts_with(dir.path().join(".git"))));
        let ranked = rank("tlv757", &all);
        assert_eq!(ranked[0].path, elsewhere);
        assert_eq!(ranked[1].name, "tlv757p.pdf");
        assert_eq!(ranked[1].detail, "datasheets");
        assert_eq!(rank("tlv", &all).len(), MAX_ROWS);
        Ok(())
    }

    #[test]
    fn a_finished_typed_path_to_a_file_is_found() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let pdf = dir.path().join("tlv757p.pdf");
        std::fs::write(&pdf, b"%PDF-")?;
        let text = format!("from \"{}\", please", pdf.display());
        let found = typed_paths(&text, false);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, pdf);
        assert_eq!(&text[found[0].0.clone()], pdf.display().to_string());
        // Still being typed: left alone until a space follows, or sent.
        let typing = format!("from {}", pdf.display());
        assert!(typed_paths(&typing, false).is_empty());
        assert_eq!(typed_paths(&typing, true).len(), 1);
        assert!(typed_paths("from /no/such/file.pdf ", false).is_empty());
        Ok(())
    }
}
