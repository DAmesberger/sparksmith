//! What a model's tokens cost, from the library's `assist/prices.json`:
//! per model id on one provider at one base URL, with a source and a date.
//! The chat estimates a turn's and a conversation's cost from what the
//! provider reported; a connection with no price shows tokens only.
use crate::settings::Wire;
use crate::usage::Usage;
use crate::{Error, Result};
use serde::Deserialize;
use sparksmith_io::shipped::{Shelf, ShippedLibrary};

/// The file in the library's `assist/`.
pub const PRICES_FILE: &str = "prices.json";
const FORMAT: u32 = 1;
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// Prices one file may list.
const MAX_PRICES: usize = 256;
/// The highest rate per million tokens accepted, in any currency.
const MAX_RATE: f64 = 1_000_000.0;

/// Rates per million tokens.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rates {
    pub input: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
}

/// One model's price.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub provider: Wire,
    pub base_url: String,
    pub model: String,
    /// The ISO 4217 code the rates are in ("USD").
    pub currency: String,
    pub per_million: Rates,
    pub source: String,
    /// When the rates were read from the source (YYYY-MM-DD).
    pub as_of: String,
}

impl Price {
    /// What `usage` costs at these rates.
    pub fn cost(&self, usage: &Usage) -> f64 {
        let r = &self.per_million;
        let part = |tokens: u64, rate: f64| tokens as f64 * rate / 1_000_000.0;
        part(usage.input_tokens, r.input)
            + part(usage.cache_write_tokens, r.cache_write)
            + part(usage.cache_read_tokens, r.cache_read)
            + part(usage.output_tokens, r.output)
    }

    /// The estimate in words: "about 0.24 USD", "under 0.01 USD".
    pub fn estimate(&self, usage: &Usage) -> String {
        let cost = self.cost(usage);
        if cost < 0.005 {
            format!("under 0.01 {}", self.currency)
        } else {
            format!("about {cost:.2} {}", self.currency)
        }
    }

    fn check(&self) -> std::result::Result<(), String> {
        let r = &self.per_million;
        for (what, rate) in [
            ("input", r.input),
            ("cache_write", r.cache_write),
            ("cache_read", r.cache_read),
            ("output", r.output),
        ] {
            if !rate.is_finite() || !(0.0..=MAX_RATE).contains(&rate) {
                return Err(format!(
                    "{}: the {what} rate must be 0 to {MAX_RATE}",
                    self.model
                ));
            }
        }
        if self.model.trim().is_empty() || self.base_url.trim().is_empty() {
            return Err("a price needs a model and a base URL".into());
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(format!(
                "{}: the currency must be a three-letter code, not {:?}",
                self.model, self.currency
            ));
        }
        if self.source.trim().is_empty() || !is_date(&self.as_of) {
            return Err(format!(
                "{}: a price needs its source and the date it was read (YYYY-MM-DD)",
                self.model
            ));
        }
        Ok(())
    }
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    format: u32,
    #[serde(default, rename = "about")]
    _about: String,
    prices: Vec<Price>,
    #[serde(default, rename = "sources")]
    _sources: serde_json::Map<String, serde_json::Value>,
}

/// The shipped prices.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Prices {
    pub prices: Vec<Price>,
}

impl Prices {
    pub fn locate() -> Result<Self> {
        let shipped = ShippedLibrary::locate().map_err(|e| Error::Config(e.to_string()))?;
        Self::load(&shipped)
    }

    /// Read and check the shipped file; a missing or broken one is
    /// refused with its path.
    pub fn load(shipped: &ShippedLibrary) -> Result<Self> {
        let path = shipped.shelf(Shelf::Assist).join(PRICES_FILE);
        let bytes = shipped
            .read(Shelf::Assist, PRICES_FILE, MAX_FILE_BYTES)
            .map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        Self::parse(&bytes).map_err(|why| Error::Config(format!("{}: {why}", path.display())))
    }

