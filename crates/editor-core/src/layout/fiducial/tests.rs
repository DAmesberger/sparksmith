//! Hand-derived geometry of the fiducial checks; the checks on generated
//! parts through the session are in `crates/automation`
//! (`session/fiducial_tests.rs`).
use super::*;

/// About a board centred on (50, 40): two diagonal marks are symmetric,
/// an L of three is not, four at the corners of a centred box are, and
/// moving one corner by 1 mm breaks it. A mark 0.5 µm off its image still
/// counts as on it; 2 µm does not.
#[test]
fn symmetry_is_a_half_turn_about_the_board_centre() {
    let c = [50.0, 40.0];
    assert!(half_turn_symmetric(&[[30.0, 25.0], [70.0, 55.0]], c));
    assert!(!half_turn_symmetric(
        &[[30.0, 25.0], [70.0, 55.0], [30.0, 55.0]],
        c
    ));
    let corners = [[30.0, 25.0], [70.0, 25.0], [70.0, 55.0], [30.0, 55.0]];
    assert!(half_turn_symmetric(&corners, c));
    let mut shifted = corners;
    shifted[3] = [31.0, 55.0];
    assert!(!half_turn_symmetric(&shifted, c));
    assert!(half_turn_symmetric(&[[30.0, 25.0], [70.0, 55.0005]], c));
    assert!(!half_turn_symmetric(&[[30.0, 25.0], [70.0, 55.002]], c));
}

/// A 0.15 mm stroke 1.1 mm from the centre of a clear area of radius 1 mm
/// (a square of side 2 here) stays out: its edge is 0.025 mm off. At
/// 1.05 mm its half width reaches 0.025 mm in.
#[test]
fn a_stroke_enters_by_its_half_width() {
    let ring = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
    assert!(!stroke_enters(&[[1.1, -2.0], [1.1, 2.0]], 0.15, &ring));
    assert!(stroke_enters(&[[1.05, -2.0], [1.05, 2.0]], 0.15, &ring));
    // Wholly inside.
    assert!(stroke_enters(&[[0.0, 0.0], [0.2, 0.0]], 0.1, &ring));
    assert!(!stroke_enters(&[], 0.1, &ring));
}
