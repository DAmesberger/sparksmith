//! Streaming redaction of decoded content, before any model observer.
//! Text and reasoning are separate logical streams: each keeps only its
//! possible key-prefix length. Matching is linear (KMP), including Unicode.
use super::Secret;
use super::stream::Delta;

/// A wire's opaque content filter. The default passes through unchanged.
/// HTTP wires construct it with their key; callers never receive the key.
///
/// Each channel retains its own order. A possible key prefix waits for that
/// channel's next characters, while safe output from the other channel and
/// tool announcements proceed. Thus deferred prefixes may cross later events
/// from another channel; preserving global order would require an unbounded
/// queue if that first channel never continued.
#[derive(Default)]
pub struct Redactor {
    key: Option<Secret>,
    pattern: Vec<char>,
    prefix: Vec<usize>,
    text: Matcher,
    reasoning: Matcher,
}

impl Redactor {
    pub(crate) fn new(key: Option<Secret>) -> Self {
        let pattern: Vec<char> = key.as_ref().map_or("", Secret::expose).chars().collect();
        let mut prefix = vec![0; pattern.len()];
        let mut matched = 0;
        for i in 1..pattern.len() {
            while matched > 0 && pattern.get(i) != pattern.get(matched) {
                matched = prefix.get(matched - 1).copied().unwrap_or(0);
            }
            if pattern.get(i) == pattern.get(matched) {
                matched += 1;
            }
            if let Some(slot) = prefix.get_mut(i) {
                *slot = matched;
            }
        }
        Self {
            key,
            pattern,
            prefix,
            ..Self::default()
        }
    }

    pub(crate) fn push(&mut self, delta: Delta, on: &mut dyn FnMut(Delta)) {
        let Some(key) = &self.key else {
            on(delta);
            return;
        };
        match delta {
            Delta::Text(text) => emit(
                self.text.push(&text, &self.pattern, &self.prefix),
                Delta::Text,
                on,
            ),
            Delta::Reasoning(text) => emit(
                self.reasoning.push(&text, &self.pattern, &self.prefix),
                Delta::Reasoning,
                on,
            ),
            Delta::ToolCall { index, name } => on(Delta::ToolCall {
                index,
                name: key.redact(&name),
            }),
        }
    }

    /// A completed reply can release nonmatching suffixes. Cancellation
    /// and errors discard both: an unfinished possible secret stays private.
    pub(crate) fn finish(&mut self, complete: bool, on: &mut dyn FnMut(Delta)) {
        emit(self.text.finish(complete, &self.pattern), Delta::Text, on);
        emit(
            self.reasoning.finish(complete, &self.pattern),
            Delta::Reasoning,
            on,
        );
    }
}

/// The held characters always equal pattern[..matched], so they need no
/// separate allocation. Both channels share the immutable pattern/table.
#[derive(Default)]
struct Matcher {
    matched: usize,
}

impl Matcher {
    fn push(&mut self, text: &str, pattern: &[char], prefix: &[usize]) -> String {
        let mut output = String::new();
        for ch in text.chars() {
            let previous = self.matched;
            while self.matched > 0 && pattern.get(self.matched) != Some(&ch) {
                self.matched = prefix.get(self.matched - 1).copied().unwrap_or(0);
            }
            if pattern.get(self.matched) == Some(&ch) {
                self.matched += 1;
            }
            if self.matched == pattern.len() {
                output.push_str("[redacted]");
                self.matched = 0;
            } else {
                let safe = previous + 1 - self.matched;
                output.extend(pattern.iter().take(safe.min(previous)));
                if safe > previous {
                    output.push(ch);
                }
            }
        }
        output
    }

    fn finish(&mut self, complete: bool, pattern: &[char]) -> String {
        let matched = std::mem::take(&mut self.matched);
        if complete {
            pattern.iter().take(matched).collect()
        } else {
            String::new()
        }
    }
}

