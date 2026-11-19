//! Golden cases: `(request → result hash)` pairs each generator crate keeps
//! in `golden.json`. The same file is checked three ways: natively here,
//! under wasmi natively, and under wasmi built for wasm32
//! (`cargo xtask generators check`); all three must give these hashes.
use crate::Generator;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use serde::Deserialize;
use sparksmith_generator_abi::v1::Request;
use sparksmith_generator_abi::{codec, hex, result_hash};

/// A generator crate's `golden.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenFile {
    pub cases: Vec<GoldenCase>,
    // The digest of everything the generator says (`outputs`). Native
    // only, like all of the parity check: the module stays the bytes it
    // was without it.
    #[cfg(not(target_arch = "wasm32"))]
    pub outputs: String,
}

/// One case: a request with complete parameters and the hex SHA-256 of its
/// canonical generation.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldenCase {
    pub name: String,
    pub request: Request,
    pub result: String,
}

/// Parse a `golden.json`.
pub fn parse(json: &str) -> Result<GoldenFile, String> {
    codec::read(json.as_bytes(), usize::MAX).map_err(|e| format!("golden.json: {e}"))
}

/// Run every case natively and compare result hashes. Returns the number of
/// cases checked, or every mismatch with the hash actually produced.
pub fn check<G: Generator>(json: &str) -> Result<usize, String> {
    let file = parse(json)?;
    if file.cases.is_empty() {
        return Err("golden.json has no cases".into());
    }
    // The description must encode canonically on every call.
    let first = codec::encode(&G::describe()).map_err(|e| format!("describe: {e}"))?;
    let again = codec::encode(&G::describe()).map_err(|e| format!("describe: {e}"))?;
    if first != again {
        return Err("describe gave different bytes on a second call".into());
    }
    let mut problems = Vec::new();
    #[cfg(not(target_arch = "wasm32"))]
    {
        let outputs = parity::outputs::<G>()?;
        if outputs.digest() != file.outputs {
            problems.push(format!(
                "outputs: digest {}, golden.json has {}; by entry:\n  {}",
                outputs.digest(),
                file.outputs,
                outputs.lines().join("\n  ")
            ));
        }
    }
    for case in &file.cases {
        match crate::land_pattern::generate::<G>(&case.request) {
            Ok(generation) => match codec::encode(&generation) {
                Ok(bytes) => {
                    let actual = hex(&result_hash(&bytes));
                    if actual != case.result {
                        problems.push(format!(
                            "{}: result {actual}, golden.json has {}",
                            case.name, case.result
                        ));
                    }
                }
                Err(e) => problems.push(format!("{}: {e}", case.name)),
            },
            Err(errors) => problems.push(format!("{}: refused: {errors:?}", case.name)),
        }
    }
    if problems.is_empty() {
        Ok(file.cases.len())
    } else {
        Err(problems.join("\n"))
    }
}

/// The native side of the module parity check
/// ([`sparksmith_generator_abi::parity`]): `golden.json` also keeps
/// `outputs`, the digest of the generator's description and its reply to
/// the defaults and every preset in every scope. [`check`] compares the
/// native build with it here, and the host's module test compares the
/// module under wasmi with it, so the module says byte for byte what the
/// native build says. Native only, so the module's bytes do not depend on
/// it.
#[cfg(not(target_arch = "wasm32"))]
pub mod parity {
    use crate::Generator;
    use alloc::format;
    use alloc::string::String;
    use sparksmith_generator_abi::codec;
    use sparksmith_generator_abi::parity::{Outputs, cases};
    use sparksmith_generator_abi::v1::{self, ErrorBody};

    /// The record of `G` run natively: the description as
    /// `sparksmith_describe` returns it and the reply to each
    /// [`cases`] request as `sparksmith_invoke` returns it (status byte,
    /// then the part or the errors).
    pub fn outputs<G: Generator>() -> Result<Outputs, String> {
        let description = crate::land_pattern::declare(G::describe());
        let bytes = codec::encode(&description).map_err(|e| format!("describe: {e}"))?;
        let mut out = Outputs::new(&bytes);
        for case in cases(&description) {
            let (status, body) = match crate::land_pattern::generate::<G>(&case.request) {
                Ok(generation) => (v1::STATUS_OK, codec::encode(&generation)),
                Err(errors) => (v1::STATUS_ERR, codec::encode(&ErrorBody { errors })),
            };
            let body = body.map_err(|e| format!("{}: {e}", case.name))?;
            out.reply(&case.name, status, &body);
        }
        Ok(out)
    }
}
