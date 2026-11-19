//! The source schematic's netlist, worked out from KiCad's drawing alone
//! (never from what the importer drew), and the check that the imported
//! schematic's nets equal it.
//!
//! The rules are those of [`crate::schematic`]: wires join where their
//! ends meet and where an end, junction, pin, label or sheet pin lies on
//! another wire; crossing wires do not join; labels join by name in their
//! scope (local and hierarchical labels on their sheet, global labels and
//! power symbols everywhere, a sheet pin with the hierarchical labels of
//! its name on the sheet it shows); a hidden power input joins the global
//! net of its name; a pin of type "not connected" joins nothing. KiCad
//! compares names exactly.
use crate::board::PointNm;
use crate::import::Session;
use crate::report::{Case, Report};
use crate::schematic::{self, KLabelKind, Schematic};
use sparksmith_core::{Endpoint, Id};
use std::collections::{BTreeMap, BTreeSet};

/// A symbol pin of the source: its reference (as the sheet instance
/// names it) and number.
pub(crate) type SourcePin = (String, String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    /// A point of a flattened sheet.
    Point(usize, PointNm),
    /// A name in its scope: a sheet's (`Some`) or everywhere's.
    Name(Option<usize>, String),
    Pin(SourcePin),
}

#[derive(Default)]
struct Sets {
    parent: BTreeMap<Node, Node>,
}

impl Sets {
    fn find(&mut self, n: &Node) -> Node {
        let mut r = n.clone();
        while let Some(p) = self.parent.get(&r) {
            if *p == r {
                break;
            }
            r = p.clone();
        }
        // Point the path at the root.
        let mut at = n.clone();
        while at != r {
            let Some(next) = self.parent.insert(at.clone(), r.clone()) else {
                break;
            };
            at = next;
        }
        r
    }
    fn union(&mut self, a: Node, b: Node) {
        self.parent.entry(a.clone()).or_insert_with(|| a.clone());
        self.parent.entry(b.clone()).or_insert_with(|| b.clone());
        let (ra, rb) = (self.find(&a), self.find(&b));
        if ra != rb {
            self.parent.insert(rb, ra);
        }
    }
    fn add(&mut self, a: Node) {
        self.parent.entry(a.clone()).or_insert(a);
    }
}

/// The source schematic's nets as groups of symbol pins; a pin that
/// meets nothing is a group of its own. Power symbols' pins are no pins
/// of a part and are left out.
pub(crate) fn source_nets(schematic: &Schematic) -> Vec<BTreeSet<SourcePin>> {
    let mut sets = Sets::default();
    for (index, sheet) in schematic.sheets.iter().enumerate() {
        let Some(file) = schematic.files.get(&sheet.file) else {
            continue;
        };
        let at = |p: PointNm| Node::Point(index, p);
        // Every connection point of the sheet: wire ends, junctions,
        // pins, labels and sheet pins.
        let mut points: BTreeSet<PointNm> = file.wires.iter().flat_map(|w| [w[0], w[1]]).collect();
        points.extend(file.junctions.iter().copied());
        for w in &file.wires {
            sets.union(at(w[0]), at(w[1]));
        }
        for label in &file.labels {
            let scope = match label.kind {
                KLabelKind::Global => None,
                KLabelKind::Local | KLabelKind::Hierarchical => Some(index),
            };
            points.insert(label.at);
            sets.union(at(label.at), Node::Name(scope, label.name.clone()));
        }
        for symbol in &file.symbols {
            let Some((is_power, pins)) = schematic.lib(file, symbol) else {
                continue;
            };
            let (reference, unit) = schematic.reference(sheet, symbol);
            for pin in pins {
                let p = schematic::pin_on_sheet(symbol, pin.at);
                if is_power {
                    points.insert(p);
                    sets.union(at(p), Node::Name(None, symbol.value.clone()));
                    continue;
                }
                if pin.unit != 0 && pin.unit != unit {
                    continue;
                }
                let node = Node::Pin((reference.clone(), pin.number.clone()));
                if pin.no_connect {
                    sets.add(node);
                } else if pin.hidden_power {
                    sets.union(node, Node::Name(None, pin.name.clone()));
                } else {
                    points.insert(p);
                    sets.union(at(p), node);
                }
            }
        }
        // Sheet pins of the sheets this one shows.
        for (child, flat) in schematic.sheets.iter().enumerate() {
            let Some((parent, k)) = flat.parent else {
                continue;
            };
            if parent != index {
                continue;
            }
            let Some(symbol) = file.sheets.get(k) else {
                continue;
            };
            for (name, p) in &symbol.pins {
                points.insert(*p);
                sets.union(at(*p), Node::Name(Some(child), name.clone()));
            }
        }
        // A connection point inside a wire joins it.
        for w in &file.wires {
            let (lo, hi) = (
                [w[0][0].min(w[1][0]), w[0][1].min(w[1][1])],
                [w[0][0].max(w[1][0]), w[0][1].max(w[1][1])],
            );
            for p in points.range([lo[0], i64::MIN]..=[hi[0], i64::MAX]) {
                if p[1] >= lo[1] && p[1] <= hi[1] && schematic::inside(*p, w[0], w[1]) {
                    sets.union(at(w[0]), at(*p));
                }
            }
        }
    }
    let nodes: Vec<Node> = sets.parent.keys().cloned().collect();
    let mut groups: BTreeMap<Node, BTreeSet<SourcePin>> = BTreeMap::new();
    for n in nodes {
        if let Node::Pin(pin) = &n {
            let root = sets.find(&n);
            groups.entry(root).or_default().insert(pin.clone());
        }
    }
    groups.into_values().collect()
}

