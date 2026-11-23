//! The chip generator: two-terminal chip parts (resistors, capacitors,
//! inductors, ferrite beads and LEDs) as a symbol, package, footprint and
//! 3D model.
//!
//! Lands follow the host's estimate for chip packages (an original rule of
//! thumb, not a land-pattern standard): a land of termination + 0.6 mm by
//! width + 0.2 mm centred on each termination, its heel trimmed so the two
//! lands stay the project's least chip land gap apart. A preset may give the
//! manufacturer's recommended lands instead (`lands: explicit`).
//!
//! The presets are the chip starter table, in nm, with its citations.
#![cfg_attr(target_arch = "wasm32", no_std)]

include!("family.rs");
use sparksmith_generator_sdk::{Generator, export_generator};
pub struct Chip;
export_generator!(Chip);
impl Generator for Chip {
    fn describe() -> Description {
        description()
    }

    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        generate(request)
    }
}

#[cfg(test)]
mod tests;
