//! Text on the board's silkscreen that belongs to no part: a board name, a
//! revision, a note to the assembler. It prints through the same legend as
//! a part's text (`layout::placed_graphics`), so Layout, the 3D films, the
//! Gerber legend and the legend checks all draw the same ink; on the bottom
//! it reads mirrored, as seen from the top.
use super::Layer;
use crate::diagnostics::{Error, Result};
use crate::ids::Id;
use crate::library::graphics::{Rotation, TEXT_HEIGHT, printable_text};
use crate::units::{Nm, PointNm};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Characters of its own text a legend item is named by.
pub const MAX_NAME_CHARS: usize = 16;

/// How a legend text is named in a message: its first line, at most
/// [`MAX_NAME_CHARS`] characters, an ellipsis when cut.
pub fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    let short: String = line.chars().take(MAX_NAME_CHARS).collect();
    if short.len() < line.len() {
        format!("{short}…")
    } else {
        short
    }
}

/// A new board text: what `layout.text_add` takes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoardTextSpec {
    /// Literal text; lines break at `\n`.
    pub text: String,
    /// The text's centre.
    pub position: PointNm,
    /// Cap height.
    pub height_nm: Nm,
    #[serde(default)]
    pub rotation: Rotation,
    /// The silkscreen it prints on.
    pub side: Layer,
}

/// Text on the board's silkscreen.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BoardText {
    pub id: Id,
    /// Literal text; lines break at `\n`.
    pub text: String,
    /// The text's centre.
    pub position: PointNm,
    /// Cap height.
    pub height_nm: Nm,
    /// Quarter turns counter-clockwise, as read from its own side.
    #[serde(default)]
    pub rotation: Rotation,
    /// The silkscreen it prints on; the bottom's reads mirrored from the
    /// top.
    pub side: Layer,
}

impl BoardText {
    pub fn from_spec(id: Id, s: BoardTextSpec) -> Self {
        Self {
            id,
            text: s.text,
            position: s.position,
            height_nm: s.height_nm,
            rotation: s.rotation,
            side: s.side,
        }
    }

    /// The text and its height are what a legend can print: printable
    /// literal text (not blank, at most 256 bytes and 16 lines) and a cap
    /// height within the library's text heights.
    pub fn check(&self) -> Result<()> {
        if !printable_text(&self.text) {
            return Err(Error::Invalid(
                "text.invalid: board text needs printable text: not blank, at most 256 bytes \
                 and 16 lines, no control characters but line breaks"
                    .into(),
            ));
        }
        if !TEXT_HEIGHT.contains(&self.height_nm.get()) {
            return Err(Error::Invalid(format!(
                "text.height: a board text is {}–{} mm high",
                Nm::try_new(*TEXT_HEIGHT.start()).map_or(0.0, Nm::to_mm_f64),
                Nm::try_new(*TEXT_HEIGHT.end()).map_or(0.0, Nm::to_mm_f64),
            )));
        }
        Ok(())
    }
}
