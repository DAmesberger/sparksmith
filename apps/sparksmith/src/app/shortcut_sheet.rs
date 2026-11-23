//! The Keyboard shortcuts sheet (⌘/ / Ctrl+/, the palette and the File
//! menu): every command with a key, grouped by where it applies, the
//! contexts in force now first. Help supplies the titles and contexts;
//! Settings supplies the effective keys through the registry.
use crate::commands::{Context, Registry};
use sparksmith_core::help::{Facts, Index};

/// One context's shortcuts: (what it does, key as shown).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Group {
    pub context: Context,
    pub here: bool,
    pub rows: Vec<(String, String)>,
}

/// The sheet's groups for `filter` (matched against the name, key and
/// context): contexts in `active` first, then the others in registry order;
/// empty groups are left out.
pub(crate) fn groups(
    index: &Index,
    registry: &Registry,
    filter: &str,
    active: &[Context],
) -> Vec<Group> {
    let filter = filter.trim().to_lowercase();
    let mut contexts: Vec<Context> = active.iter().rev().copied().collect();
    contexts.extend(Context::ALL.into_iter().filter(|c| !active.contains(c)));
    contexts
        .into_iter()
        .filter_map(|context| {
            // One line per name, with all its keys (the palette's two).
            let mut named: Vec<(String, String)> = Vec::new();
            for entry in index.values().filter(
                |e| matches!(&e.facts, Facts::Command { context: c, .. } if c == context.label()),
            ) {
                let Some(command) = registry.get(entry.id.name()) else {
                    continue;
                };
                let Some(key) = registry.shortcut(command) else {
                    continue;
                };
                match named.iter_mut().find(|(title, _)| *title == entry.title) {
                    Some((_, keys)) if keys.split(", ").any(|k| k == key) => {}
                    Some((_, keys)) => {
                        keys.push_str(", ");
                        keys.push_str(&key);
                    }
                    None => named.push((entry.title.clone(), key)),
                }
            }
            let rows: Vec<(String, String)> = named
                .into_iter()
                .filter(|(title, key)| {
                    filter.is_empty()
                        || title.to_lowercase().contains(&filter)
                        || key.to_lowercase().contains(&filter)
                        || context.label().to_lowercase().contains(&filter)
                })
                .collect();
            (!rows.is_empty()).then(|| Group {
                context,
                here: active.contains(&context),
                rows,
            })
        })
        .collect()
}
