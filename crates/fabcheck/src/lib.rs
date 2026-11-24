//! Fabrication output read back, for verification: a Gerber X2 reader
//! ([`gerber`]) and an Excellon reader ([`excellon`]) written from Ucamco's
//! public Gerber Layer Format Specification and the public Excellon format
//! description, a bitmap rasteriser ([`raster`]), exact polygon rendering in
//! integer nanometres ([`polygons`]), a layer comparison reporting the
//! difference area and its widest piece ([`compare`]) and drill-hit pairing
//! ([`drills`]), board profiles compared contour by contour ([`outline`]),
//! the spacing of fill copper measured in the files ([`spacing`]), whole
//! sets paired by file function ([`sets`]), tables read back as records
//! ([`csv`]), the drawings' text and circles ([`pdf`]), and, with the
//! feature `reference`, reference files from an installed `kicad-cli` run
//! as a black-box oracle on a temporary copy ([`reference`]).
//!
//! No other tool's reader or writer was consulted. The readers ship: every
//! export reads its own files back with them (`sparksmith_core::fabrication::
//! readback`). So what they read may be anything: they refuse, and do not
//! panic. `reference` is for tests and `xtask` only.
pub mod compare;
pub mod csv;
pub mod drills;
pub mod excellon;
pub mod gerber;
pub mod outline;
pub mod pdf;
pub mod polygons;
pub mod raster;
#[cfg(feature = "reference")]
pub mod reference;
pub mod sets;
pub mod spacing;

/// The error type of this crate's fallible functions.
pub type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// A point in millimetres.
pub type Point = [f64; 2];
