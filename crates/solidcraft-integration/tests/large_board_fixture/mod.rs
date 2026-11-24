//! The large board of the scale benchmarks, built in code (original: a
//! grid of the showcase's parts, a via field and routed channels; no real
//! design). Shared by `large_board.rs` here and the automation crate's
//! `large_board_reply.rs`.
#![allow(dead_code)]
use sparksmith_core::layout;
use sparksmith_core::model::{Layer, Placement, Stackup};
use sparksmith_core::{Engine, Id, Project, Track, Via, ViaSpan};

pub type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Via field: `VIA_GRID` × `VIA_GRID` through vias at `PITCH`.
pub const VIA_GRID: usize = 100;
pub const PITCH: f64 = 1.27;
/// Clear of the showcase's own parts and copper, which stay where they are.
pub const VIA_ORIGIN: [f64; 2] = [80.0, 10.0];
/// Routed channels between via rows, on every copper layer, of segments
/// two pitches long.
pub const SEGMENT_PITCHES: usize = 2;
pub const TRACK_WIDTH: f64 = 0.15;
pub const PLACEMENTS: usize = 2_000;
/// Placement grid pitch, mm; above the via field.
pub const PART_PITCH: f64 = 9.0;
pub const PART_ORIGIN: [f64; 2] = [12.0, 150.0];
pub const BOARD: [f64; 2] = [500.0, 560.0];

/// The large board: the showcase's 4-layer copy with its small parts
/// repeated `PLACEMENTS` times, `VIA_GRID`² vias on two nets and about
/// 20,000 tracks in the channels between them. Legal by construction:
/// via gap 0.67 mm, track to via 0.26 mm, parts 9 mm apart.
pub fn large_board() -> Fallible<Project> {
    let mut p = Engine::showcase()?.project().clone();
    p.stackup = Stackup::preset(4)?;
    p.board_size = BOARD;
    for fill in p.fills.iter_mut() {
        fill.computed = None;
    }
    let nets: Vec<Id> = p.nets.iter().map(|n| n.id).collect();
    let [a, b] = [nets[0], nets[1 % nets.len()]];
    // Parts: the showcase's placed parts whose pads fit a pitch cell.
    let pads = layout::pads(&p);
    let template: Vec<Placement> = p
        .placements
        .iter()
        .filter(|placement| {
            pads.iter()
                .filter(|pad| pad.instance == placement.instance)
                .all(|pad| {
                    (pad.position[0] - placement.position[0]).abs() < PART_PITCH * 0.3
                        && (pad.position[1] - placement.position[1]).abs() < PART_PITCH * 0.3
                })
        })
        .cloned()
        .collect();
    assert!(!template.is_empty(), "no small parts to repeat");
    let columns = ((BOARD[0] - 2.0 * PART_ORIGIN[0]) / PART_PITCH) as usize;
    for k in 0..PLACEMENTS {
        let source = &template[k % template.len()];
        let mut instance = p
            .instance(source.instance)
            .ok_or("template instance")?
            .clone();
        let id = p.allocate()?;
        instance.id = id;
        instance.designator = format!("{}S{k}", instance.designator);
        p.instances.push(instance);
        p.placements.push(Placement {
            instance: id,
            position: [
                PART_ORIGIN[0] + (k % columns) as f64 * PART_PITCH,
                PART_ORIGIN[1] + (k / columns) as f64 * PART_PITCH,
            ],
            rotation_deg: 0.0,
            layer: Layer::Top,
            locked: false,
            fill_override: Default::default(),
            pad_fill_overrides: Vec::new(),
        });
    }
    // Each new part's terminals start on nets of their own, as placing a
    // part from the library leaves them.
    sparksmith_core::nets::reconcile(&mut p)?;
    for i in 0..VIA_GRID {
        for j in 0..VIA_GRID {
            let id = p.allocate()?;
            p.vias.push(Via {
                id,
                net: if (i + j) % 2 == 0 { a } else { b },
                position: [
                    VIA_ORIGIN[0] + i as f64 * PITCH,
                    VIA_ORIGIN[1] + j as f64 * PITCH,
                ],
                drill: 0.3,
                diameter: 0.6,
                span: ViaSpan::Through,
                tented: true,
                fill: Default::default(),
                locked: false,
            });
        }
    }
    let layers = p.stackup.layers();
    let length = SEGMENT_PITCHES as f64 * PITCH;
    for (l, layer) in layers.iter().enumerate() {
        for row in 0..VIA_GRID - 1 {
            let y = VIA_ORIGIN[1] + (row as f64 + 0.5) * PITCH;
            let net = if (row + l) % 2 == 0 { a } else { b };
            for s in 0..(VIA_GRID - 1) / SEGMENT_PITCHES {
                let x = VIA_ORIGIN[0] + s as f64 * length;
                let id = p.allocate()?;
                p.tracks.push(Track {
                    id,
                    net,
                    start: [x, y],
                    end: [x + length, y],
                    width: TRACK_WIDTH,
                    layer: *layer,
                    locked: false,
                });
            }
        }
    }
    Ok(p)
}
