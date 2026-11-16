//! Where a placed symbol goes on its sheet when `component.place` names
//! no position, and the check that a named position lies on the page: a
//! symbol outside the frame is not seen at the sheet's fit.
use crate::diagnostics::{Error, Result};
use crate::geometry::{Bounds, Point};
use crate::ids::{Id, SheetId};
use crate::library::TextMetric;
use crate::model::{PaperOrientation, Project, Sheet};
use crate::units::{Nm, PointNm};

/// The schematic grid a found spot lies on, nm (0.1 in).
const GRID_NM: i64 = 2_540_000;
/// The step between the spots tried, in grid units.
const STEP_GRIDS: i64 = 2;
/// Space kept free inside the page edge, mm (the frame's border and
/// zone marks).
const MARGIN_MM: f64 = 15.0;
/// Space kept free at the bottom, mm, where the title block sits.
const TITLE_BLOCK_MM: f64 = 45.0;
/// Clearance kept around every symbol, mm.
const CLEARANCE_MM: f64 = 5.08;
/// Spots tried at most, so a huge sheet stays bounded.
const MAX_SPOTS: usize = 200_000;

/// The page's extent on the sheet, mm: `[min, max]` with y up.
fn page(sheet: &Sheet) -> Bounds {
    let [left, top] = sheet.top_left();
    let [w, h] = sheet.size_mm();
    Bounds {
        min: [left, top - h],
        max: [left + w, top],
    }
}

/// The sheet's name, paper and orientation, as a person reads them.
fn described(sheet: &Sheet) -> String {
    let orientation = match sheet.orientation {
        PaperOrientation::Landscape => "landscape",
        PaperOrientation::Portrait => "portrait",
    };
    format!("{} ({} {orientation})", sheet.name, sheet.paper.label())
}

fn sheet_of(project: &Project, sheet: SheetId) -> Result<&Sheet> {
    project
        .sheet(sheet)
        .ok_or_else(|| Error::NotFound("Unknown sheet.".into()))
}

/// Refuse `position` when it lies outside `sheet`'s page frame, naming the
/// frame.
pub fn check_on_page(project: &Project, sheet: SheetId, position: PointNm) -> Result<()> {
    let sheet = sheet_of(project, sheet)?;
    let frame = page(sheet);
    let at = position.to_mm_f64();
    if frame.contains(at) {
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "({}, {}) mm is outside {}: its frame spans x {}…{} mm and y {}…{} mm, y up. \
         Give a position inside it, or leave position out to place the symbol in the next free spot.",
        at[0],
        at[1],
        described(sheet),
        frame.min[0],
        frame.max[0],
        frame.min[1],
        frame.max[1],
    )))
}

fn overlaps(a: &Bounds, b: &Bounds) -> bool {
    a.min[0] < b.max[0] && b.min[0] < a.max[0] && a.min[1] < b.max[1] && b.min[1] < a.max[1]
}

/// The first free spot for `occurrence` (drawn now at the origin of its
/// sheet) on the grid inside the page, scanning rows from the top left: its
/// drawing, with clearance, overlaps no other symbol of the sheet and stays
/// clear of the edge and the title block. Refused when the sheet is full.
pub fn free_spot(project: &Project, occurrence: Id) -> Result<PointNm> {
    let item = project
        .occurrence(occurrence)
        .ok_or_else(|| Error::NotFound("Unknown symbol occurrence.".into()))?;
    let sheet = sheet_of(project, item.sheet)?;
    let m = TextMetric::default();
    let origin = item.position.to_mm_f64();
    let own = super::placed_unit(project, occurrence)?
        .bounds(m)
        .unwrap_or_else(|| Bounds::point(origin));
    // The drawing relative to the unit's origin, with clearance.
    let own = Bounds {
        min: [own.min[0] - origin[0], own.min[1] - origin[1]],
        max: [own.max[0] - origin[0], own.max[1] - origin[1]],
    }
    .inflated(CLEARANCE_MM);
    let others: Vec<Bounds> = project
        .occurrences
        .iter()
        .filter(|o| o.sheet == item.sheet && o.id != occurrence)
        .map(|o| {
            super::placed_unit(project, o.id)
                .ok()
                .and_then(|p| p.bounds(m))
                .unwrap_or_else(|| Bounds::point(o.position.to_mm_f64()))
        })
        .collect();
    let frame = page(sheet);
    let usable = Bounds {
        min: [frame.min[0] + MARGIN_MM, frame.min[1] + TITLE_BLOCK_MM],
        max: [frame.max[0] - MARGIN_MM, frame.max[1] - MARGIN_MM],
    };
    // Grid positions whose drawing fits the usable area.
    let to_grid = |mm: f64, up: bool| {
        let units = mm * 1e6 / GRID_NM as f64;
        (if up { units.ceil() } else { units.floor() }) as i64
    };
    let x_first = to_grid(usable.min[0] - own.min[0], true);
    let x_last = to_grid(usable.max[0] - own.max[0], false);
    let y_first = to_grid(usable.max[1] - own.max[1], false);
    let y_last = to_grid(usable.min[1] - own.min[1], true);
    let mut tried = 0usize;
    let mut y = y_first;
    while y >= y_last && tried < MAX_SPOTS {
        let mut x = x_first;
        while x <= x_last && tried < MAX_SPOTS {
            tried += 1;
            let at: Point = [(x * GRID_NM) as f64 / 1e6, (y * GRID_NM) as f64 / 1e6];
            let placed = Bounds {
                min: [own.min[0] + at[0], own.min[1] + at[1]],
                max: [own.max[0] + at[0], own.max[1] + at[1]],
            };
            if !others.iter().any(|o| overlaps(&placed, o)) {
                return Ok(PointNm::new(
                    Nm::new(x.saturating_mul(GRID_NM))?,
                    Nm::new(y.saturating_mul(GRID_NM))?,
                ));
            }
            x += STEP_GRIDS;
        }
        y -= STEP_GRIDS;
    }
    Err(Error::Invalid(format!(
        "{} has no free spot left for this symbol. Give a position inside its frame, place it on another sheet, or choose a larger paper.",
        described(sheet)
    )))
}
