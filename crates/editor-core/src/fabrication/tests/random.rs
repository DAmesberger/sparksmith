//! Random boards: small legal boards drawn from a seed, random edits of
//! them, and what must hold after every one ([`invariants`]), whatever the
//! board. A failure names the smallest part of its seed that still fails and
//! how to run it again (`docs/development.md`, "Random boards").
//!
//! - [`plan`]: a case, as atoms drawn from its seed.
//! - [`build`]: the board a case makes; [`edits`]: its edits as commands.
//! - [`truth`]: what is on a board, worked out again from the document.
//! - [`invariants`]: the rules; [`mutants`]: that each is alive.
mod build;
mod edits;
mod invariants;
mod mutants;
mod plan;
mod truth;

use crate::{Engine, format};
use invariants::{Broken, Kept, Rule, Seen, State, broken};
use plan::{Atom, CUTS, Case, Edit, FORMS, PRINTS};
use std::collections::{BTreeMap, BTreeSet};

pub(super) type Fallible<T = ()> = super::Fallible<T>;

/// Pixels of the pictures after an edit, and of the last ones, mm. After an
/// edit the lists ([`Rule::Inventory`]) and the export check look closer.
const PIXEL: f64 = 0.04;
const FINE: f64 = 0.01;

/// Cases of a long run: `first..last` seeds.
const BOARDS: &str = "SPARKSMITH_RANDOM_BOARDS";
/// One case, as a failure names it.
const CASE: &str = "SPARKSMITH_RANDOM_CASE";

/// Where a case first broke a rule.
struct Failure {
    /// The number of the atom it broke at; none: at the end.
    at: Option<usize>,
    broken: Vec<Broken>,
}

impl Failure {
    fn rules(&self) -> BTreeSet<Rule> {
        self.broken.iter().map(|b| b.rule).collect()
    }
}

/// What the cases did, for the tests that the cases do something.
#[derive(Default)]
struct Tally {
    tried: BTreeSet<String>,
    applied: BTreeMap<String, usize>,
}

