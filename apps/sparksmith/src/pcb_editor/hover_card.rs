//! The hover card ([`crate::about`]) of Layout and PCB 3D: what is under the
//! pointer, as a click there would pick it, and where it is on screen. Found
//! again only when the pointer, the view or the document changed.
use super::group::{Pick, allows};
use super::{BoardView, Layers, PcbEditor, Tool, body_polygon, menu::Hit};
use crate::about::{Card, Subject, Target};
use crate::kit::hover_card::{Content, Over};
use crate::scene::SceneController;
use egui::{Pos2, Rect, Response, Ui};
use sparksmith_core::library::graphics::TextMetric;
use sparksmith_core::{CopperLayer, Engine, Id, Point, Project};

/// What the hovered object was found for.
#[derive(Clone, PartialEq)]
pub(super) struct At {
    point: Point,
    scale: f32,
    revision: u64,
    layer: CopperLayer,
    layers: Layers,
    filter: super::group::SelectFilter,
}

/// Where a subject lies on the board, mm.
#[derive(Clone, Debug)]
enum Place {
    Corners(Vec<Point>),
    Path([Point; 2]),
}

#[derive(Default)]
pub(super) struct HoverCard {
    pub(super) card: Card,
    found: Option<(At, Option<(Subject, Place)>)>,
    /// The drawn dimension under the pointer this frame, its label and what
    /// it measures.
    pub(super) dimension: Option<(Id, Rect, Content)>,
}

fn around(centre: Point, r: f64) -> Place {
    Place::Corners(vec![
        [centre[0] - r, centre[1] - r],
        [centre[0] + r, centre[1] + r],
    ])
}

impl PcbEditor {
    /// What a click at `at` would pick, as a card's subject, and where it
    /// is.
    fn hover_subject(
        &self,
        project: &Project,
        at: Point,
        tolerance: f64,
        metric: TextMetric,
    ) -> Option<(Subject, Place)> {
        let subject = match self.hit_at(project, at, tolerance, metric) {
            Hit::Pad(instance, _) => {
                let layers = self.seen();
                match self.pick_pad(at, |p| layers.pad(p)) {
                    Some(pad) => Subject::Pad {
                        instance,
                        pad: pad.pad.id,
                    },
                    None => Subject::Part(instance),
                }
            }
            Hit::Part(id) | Hit::Tie(id, _) => Subject::Part(id),
            Hit::Via(id, _) => Subject::Via(id),
            Hit::Track(id, _) => Subject::Track(id),
            Hit::Hole(_)
            | Hit::Fill(..)
            | Hit::Block(_)
            | Hit::Keepout(_)
            | Hit::BoardText(_)
            | Hit::Nothing => {
                return None;
            }
        };
        let place = self.place_of(project, subject)?;
        // A part's drawing may reach past its body and pads.
        Some(match place {
            Place::Corners(mut corners) => {
                corners.push(at);
                (subject, Place::Corners(corners))
            }
            path => (subject, path),
        })
    }

    fn place_of(&self, project: &Project, subject: Subject) -> Option<Place> {
        let pads = |instance: Id| {
            self.board_pads
                .iter()
                .filter(move |p| p.placed.instance == instance)
        };
        let corners = |b: sparksmith_core::geometry::Bounds| [b.min, b.max];
        Some(match subject {
            Subject::Part(id) => {
                let placement = project.placement_of(id)?;
                let mut points: Vec<Point> = body_polygon(project, placement)
                    .into_iter()
                    .flatten()
                    .collect();
                points.extend(pads(id).flat_map(|p| corners(p.outline.bounds(0.0))));
                if points.is_empty() {
                    points.push(placement.position);
                }
                Place::Corners(points)
            }
            Subject::Pad { instance, pad } => {
                let pad = pads(instance).find(|p| p.placed.pad.id == pad)?;
                Place::Corners(corners(pad.outline.bounds(0.0)).to_vec())
            }
            Subject::Via(id) => {
                let via = project.via(id)?;
                let (centre, r) = self
                    .layout_copper
                    .via_extent(id)
                    .unwrap_or((via.position, via.diameter / 2.0));
                around(centre, r)
            }
            Subject::Track(id) => {
                let track = project.tracks.iter().find(|t| t.id == id)?;
                Place::Path([track.start, track.end])
            }
            Subject::Pin(_) | Subject::Wire { .. } | Subject::Dimension(_) => return None,
        })
    }

    /// The selected object, for a canvas that has the keyboard.
    fn selected_subject(&self, project: &Project) -> Option<(Subject, Place)> {
        if !self.group.is_empty() {
            return None;
        }
        let subject = match (self.instance, self.via, self.track) {
            (Some(id), ..) => Subject::Part(id),
            (None, Some(id), _) => Subject::Via(id),
            (None, None, Some(id)) => Subject::Track(id),
            _ => return None,
        };
        Some((subject, self.place_of(project, subject)?))
    }

    /// Whether a card may show: no tool, no pick and no drag runs.
    fn card_rests(&self, ui: &Ui) -> bool {
        self.tool == Tool::Select
            && self.anchor_pick.is_none()
            && self.box_from.is_none()
            && Card::rests(ui.ctx())
    }

    /// The drawn dimension under the pointer, as the card's target.
    fn dimension_target(&mut self) -> Option<Target> {
        let (id, rect, content) = self.hover_card.dimension.take()?;
        Some(Target {
            subject: Subject::Dimension(id),
            over: Over::Rect(rect),
            content: Some(content),
        })
    }

