//! Board text commands (`layout.text_add`, `layout.text_edit`,
//! `layout.text_delete`): each checks the text it keeps and is one undo
//! step. The text prints through [`super::placed_graphics`].
use super::EPS;
use super::limits::{at_least_mm, cite, mm, mm_against};
use crate::diagnostics::{Error, Finding, FindingCode, ObjectRef, Result};
use crate::ids::Id;
use crate::model::board_text::first_line;
use crate::model::{BoardText, BoardTextSpec, Layer, Project};
use crate::rules::RuleKey;

fn missing(id: Id) -> Error {
    Error::NotFound(format!("text.missing: no board text {id}"))
}

/// Add `spec` as a new board text; its id.
pub fn add(project: &mut Project, spec: BoardTextSpec) -> Result<Id> {
    let id = project.allocate()?;
    let text = BoardText::from_spec(id, spec);
    text.check()?;
    project.board_texts.push(text);
    Ok(id)
}

/// Replace the board text of `text.id` with `text`.
pub fn edit(project: &mut Project, text: BoardText) -> Result<()> {
    text.check()?;
    let slot = project
        .board_texts
        .iter_mut()
        .find(|t| t.id == text.id)
        .ok_or_else(|| missing(text.id))?;
    *slot = text;
    Ok(())
}

/// Delete the board text `id`.
pub fn delete(project: &mut Project, id: Id) -> Result<()> {
    let before = project.board_texts.len();
    project.board_texts.retain(|t| t.id != id);
    if project.board_texts.len() == before {
        return Err(missing(id));
    }
    Ok(())
}

/// Board text on a printed silkscreen shorter than the rules' minimum
/// silkscreen text height (`board.text_height`): the fabricator cannot
/// print it legibly. A side the stackup does not print is not checked.
pub fn findings(project: &Project) -> Vec<Finding> {
    let rules = project.rules.effective();
    let min = rules.silkscreen.text_height_min;
    project
        .board_texts
        .iter()
        .filter(|t| match t.side {
            Layer::Top => project.stackup.silkscreen_top,
            Layer::Bottom => project.stackup.silkscreen_bottom,
        })
        .filter_map(|t| {
            let height = t.height_nm.to_mm_f64();
            (height < min - EPS).then(|| {
                Finding::new(
                    FindingCode::BoardTextHeight,
                    format!(
                        "Board text {} is {} mm high; needs {} mm ({})",
                        first_line(&t.text),
                        mm_against(height, min),
                        mm(min),
                        cite(&project.rules, RuleKey::SilkTextHeightMin)
                    ),
                    vec![ObjectRef::BoardText { id: t.id }],
                )
                .measuring([at_least_mm(height, min)])
            })
        })
        .collect()
}