/// The name of an edit's kind.
fn kind(edit: &Edit) -> String {
    let text = format!("{edit:?}");
    text.split([' ', '(', '{'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// How much is looked at after an edit.
#[derive(Clone, Copy, PartialEq)]
enum Look {
    /// Nothing: the case is only made.
    None,
    Board,
    Files,
}

/// Make one edit and look at the board after it.
fn step(engine: &mut Engine, edit: &Edit, look: Look, tally: &mut Tally) -> Fallible<Vec<Broken>> {
    let mut out = Vec::new();
    tally.tried.insert(kind(edit));
    let before = State::of(engine);
    let mut rerouted = Default::default();
    match edit {
        Edit::Undo { steps } => {
            let mut done = 0;
            while done < *steps && engine.can_undo() {
                engine.undo()?;
                done += 1;
            }
            // Forth again, all of it, is where it was.
            let back = engine.document().clone();
            for _ in 0..done {
                engine.redo()?;
            }
            if !invariants::same(before.document(), engine.document()) {
                out.push(broken(Rule::History, "undone and redone, another board"));
            }
            for _ in 0..done {
                engine.undo()?;
            }
            if !invariants::same(&back, engine.document()) {
                out.push(broken(Rule::History, "undone twice, another board"));
            }
        }
        Edit::Redo { steps } => {
            let mut done = 0;
            while done < *steps && engine.can_redo() {
                engine.redo()?;
                done += 1;
            }
            let forth = engine.document().clone();
            for _ in 0..done {
                engine.undo()?;
            }
            if !invariants::same(before.document(), engine.document()) {
                out.push(broken(Rule::History, "redone and undone, another board"));
            }
            for _ in 0..done {
                engine.redo()?;
            }
            if !invariants::same(&forth, engine.document()) {
                out.push(broken(Rule::History, "redone twice, another board"));
            }
        }
        Edit::Reopen => {
            let bytes = format::encode(&engine.to_file())?;
            *engine = Engine::from_file(format::decode(&bytes)?)?;
        }
        _ => {
            let Some(operation) = edits::operation(engine, edit)? else {
                return Ok(out);
            };
            if engine.apply(engine.revision(), operation).is_err() {
                out.extend(invariants::refused(&before, &State::of(engine)));
                return Ok(out);
            }
            rerouted = engine.rerouted().clone();
            if engine.revision() != before.revision() {
                let after = engine.document().clone();
                engine.undo()?;
                let back = engine.document().clone();
                engine.redo()?;
                out.extend(invariants::history(
                    before.document(),
                    &after,
                    &back,
                    engine.document(),
                ));
            }
        }
    }
    *tally.applied.entry(kind(edit)).or_default() += 1;
    if look == Look::None {
        return Ok(out);
    }
    let seen = match look {
        Look::Files => Seen::files(engine.project())?,
        _ => Seen::board(engine.project())?,
    };
    if !edits::deletes(edit) {
        out.extend(invariants::lost(
            &before.document().project,
            engine.project(),
            &rerouted,
            &seen.findings,
        ));
    }
    out.extend(looked(engine, &seen, look)?);
    Ok(out)
}

fn looked(engine: &Engine, seen: &Seen, look: Look) -> Fallible<Vec<Broken>> {
    let mut out = invariants::all(seen, PIXEL)?;
    out.extend(invariants::cached(&Kept::of(engine)?, engine, seen)?);
    if look == Look::Files {
        let bytes = format::encode(&engine.to_file())?;
        out.extend(invariants::reopened(seen, engine, &bytes)?);
    }
    Ok(out)
}

/// Play a case to its end, or to the first rule it breaks. `looking` false:
/// make it and look at nothing.
fn play(case: &Case, looking: bool, tally: &mut Tally) -> Fallible<(Engine, Option<Failure>)> {
    let mut engine = build::built(case)?;
    // As a host has it: what is made of the board reads its standards.
    let _context = engine.enter_context();
    let mut started = false;
    for (number, atom) in &case.atoms {
        let (edit, look) = match atom {
            Atom::Copper(edit) => (edit, Look::Board),
            Atom::Edit(edit) => (edit, Look::Files),
            _ => continue,
        };
        // A host fills the board before anyone edits it.
        if look == Look::Files && !started {
            started = true;
            engine.refill_stale()?;
            if looking {
                let seen = Seen::files(engine.project())?;
                let broken = looked(&engine, &seen, Look::Files)?;
                if !broken.is_empty() {
                    let at = Some(*number);
                    return Ok((engine, Some(Failure { at, broken })));
                }
            }
        }
        let look = if looking { look } else { Look::None };
        let broken = step(&mut engine, edit, look, tally)
            .unwrap_or_else(|e| vec![broken(Rule::Error, format!("{edit:?}: {e}"))]);
        if !broken.is_empty() {
            let at = Some(*number);
            return Ok((engine, Some(Failure { at, broken })));
        }
        // As its worker does in the pauses; else the next edit meets
        // copper of before.
        if started && number % 2 == 0 {
            engine.refill_stale()?;
        }
    }
    if !looking {
        return Ok((engine, None));
    }
    let seen = Seen::files(engine.project())?;
    let broken = invariants::all(&seen, FINE)?;
    let failure = (!broken.is_empty()).then_some(Failure { at: None, broken });
    Ok((engine, failure))
}

/// Where a case fails; one that cannot be made fails by [`Rule::Error`].
fn failure(case: &Case) -> Option<Failure> {
    match play(case, true, &mut Tally::default()) {
        Ok((_, failure)) => failure,
        Err(e) => Some(Failure {
            at: None,
            broken: vec![broken(Rule::Error, e.to_string())],
        }),
    }
}

/// The least of `case` that `fails`: halves of its atoms taken away, then
/// quarters, down to single ones, for as long as any can go.
fn shrunk(case: &Case, fails: &dyn Fn(&Case) -> bool) -> Case {
    let mut kept = case.numbers();
    let mut size = kept.len().div_ceil(2);
    while size > 0 {
        let (mut k, mut less) = (0, false);
        while k < kept.len() {
            let mut rest = kept.clone();
            rest.drain(k..(k + size).min(kept.len()));
            if fails(&case.only(&rest)) {
                kept = rest;
                less = true;
            } else {
                k += size;
            }
        }
        if size == 1 && !less {
            break;
        }
        size = size.div_ceil(2);
    }
    case.only(&kept)
}

/// Check a case; a failure is told with the least of it that fails alike.
fn check(case: &Case) -> Fallible {
    let Some(found) = failure(case) else {
        return Ok(());
    };
    let rules = found.rules();
    let cut = match found.at {
        // Nothing after the atom it broke at takes part.
        Some(at) => case.only(
            &case
                .numbers()
                .into_iter()
                .filter(|k| *k <= at)
                .collect::<Vec<_>>(),
        ),
        None => case.clone(),
    };
    let alike = |c: &Case| failure(c).is_some_and(|f| !f.rules().is_disjoint(&rules));
    let least = match alike(&cut) {
        true => shrunk(&cut, &alike),
        false => case.clone(),
    };
    let last = failure(&least).unwrap_or(found);
    let mut text = format!(
        "a random board breaks {:?} (seed {}, {} layers, {} of {} atoms)\n",
        last.rules(),
        case.seed,
        case.layers,
        least.atoms.len(),
        case.atoms.len()
    );
    for b in last.broken.iter().take(12) {
        text.push_str(&format!("  {:?}: {}\n", b.rule, b.text));
    }
    text.push_str("the least that fails:\n");
    for (number, atom) in &least.atoms {
        let mark = if last.at == Some(*number) { '>' } else { ' ' };
        text.push_str(&format!("{mark} {number}: {atom:?}\n"));
    }
    text.push_str(&format!(
        "again: {CASE}='{}' cargo test -p sparksmith-core --lib random::the_named_case -- --nocapture",
        least.name()
    ));
    Err(text.into())
}

fn seeds(of: impl IntoIterator<Item = u64>) -> Fallible {
    let failed: Vec<String> = of
        .into_iter()
        .filter_map(|seed| check(&Case::of(seed)).err())
        .map(|e| e.to_string())
        .collect();
    match failed.is_empty() {
        true => Ok(()),
        false => Err(failed.join("\n\n").into()),
    }
}

// The fixed boards of every run, by layers (the seed's remainder by three).

#[test]
fn boards_of_two_layers_keep_every_rule() -> Fallible {
    seeds([0, 3, 6, 9])
}

#[test]
fn boards_of_four_layers_keep_every_rule() -> Fallible {
    seeds([1, 4, 7, 10])
}

#[test]
fn boards_of_six_layers_keep_every_rule() -> Fallible {
    seeds([2, 5, 8, 11])
}

/// The long run: the seeds `SPARKSMITH_RANDOM_BOARDS=first..last` names;
/// nothing without it.
#[test]
fn the_boards_of_the_long_run_keep_every_rule() -> Fallible {
    let Ok(range) = std::env::var(BOARDS) else {
        return Ok(());
    };
    let (first, last) = range
        .split_once("..")
        .ok_or_else(|| format!("{BOARDS} is first..last, not {range}"))?;
    let range = first.trim().parse::<u64>()?..last.trim().parse::<u64>()?;
    if range.is_empty() {
        return Err(format!("{BOARDS} names no seed").into());
    }
    seeds(range)
}

/// One case again, as a failure named it (`SPARKSMITH_RANDOM_CASE`);
/// nothing without it.
#[test]
fn the_named_case_keeps_every_rule() -> Fallible {
    match std::env::var(CASE) {
        Ok(name) => check(&Case::named(&name)?),
        Err(_) => Ok(()),
    }
}

/// Seeds whose boards are only made, for what the generator reaches.
const MADE: u64 = 84;

#[test]
fn the_generator_makes_everything_and_most_edits_are_taken() -> Fallible {
    let mut tally = Tally::default();
    let (mut forms, mut cuts, mut prints) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    let (mut layers, mut joined, mut capped, mut filled) = (BTreeSet::new(), 0, 0, 0);
    for seed in 0..MADE {
        let case = Case::of(seed);
        layers.insert(case.layers);
        for (_, atom) in &case.atoms {
            match atom {
                Atom::Pad { form, .. } => {
                    forms.insert(*form);
                }
                Atom::Cut(cut) => {
                    cuts.insert(*cut);
                }
                Atom::Print { print, .. } => {
                    prints.insert(*print);
                }
                _ => {}
            }
        }
        let (engine, failure) = play(&case, false, &mut tally)?;
        if let Some(failure) = failure {
            return Err(format!("seed {seed}: {:?}", failure.broken).into());
        }
        let project = engine.project();
        capped += project.vias.iter().filter(|v| !v.fill.is_none()).count();
        filled += project
            .fills
            .iter()
            .filter(|f| f.computed.iter().any(|c| !c.regions.is_empty()))
            .count();
        // Nets of which copper joins two pads.
        joined += invariants::islands(&crate::layout::CopperGraph::of(project))
            .values()
            .filter(|pads| {
                let islands: BTreeSet<_> = pads.values().collect();
                islands.len() < pads.len()
            })
            .count();
    }
    assert_eq!(layers, BTreeSet::from([2, 4, 6]));
    assert_eq!(forms, BTreeSet::from(FORMS));
    assert_eq!(cuts, BTreeSet::from(CUTS));
    assert_eq!(prints, BTreeSet::from(PRINTS));
    assert!(
        capped > 0 && filled > 20 && joined > 20,
        "{capped} filled vias, {filled} fills with copper, {joined} nets joined"
    );
    eprintln!("edits taken: {:?}", tally.applied);
    let never: Vec<&str> = [
        "Move",
        "Flip",
        "MoveTwo",
        "Link",
        "Straight",
        "Arc",
        "Via",
        "Drag",
        "MoveVia",
        "DeleteVia",
        "DeleteTrack",
        "Width",
        "Cap",
        "Tent",
        "LockVia",
        "LockTrack",
        "Pour",
        "Repour",
        "Refill",
        "Keepout",
        "Rule",
        "Undo",
        "Redo",
        "Reopen",
        "Paste",
    ]
    .into_iter()
    .filter(|k| !tally.applied.contains_key(*k))
    .collect();
    assert!(never.is_empty(), "never taken: {never:?}");
    // And what must be refused was tried and never taken.
    assert!(tally.tried.contains("Wrong") && !tally.applied.contains_key("Wrong"));
    Ok(())
}

#[test]
fn a_seed_is_the_same_board_each_time() -> Fallible {
    for seed in [4, 8] {
        let made = || -> Fallible<_> {
            let (engine, _) = play(&Case::of(seed), false, &mut Tally::default())?;
            let files = Seen::files(engine.project())?.files;
            Ok((format::encode(&engine.to_file())?, files))
        };
        // The second run's hash maps have another order.
        assert!(made()? == made()?, "seed {seed}");
    }
    Ok(())
}

#[test]
fn a_case_shrinks_to_what_fails_and_is_named_by_it() -> Fallible {
    let case = Case::of(5);
    let numbers = case.numbers();
    let [a, b] = [numbers[3], numbers[numbers.len() - 2]];
    // It fails as long as these two are there, however much goes with them.
    let least = shrunk(&case, &|c| {
        let left = c.numbers();
        left.contains(&a) && left.contains(&b)
    });
    assert_eq!(least.numbers(), [a, b]);
    let again = Case::named(&least.name())?;
    assert_eq!(again.atoms, least.atoms);
    assert_eq!(Case::named("5")?.atoms, case.atoms);
    assert_eq!(Case::named(&case.name())?.atoms, case.atoms);
    // A case that fails with nothing left is told as that.
    assert!(shrunk(&case, &|_| true).atoms.is_empty());
    Ok(())
}
