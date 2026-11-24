//! A committed route is validated against an index of the copper near it
//! alone ([`world::RouteIndex::near`]); it must refuse exactly what the
//! whole board's index refuses, with the same words, for pieces anywhere
//! on the routed showcase.
use super::*;

/// A small deterministic generator (xorshift64*), so a failure replays.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn committed_routes_are_refused_as_the_whole_board_refuses_them() -> Fallible {
    let engine = Engine::showcase()?;
    let _context = engine.enter_context();
    let project = engine.project();
    let whole = world::RouteIndex::of(project);
    let stack = project.stackup.layers();
    let nets: Vec<Id> = project.nets.iter().map(|n| n.id).collect();
    let [w, h] = project.board_size;
    let mut rng = Rng(0x0dd_ba11_cafe_f00d);
    let (mut refused, mut clear) = (0, 0);
    for k in 0..400 {
        let net = nets[(rng.next() * nets.len() as f64) as usize % nets.len()];
        let at = [1.0 + rng.next() * (w - 2.0), 1.0 + rng.next() * (h - 2.0)];
        let mut pieces = Vec::new();
        if k % 3 == 0 {
            let via = Via {
                id: 0,
                net,
                position: at,
                drill: 0.3,
                diameter: 0.6,
                span: crate::model::ViaSpan::Through,
                tented: true,
                fill: Default::default(),
                locked: false,
            };
            pieces.push(world::Piece::via(&via, &stack));
        } else {
            let to = [
                at[0] + rng.next() * 6.0 - 3.0,
                at[1] + rng.next() * 6.0 - 3.0,
            ];
            let layer = stack[(rng.next() * stack.len() as f64) as usize % stack.len()];
            pieces.push(world::Piece::track(
                net,
                layer,
                at,
                to,
                0.1 + rng.next() * 0.5,
            ));
        }
        let area = pieces
            .iter()
            .filter_map(|p| p.ring.bounds)
            .reduce(crate::geometry::Bounds::union)
            .ok_or("no bounds")?;
        let near = world::RouteIndex::near(project, &area);
        let a = check_pieces(project, &whole, &pieces).map_err(|e| e.to_string());
        let b = check_pieces(project, &near, &pieces).map_err(|e| e.to_string());
        assert_eq!(a, b, "piece {k}");
        if a.is_err() { refused += 1 } else { clear += 1 }
    }
    assert!(
        refused > 50 && clear > 20,
        "{refused} refused, {clear} clear"
    );
    Ok(())
}
