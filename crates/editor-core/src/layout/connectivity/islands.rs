//! What each island holds, and the copper that meets a pad: the questions
//! Update PCB asks of the updated board's graph.
use super::{CopperGraph, Dsu, ElementKind};
use crate::geometry::{Point, polygon_contains};
use crate::ids::Id;
use crate::layout::fill::FILL_CHORD_MM;
use crate::model::fill::FillRegion;
use crate::model::{Project, Track, Via};
use std::collections::{BTreeMap, BTreeSet};

/// The pads and copper of one island.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Island {
    /// The net of its copper: every element of an island shares one.
    pub net: Option<Id>,
    /// Its pads, as indices into [`CopperGraph::pads`], ascending.
    pub pads: Vec<usize>,
    /// Its tracks, in document order.
    pub tracks: Vec<Id>,
    /// Its vias, in document order.
    pub vias: Vec<Id>,
}

impl CopperGraph {
    /// Every island by number ([`Self::island_of`]): the pads it holds and
    /// the tracks and vias in it.
    pub fn islands(&self) -> Vec<Island> {
        let mut out = vec![Island::default(); self.islands];
        let island = |element: usize| self.island_of(self.vertices_of(element).start);
        for (element, e) in self.elements.iter().enumerate() {
            let Some(slot) = island(element).and_then(|i| out.get_mut(i)) else {
                continue;
            };
            slot.net = Some(e.net);
            if let ElementKind::Track { index } = e.kind
                && let Some(track) = self.tracks.get(index)
            {
                slot.tracks.push(track.id);
            }
        }
        for (via, lands) in self.via_lands.iter().enumerate() {
            if let Some(v) = self.vias.get(via)
                && let Some(slot) = island(lands.start).and_then(|i| out.get_mut(i))
            {
                slot.vias.push(v.id);
            }
        }
        for (pad, element) in self.pad_lands.iter().enumerate() {
            if let Some(slot) = island(element.start).and_then(|i| out.get_mut(i)) {
                slot.pads.push(pad);
            }
        }
        out
    }

    /// How many groups the pads of `net` fall into when each fill of the
    /// net in `project` joins the copper within its outline, on a graph
    /// built without fills. What a refill will join, as far as its outline
    /// says: copper it has to leave out between is not seen.
    pub fn pad_groups(&self, project: &Project, net: Id) -> usize {
        let layers = project.stackup.layers();
        let board = project.board();
        let pours: Vec<_> = project
            .fills
            .iter()
            .filter(|f| f.net == Some(net))
            .map(|f| {
                // One that follows the board takes its outline after the
                // edit; it is there already.
                let outline = match f.region {
                    FillRegion::Board => &board.outer,
                    FillRegion::Drawn => &f.outline,
                };
                (f.layer, outline.flatten(FILL_CHORD_MM))
            })
            .collect();
        let islands: Vec<Island> = self
            .islands()
            .into_iter()
            .filter(|i| i.net == Some(net))
            .collect();
        let tracks: BTreeMap<Id, &Track> = self.tracks.iter().map(|t| (t.id, t)).collect();
        let vias: BTreeMap<Id, &Via> = self.vias.iter().map(|v| (v.id, v)).collect();
        let mut dsu = Dsu::new(islands.len() + pours.len());
        for (i, island) in islands.iter().enumerate() {
            for (k, (layer, ring)) in pours.iter().enumerate() {
                let within = |p: Point| polygon_contains(ring, p);
                let met = island
                    .pads
                    .iter()
                    .filter_map(|p| self.pads.get(*p))
                    .any(|p| p.on(*layer) && within(p.position))
                    || island
                        .vias
                        .iter()
                        .filter_map(|id| vias.get(id))
                        .any(|v| v.on(*layer, &layers) && within(v.position))
                    || island
                        .tracks
                        .iter()
                        .filter_map(|id| tracks.get(id))
                        .any(|t| t.layer == *layer && (within(t.start) || within(t.end)));
                if met {
                    dsu.union(i, islands.len() + k);
                }
            }
        }
        let roots: BTreeSet<usize> = islands
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.pads.is_empty())
            .map(|(i, _)| dsu.find(i))
            .collect();
        roots.len()
    }

    /// The tracks and vias whose copper meets the pad at `index` in
    /// [`Self::pads`] directly, on any of its lands, in document order.
    pub fn copper_at_pad(&self, index: usize) -> (Vec<Id>, Vec<Id>) {
        let (mut tracks, mut vias): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
        let Some(lands) = self.pad_lands.get(index) else {
            return (Vec::new(), Vec::new());
        };
        for land in lands.clone() {
            for vertex in self.vertices_of(land) {
                for &other in self.neighbours(vertex) {
                    let kind = self
                        .vertices
                        .get(other)
                        .and_then(|v| self.elements.get(v.element))
                        .map(|e| e.kind);
                    match kind {
                        Some(ElementKind::Track { index }) => tracks.push(index),
                        Some(ElementKind::Via { index }) => vias.push(index),
                        _ => {}
                    }
                }
            }
        }
        for list in [&mut tracks, &mut vias] {
            list.sort_unstable();
            list.dedup();
        }
        (
            tracks
                .iter()
                .filter_map(|i| self.tracks.get(*i).map(|t| t.id))
                .collect(),
            vias.iter()
                .filter_map(|i| self.vias.get(*i).map(|v| v.id))
                .collect(),
        )
    }
}