    /// Layout's card, after the board is drawn.
    pub(super) fn layout_hover_card(
        &mut self,
        ui: &Ui,
        engine: &Engine,
        view: &BoardView,
        response: &Response,
        (tolerance, metric): (f64, TextMetric),
        room: Rect,
    ) {
        let project = engine.project();
        let dimension = self.dimension_target();
        let rests = self.card_rests(ui);
        let pointer = response.hover_pos().filter(|_| rests && response.hovered());
        let found = match pointer {
            _ if !rests || dimension.is_some() => None,
            Some(p) => {
                let at = At {
                    point: view.world(p),
                    scale: view.view.scale,
                    revision: engine.revision(),
                    layer: self.layer,
                    layers: self.seen(),
                    filter: self.filter,
                };
                match self.hover_card.found.take() {
                    Some((was, found)) if was == at => self.hover_card.found = Some((was, found)),
                    _ => {
                        let found = self.hover_subject(project, at.point, tolerance, metric);
                        self.hover_card.found = Some((at, found));
                    }
                }
                self.hover_card.found.as_ref().and_then(|(_, f)| f.clone())
            }
            None if response.has_focus() => self.selected_subject(project),
            None => None,
        };
        let target = found.and_then(|(subject, place)| {
            let over = match place {
                Place::Corners(points) => Over::bounds(points.into_iter().map(|p| view.screen(p)))?,
                Place::Path(ends) => Over::Path(ends.map(|p| view.screen(p)).to_vec()),
            };
            Some(Target::new(subject, over))
        });
        self.hover_card.card.show(
            ui,
            project,
            engine.revision(),
            dimension.filter(|_| rests).or(target),
            pointer,
            room,
        );
    }

    /// PCB 3D's card, after the view is drawn: of the body the hover ray
    /// met ([`PcbEditor::hover_3d`]).
    pub(super) fn hover_card_3d(
        &mut self,
        ui: &Ui,
        scene: &SceneController,
        engine: &Engine,
        pointer: Option<Pos2>,
        (view, room): (Rect, Rect),
    ) {
        let dimension = self.dimension_target();
        let rests = self.card_rests(ui);
        let pointer = pointer.filter(|p| !crate::viewport::nav_cube::owns(view, *p));
        let hit = self
            .hover_3d_at
            .as_ref()
            .filter(|_| rests && pointer.is_some());
        let target = hit.and_then(|hit| {
            let body = hit.body.as_ref()?;
            let ([x0, y0, z0], [x1, y1, z1]) = body.bounds;
            let screen = |p: [f64; 3]| scene.viewport.to_screen(p);
            let over = match body.subject {
                Subject::Track(id) => {
                    let track = engine.project().tracks.iter().find(|t| t.id == id)?;
                    let z = (z0 + z1) / 2.0;
                    Over::Path(vec![
                        screen([track.start[0], track.start[1], z])?,
                        screen([track.end[0], track.end[1], z])?,
                    ])
                }
                _ => Over::bounds(
                    [
                        [x0, y0, z0],
                        [x1, y0, z0],
                        [x0, y1, z0],
                        [x1, y1, z0],
                        [x0, y0, z1],
                        [x1, y0, z1],
                        [x0, y1, z1],
                        [x1, y1, z1],
                    ]
                    .into_iter()
                    .filter_map(screen)
                    .chain(pointer),
                )?,
            };
            Some(Target::new(body.subject, over))
        });
        let target = match pointer {
            None if rests && scene.viewport.focused() => self.selected_in_3d(scene, engine),
            _ => target,
        };
        self.hover_card.card.show(
            ui,
            engine.project(),
            engine.revision(),
            dimension.filter(|_| rests).or(target),
            pointer,
            room,
        );
    }

    /// The selected object's card while PCB 3D has the keyboard.
    fn selected_in_3d(&self, scene: &SceneController, engine: &Engine) -> Option<Target> {
        let project = engine.project();
        let (subject, place) = self.selected_subject(project)?;
        let (_, z) = project.stackup.copper_span(CopperLayer::Top)?;
        let points = match place {
            Place::Corners(points) => points,
            Place::Path(ends) => ends.to_vec(),
        };
        let screen = |p: Point| scene.viewport.to_screen([p[0], p[1], z]);
        let over = Over::bounds(points.into_iter().filter_map(screen))?;
        Some(Target::new(subject, over))
    }

    /// What a 3D body is, as a card's subject, when a click would pick it.
    pub(super) fn body_subject(
        &self,
        source: &sparksmith_solidcraft::project::Source,
    ) -> Option<Subject> {
        let pick = Pick::of_body(source).filter(|p| allows(&self.filter, *p))?;
        Some(match (pick, source.pad) {
            (Pick::Part(instance), Some(pad)) if source.tie.is_none() => {
                Subject::Pad { instance, pad }
            }
            (Pick::Part(id), _) => Subject::Part(id),
            (Pick::Via(id), _) => Subject::Via(id),
            (Pick::Track(id), _) => Subject::Track(id),
            (Pick::Fill(_) | Pick::Block(_), _) => return None,
        })
    }
}

/// What PCB 3D's hover ray met.
pub(super) struct Hover3d {
    pub(super) at: Pos2,
    pub(super) pick: Option<Pick>,
    pub(super) body: Option<Body>,
}

/// A hovered body: what it is and its bounds in the model.
pub(super) struct Body {
    pub(super) subject: Subject,
    pub(super) bounds: ([f64; 3], [f64; 3]),
}

#[cfg(test)]
#[path = "hover_card_tests.rs"]
mod tests;
