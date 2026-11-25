//! Which fills' copper reads which, and which islands stay.
//!
//! A fill's copper reads:
//!
//! - the copper of the higher-ranking fills of other nets it clears, and
//! - unless it keeps every island, the islands of every fill of its net on
//!   its layer that its own could join, directly or through others of them:
//!   an island stays when it joins a pad, via or track itself or through
//!   theirs (`connectivity::pours_reaching`, the graph's own rule).
//!
//! The second goes both ways, and with the first it can go round: a fill may
//! be cleared by one of another net that is cleared in turn by a fill the
//! first one joins. Fills that read each other are settled together, after
//! those they read and before those that only read them. So the copper is
//! the same whichever fill is asked for first and whatever is stored, and a
//! fill's key is of its own inputs and those of every fill it reads.
//!
//! Fills that read each other are settled in rank order. Each is computed
//! with what stands of the ones it clears; an island that joins nothing and
//! is within no other fill of the net's outline does not stand. Then the
//! highest-ranking fill that has islands joining nothing loses them, and
//! those that clear it, and those that clear these, are computed again;
//! until every island that stands joins a pad, via or track. This ends: a
//! fill gets an island back only after a higher-ranking one lost some.
//!
//! So a higher-ranking fill's islands are judged with the lower ones as
//! they are poured at that point: what a lower fill of another net keeps
//! apart then, and gives up later, is not asked again.
use super::FILL_CHORD_MM;
use super::compute::{Ctx, Raw, outranks};
use crate::diagnostics::Result;
use crate::ids::Id;
use crate::layout::connectivity::{Pour, pours_reaching};
use crate::model::Fill;
use crate::model::fill::{FilledCopper, FilledRegion, IslandRemoval};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// Whether islands of the fill that join nothing are removed. A fill
/// without a net has nothing to join.
fn removes(fill: &Fill) -> bool {
    fill.net.is_some() && fill.settings.islands != IslandRemoval::Never
}

/// Which islands of the first of `all` could join a pad, via or track: those
/// that do, and those within reach of where another of `all` may be poured.
/// The others stand in no fill's way while the rest is settled.
fn hopeful(all: &[&Fill], islands: &[FilledRegion], direct: &[bool]) -> Vec<bool> {
    let outlines: Vec<[FilledRegion; 1]> = all
        .iter()
        .skip(1)
        .map(|f| {
            [FilledRegion {
                outer: f.outline.flatten(FILL_CHORD_MM),
                holes: Vec::new(),
            }]
        })
        .collect();
    let mut pours = vec![Pour { islands, direct }];
    pours.extend(outlines.iter().map(|o| Pour {
        islands: o,
        direct: &[true],
    }));
    pours_reaching(&pours)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// One fill being settled.
struct Settled {
    raw: Raw,
    /// The islands of `raw` that stand.
    stands: Vec<bool>,
    /// The islands of `raw` that join a pad, via or track themselves.
    direct: Vec<bool>,
}

impl Settled {
    fn standing<'s, T: Clone>(&'s self, of: &'s [T]) -> Vec<T> {
        of.iter()
            .zip(&self.stands)
            .filter(|(_, stands)| **stands)
            .map(|(x, _)| x.clone())
            .collect()
    }

    fn islands(&self) -> Vec<FilledRegion> {
        self.standing(&self.raw.islands)
    }
}

