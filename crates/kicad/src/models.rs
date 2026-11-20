//! Model references from the public file-format documentation. Resolution and
//! CAD evaluation belong to the host; this reader never accesses a filesystem.
use crate::board::BoardError;
use crate::sexpr::Node;
use sparksmith_core::Engine;
use sparksmith_core::library::Pad;
use sparksmith_core::units::{Nm, PointNm};
use sparksmith_core::{BindingRef, ComponentRef, FootprintRef, SymbolRef};

pub const MAX_MODELS: usize = 32;
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub path: String,
    pub offset_nm: [Nm; 3],
    pub rotation_deg: [f64; 3],
    pub scale: [f64; 3],
    pub hidden: bool,
}
impl Reference {
    pub fn read(node: &Node) -> Result<Self, BoardError> {
        let path = node
            .arg(0)
            .ok_or_else(|| BoardError::Value("model/path".into()))?;
        if path.len() > 4096 {
            return Err(BoardError::Limit("model path exceeds 4096 bytes"));
        }
        let triple = |key: &str, default: [f64; 3]| -> Result<[f64; 3], BoardError> {
            let Some(part) = node.child(key) else {
                return Ok(default);
            };
            let xyz = part
                .child("xyz")
                .ok_or_else(|| BoardError::Value(format!("model/{key}/xyz")))?;
            let mut out = [0.0; 3];
            for (n, v) in out.iter_mut().enumerate() {
                *v = xyz
                    .arg(n)
                    .and_then(|s| s.parse().ok())
                    .filter(|v: &f64| v.is_finite())
                    .ok_or_else(|| BoardError::Value(format!("model/{key}")))?;
            }
            Ok(out)
        };
        let offset = if node.child("offset").is_some() {
            triple("offset", [0.0; 3])?
        } else {
            triple("at", [0.0; 3])?.map(|v| v * 25.4)
        };
        let nm =
            |v| Nm::from_mm_f64_rounded(v).map_err(|_| BoardError::Value("model/offset".into()));
        let scale = triple("scale", [1.0; 3])?;
        if scale.iter().any(|v| !(1e-6..=1e6).contains(v)) {
            return Err(BoardError::Value("model/scale".into()));
        }
        Ok(Self {
            path: path.into(),
            offset_nm: [nm(offset[0])?, nm(offset[1])?, nm(offset[2])?],
            rotation_deg: triple("rotate", [0.0; 3])?,
            scale,
            hidden: node.child("hide").is_some()
                || node.args().iter().any(|n| n.atom() == Some("hide")),
        })
    }
}

/// The manufacturer part number and manufacturer a part's properties
/// name, by the conventional property names (any letter case, spaces,
/// `_` and `-` ignored): `MPN`, `Manufacturer Part Number`, `MFR PN`,
/// `Part Number` …; `Manufacturer`, `MFR`, `Manf`. Empty values and KiCad's
/// `~` name nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartNumber {
    pub mpn: Option<String>,
    pub manufacturer: Option<String>,
}

/// Longest part number or manufacturer kept, bytes.
const MAX_PART_TEXT: usize = 256;

impl PartNumber {
    /// Take `name`'s `value` when it is a part number or manufacturer
    /// property (the first of each wins).
    pub fn read(&mut self, name: &str, value: &str) {
        let key: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        let value = value.trim();
        if value.is_empty()
            || value == "~"
            || value.len() > MAX_PART_TEXT
            || value.contains(char::is_control)
        {
            return;
        }
        let slot = match key.as_str() {
            "MPN"
            | "MANUFACTURERPARTNUMBER"
            | "MANUFACTURERPN"
            | "MFRPN"
            | "MFRPARTNUMBER"
            | "MFGPN"
            | "MFGPARTNUMBER"
            | "MANFPARTNUMBER"
            | "MANFPN"
            | "PARTNUMBER" => &mut self.mpn,
            "MANUFACTURER" | "MANUFACTURERNAME" | "MFR" | "MFG" | "MANF" => &mut self.manufacturer,
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(value.to_owned());
        }
    }
}

