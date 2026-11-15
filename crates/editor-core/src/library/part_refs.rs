//! The five lineages of one part set (component, symbol, package,
//! footprint, binding) and the revisions they pin of each other.
use crate::ids::{BindingRef, DefinitionId, FootprintRef, PackageRef, SymbolRef};

/// The five lineages of one part set and the revisions they pin of each
/// other (normally the drafts' pending revisions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleRefs {
    pub component: DefinitionId,
    pub symbol: SymbolRef,
    pub package: PackageRef,
    pub footprint: FootprintRef,
    pub binding: BindingRef,
}
