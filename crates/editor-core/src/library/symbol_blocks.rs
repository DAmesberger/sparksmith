//! Rectangular symbol layouts for any number of terminals: the block
//! (pins down the left, then up the right), the column (pins down the left
//! only) and fitting a block's width to its pin names. Used by the pinout
//! import, the symbol styles and the KiCad import.
use super::graphics::{self, HAlign, Justify, PIN_TEXT_HEIGHT, ShapeNm, TextContent, VAlign};
use super::{SymbolDefinition, Unit};
use crate::diagnostics::Result;
use crate::geometry::Bounds;
use crate::ids::{DefinitionId, GraphicId, TerminalId, UnitId};
use crate::units::{Nm, PointNm};

/// A rectangle with the first half of `terminals` down its left side and
/// the rest up its right side, on the default symbol grid
/// ([`graphics::SYMBOL_MODULE_MM`]), with the designator above and the
/// name below.
pub fn block_symbol(component: DefinitionId, terminals: &[TerminalId]) -> Result<SymbolDefinition> {
    const STEP: f64 = graphics::SYMBOL_MODULE_MM;
    let left = terminals.len().div_ceil(2);
    let rows = left.max(1) as f64;
    let top = ((rows - 1.0) / 2.0).floor() * STEP;
    let body = Bounds {
        min: [-2.0 * STEP, top - rows * STEP],
        max: [2.0 * STEP, top + STEP],
    };
    let gap = 0.5;
    let pins = terminals
        .iter()
        .enumerate()
        .map(|(k, terminal)| {
            let position = if k < left {
                [-3.0 * STEP, top - k as f64 * STEP]
            } else {
                let j = terminals.len() - 1 - k;
                [3.0 * STEP, top - j as f64 * STEP]
            };
            graphics::pin_toward(*terminal, position, body)
        })
        .collect::<Result<_>>()?;
    Ok(SymbolDefinition {
        component,
        next_local: 5,
        pin_text_height_nm: PIN_TEXT_HEIGHT,
        units: vec![Unit {
            id: UnitId(1),
            label: "A".into(),
            graphics: vec![
                graphics::body_rect(GraphicId(2), body)?,
                graphics::text(
                    GraphicId(3),
                    TextContent::Designator,
                    [body.center()[0], body.max[1] + gap],
                    Justify {
                        h: HAlign::Center,
                        v: VAlign::Bottom,
                    },
                )?,
                graphics::text(
                    GraphicId(4),
                    TextContent::ComponentName,
                    [body.center()[0], body.min[1] - gap],
                    Justify {
                        h: HAlign::Center,
                        v: VAlign::Top,
                    },
                )?,
            ],
            pins,
        }],
    })
}

/// [`block_symbol`] wide enough for `terminals`' names (see
/// [`fit_pin_names`]).
pub fn named_block_symbol(
    component: DefinitionId,
    terminals: &[(TerminalId, &str)],
) -> Result<SymbolDefinition> {
    let ids: Vec<TerminalId> = terminals.iter().map(|(id, _)| *id).collect();
    let mut symbol = block_symbol(component, &ids)?;
    fit_pin_names(&mut symbol, |id| {
        terminals
            .iter()
            .find(|(t, _)| *t == id)
            .map(|(_, name)| (*name).to_owned())
    });
    Ok(symbol)
}

/// Width of one pin-name character in pin text heights: printed capitals,
/// as `schematic::scene::LABEL_METRIC` measures them, wider than the core
/// metric's average, so the fit holds in the PDF.
const PIN_NAME_EM: f64 = 0.95;
/// The least space between a left and a right pin name on one row, mm.
const PIN_NAME_GAP_MM: f64 = graphics::SYMBOL_MODULE_MM / 2.0;

