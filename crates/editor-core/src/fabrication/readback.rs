//! A set's files read back and held to the design, before an export releases
//! any of them (`sparksmith_io::fabrication::export`). One difference refuses
//! the whole set; it is not a finding, so no waiver accepts it.
//!
//! The files are read by `sparksmith-fabcheck`, readers written apart from
//! the writers. What they must hold comes from the document by two ways:
//!
//! - **Apart from the writer** ([`oracle`]): every land, via, track, washer,
//!   bridge, mask and paste opening, silkscreen shape, hole, slot and contour,
//!   by exact distances to the document's own primitives, with none of the
//!   production geometry (placement transform, pad growth, slot reading); the
//!   part positions and angles of the pick-and-place files; the board's size
//!   and stackup.
//! - **Shared with the writer**, so a fault in it is in both and is not told:
//!   the document itself (its solved outline and the stored fill regions,
//!   which the fill audit checks), which footprint and package a part
//!   resolves to, which parts are fitted and on the bill of materials and
//!   what it says of them, net names and which pad is on which net, pad
//!   labels, the effective rule values, the drill-size allowance, the names
//!   of finishes, where the strokes of text lie, and, for the drawings,
//!   what is to be printed ([`super::DrawingData`]; its holes, parts and
//!   release are held to the document).
//!
//! What is compared, by file, and what is not: docs/fabrication-output.md,
//! "Read back before release".
pub(crate) mod oracle;

mod drawings;
mod drills;
mod image;
mod layers;
mod tables;

use super::{DrawingData, FabricationOptions, Identity, ProductionSnapshot};
use crate::diagnostics::{Error, Result};
use crate::geometry::Point;
use crate::model::Project;
use sparksmith_fabcheck::gerber::{self, GerberFile};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Fallible<T> = sparksmith_fabcheck::Fallible<T>;

/// The rule code of a refused set.
pub const MISMATCH: &str = "fabrication.readback_mismatch";
/// The rule code of a check that was stopped.
pub const STOPPED: &str = "fabrication.stopped";

/// Differences one check of one file lists.
const MOST: usize = 4;
/// Differences a refusal's text lists.
const LISTED: usize = 5;

/// One place where a file is not the design.
#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub file: String,
    /// Where on the board, mm.
    pub at: Option<Point>,
    /// What the file has there, and what the design has.
    pub written: String,
    pub design: String,
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.file)?;
        if let Some([x, y]) = self.at {
            write!(f, " at ({x:.3}, {y:.3}) mm")?;
        }
        write!(
            f,
            ": the file has {}, the design has {}",
            self.written, self.design
        )
    }
}

/// The refusal of a set with `found` differences.
pub fn refusal(found: &[Mismatch]) -> Error {
    let mut text =
        format!("{MISMATCH}: The files do not match the design, so nothing was written. ");
    let listed: Vec<String> = found.iter().take(LISTED).map(ToString::to_string).collect();
    text.push_str(&listed.join("; "));
    if found.len() > LISTED {
        text.push_str(&format!("; and {} more", found.len() - LISTED));
    }
    Error::Conflict(text)
}

/// The refusal of an export that was stopped.
pub fn stopped() -> Error {
    Error::Conflict(format!(
        "{STOPPED}: The export was stopped; nothing was written"
    ))
}

/// What is to be read back.
pub struct Set<'a> {
    pub snapshot: &'a ProductionSnapshot<'a>,
    pub options: &'a FabricationOptions,
    /// The release every file states.
    pub identity: &'a Identity,
    /// What the drawings print, when they are in the set.
    pub drawing: Option<&'a DrawingData>,
}

/// Where `files` (names and bytes, as they would be released) are not the
/// design of `set`; empty when every file is. `go` is called before each
/// file and within long ones, with the file's name; `false` stops the check
/// ([`STOPPED`]).
pub fn check(
    set: &Set<'_>,
    files: &[(String, Vec<u8>)],
    go: &mut dyn FnMut(&str) -> bool,
) -> Result<Vec<Mismatch>> {
    let project = set.snapshot.project();
    let stem = super::base_name(project, set.options)?;
    let mut going = true;
    let mut ask = |name: &str| {
        going = going && go(name);
        going
    };
    let mut reading = Reading {
        project,
        options: set.options,
        identity: set.identity,
        stem,
        files: BTreeMap::new(),
        seen: BTreeSet::new(),
        found: Vec::new(),
        go: &mut ask,
    };
    for (name, bytes) in files {
        if reading.files.insert(name.as_str(), bytes).is_some() {
            reading.say(name, None, "this file twice", "it once");
        }
    }
    if let Err(e) = read(set, &mut reading) {
        // What the check itself cannot tell releases nothing either.
        reading.say(
            "The set",
            None,
            &format!("what cannot be checked ({e})"),
            "a board that can",
        );
    }
    let extra: Vec<String> = reading
        .files
        .keys()
        .filter(|n| !reading.seen.contains(**n))
        .map(|n| (*n).to_owned())
        .collect();
    for name in extra {
        reading.say(&name, None, "this file", "no such file");
    }
    let found = std::mem::take(&mut reading.found);
    drop(reading);
    if going { Ok(found) } else { Err(stopped()) }
}

