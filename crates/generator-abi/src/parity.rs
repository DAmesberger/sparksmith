//! Native-and-module parity: one record of everything a generator says, so
//! its native build and its WebAssembly module can be compared byte for
//! byte although they run in different processes.
//!
//! The record covers the description exactly as `sparksmith_describe`
//! returns it, and the reply (status byte and body) to every request
//! [`cases`] lists: the defaults and each preset, in each scope the
//! generator makes. The generator's native test computes it with the SDK
//! and the host's module test computes it from the module under wasmi; both
//! compare it with the `outputs` digest the crate's `golden.json` keeps.
//! (A miscompiling interpreter once cut a parameter group name in the
//! module while the native build and the golden part hashes stayed right.)
use crate::codec::{hex, sha256};
use crate::v1::{self, Description, Request, Scope};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// One request of the record, named for the report.
pub struct Case {
    pub name: String,
    pub request: Request,
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::Component => "component",
        Scope::Package => "package",
    }
}

/// The requests of the record: the parameter defaults, then each preset's
/// parameters over the defaults, in each scope of the part capability.
pub fn cases(description: &Description) -> Vec<Case> {
    let mut defaults = v1::Params::new();
    for param in description.params() {
        defaults.insert(param.key.clone(), param.default.clone());
    }
    let mut sets: Vec<(String, v1::Params)> = Vec::new();
    sets.push(("defaults".into(), defaults.clone()));
    for preset in description.presets() {
        let mut params = defaults.clone();
        for (key, value) in &preset.params {
            params.insert(key.clone(), value.clone());
        }
        sets.push((format!("preset {}", preset.key), params));
    }
    let mut out = Vec::new();
    for (name, params) in sets {
        for &scope in description.scopes() {
            out.push(Case {
                name: format!("{name}, {}", scope_name(scope)),
                request: Request {
                    abi: v1::ABI,
                    scope,
                    params: params.clone(),
                },
            });
        }
    }
    out
}

/// The record being built: each entry's name and the SHA-256 of its bytes.
pub struct Outputs {
    entries: Vec<(String, [u8; 32])>,
}

impl Outputs {
    /// Start with the canonical description bytes.
    pub fn new(description: &[u8]) -> Self {
        Self {
            entries: alloc::vec![("description".into(), sha256(description))],
        }
    }

    /// Add the reply to the case `name`: its status byte and body.
    pub fn reply(&mut self, name: &str, status: u8, body: &[u8]) {
        let mut bytes = Vec::with_capacity(body.len() + 1);
        bytes.push(status);
        bytes.extend_from_slice(body);
        self.entries.push((name.into(), sha256(&bytes)));
    }

    /// The digest `golden.json` keeps as `outputs`: SHA-256 over each
    /// entry's name, a zero byte and its hash, in order.
    pub fn digest(&self) -> String {
        let mut all = Vec::new();
        for (name, hash) in &self.entries {
            all.extend_from_slice(name.as_bytes());
            all.push(0);
            all.extend_from_slice(hash);
        }
        hex(&sha256(&all))
    }

    /// One line per entry, `name: hash`, to compare two records by eye.
    pub fn lines(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(name, hash)| format!("{name}: {}", hex(hash)))
            .collect()
    }
}
