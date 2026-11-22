//! The one finalization of an edited candidate, for every entry path: the
//! engine runs it after a direct operation, and an update's preview runs it
//! on the candidate the review shows and the apply commits. So a reviewed
//! edit and the same edit made directly end in the same design: the nets
//! are reconciled once, and the net changes of that one pass are what the
//! class assignments, pairs and length groups follow.
use crate::diagnostics::Result;
use crate::ids::Id;
use crate::layout::{self, CopperGraph, outline::BoardShapeEvaluator};
use crate::model::Project;
use crate::{instances, interface, nets, rules, schematic};

/// How the operation that made the candidate finalizes.
pub(crate) struct Finalize<'a> {
    /// The copper graph of the base document.
    pub graph: &'a CopperGraph,
    pub evaluator: Option<&'a dyn BoardShapeEvaluator>,
    /// Parts the operation moved by hand: their attached copper is carried.
    pub moved_parts: &'a [Id],
    /// Selected loose copper a group drag slides with its parts.
    pub loose: Option<&'a layout::arrange::CopperMove>,
    /// A board edit that must keep every connection its parts carry.
    pub preserve_copper: bool,
    /// A reviewed update: routed nets may change membership, because the
    /// review listed the copper that goes.
    pub reviewed: bool,
    /// Only the assembly link's own commands move what the linked design
    /// drives.
    pub assembly_command: bool,
}

/// What a finalization changed besides the candidate itself.
pub(crate) struct Finalized {
    /// The copper re-routed to follow carried parts.
    pub rerouted: layout::router::reroute::Rerouted,
}

/// Finalize `project`, edited from `base`: the schematic's normal form and
/// nets, what follows the nets, the solved and carried parts, the board
/// fills, the routed-net guard, the interface's automatic datums and the
/// linked design's guards. A refusal leaves the caller's base untouched;
/// `project` is the caller's candidate.
pub(crate) fn finalize(
    base: &Project,
    project: &mut Project,
    f: &Finalize<'_>,
) -> Result<Finalized> {
    // A board edit leaves the schematic as it was: its normal form and nets
    // stand, so they are not derived again.
    let schematic_edited = !project.same_schematic(base);
    let changes = if schematic_edited {
        schematic::tidy(project)?;
        // A removed part takes its variant overrides with it.
        instances::variants::follow_instances(project);
        nets::reconcile(project)?
    } else {
        nets::NetChanges::default()
    };
    // Explicit net-class assignments follow their nets through merges and
    // splits; a net that no longer exists takes its assignment with it.
    // Pairs and length groups follow them too.
    let rules_ = &project.rules;
    if !rules_.assignments.is_empty()
        || !rules_.diff_pairs.is_empty()
        || !rules_.length_groups.is_empty()
    {
        let live: std::collections::BTreeSet<Id> = project.nets.iter().map(|n| n.id).collect();
        rules::netclass::follow_nets(
            &mut project.rules,
            &changes.merged,
            &changes.split,
            &changes.created,
            &live,
        );
        rules::signal::follow_nets(&mut project.rules, &changes.merged, &live);
    }
    // Dimensioned parts, fills and keepouts are solved after the board (and
    // a move that cannot land is refused); a solved part, like a part moved
    // by hand, carries the copper attached to its pads.
    let mut placed = layout::dimensions::follow(base, project, f.evaluator)?;
    for instance in f.moved_parts {
        if !placed.contains(instance) {
            placed.push(*instance);
        }
    }
    let mut rerouted = layout::router::reroute::Rerouted::default();
    if !placed.is_empty() {
        rerouted = layout::dimensions::carry(base, project, f.graph, &placed, f.loose)?;
        if f.preserve_copper {
            layout::dimensions::protect_connections(base, project, &rerouted)?;
        }
    }
    // Board-following fills take the (possibly changed) outline.
    layout::fill::follow_board(project)?;
    if !f.reviewed && schematic_edited {
        schematic::protect_routed_nets(base, project)?;
    }
    // The interface's automatic datums follow the board: a new hole takes a
    // new slot, a deleted one retires its slot.
    if let Some(table) = interface::auto::follow(base, project)? {
        project.interface = table.into();
    }
    // A board edit never moves what the linked design drives, and the loops
    // between the two files follow the edit (slice B5).
    if !f.assembly_command {
        interface::assembly_link::guard_driven(base, project, f.evaluator)?;
    }
    interface::assembly_link::follow_cycles(base, project, f.evaluator)?;
    Ok(Finalized { rerouted })
}