fn read(set: &Set<'_>, reading: &mut Reading<'_, '_>) -> Fallible<()> {
    let outputs = set.options.outputs;
    if outputs.gerber {
        layers::check(reading)?;
    }
    if outputs.drill {
        drills::check(reading)?;
    }
    if outputs.gerber {
        tables::job(reading)?;
    }
    if outputs.assembly {
        tables::assembly(reading)?;
    }
    if let Some(drawing) = set.drawing {
        drawings::check(reading, drawing)?;
    }
    Ok(())
}

/// The files being read back.
struct Reading<'a, 'g> {
    project: &'a Project,
    options: &'a FabricationOptions,
    identity: &'a Identity,
    stem: String,
    files: BTreeMap<&'a str, &'a [u8]>,
    /// The names a check asked for.
    seen: BTreeSet<String>,
    found: Vec<Mismatch>,
    go: &'g mut dyn FnMut(&str) -> bool,
}

impl<'a> Reading<'a, '_> {
    fn say(&mut self, file: &str, at: Option<Point>, written: &str, design: &str) {
        self.found.push(Mismatch {
            file: file.to_owned(),
            at,
            written: written.to_owned(),
            design: design.to_owned(),
        });
    }

    /// The name of a file of the set.
    fn name(&self, suffix: &str) -> String {
        format!("{}-{suffix}", self.stem)
    }

    /// Whether to go on, about to read `name`.
    fn going(&mut self, name: &str) -> bool {
        (self.go)(name)
    }

    /// The bytes of a file, when it is there; a `required` one that is not
    /// is a difference.
    fn bytes(&mut self, name: &str, required: bool) -> Option<&'a [u8]> {
        self.seen.insert(name.to_owned());
        if !self.going(name) {
            return None;
        }
        let bytes = self.files.get(name).copied();
        if bytes.is_none() && required {
            self.say(name, None, "no such file", "this file");
        }
        bytes
    }

    fn text(&mut self, name: &str, required: bool) -> Option<&'a str> {
        let bytes = self.bytes(name, required)?;
        match std::str::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(e) => {
                self.unreadable(name, &e.to_string());
                None
            }
        }
    }

    fn unreadable(&mut self, name: &str, why: &str) {
        self.say(
            name,
            None,
            &format!("what cannot be read ({why})"),
            "a file that can",
        );
    }

    /// A Gerber file with the head every one has: millimetres to the
    /// nanometre, its function and polarity, and the release.
    fn gerber(
        &mut self,
        suffix: &str,
        function: &str,
        polarity: &str,
        required: bool,
    ) -> Option<(String, GerberFile)> {
        let name = self.name(suffix);
        let text = self.text(&name, required)?;
        let file = match gerber::parse(text) {
            Ok(file) => file,
            Err(e) => {
                self.unreadable(&name, &e.to_string());
                return None;
            }
        };
        if !file.metric || file.decimals != 6 {
            let unit = if file.metric { "mm" } else { "inches" };
            self.say(
                &name,
                None,
                &format!("{unit} to {} decimals", file.decimals),
                "mm to 6 decimals",
            );
        }
        let stated = |k: &str| file.file_attribute(k).map(<[String]>::to_vec);
        self.head(&name, stated, function, polarity);
        Some((name, file))
    }

    /// The attributes every layer and drill file states.
    fn head(
        &mut self,
        name: &str,
        stated: impl Fn(&str) -> Option<Vec<String>>,
        function: &str,
        polarity: &str,
    ) {
        let identity = self.identity;
        let release = format!("{},{},{}", identity.name, identity.guid, identity.revision);
        for (what, attribute, design) in [
            ("the file function", ".FileFunction", function),
            ("the polarity", ".FilePolarity", polarity),
            ("the release", ".ProjectId", release.as_str()),
            ("the same coordinates", ".SameCoordinates", ""),
        ] {
            let written = stated(attribute).map(|v| v.join(","));
            if written.as_deref() != Some(design) {
                let written = written.unwrap_or_else(|| "none".into());
                self.say(
                    name,
                    None,
                    &format!("{what} “{written}”"),
                    &format!("“{design}”"),
                );
            }
        }
    }

    /// What an image check found, in words for `material`.
    fn wrong(&mut self, name: &str, wrong: Vec<oracle::Wrong>, material: &str) {
        for w in wrong.into_iter().take(2 * MOST) {
            let there = if w.what.is_empty() {
                material.to_owned()
            } else {
                format!("{material} ({})", w.what)
            };
            let none = format!("no {material}");
            if w.missing {
                self.say(name, Some(w.at), &none, &there);
            } else {
                self.say(name, Some(w.at), material, &none);
            }
        }
    }
}

/// A length in the files' units, nm.
fn nm(mm: f64) -> i64 {
    (mm * 1e6).round() as i64
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