/// Widen every unit's rectangular body so the names of its left and right
/// pins fit inside without meeting: on each row the two names, their
/// offsets from the edges and a half-module gap; a name alone reaches no
/// closer than the gap to the far edge. The body grows by whole modules on
/// both sides and the side pins move out with its edges, so connection
/// ends stay on the grid and the drawing stays centred. A unit without a
/// rectangle, or one already wide enough, is left as it is. Returns whether
/// anything moved. `name` gives a terminal's name (`None` hides nothing:
/// the pin is treated as unnamed).
pub fn fit_pin_names(
    symbol: &mut SymbolDefinition,
    name: impl Fn(TerminalId) -> Option<String>,
) -> bool {
    const STEP: f64 = graphics::SYMBOL_MODULE_MM;
    let em = PIN_NAME_EM * symbol.pin_text_height_nm.to_mm_f64();
    let mut changed = false;
    for unit in &mut symbol.units {
        let Some((index, body)) =
            unit.graphics
                .iter()
                .enumerate()
                .find_map(|(i, g)| match &g.item {
                    super::Item::Shape {
                        shape: ShapeNm::Rect { a, b, .. },
                        ..
                    } => Some((i, Bounds::of([a.to_mm_f64(), b.to_mm_f64()])?)),
                    _ => None,
                })
        else {
            continue;
        };
        // Name width plus its offset from the edge, per side, per row.
        let mut rows: Vec<(f64, f64, f64)> = Vec::new();
        for pin in &unit.pins {
            let position = pin.position_mm();
            let left =
                pin.direction == graphics::PinDirection::Right && position[0] <= body.min[0] + 1e-9;
            let right =
                pin.direction == graphics::PinDirection::Left && position[0] >= body.max[0] - 1e-9;
            if !(left || right) || !pin.name.visible {
                continue;
            }
            let Some(text) = name(pin.terminal) else {
                continue;
            };
            let width = text.chars().count() as f64 * em + pin.name.offset_nm.to_mm_f64().max(0.0);
            let y = position[1];
            let row = match rows.iter_mut().find(|r| (r.0 - y).abs() < 1e-9) {
                Some(row) => row,
                None => {
                    rows.push((y, 0.0, 0.0));
                    let Some(row) = rows.last_mut() else {
                        continue;
                    };
                    row
                }
            };
            if left {
                row.1 = row.1.max(width);
            } else {
                row.2 = row.2.max(width);
            }
        }
        let need = rows
            .iter()
            .map(|(_, l, r)| l + r + PIN_NAME_GAP_MM)
            .fold(0.0, f64::max);
        let width = body.max[0] - body.min[0];
        if need <= width + 1e-9 {
            continue;
        }
        let grow = (((need - width) / 2.0) / STEP).ceil() * STEP;
        if !grow.is_finite() || grow <= 0.0 || grow > 100.0 * STEP {
            continue;
        }
        let (min_x, max_x) = (body.min[0], body.max[0]);
        // Whole modules, exact in nanometres; a unit that would leave the
        // coordinate bound is left as it is.
        let Ok(grow) = Nm::from_mm_f64_rounded(grow) else {
            continue;
        };
        let moved = |x: Nm, keep_inner: bool| -> Result<Nm> {
            let mm = x.to_mm_f64();
            if mm <= min_x + 1e-9 {
                x.checked_sub(grow)
            } else if !keep_inner || mm >= max_x - 1e-9 {
                x.checked_add(grow)
            } else {
                Ok(x)
            }
        };
        let corners = match unit.graphics.get(index).map(|g| &g.item) {
            Some(super::Item::Shape {
                shape: ShapeNm::Rect { a, b, .. },
                ..
            }) => [*a, *b]
                .into_iter()
                .map(|c| Ok(PointNm::new(moved(c.x_nm, false)?, c.y_nm)))
                .collect::<Result<Vec<_>>>(),
            _ => Ok(Vec::new()),
        };
        let pins = unit
            .pins
            .iter()
            .map(|p| moved(p.position.x_nm, true))
            .collect::<Result<Vec<_>>>();
        let (Ok(corners), Ok(pins)) = (corners, pins) else {
            continue;
        };
        if let Some(graphic) = unit.graphics.get_mut(index)
            && let super::Item::Shape {
                shape: ShapeNm::Rect { a, b, .. },
                ..
            } = &mut graphic.item
            && let [new_a, new_b] = corners[..]
        {
            *a = new_a;
            *b = new_b;
        }
        for (pin, x) in unit.pins.iter_mut().zip(pins) {
            pin.position.x_nm = x;
        }
        changed = true;
    }
    changed
}

/// A narrow body with every terminal down its left side in package order,
/// on the default symbol grid (a connector's starting point).
pub fn column_symbol(
    component: DefinitionId,
    terminals: &[TerminalId],
) -> Result<SymbolDefinition> {
    const STEP: f64 = graphics::SYMBOL_MODULE_MM;
    let mut symbol = block_symbol(component, &[])?;
    let rows = terminals.len().max(1) as f64;
    let body = Bounds {
        min: [-STEP, -rows * STEP],
        max: [STEP, STEP],
    };
    let gap = 0.5;
    if let Some(unit) = symbol.units.first_mut() {
        unit.graphics = vec![
            graphics::body_rect(GraphicId(2), body)?,
            graphics::text(
                GraphicId(3),
                TextContent::Designator,
                [body.center()[0], body.max[1] + gap],
                Justify {
                    h: HAlign::Center,
                    v: VAlign::Bottom,
                },
            )?,
            graphics::text(
                GraphicId(4),
                TextContent::ComponentName,
                [body.center()[0], body.min[1] - gap],
                Justify {
                    h: HAlign::Center,
                    v: VAlign::Top,
                },
            )?,
        ];
        unit.pins = terminals
            .iter()
            .enumerate()
            .map(|(k, terminal)| {
                graphics::pin_toward(*terminal, [-2.0 * STEP, -(k as f64) * STEP], body)
            })
            .collect::<Result<_>>()?;
    }
    Ok(symbol)
}