/// Where a terminal is in the imported schematic: on a net, or alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Net(Id),
    Alone(Endpoint),
}

/// Compare the imported schematic's nets with the source's, as groups of
/// the terminals the source pins connect (each pin through the pad of its
/// number). Every source net the import splits and every imported net that
/// joins source nets is an import finding ([`Case::SchematicNetSplit`],
/// [`Case::SchematicNetJoined`]), its pins in the private detail; the
/// nets that are equal are counted.
pub(crate) fn verify(session: &Session<'_>, schematic: &Schematic, report: &mut Report) {
    let project = session.engine.project();
    let source = source_nets(schematic);
    let mut instances: BTreeMap<String, Option<Id>> = BTreeMap::new();
    let mut end_of = |pin: &SourcePin| -> Option<Endpoint> {
        let instance = *instances.entry(pin.0.clone()).or_insert_with(|| {
            project
                .instance_by_designator(&pin.0)
                .filter(|i| !i.board_only && project.occurrences_of(i.id).next().is_some())
                .map(|i| i.id)
        });
        let instance = instance?;
        let terminal = crate::sheets::terminal(project, instance, &pin.1)?;
        Some(Endpoint { instance, terminal })
    };
    let place = |e: Endpoint| {
        project
            .net_of(e)
            .map_or(Place::Alone(e), |n| Place::Net(n.id))
    };
    let named = |pins: &[&SourcePin]| {
        pins.iter()
            .map(|(r, n)| format!("{r}.{n}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    // Each source net's terminals; and which source nets each imported
    // net holds.
    let mut holds: BTreeMap<Place, BTreeSet<usize>> = BTreeMap::new();
    let mut members: Vec<Vec<(&SourcePin, Endpoint)>> = Vec::with_capacity(source.len());
    for (k, net) in source.iter().enumerate() {
        let ends: Vec<(&SourcePin, Endpoint)> =
            net.iter().filter_map(|p| Some((p, end_of(p)?))).collect();
        for (_, e) in &ends {
            holds.entry(place(*e)).or_default().insert(k);
        }
        members.push(ends);
    }
    let mut equal = 0usize;
    for ends in &members {
        if ends.is_empty() {
            continue;
        }
        let mut by_place: BTreeMap<Place, Vec<&SourcePin>> = BTreeMap::new();
        for (pin, e) in ends {
            by_place.entry(place(*e)).or_default().push(pin);
        }
        let joined = by_place
            .keys()
            .any(|p| holds.get(p).is_some_and(|nets| nets.len() > 1));
        if by_place.len() > 1 {
            let text = by_place
                .values()
                .map(|pins| named(pins))
                .collect::<Vec<_>>()
                .join(" | ");
            report.add_detail(Case::SchematicNetSplit, "", "", &text);
        } else if !joined {
            equal += 1;
        }
    }
    for (p, nets) in &holds {
        if nets.len() < 2 {
            continue;
        }
        let text = nets
            .iter()
            .filter_map(|k| members.get(*k))
            .map(|ends| {
                named(
                    &ends
                        .iter()
                        .filter(|(_, e)| place(*e) == *p)
                        .map(|(pin, _)| *pin)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        report.add_detail(Case::SchematicNetJoined, "", "", &text);
    }
    report.counts.source_nets = members.iter().filter(|m| !m.is_empty()).count();
    report.counts.source_nets_equal = equal;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Four two-pin parts and the wiring between them: a crossing that
    /// joins nothing, a wire ending on another's middle (a T), and two
    /// labels of one name.
    #[test]
    fn the_source_netlist_follows_kicads_drawing() -> Result<(), Box<dyn std::error::Error>> {
        let lib = r#"(symbol "Original:Two" (in_bom yes) (on_board yes)
          (symbol "Two_1_1"
            (pin passive line (at -2.54 0 0) (length 1.27) (name "~") (number "1"))
            (pin passive line (at 2.54 0 180) (length 1.27) (name "~") (number "2"))))"#;
        let symbol = |reference: &str, x: f64, y: f64| {
            format!(
                r#"(symbol (lib_id "Original:Two") (at {x} {y} 0) (unit 1)
                  (property "Reference" "{reference}" (at 0 0 0))
                  (instances (project "t" (path "/00000000-0000-4000-8000-000000000001" (reference "{reference}") (unit 1)))))"#
            )
        };
        // R1.2 (12.54, 10) runs right to (30, 10); R2.2 (12.54, 20) runs
        // right and up across it to (20, 0): a crossing. R3.1 (37.46, 20)
        // runs left and up to the middle of R1's wire at (25, 10): a T.
        // R4.1 and R4.2 carry labels of one name.
        let text = format!(
            r#"(kicad_sch (version 20231120) (generator "t")
              (uuid "00000000-0000-4000-8000-000000000001") (paper "A4")
              (lib_symbols {lib})
              {} {} {} {}
              (wire (pts (xy 12.54 10) (xy 30 10)))
              (wire (pts (xy 12.54 20) (xy 20 20)))
              (wire (pts (xy 20 20) (xy 20 0)))
              (wire (pts (xy 37.46 20) (xy 25 20)))
              (wire (pts (xy 25 20) (xy 25 10)))
              (label "N" (at 47.46 40 0))
              (label "N" (at 52.54 40 0)))"#,
            symbol("R1", 10.0, 10.0),
            symbol("R2", 10.0, 20.0),
            symbol("R3", 40.0, 20.0),
            symbol("R4", 50.0, 40.0),
        );
        let mut files = BTreeMap::new();
        files.insert("t.kicad_sch".to_owned(), text.into_bytes());
        let schematic = schematic::read(schematic::SchematicFiles {
            root: "t.kicad_sch",
            files: &files,
        })?;
        let pin = |r: &str, n: &str| (r.to_owned(), n.to_owned());
        let nets = source_nets(&schematic);
        let net_of = |p: SourcePin| nets.iter().position(|n| n.contains(&p));
        // The T joins R1.2 and R3.1; the crossing leaves R2.2 apart.
        assert_eq!(net_of(pin("R1", "2")), net_of(pin("R3", "1")));
        assert_ne!(net_of(pin("R1", "2")), net_of(pin("R2", "2")));
        // Labels of one name join.
        assert_eq!(net_of(pin("R4", "1")), net_of(pin("R4", "2")));
        // Pins that meet nothing are nets of their own.
        assert_ne!(net_of(pin("R1", "1")), net_of(pin("R2", "1")));
        assert_eq!(nets.len(), 6, "{nets:?}");
        Ok(())
    }
}