/// What the source footprint says of its package besides its lands: hints
/// for telling equally fitting presets apart, never geometry to import.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
    /// The footprint's name, without its library nickname.
    pub name: String,
    /// The extent (x, y) of its fabrication-layer outline, footprint frame.
    pub fab_extent_nm: Option<[Nm; 2]>,
    /// The extent (x, y) of its courtyard, footprint frame.
    pub courtyard_extent_nm: Option<[Nm; 2]>,
}
impl Facts {
    pub fn of(footprint: &crate::board::Footprint) -> Self {
        let extent = |b: Option<[crate::board::PointNm; 2]>| {
            let [lo, hi] = b?;
            let along = |a: usize| Nm::new(hi[a].checked_sub(lo[a])?).ok();
            Some([along(0)?, along(1)?])
        };
        let name = footprint
            .lib_id
            .rsplit_once(':')
            .map_or(footprint.lib_id.as_str(), |(_, name)| name);
        Self {
            name: name.to_owned(),
            fab_extent_nm: extent(footprint.fab_bounds),
            courtyard_extent_nm: extent(footprint.courtyard_bounds),
        }
    }
}

/// Why `footprint` is no part to draw, as its source has it: it names no 3D
/// model and nothing is assembled there (KiCad's board-only or
/// exclude-from-position-files attributes; a part not populated keeps its
/// body, which a variant may fit), or it is bare copper (no pad on a net on a board with nets:
/// a fiducial, a logo;
/// a net tie; one surface pad: a test or contact pad). `None` for a part
/// that needs a body.
pub fn no_body(footprint: &crate::board::Footprint, board_nets: bool) -> Option<&'static str> {
    use crate::board::PadType;
    if !footprint.models.is_empty() {
        return None;
    }
    if footprint.board_only || footprint.not_assembled {
        Some("not assembled (KiCad's board-only or exclude attribute)")
    } else if footprint.net_tie {
        Some("a net tie")
    } else if board_nets && footprint.pads.iter().all(|p| p.net.is_none()) {
        Some("no pad on a net (a fiducial, a logo or a mark)")
    } else if let [pad] = footprint.pads.as_slice()
        && pad.kind == PadType::Smd
    {
        Some("one surface pad (a test or contact pad)")
    } else {
        None
    }
}

/// One source footprint, as the host is asked to make its part.
pub struct PartRequest<'a> {
    /// Its pads in the footprint's frame, numbered, each landing its
    /// source terminal.
    pub pads: &'a [Pad],
    /// The designator prefix of its first reference.
    pub prefix: &'a str,
    /// Its model references (sources for a recreation, never a body).
    pub models: &'a [Reference],
    pub facts: &'a Facts,
    /// The board net of each pad (by index into `pads`), for every
    /// placed copy of the footprint: a generator terminal may never join
    /// pads a copy keeps on different nets.
    pub nets: &'a [Vec<Option<u32>>],
    /// The manufacturer part number the source names, when it names one.
    pub mpn: Option<&'a str>,
}

/// A part a generator made for a source footprint, through the same
/// `library.generate` as any generated part: its land pattern is the
/// source footprint's lands, so the board's copper is unchanged.
#[derive(Clone, Debug)]
pub struct Generated {
    pub component: ComponentRef,
    pub symbol: SymbolRef,
    pub binding: BindingRef,
    pub footprint: FootprintRef,
    /// The part's package frame in the source footprint's frame: turned
    /// counter-clockwise by `turn_deg` (a quarter turn), then moved by
    /// `offset`. A placement of the source footprint composed with it
    /// places the part's copper exactly where the source's is.
    pub turn_deg: f64,
    pub offset: PointNm,
    /// The symbol's units.
    pub units: u32,
    /// The generator and preset, and how the lands were paired, in words.
    pub reason: String,
    /// The generator family's name (`chip`, `gull-wing`, …).
    pub generator: String,
}

