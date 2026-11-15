//! A package's shape as the reserved filters `pins:`, `pitch:` and `body:`
//! test it, read from the package's own data: its lead contacts (else its
//! terminals) and its envelope.
use super::Term;
use super::query::{BODY_TOLERANCE_MM, PITCH_TOLERANCE_MM};
use crate::library::PackageDefinition;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A package's shape: what `pins:`, `pitch:` and `body:` test.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PackageShape {
    /// Lead contacts, else terminals.
    pub pins: u32,
    /// The least distance between two lead contacts, in mm; `None` for a
    /// package with fewer than two.
    #[serde(default)]
    pub pitch: Option<f64>,
    /// The envelope's length, width and height in mm.
    #[serde(default)]
    pub body: Option<[f64; 3]>,
}

impl PackageShape {
    /// The shape of `package`; `None` for a package with no terminals.
    pub fn of(package: &PackageDefinition) -> Option<Self> {
        let count = if package.leads.is_empty() {
            package.terminals.len()
        } else {
            package.leads.len()
        };
        let pins = u32::try_from(count).ok().filter(|n| *n > 0)?;
        let points: Vec<[f64; 2]> = package
            .leads
            .iter()
            .map(|l| l.contact.to_mm_f64())
            .collect();
        let mut pitch: Option<f64> = None;
        for (k, a) in points.iter().enumerate() {
            for b in &points[k + 1..] {
                let d = (a[0] - b[0]).hypot(a[1] - b[1]);
                if d > 1e-6 && pitch.is_none_or(|p| d < p) {
                    pitch = Some(d);
                }
            }
        }
        let body = package.envelope_valid().then(|| package.envelope());
        Some(Self { pins, pitch, body })
    }

    /// Whether the shape meets a `pins:`, `pitch:` or `body:` term; other
    /// terms are not shape terms and are met.
    pub fn meets(&self, term: &Term) -> bool {
        match term {
            Term::Pins { op, count } => {
                op.holds_within(f64::from(self.pins), f64::from(*count), 0.0)
            }
            Term::Pitch { op, mm } => self
                .pitch
                .is_some_and(|p| op.holds_within(p, *mm, PITCH_TOLERANCE_MM)),
            Term::Body { size } => self.body.is_some_and(|[l, w, h]| {
                let near = |a: f64, b: f64| (a - b).abs() <= BODY_TOLERANCE_MM;
                let plane = match size.as_slice() {
                    [a, b, ..] => (near(l, *a) && near(w, *b)) || (near(l, *b) && near(w, *a)),
                    _ => false,
                };
                plane && size.get(2).is_none_or(|z| near(h, *z))
            }),
            _ => true,
        }
    }
}