fn emit(text: String, delta: fn(String) -> Delta, on: &mut dyn FnMut(Delta)) {
    if !text.is_empty() {
        on(delta(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These are the two actual observer buffers, never their arbitrary
    // interleaved concatenation.
    fn projections(deltas: &[Delta]) -> (String, String) {
        let mut answer = String::new();
        let mut reasoning = String::new();
        for delta in deltas {
            match delta {
                Delta::Text(text) => answer.push_str(text),
                Delta::Reasoning(text) => reasoning.push_str(text),
                _ => {}
            }
        }
        (answer, reasoning)
    }

    #[test]
    fn unicode_overlap_and_repeated_matches_are_independent_of_chunks() {
        for key in ["sk-test-secret", "abababx", "κλειδί-秘密", "a"] {
            let input = format!("prefix {key} middle {key} suffix ababa");
            let expected = input.replace(key, "[redacted]");
            for split in input.char_indices().map(|(i, _)| i).chain([input.len()]) {
                let mut filter = Redactor::new(Secret::new(key));
                let mut shown = Vec::new();
                for delta in [
                    Delta::Text(input[..split].into()),
                    Delta::Reasoning(input[..split].into()),
                    Delta::Text(input[split..].into()),
                    Delta::Reasoning(input[split..].into()),
                ] {
                    filter.push(delta, &mut |d| shown.push(d));
                }
                filter.finish(true, &mut |d| shown.push(d));
                assert_eq!(
                    projections(&shown),
                    (expected.clone(), expected.clone()),
                    "split {split}"
                );
            }
        }
    }

    #[test]
    fn interleaved_channels_each_hide_their_own_split_key() {
        let key = "sk-test-secret";
        for split in 0..=key.len() {
            let mut filter = Redactor::new(Secret::new(key));
            let mut shown = Vec::new();
            for delta in [
                Delta::Text(format!("answer {}", &key[..split])),
                Delta::Reasoning(format!("thinking {}", &key[..split])),
                Delta::Text(format!("{} done", &key[split..])),
                Delta::Reasoning(format!("{} done", &key[split..])),
            ] {
                filter.push(delta, &mut |d| shown.push(d));
            }
            filter.finish(true, &mut |d| shown.push(d));
            assert_eq!(
                projections(&shown),
                (
                    "answer [redacted] done".into(),
                    "thinking [redacted] done".into()
                ),
                "split {split}"
            );
        }
    }

    #[test]
    fn unrelated_other_channel_does_not_flush_a_key_prefix() {
        let key = "sk-test-secret";
        for answer_first in [true, false] {
            let [first, other] = if answer_first {
                [Delta::Text as fn(String) -> Delta, Delta::Reasoning]
            } else {
                [Delta::Reasoning as fn(String) -> Delta, Delta::Text]
            };
            let mut filter = Redactor::new(Secret::new(key));
            let mut shown = Vec::new();
            filter.push(first("before sk-test-".into()), &mut |d| shown.push(d));
            filter.push(other("unrelated".into()), &mut |d| shown.push(d));
            filter.push(first("secret after".into()), &mut |d| shown.push(d));
            filter.finish(true, &mut |d| shown.push(d));
            let expected = if answer_first {
                ("before [redacted] after".into(), "unrelated".into())
            } else {
                ("unrelated".into(), "before [redacted] after".into())
            };
            assert_eq!(projections(&shown), expected);
        }
    }

    #[test]
    fn completion_releases_both_suffixes_but_cancellation_discards_them() {
        for complete in [false, true] {
            let mut filter = Redactor::new(Secret::new("sk-test-secret"));
            let mut shown = Vec::new();
            filter.push(Delta::Text("answer sk-test-".into()), &mut |d| {
                shown.push(d)
            });
            filter.push(Delta::Reasoning("thinking sk-".into()), &mut |d| {
                shown.push(d)
            });
            assert_eq!(projections(&shown), ("answer ".into(), "thinking ".into()));
            filter.finish(complete, &mut |d| shown.push(d));
            let expected = if complete {
                ("answer sk-test-".into(), "thinking sk-".into())
            } else {
                ("answer ".into(), "thinking ".into())
            };
            assert_eq!(projections(&shown), expected);
            let count = shown.len();
            filter.finish(complete, &mut |d| shown.push(d));
            assert_eq!(shown.len(), count, "finishing twice emits nothing twice");
        }
    }

    #[test]
    fn paused_channel_does_not_buffer_tools_or_other_channel_output() {
        let mut filter = Redactor::new(Secret::new("sk-test-secret"));
        let mut shown = Vec::new();
        filter.push(Delta::Text("before sk-test-".into()), &mut |d| {
            shown.push(d)
        });
        assert_eq!(shown, [Delta::Text("before ".into())]);
        for index in 0..1000 {
            shown.clear();
            filter.push(
                Delta::ToolCall {
                    index,
                    name: "query-sk-test-secret".into(),
                },
                &mut |d| shown.push(d),
            );
            filter.push(Delta::Reasoning("ordinary reasoning".into()), &mut |d| {
                shown.push(d)
            });
            assert_eq!(
                shown,
                [
                    Delta::ToolCall {
                        index,
                        name: "query-[redacted]".into()
                    },
                    Delta::Reasoning("ordinary reasoning".into()),
                ]
            );
        }
        shown.clear();
        filter.push(Delta::Text("secret after".into()), &mut |d| shown.push(d));
        filter.finish(true, &mut |d| shown.push(d));
        assert_eq!(shown, [Delta::Text("[redacted] after".into())]);
    }

    #[test]
    fn default_filter_passes_events_through_without_delay() {
        let mut filter = Redactor::default();
        let input = [
            Delta::Text("sk-".into()),
            Delta::Reasoning("thought".into()),
            Delta::ToolCall {
                index: 0,
                name: "query".into(),
            },
        ];
        let mut shown = Vec::new();
        for delta in &input {
            filter.push(delta.clone(), &mut |d| shown.push(d));
        }
        filter.finish(true, &mut |d| shown.push(d));
        assert_eq!(shown, input);
    }
}