impl<'a> Ctx<'a> {
    /// The fills whose islands `fill`'s could join, directly or through
    /// others of them, `fill` first.
    fn joinable(&self, fill: &'a Fill) -> Vec<&'a Fill> {
        let mut out = vec![fill];
        let mut at = 0;
        while let Some(f) = out.get(at).copied() {
            at += 1;
            for other in &self.project.fills {
                if self.could_join(f, other) && !out.iter().any(|o| o.id == other.id) {
                    out.push(other);
                }
            }
        }
        out
    }

    /// Every fill the copper of `id` reads, directly or through others;
    /// `id` itself when they read it too.
    pub(super) fn reads(&mut self, id: Id) -> Result<Rc<BTreeSet<Id>>> {
        if let Some(known) = self.reads.get(&id) {
            return Ok(known.clone());
        }
        let mut found = BTreeSet::new();
        let mut ask = vec![id];
        while let Some(next) = ask.pop() {
            let fill = self.fill(next)?;
            let mut read = self.higher(fill);
            if removes(fill) {
                read.extend(self.joinable(fill).iter().skip(1).map(|f| f.id));
            }
            ask.extend(read.into_iter().filter(|r| found.insert(*r)));
        }
        let found = Rc::new(found);
        self.reads.insert(id, found.clone());
        Ok(found)
    }

    /// The copper of fill `id`: stored copper when its key is current (and
    /// it is not to be computed again), otherwise computed.
    pub(crate) fn copper(&mut self, id: Id) -> Result<FilledCopper> {
        if let Some(c) = self.copper.get(&id) {
            return Ok(c.clone());
        }
        let key = self.key(id)?;
        if !self.again.contains(&id)
            && let Some(stored) = &self.fill(id)?.computed
            && stored.key == key
        {
            self.copper.insert(id, stored.clone());
            return Ok(stored.clone());
        }
        self.settle(id)?;
        self.copper.get(&id).cloned().ok_or_else(|| {
            crate::diagnostics::Error::Invalid(format!("fill.order: fill {id} was not settled"))
        })
    }

    /// Compute `id` and every fill that reads it and that it reads.
    fn settle(&mut self, id: Id) -> Result<()> {
        let reads = self.reads(id)?;
        let mut members: Vec<&'a Fill> = vec![self.fill(id)?];
        for other in reads.iter().filter(|o| **o != id) {
            if self.reads(*other)?.contains(&id) {
                members.push(self.fill(*other)?);
            }
        }
        members.sort_by(|a, b| match (outranks(a, b), outranks(b, a)) {
            (true, _) => std::cmp::Ordering::Less,
            (_, true) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        });
        let member: BTreeSet<Id> = members.iter().map(|f| f.id).collect();
        // What they read that does not read them.
        let mut cut: BTreeMap<Id, Vec<FilledRegion>> = BTreeMap::new();
        for other in reads.iter().filter(|o| !member.contains(o)) {
            let copper = self.copper(*other)?;
            cut.insert(*other, copper.regions);
        }
        let clears: BTreeMap<Id, BTreeSet<Id>> = members
            .iter()
            .map(|f| (f.id, self.higher(f).into_iter().collect()))
            .collect();
        let cleared_by = |id: Id| {
            clears
                .iter()
                .filter(move |(_, higher)| higher.contains(&id))
                .map(|(by, _)| *by)
        };
        // The fills each one's islands could join; those that are no members
        // keep every island, and their copper is known.
        let joinable: BTreeMap<Id, Vec<&'a Fill>> = members
            .iter()
            .filter(|f| removes(f))
            .map(|f| (f.id, self.joinable(f)))
            .collect();
        let mut known: BTreeMap<Id, Vec<bool>> = BTreeMap::new();
        for other in joinable.values().flatten() {
            if !member.contains(&other.id) && !known.contains_key(&other.id) {
                let islands = cut.get(&other.id).cloned().unwrap_or_default();
                known.insert(other.id, self.direct(other, &islands));
            }
        }
        let joins = |f: &Fill| {
            joinable
                .values()
                .any(|all| all.iter().any(|o| o.id == f.id))
        };
        let mut settled: BTreeMap<Id, Settled> = BTreeMap::new();
        let mut stale = member.clone();
        loop {
            for f in &members {
                if !stale.remove(&f.id) {
                    continue;
                }
                let raw = self.raw(f, &cut)?;
                let direct = match joins(f) {
                    true => self.direct(f, &raw.islands),
                    false => vec![false; raw.islands.len()],
                };
                let hope = match joinable.get(&f.id) {
                    Some(all) => hopeful(all, &raw.islands, &direct),
                    None => vec![true; raw.islands.len()],
                };
                let one = Settled {
                    stands: (raw.small.iter().zip(hope))
                        .map(|(small, hope)| !small && hope)
                        .collect(),
                    raw,
                    direct,
                };
                cut.insert(f.id, one.islands());
                settled.insert(f.id, one);
                stale.extend(cleared_by(f.id));
            }
            for f in &members {
                let Some(all) = joinable.get(&f.id) else {
                    continue;
                };
                // What stands of the others, and what of it joins itself.
                let standing: Vec<(Vec<FilledRegion>, Vec<bool>)> = all
                    .iter()
                    .map(|o| match settled.get(&o.id) {
                        Some(s) => (s.islands(), s.standing(&s.direct)),
                        None => (
                            cut.get(&o.id).cloned().unwrap_or_default(),
                            known.get(&o.id).cloned().unwrap_or_default(),
                        ),
                    })
                    .collect();
                let pours: Vec<Pour<'_>> = standing
                    .iter()
                    .map(|(islands, direct)| Pour { islands, direct })
                    .collect();
                let reaching = pours_reaching(&pours)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                if reaching.iter().all(|r| *r) {
                    continue;
                }
                let Some(one) = settled.get_mut(&f.id) else {
                    continue;
                };
                let mut reaching = reaching.into_iter();
                for stands in one.stands.iter_mut().filter(|s| **s) {
                    *stands = reaching.next().unwrap_or(false);
                }
                cut.insert(f.id, one.islands());
                stale.extend(cleared_by(f.id));
                if !stale.is_empty() {
                    break;
                }
            }
            if stale.is_empty() {
                break;
            }
        }
        for (id, one) in settled {
            let key = self.key(id)?;
            let regions = one.islands();
            let removed = one.raw.islands.len().saturating_sub(regions.len());
            self.copper.entry(id).or_insert(FilledCopper {
                key,
                regions,
                removed_islands: u32::try_from(removed).unwrap_or(u32::MAX),
                removed_spokes: one.raw.removed_spokes,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
