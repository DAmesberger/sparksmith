//! Layout blocks saved with the design (docs/design/layout-blocks.md):
//! data only. The rules (who may be in a block, what moves it, its border)
//! live in `layout::blocks`.
use crate::ids::Id;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bytes of a block's name.
pub const MAX_BLOCK_NAME: usize = 64;

/// Parts that belong together on the board. A part is in at most one block.
/// The border is not stored: it is derived from the members' courtyards.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub id: Id,
    /// Unique among the blocks, whatever its case.
    pub name: String,
    /// The component instances in it, in identity order; at least one. A
    /// member need not be on the board yet.
    pub members: Vec<Id>,
    /// Nothing moves the block or one part of it by hand. The parts' own
    /// locks are separate.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
}
