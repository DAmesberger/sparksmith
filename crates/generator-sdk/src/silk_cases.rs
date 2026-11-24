//! One pad on the top edge of a body, in every way its mask can open: where
//! the silkscreen outline stops beside it. Shared by the tests of both paths
//! that draw a footprint's silkscreen, the generator SDK's
//! (`land::silk_outline`) and the core's
//! (`library::footprint_drawings::silk_outline`). Lengths in nanometres.

/// The body, centred on the origin.
pub const BODY: [i64; 2] = [4_000_000, 2_000_000];
/// The pad's centre: on the outline's top line.
pub const PAD_AT: [i64; 2] = [0, TOP_LINE_Y];
pub const PAD_SIZE: [i64; 2] = [1_000_000, 500_000];
/// The outline's top line, 0.15 mm outside the body.
pub const TOP_LINE_Y: i64 = 1_150_000;

/// How the pad's mask and paste open.
#[derive(Clone, Copy)]
pub enum Mask {
    /// The footprint's default mask expansion.
    Inherit,
    /// The pad's own expansion.
    Expansion(i64),
    /// No opening.
    Tented,
    /// An inherited mask opening with paste in windows.
    PasteWindows,
}

pub struct Case {
    pub name: &'static str,
    pub mask: Mask,
    /// The default mask expansion.
    pub default_nm: i64,
    /// The outline's line stops this far either side of the pad's centre:
    /// half the pad (0.5 mm), what of the mask opening reaches beyond it,
    /// 0.1 mm clearance and half the 0.15 mm stroke.
    pub half_nm: i64,
}

const fn case(name: &'static str, mask: Mask, default_nm: i64, half_nm: i64) -> Case {
    Case {
        name,
        mask,
        default_nm,
        half_nm,
    }
}

pub const CASES: [Case; 9] = [
    case("ordinary, inherited", Mask::Inherit, 50_000, 725_000),
    case(
        "ordinary, its own",
        Mask::Expansion(120_000),
        50_000,
        795_000,
    ),
    case("opening equal to the pad", Mask::Inherit, 0, 675_000),
    case("shrunk, inherited", Mask::Inherit, -50_000, 675_000),
    case("shrunk, its own", Mask::Expansion(-80_000), 50_000, 675_000),
    case(
        "shrunk to nothing",
        Mask::Expansion(-600_000),
        50_000,
        675_000,
    ),
    case("tented", Mask::Tented, 50_000, 675_000),
    case("tented, shrunk default", Mask::Tented, -50_000, 675_000),
    case("paste windows", Mask::PasteWindows, 50_000, 725_000),
];
