//! Complete parameters: the host fills every default from the description
//! before it calls and before it hashes, so a default that changes in a
//! later module revision can never silently change a part's identity. The
//! completion is the ABI's own, the one the core runs too.
pub use sparksmith_generator_abi::params::{complete, fits};
