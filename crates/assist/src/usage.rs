//! Token usage a model reports, in neutral names, and the run's tally of
//! it. The run's caps (requests, output tokens, total tokens, time) are
//! checked against the tally; Sparksmith keeps no prices and no spend.
use serde::{Deserialize, Serialize};

/// Estimated tokens from UTF-8 bytes when the provider has no count. The
/// chat's scripted workload provides the reference for this estimate.
pub fn estimated_tokens(bytes: u64) -> u64 {
    bytes.div_ceil(3)
}

/// Estimated output includes thinking text and tool arguments as generated.
pub fn estimated_output_tokens(parts: &[crate::model::Part]) -> u64 {
    use crate::model::Part;
    let bytes = parts
        .iter()
        .map(|part| match part {
            Part::Reasoning { text, .. } | Part::Text { text, .. } => text.len() as u64,
            Part::ToolCall { input, .. } => input.to_string().len() as u64,
            _ => 0,
        })
        .fold(0, u64::saturating_add);
    estimated_tokens(bytes)
}

/// Token counts of one response, in neutral names.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
}

impl Usage {
    /// The whole prompt: uncached, written and read.
    pub fn prompt(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_write_tokens)
            .saturating_add(self.cache_read_tokens)
    }

    pub fn add(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(other.cache_write_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
    }
}

/// Attempted requests and reported tokens so far. The shared request driver
/// counts an attempt before sending; failed attempts may have no token report.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct Tally {
    pub usage: Usage,
    pub requests: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_all_four_reported_token_kinds() {
        let usage = Usage {
            input_tokens: 1_000,
            output_tokens: 2_000,
            cache_write_tokens: 100_000,
            cache_read_tokens: 1_000_000,
        };
        let mut total = Usage::default();
        total.add(&usage);
        total.add(&usage);
        assert_eq!(total.prompt(), 2 * 1_101_000);
        assert_eq!(total.output_tokens, 4_000);
    }
}