    pub fn parse(bytes: &[u8]) -> std::result::Result<Self, String> {
        let file: File = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if file.format != FORMAT {
            return Err(format!(
                "format {} is not supported (this version reads format {FORMAT})",
                file.format
            ));
        }
        if file.prices.len() > MAX_PRICES {
            return Err(format!("at most {MAX_PRICES} prices"));
        }
        for price in &file.prices {
            price.check()?;
        }
        Ok(Self {
            prices: file.prices,
        })
    }

    /// The price of `model` on `provider` at `base_url`, if one is listed.
    pub fn find(&self, provider: Wire, base_url: &str, model: &str) -> Option<&Price> {
        let base = base_url.trim().trim_end_matches('/');
        self.prices.iter().find(|p| {
            p.provider == provider
                && p.base_url.trim_end_matches('/') == base
                && p.model == model.trim()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Fallible = std::result::Result<(), Box<dyn std::error::Error>>;

    /// The shipped file reads, and the default model has a price with a
    /// source and a date.
    #[test]
    fn the_shipped_prices_load_and_price_the_default_model() -> Fallible {
        let prices = Prices::locate()?;
        let opus = prices
            .find(
                Wire::Anthropic,
                "https://api.anthropic.com/",
                "claude-opus-5-5",
            )
            .ok_or("Opus 5.5 is priced")?;
        assert_eq!(opus.currency, "USD");
        assert!(opus.source.starts_with("https://"));
        assert!(is_date(&opus.as_of));
        Ok(())
    }

    /// Each kind of token at its own rate.
    #[test]
    fn a_cost_counts_each_kind_of_token_at_its_rate() -> Fallible {
        let prices = Prices::locate()?;
        let opus = prices
            .find(
                Wire::Anthropic,
                "https://api.anthropic.com",
                "claude-opus-5-5",
            )
            .ok_or("priced")?;
        let usage = Usage {
            input_tokens: 1_000_000,
            output_tokens: 100_000,
            cache_write_tokens: 200_000,
            cache_read_tokens: 2_000_000,
        };
        // 4 + 2 (output) + 1 (write) + 0.4 (read).
        assert!((opus.cost(&usage) - 7.4).abs() < 1e-9);
        assert_eq!(opus.estimate(&usage), "about 7.40 USD");
        assert_eq!(opus.estimate(&Usage::default()), "under 0.01 USD");
        Ok(())
    }

    /// A local model, another server or an unlisted model has no price:
    /// nothing is guessed.
    #[test]
    fn an_unlisted_connection_has_no_price() -> Fallible {
        let prices = Prices::locate()?;
        assert!(
            prices
                .find(Wire::Openai, "http://localhost:11434/v1", "claude-opus-5-5")
                .is_none()
        );
        assert!(
            prices
                .find(Wire::Anthropic, "https://proxy.example", "claude-opus-5-5")
                .is_none()
        );
        assert!(
            prices
                .find(
                    Wire::Anthropic,
                    "https://api.anthropic.com",
                    "claude-unknown"
                )
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn a_price_without_a_source_or_date_is_refused() {
        let text = |source: &str, date: &str| {
            format!(
                r#"{{"format": 1, "prices": [{{"provider": "anthropic", "base_url": "https://a", "model": "m",
                "currency": "USD", "per_million": {{"input": 1, "cache_write": 1, "cache_read": 1, "output": 1}},
                "source": "{source}", "as_of": "{date}"}}]}}"#
            )
        };
        assert!(Prices::parse(text("https://s", "2026-11-03").as_bytes()).is_ok());
        assert!(Prices::parse(text("", "2026-11-03").as_bytes()).is_err());
        assert!(Prices::parse(text("https://s", "November").as_bytes()).is_err());
        let negative = text("https://s", "2026-11-03").replace("\"output\": 1", "\"output\": -1");
        assert!(Prices::parse(negative.as_bytes()).is_err());
    }
}
