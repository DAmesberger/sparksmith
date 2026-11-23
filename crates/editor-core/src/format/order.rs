//! The order collections are saved in (docs/file-format.md §1.1, slice F2):
//! every collection whose order means nothing is sorted by its key, so the
//! order in which objects were created or edited never shows in a file and
//! opening then saving changes no byte. Order that means something (sheets
//! in page order, stackup layers, outline segments, the interface's
//! append-only slot table, the part dimensions' list order) is left alone.
//! The audit of every collection is recorded in docs/file-format.md §4.1.
//!
//! [`sort`] runs on write and on read. A collection already in order is not
//! touched, so a shared section stays shared.
use crate::library::{Library, Published};
use crate::model::Project;
use std::ops::DerefMut;

/// Sort `project`'s collections by their keys (stable, so records that
/// share a key keep their order).
pub fn sort(project: &mut Project) {
    by_key(&mut project.instances, |i| i.id);
    by_key(&mut project.occurrences, |o| o.id);
    by_key(&mut project.wires, |w| w.id);
    by_key(&mut project.junctions, |j| j.id);
    by_key(&mut project.nets, |n| n.id);
    by_key(&mut project.placements, |p| p.instance);
    by_key(&mut project.tracks, |t| t.id);
    by_key(&mut project.vias, |v| v.id);
    // Part dimensions are not sorted: their order is the Dimensions
    // list's, set by `layout.dimension_order`.
    by_key(&mut project.fills, |f| f.id);
    by_key(&mut project.keepouts, |k| k.id);
    by_key(&mut project.labels, |l| l.id);
    by_key(&mut project.sheet_symbols, |s| s.id);
    by_key(&mut project.sheets.locals, |i| i.id);
    if !project.sheets.uses.is_sorted_by(|a, b| a.path <= b.path) {
        project.sheets.uses.sort_by(|a, b| a.path.cmp(&b.path));
    }
    by_key(&mut project.drawings, |d| d.id);
    library(&mut project.library);
}

fn library(library: &mut Library) {
    revisions(&mut library.components);
    revisions(&mut library.symbols);
    revisions(&mut library.packages);
    revisions(&mut library.footprints);
    revisions(&mut library.bindings);
    revisions(&mut library.quantity_kinds);
    revisions(&mut library.fields);
    revisions(&mut library.parts);
    revisions(&mut library.models);
    revisions(&mut library.package_rows);
    by_key(&mut library.origins, |o| o.local);
    by_key(&mut library.assets, |a| a.hash);
    by_key(&mut library.generated, |g| g.root);
    by_key(&mut library.reviews, |r| r.component);
}

/// Published revisions by lineage, then revision.
fn revisions<T>(items: &mut impl DerefMut<Target = Vec<Published<T>>>) {
    by_key(items, |p| (p.id, p.revision));
}

/// Reads first, and writes (which copies a shared section) only when the
/// collection is out of order.
fn by_key<T, K: Ord>(items: &mut impl DerefMut<Target = Vec<T>>, key: impl Fn(&T) -> K) {
    if !items.is_sorted_by_key(&key) {
        items.sort_by_key(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Engine;
    use crate::shared::Shared;

    #[test]
    fn sorting_a_sorted_project_shares_every_section() -> crate::Result<()> {
        let project = Engine::showcase()?.project().clone();
        let mut sorted = project.clone();
        sort(&mut sorted);
        assert!(Shared::ptr_eq(&project.tracks, &sorted.tracks));
        assert!(Shared::ptr_eq(&project.placements, &sorted.placements));
        assert!(Shared::ptr_eq(
            &project.library.components,
            &sorted.library.components
        ));
        assert_eq!(project, sorted);
        Ok(())
    }

    #[test]
    fn collections_are_sorted_by_their_keys() -> crate::Result<()> {
        let mut project = Engine::showcase()?.project().clone();
        project.tracks.reverse();
        project.placements.reverse();
        project.library.components.reverse();
        assert!(project.tracks.len() > 1 && project.placements.len() > 1);
        sort(&mut project);
        assert!(project.tracks.is_sorted_by_key(|t| t.id));
        assert!(project.placements.is_sorted_by_key(|p| p.instance));
        assert!(
            project
                .library
                .components
                .is_sorted_by_key(|c| (c.id, c.revision))
        );
        Ok(())
    }
}