/// What one source footprint becomes.
pub enum Made {
    Generated(Box<Generated>),
    /// No generator makes it: a project part of the source lands, with
    /// its case, the reason and why no generator fits.
    Source {
        case: crate::report::Case,
        reason: String,
        placeholder: Option<crate::report::Placeholder>,
    },
}

/// What the host found besides the part: the package the source's part
/// number names, when it is not the package the lands are (the lands are
/// kept as the source has them).
pub struct Outcome {
    pub made: Made,
    pub contradiction: Option<String>,
}

/// The host makes parts while the new engine is still private. Any error
/// discards it; no existing document is changed.
pub trait Provider {
    fn part(&mut self, engine: &mut Engine, request: &PartRequest<'_>) -> Result<Outcome, String>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_modern_offsets_scale_rotation_and_legacy_inches()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = crate::sexpr::parse(br#"(model "body.step" (offset (xyz 1 2 3)) (scale (xyz 2 3 4)) (rotate (xyz 90 -20 5)))"#)?;
        let model = Reference::read(&root)?;
        assert_eq!(model.offset_nm.map(Nm::to_mm_f64), [1.0, 2.0, 3.0]);
        assert_eq!(model.scale, [2.0, 3.0, 4.0]);
        assert_eq!(model.rotation_deg, [90.0, -20.0, 5.0]);
        let root = crate::sexpr::parse(br#"(model "body.step" (at (xyz 1 0 0)) hide)"#)?;
        let legacy = Reference::read(&root)?;
        assert_eq!(legacy.offset_nm[0].to_mm_f64(), 25.4);
        assert!(legacy.hidden);
        Ok(())
    }
    #[test]
    fn facts_carry_the_footprint_name_and_its_body_and_courtyard_extents()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"(kicad_pcb (version 20240108) (generator "original facts fixture")
            (layers (0 "F.Cu" signal))
            (footprint "Own:Body_4x3" (layer "F.Cu") (at 10 10 90)
              (property "Reference" "U1")
              (fp_line (start -2 -1.5) (end 2 -1.5) (layer "F.Fab"))
              (fp_poly (pts (xy 2 -1.5) (xy 2 1.5) (xy -2 1.5)) (layer "F.Fab"))
              (fp_circle (center 0 0) (end 2.5 0) (layer "F.CrtYd"))
              (fp_line (start -9 -9) (end 9 9) (layer "F.SilkS"))
              (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu")))
            (footprint "NoOutline" (layer "F.Cu") (at 0 0)
              (property "Reference" "U2")
              (pad "1" smd rect (at 0 0) (size 1 1) (layers "F.Cu"))))"#;
        let board = crate::board::read(text.as_bytes(), &mut crate::report::Report::default())?;
        let facts = Facts::of(&board.footprints[0]);
        assert_eq!(facts.name, "Body_4x3");
        let mm = |e: Option<[Nm; 2]>| e.map(|e| e.map(Nm::to_mm_f64));
        assert_eq!(mm(facts.fab_extent_nm), Some([4.0, 3.0]));
        assert_eq!(mm(facts.courtyard_extent_nm), Some([5.0, 5.0]));
        let bare = Facts::of(&board.footprints[1]);
        assert_eq!(bare.name, "NoOutline");
        assert_eq!((bare.fab_extent_nm, bare.courtyard_extent_nm), (None, None));
        Ok(())
    }
    #[test]
    fn malformed_transforms_refuse_instead_of_showing_a_wrong_model()
    -> Result<(), Box<dyn std::error::Error>> {
        for text in [
            r#"(model "x.step" (scale (xyz 0 1 1)))"#,
            r#"(model "x.step" (rotate (xyz NaN 0 0)))"#,
            r#"(model "x.step" (offset (xyz 1 2)))"#,
        ] {
            assert!(Reference::read(&crate::sexpr::parse(text.as_bytes())?).is_err());
        }
        Ok(())
    }
}
