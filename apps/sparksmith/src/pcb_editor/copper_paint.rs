//! Layout's copper is the board plan's ([`plan::copper`]): the same pad
//! lands, track stadiums, via rings and fill tiles the 3D view meshes and
//! the exact build builds, layer by layer ([`BoardPlan::layer_copper`]).
//! Layout only flattens their loops for drawing, once per revision, and
//! derives no copper of its own, so the 2D and 3D copper cannot differ
//! (`cargo xtask copper-source` keeps it so).
use sparksmith_core::geometry::Shape as CoreShape;
use sparksmith_core::model::fill::FilledRegion;
use sparksmith_core::{CopperLayer, Id, PadId, Point, Project};
use sparksmith_solidcraft::material::BodyKind;
use sparksmith_solidcraft::pad_outline::{Contour, FLATTEN_TOLERANCE_MM};
use sparksmith_solidcraft::project::plan::{self, BoardPlan, Origin, PlanBody};
use std::collections::HashMap;

/// One fill's copper as the plan has it: the tiles the 3D view meshes (the
/// copper), and the fill's regions (their edges, for outlines and hatching).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct FillCopper {
    pub tiles: Vec<FilledRegion>,
    pub regions: Vec<FilledRegion>,
}

/// The board's copper per revision, flattened for drawing.
#[derive(Default)]
pub(super) struct LayoutCopper {
    /// Each pad's land on every layer it has copper on.
    pads: HashMap<(Id, PadId), Vec<(CopperLayer, CoreShape)>>,
    /// Each track's stadium.
    tracks: HashMap<Id, Vec<Point>>,
    /// Each via's ring per layer: centre and outer radius.
    vias: HashMap<Id, Vec<(CopperLayer, Point, f64)>>,
    fills: HashMap<Id, FillCopper>,
    memo: plan::Memo,
    /// Why the plan of the last revision was refused (nothing drawn).
    error: Option<String>,
}

impl LayoutCopper {
    /// Take the copper of `project`'s board from the plan.
    pub(super) fn update(&mut self, project: &Project) {
        self.pads.clear();
        self.tracks.clear();
        self.vias.clear();
        self.fills.clear();
        let plan = match plan::copper(project, &mut self.memo) {
            Ok(plan) => plan,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        self.error = None;
        self.take(&plan);
    }

    fn take(&mut self, plan: &BoardPlan) {
        for body in plan.bodies() {
            let Some(layer) = body.layer else {
                continue;
            };
            match body.source {
                Origin::Pad { instance, pad, .. } => {
                    if let Some(ring) = outer_ring(body) {
                        self.pads
                            .entry((instance, pad))
                            .or_default()
                            .push((layer, CoreShape::polygon(ring)));
                    }
                }
                Origin::Track(id) => {
                    if let Some(ring) = outer_ring(body) {
                        self.tracks.insert(id, ring);
                    }
                }
                Origin::Via { via, .. } => {
                    if let Some(Contour::Circle { center, radius }) =
                        body.board_loops().first().cloned()
                    {
                        self.vias
                            .entry(via)
                            .or_default()
                            .push((layer, center, radius));
                    }
                }
                Origin::Fill { .. } | Origin::Laminate(_) => {}
            }
        }
        for fill in &plan.fills {
            self.fills.insert(
                fill.fill,
                FillCopper {
                    tiles: fill.tiles.iter().map(|(_, r)| r.clone()).collect(),
                    regions: fill.regions.clone(),
                },
            );
        }
    }

    /// The pad's land on `layer`, if it has copper there.
    pub(super) fn land(&self, instance: Id, pad: PadId, layer: CopperLayer) -> Option<&CoreShape> {
        self.pads
            .get(&(instance, pad))?
            .iter()
            .find(|(l, _)| *l == layer)
            .map(|(_, shape)| shape)
    }

    /// The pad's lands, by layer.
    pub(super) fn lands(&self, instance: Id, pad: PadId) -> &[(CopperLayer, CoreShape)] {
        self.pads
            .get(&(instance, pad))
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The track's copper outline.
    pub(super) fn track(&self, id: Id) -> Option<&[Point]> {
        self.tracks.get(&id).map(Vec::as_slice)
    }

    /// The via's ring on `layer`: centre and outer radius.
    pub(super) fn via_ring(&self, id: Id, layer: CopperLayer) -> Option<(Point, f64)> {
        self.vias
            .get(&id)?
            .iter()
            .find(|(l, ..)| *l == layer)
            .map(|(_, c, r)| (*c, *r))
    }

    /// The via's largest ring on any layer: centre and outer radius.
    pub(super) fn via_extent(&self, id: Id) -> Option<(Point, f64)> {
        self.vias
            .get(&id)?
            .iter()
            .map(|(_, c, r)| (*c, *r))
            .max_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The fill's copper.
    pub(super) fn fill(&self, id: Id) -> Option<&FillCopper> {
        self.fills.get(&id)
    }

    /// Why the board's copper could not be drawn, if it could not.
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

/// A copper body's outer loop on the board, flattened (its holes are
/// drills, which Layout draws over the copper).
fn outer_ring(body: &PlanBody) -> Option<Vec<Point>> {
    if !matches!(
        body.kind,
        BodyKind::PadCopper | BodyKind::CopperTrack | BodyKind::ViaCopper
    ) {
        return None;
    }
    let outer = body.loops.first()?;
    let ring = body
        .frame
        .contour(outer)
        .flatten(FLATTEN_TOLERANCE_MM)
        .ok()?;
    (ring.len() >= 3).then_some(ring)
}
