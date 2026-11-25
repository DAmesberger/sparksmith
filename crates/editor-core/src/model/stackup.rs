//! The board's layer stackup: copper foils and the dielectrics between them,
//! top to bottom, plus which sides carry soldermask and silkscreen. The
//! stackup sets the native board's thickness and copper heights; its rules
//! (`Stackup::check`) are pure data validation.
//!
//! What a new stackup is built of (foil, mask, dielectric thicknesses,
//! glass and its Dk estimates) is shipped data, a [`StackupPresets`]
//! family that `sparksmith-io` loads from `library/stackups/`; this module
//! holds only the build logic and the input bounds.
use super::Layer;
use crate::diagnostics::{Error, Result};
use crate::units::Nm;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Copper foil thickness of one ounce per square foot, in µm: a unit
/// conversion (IPC-4562 nominal 34.8 µm, rounded as fabricators quote it),
/// used to show foils in ounces. Which foil a new stackup gets is shipped
/// data ([`StackupPresets::copper_um`]).
pub const OUNCE_UM: f64 = 35.0;
/// Fewest copper layers of a stackup: a double-sided board.
pub const MIN_COPPER_LAYERS: usize = 2;
/// Most copper layers of a stackup. Not a fabrication limit: production
/// boards above about 50 layers are rare and 64 stays above them. It bounds
/// the per-layer work a hostile or mistaken file can ask for (layer rows
/// and commands, artwork files, 3D foils and edge bands).
pub const MAX_COPPER_LAYERS: usize = 64;
/// Finished copper foil thicknesses a stackup accepts, in µm.
pub const COPPER_UM_RANGE: std::ops::RangeInclusive<f64> = 5.0..=210.0;
/// Dielectric thicknesses a stackup accepts, in mm.
pub const DIELECTRIC_MM_RANGE: std::ops::RangeInclusive<f64> = 0.025..=3.2;
/// Soldermask thicknesses (over copper or laminate) a stackup accepts, in
/// µm. An input
/// bound, not a fabrication limit: from below the thinnest LPI coat to
/// above dry-film masks.
pub const MASK_UM_RANGE: std::ops::RangeInclusive<f64> = 5.0..=100.0;

/// A named copper layer of the board, top to bottom. Its identity is its
/// name: `Inner(1)` is the first inner layer below the top, whatever the
/// stackup's count. Serialised as `"Top"`, `{"Inner": n}` and `"Bottom"`,
/// so outer layers read as the board sides they are.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub enum CopperLayer {
    Top,
    /// Inner layer `n`, counted from 1 below the top.
    Inner(u8),
    Bottom,
}

impl From<CopperLayer> for crate::rules::ClassLayer {
    fn from(layer: CopperLayer) -> Self {
        match layer {
            CopperLayer::Top => Self::Top,
            CopperLayer::Inner(n) => Self::Inner(n),
            CopperLayer::Bottom => Self::Bottom,
        }
    }
}

impl CopperLayer {
    /// The board side of an outer layer; `None` for an inner layer.
    pub fn side(self) -> Option<Layer> {
        match self {
            CopperLayer::Top => Some(Layer::Top),
            CopperLayer::Bottom => Some(Layer::Bottom),
            CopperLayer::Inner(_) => None,
        }
    }

    /// The user's name of the layer ("Top", "Inner 1", "Bottom").
    pub fn name(self) -> String {
        match self {
            CopperLayer::Top => "Top".into(),
            CopperLayer::Inner(n) => format!("Inner {n}"),
            CopperLayer::Bottom => "Bottom".into(),
        }
    }
}

impl From<Layer> for CopperLayer {
    fn from(side: Layer) -> Self {
        match side {
            Layer::Top => CopperLayer::Top,
            Layer::Bottom => CopperLayer::Bottom,
        }
    }
}

impl PartialEq<Layer> for CopperLayer {
    fn eq(&self, side: &Layer) -> bool {
        *self == CopperLayer::from(*side)
    }
}

/// One copper foil, as plated (outer) or laminated (inner).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CopperFoil {
    /// Finished copper thickness in µm (35 µm is 1 oz).
    pub thickness_um: f64,
    /// A reference plane (ground or power) for controlled impedance. When
    /// no foil of a stackup is marked, the impedance model takes the
    /// copper layers next to a signal layer as its references
    /// (`layout::impedance`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub plane: bool,
}

impl CopperFoil {
    /// A foil `thickness_um` thick, not a plane.
    pub fn of_um(thickness_um: f64) -> Self {
        Self {
            thickness_um,
            plane: false,
        }
    }
}

/// How a dielectric is built: a cured core laminate or prepreg bonding plies.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DielectricKind {
    Core,
    Prepreg,
}

/// E-glass fabric style (IPC-4412 designations), which sets the weave pitch
/// the routed board edge shows. `Unspecified`: the source names no one style
/// (a core listed by thickness, plies of several styles between two foils),
/// so it has no Dk estimate and shows no weave.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum GlassStyle {
    Style106,
    Style1080,
    Style2116,
    Style2313,
    Style3313,
    Style7628,
    Unspecified,
}

impl GlassStyle {
    /// Every style, finest first, `Unspecified` last.
    pub const ALL: [Self; 7] = [
        Self::Style106,
        Self::Style1080,
        Self::Style2313,
        Self::Style3313,
        Self::Style2116,
        Self::Style7628,
        Self::Unspecified,
    ];

    /// Its designation ("2116"); `None` when it names no style.
    pub const fn designation(self) -> Option<&'static str> {
        match self {
            Self::Style106 => Some("106"),
            Self::Style1080 => Some("1080"),
            Self::Style2116 => Some("2116"),
            Self::Style2313 => Some("2313"),
            Self::Style3313 => Some("3313"),
            Self::Style7628 => Some("7628"),
            Self::Unspecified => None,
        }
    }

    /// The style of a designation.
    pub fn from_designation(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|g| g.designation() == Some(text))
    }
}

/// One sheet of a dielectric made of several, as its fabricator lists it.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DielectricSheet {
    pub material: String,
    /// As pressed.
    pub thickness_nm: Nm,
    pub dk: f64,
}

impl DielectricSheet {
    /// A sheet `mm` thick; none when the thickness is not a storable length.
    pub fn from_mm(material: String, mm: f64, dk: f64) -> Option<Self> {
        Some(Self {
            material,
            thickness_nm: Nm::from_mm_f64_rounded(mm).ok()?,
            dk,
        })
    }
}

/// One dielectric between two consecutive copper foils.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Dielectric {
    pub kind: DielectricKind,
    pub thickness_mm: f64,
    pub glass: GlassStyle,
    /// Relative permittivity (Dk) of this layer: the one the fabricator
    /// quotes, or calculated from `sheets`. When absent,
    /// [`Stackup::permittivity`] estimates it from the glass style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dk: Option<f64>,
    /// The sheets `dk` is calculated from, when they have several Dk. An
    /// edit they no longer give drops them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<DielectricSheet>,
}

/// The Dk of a path through dielectrics of several, each a thickness (mm)
/// and its Dk: averaged by thickness. The one rule for the sheets of a
/// dielectric and for the dielectrics between a trace and its plane
/// (`layout::impedance`). `None` without a thickness or with a part that
/// has no Dk.
pub fn weighted_dk(parts: impl IntoIterator<Item = (f64, Option<f64>)>) -> Option<f64> {
    let (mut thick, mut weighted) = (0.0, 0.0);
    for (mm, dk) in parts {
        thick += mm;
        weighted += mm * dk?;
    }
    (thick > 0.0).then(|| weighted / thick)
}

/// The Dk a dielectric of `sheets` takes: [`weighted_dk`] to four decimals.
pub fn sheets_dk(sheets: &[DielectricSheet]) -> Option<f64> {
    weighted_dk(
        sheets
            .iter()
            .map(|s| (s.thickness_nm.to_mm_f64(), Some(s.dk))),
    )
    .map(|dk| (dk * 1e4).round() / 1e4)
}

/// Sheets one dielectric names, at most.
pub const MAX_SHEETS: usize = 64;
/// Values this near (mm, or Dk) are the same.
const SAME: f64 = 1e-6;

/// The Dk values [`Dielectric::dk`] accepts.
pub const DK_RANGE: std::ops::RangeInclusive<f64> = 1.0..=20.0;

impl Dielectric {
    /// Whether [`Stackup::permittivity`] is an estimate: no Dk was entered.
    pub fn permittivity_estimated(&self) -> bool {
        self.dk.is_none()
    }

    /// The sheets the Dk is calculated from; none when it was entered, and
    /// none once the sheets no longer give this thickness and Dk.
    pub fn calculated_from(&self) -> Option<&[DielectricSheet]> {
        let mm: f64 = self.sheets.iter().map(|s| s.thickness_nm.to_mm_f64()).sum();
        let gives = sheets_dk(&self.sheets)
            .zip(self.dk)
            .is_some_and(|(sheets, dk)| (sheets - dk).abs() < SAME);
        (gives && (mm - self.thickness_mm).abs() < SAME).then_some(self.sheets.as_slice())
    }

    /// "Calculated by thickness from 0.1 mm 2116 of Dk 3.9 and 0.2 mm 7628
    /// of Dk 4.2", when it is.
    pub fn calculated_text(&self) -> Option<String> {
        let sheets: Vec<String> = self
            .calculated_from()?
            .iter()
            .map(|s| {
                let mm = s.thickness_nm.to_mm_f64();
                format!("{mm} mm {} of Dk {}", s.material, s.dk)
            })
            .collect();
        Some(format!(
            "Calculated by thickness from {}",
            sheets.join(" and ")
        ))
    }

    /// The same material, thickness, glass and Dk.
    fn same_layer(&self, other: &Self) -> bool {
        let dk = match (self.dk, other.dk) {
            (Some(a), Some(b)) => (a - b).abs() < SAME,
            (a, b) => a == b,
        };
        dk && self.kind == other.kind
            && self.glass == other.glass
            && (self.thickness_mm - other.thickness_mm).abs() < SAME
    }
}

/// The fabricator's stackup a board's layers were picked from. It names
/// them only while they are the ones picked: [`Stackup::picked`].
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PickedStackup {
    /// The id `layout.stackups` lists it by.
    pub id: String,
    /// The fabricator's own name for it.
    pub name: String,
    /// The layers it was picked with ([`Stackup::layers_digest`]).
    pub layers: String,
}

/// A value carried with the document for computation but not part of it:
/// never serialised, and every two are equal, so it takes no part in
/// equality, undo or revision comparisons.
#[derive(Clone, Debug, Default)]
pub struct Unsaved<T>(pub T);

impl<T> PartialEq for Unsaved<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// Relative permittivity estimates by glass style, for dielectrics without
/// an entered Dk. Shipped data with its sources (`library/stackups/`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(transparent)]
pub struct GlassDk(pub BTreeMap<GlassStyle, f64>);

impl GlassDk {
    /// The estimate for `glass`, if the table has one.
    pub fn get(&self, glass: GlassStyle) -> Option<f64> {
        self.0.get(&glass).copied()
    }
}

/// Nominal radial thickness of the copper on a finished plated hole, and
/// the measurement or published figure supporting it. An assumed value is
/// carried with the design until the stackup specifies a confirmed one.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BarrelPlating {
    pub wall_nm: Nm,
    pub source: String,
    pub assumed: bool,
}

impl BarrelPlating {
    fn check(&self) -> Result<()> {
        if self.wall_nm <= Nm::ZERO
            || self.source.trim().is_empty()
            || self.source.len() > crate::rules::MAX_PROFILE_NOTES
            || self.source.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "The nominal plated hole wall needs a positive thickness and a printable source."
                    .into(),
            ));
        }
        Ok(())
    }
}

/// The board's stackup. `copper` lists foils top to bottom; `dielectrics[i]`
/// lies between `copper[i]` and `copper[i + 1]`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stackup {
    pub copper: Vec<CopperFoil>,
    pub dielectrics: Vec<Dielectric>,
    /// A stackup edit may omit the wall to keep the current one. A saved
    /// board always carries its value and source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub barrel_plating: Option<BarrelPlating>,
    pub mask_top: bool,
    pub mask_bottom: bool,
    /// Soldermask thickness over copper in µm, on every side with mask;
    /// 0 or left out keeps the board's.
    #[serde(default)]
    pub mask_um: f64,
    /// Soldermask thickness over bare laminate, on every side with mask;
    /// 0 or left out keeps the board's.
    #[serde(default)]
    pub mask_laminate_nm: Nm,
    pub silkscreen_top: bool,
    pub silkscreen_bottom: bool,
    /// The Dk estimates by glass style for dielectrics without an entered
    /// Dk: the shipped data the stackup was made or opened with
    /// ([`StackupPresets::glass_dk`]). Never saved and never part of the
    /// stackup's equality ([`Unsaved`]).
    #[serde(skip)]
    #[schemars(skip)]
    pub dk_estimates: Unsaved<GlassDk>,
    /// The fabricator's stackup these layers were picked from
    /// (`layout.use_stackup`). Any edit of a layer drops it; read it through
    /// [`Stackup::picked`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picked: Option<PickedStackup>,
}

/// The glass a preset dielectric gets: the first step whose `from_mm` its
/// thickness reaches (else the last step).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GlassStep {
    pub from_mm: f64,
    pub glass: GlassStyle,
}

/// One dielectric a preset or an added layer uses.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PresetDielectric {
    pub thickness_mm: f64,
    pub glass: GlassStyle,
}

/// A family of stackup presets: every number a new stackup, a preset of
/// any layer count and an added layer read. It is shipped data
/// (`library/stackups/`, loaded by `sparksmith-io`); this type holds the
/// build logic only.
///
/// Every preset's finished thickness, copper foils included, is [`Self::board_mm`] up to [`Self::board_mm_up_to_layers`]
/// layers and [`Self::board_mm_per_layer_above`] a layer above that. A
/// two-layer preset is one core of what its two foils leave. A preset of
/// more layers is a symmetric foil build: prepreg outside and between the
/// cores, which carry the inner layers in pairs; its prepregs are [`Self::prepreg_mm`] up to
/// [`Self::prepreg_mm_up_to_layers`] layers and [`Self::prepreg_mm_above`]
/// above; the cores share the rest, rounded to µm.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StackupPresets {
    pub id: String,
    pub name: String,
    /// Copper foil of every preset layer and added layer, µm.
    pub copper_um: f64,
    pub barrel_plating: BarrelPlating,
    /// Soldermask over copper, µm.
    pub mask_um: f64,
    /// Soldermask over bare laminate, µm.
    pub mask_laminate_um: f64,
    pub board_mm: f64,
    pub board_mm_up_to_layers: usize,
    pub board_mm_per_layer_above: f64,
    pub prepreg_mm: f64,
    pub prepreg_mm_up_to_layers: usize,
    pub prepreg_mm_above: f64,
    /// Glass of a preset prepreg by thickness, thickest step first.
    pub prepreg_glass: Vec<GlassStep>,
    /// Glass of a preset core by thickness, thickest step first.
    pub core_glass: Vec<GlassStep>,
    /// The prepreg an added layer gets.
    pub added_prepreg: PresetDielectric,
    /// The core an added layer gets.
    pub added_core: PresetDielectric,
    pub glass_dk: GlassDk,
    /// Where the values come from, by field name.
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
}

impl StackupPresets {
    /// The values' own rules: every thickness within the stackup ranges,
    /// glass steps given and every Dk estimate within [`DK_RANGE`].
    pub fn check(&self) -> Result<()> {
        self.barrel_plating.check()?;
        let mm = |v: f64| v.is_finite() && v > 0.0 && v <= *DIELECTRIC_MM_RANGE.end();
        let ok = COPPER_UM_RANGE.contains(&self.copper_um)
            && MASK_UM_RANGE.contains(&self.mask_um)
            && MASK_UM_RANGE.contains(&self.mask_laminate_um)
            && [
                self.prepreg_mm,
                self.prepreg_mm_above,
                self.added_prepreg.thickness_mm,
                self.added_core.thickness_mm,
                self.board_mm,
                self.board_mm_per_layer_above,
            ]
            .into_iter()
            .all(mm)
            && !self.prepreg_glass.is_empty()
            && !self.core_glass.is_empty()
            && self
                .glass_dk
                .0
                .values()
                .all(|dk| dk.is_finite() && DK_RANGE.contains(dk));
        if ok {
            Ok(())
        } else {
            Err(Error::Invalid(format!(
                "the stackup presets {} hold a value outside the stackup ranges",
                self.id
            )))
        }
    }

    fn dielectric(kind: DielectricKind, layer: PresetDielectric) -> Dielectric {
        Dielectric {
            kind,
            thickness_mm: layer.thickness_mm,
            glass: layer.glass,
            dk: None,
            sheets: Vec::new(),
        }
    }

    /// The dielectric of `kind` an added layer gets.
    pub fn added(&self, kind: DielectricKind) -> Dielectric {
        match kind {
            DielectricKind::Prepreg => Self::dielectric(kind, self.added_prepreg),
            DielectricKind::Core => Self::dielectric(kind, self.added_core),
        }
    }

    /// One foil of the family's copper.
    pub fn foil(&self) -> CopperFoil {
        CopperFoil::of_um(self.copper_um)
    }

    fn glass(steps: &[GlassStep], mm: f64) -> Result<GlassStyle> {
        steps
            .iter()
            .find(|step| mm >= step.from_mm)
            .or(steps.last())
            .map(|step| step.glass)
            .ok_or_else(|| Error::Invalid("the stackup presets list no glass".into()))
    }

    /// The symmetric preset of `count` copper layers, for any count the
    /// stackup rules allow, with mask and silkscreen on both sides and
    /// this family's Dk estimates.
    pub fn preset(&self, count: usize) -> Result<Stackup> {
        check_count(count)?;
        let lossy = |v: usize| f64::from(u32::try_from(v).unwrap_or(u32::MAX));
        let board = if count <= self.board_mm_up_to_layers {
            self.board_mm
        } else {
            self.board_mm_per_layer_above * lossy(count)
        };
        // The foils are part of the finished thickness.
        let total = board - self.copper_um / 1000.0 * lossy(count);
        let dielectrics = if count == MIN_COPPER_LAYERS {
            let core_mm = (total * 1000.0).round() / 1000.0;
            vec![Dielectric {
                kind: DielectricKind::Core,
                thickness_mm: core_mm,
                glass: Self::glass(&self.core_glass, core_mm)?,
                dk: None,
                sheets: Vec::new(),
            }]
        } else {
            let prepregs = count / 2;
            let cores = prepregs - 1;
            let prepreg_mm = if count <= self.prepreg_mm_up_to_layers {
                self.prepreg_mm
            } else {
                self.prepreg_mm_above
            };
            let core_mm =
                ((total - prepreg_mm * lossy(prepregs)) / lossy(cores) * 1000.0).round() / 1000.0;
            let prepreg = Dielectric {
                kind: DielectricKind::Prepreg,
                thickness_mm: prepreg_mm,
                glass: Self::glass(&self.prepreg_glass, prepreg_mm)?,
                dk: None,
                sheets: Vec::new(),
            };
            let core = Dielectric {
                kind: DielectricKind::Core,
                thickness_mm: core_mm,
                glass: Self::glass(&self.core_glass, core_mm)?,
                dk: None,
                sheets: Vec::new(),
            };
            (0..count - 1)
                .map(|i| if i % 2 == 0 { &prepreg } else { &core })
                .cloned()
                .collect()
        };
        let stackup = Stackup {
            copper: vec![self.foil(); count],
            dielectrics,
            barrel_plating: Some(self.barrel_plating.clone()),
            mask_top: true,
            mask_bottom: true,
            mask_um: self.mask_um,
            mask_laminate_nm: Nm::from_mm_f64_rounded(self.mask_laminate_um / 1000.0)?,
            silkscreen_top: true,
            silkscreen_bottom: true,
            dk_estimates: Unsaved(self.glass_dk.clone()),
            picked: None,
        };
        stackup.check()?;
        Ok(stackup)
    }

    /// A test fixture, never in the product: the original test family
    /// `model/fixtures/stackup-presets.json`, not shipped and not offered.
    /// Its values equal the shipped family's, so the tests measured
    /// against a 1.6 mm two-layer board still are (a `sparksmith-io` test
    /// holds the two equal).
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn fixture() -> Result<Self> {
        let presets: Self = serde_json::from_str(include_str!("fixtures/stackup-presets.json"))
            .map_err(|e| Error::Invalid(format!("the fixture stackup presets: {e}")))?;
        presets.check()?;
        Ok(presets)
    }
}

/// A test fixture, never in the product: the fixture family's two-layer
/// preset ([`StackupPresets::fixture`]). A broken fixture gives an empty
/// stackup, which every check refuses.
#[cfg(any(test, feature = "test-fixtures"))]
impl Default for Stackup {
    fn default() -> Self {
        Self::preset(MIN_COPPER_LAYERS).unwrap_or_else(|_| Stackup {
            copper: Vec::new(),
            dielectrics: Vec::new(),
            barrel_plating: None,
            mask_top: false,
            mask_bottom: false,
            mask_um: 0.0,
            mask_laminate_nm: Nm::ZERO,
            silkscreen_top: false,
            silkscreen_bottom: false,
            dk_estimates: Unsaved::default(),
            picked: None,
        })
    }
}

impl Stackup {
    /// A test fixture, never in the product: the fixture family's preset
    /// of `count` layers ([`StackupPresets::preset`]).
    #[cfg(any(test, feature = "test-fixtures"))]
    pub fn preset(count: usize) -> Result<Self> {
        StackupPresets::fixture()?.preset(count)
    }

    /// The relative permittivity the impedance model uses for `d`: its
    /// entered [`Dielectric::dk`], else the estimate for its glass style
    /// ([`Stackup::dk_estimates`]); `None` when neither is known.
    pub fn permittivity(&self, d: &Dielectric) -> Option<f64> {
        d.dk.or_else(|| self.dk_estimates.0.get(d.glass))
    }

    /// This stackup with solder mask `over_copper_um` over copper and
    /// `over_laminate_um` over bare laminate.
    pub fn with_mask(mut self, over_copper_um: f64, over_laminate_um: f64) -> Result<Self> {
        self.mask_um = over_copper_um;
        self.mask_laminate_nm = Nm::from_mm_f64_rounded(over_laminate_um / 1000.0)?;
        Ok(self)
    }

    /// This stackup with what a request left out (mask thicknesses of 0,
    /// no Dk estimates) taken from `current`, the board's stackup.
    pub fn keeping_unstated(mut self, current: &Stackup) -> Self {
        if self.barrel_plating.is_none() {
            self.barrel_plating.clone_from(&current.barrel_plating);
        }
        if self.mask_um == 0.0 {
            self.mask_um = current.mask_um;
        }
        if self.mask_laminate_nm == Nm::ZERO {
            self.mask_laminate_nm = current.mask_laminate_nm;
        }
        if self.dk_estimates.0.0.is_empty() {
            self.dk_estimates = current.dk_estimates.clone();
        }
        for (d, was) in self.dielectrics.iter_mut().zip(&current.dielectrics) {
            if d.sheets.is_empty() && d.same_layer(was) {
                d.sheets.clone_from(&was.sheets);
            }
        }
        if self.picked.is_none() && self.same_layers(current) {
            self.picked.clone_from(&current.picked);
        }
        self.settled()
    }

    /// Whether the foils and dielectrics are `other`'s: thicknesses,
    /// materials, glass and Dk. Plane marks, mask, legend and hole wall are
    /// not layers.
    pub fn same_layers(&self, other: &Self) -> bool {
        self.copper.len() == other.copper.len()
            && self.dielectrics.len() == other.dielectrics.len()
            && self
                .copper
                .iter()
                .zip(&other.copper)
                .all(|(a, b)| (a.thickness_um - b.thickness_um).abs() / 1000.0 < SAME)
            && self
                .dielectrics
                .iter()
                .zip(&other.dielectrics)
                .all(|(a, b)| a.same_layer(b))
    }

    /// The layers in one word: what [`Self::same_layers`] compares, to a
    /// nanometre.
    pub fn layers_digest(&self) -> String {
        let fine = |v: f64| (v / SAME).round() as i64;
        let copper: Vec<i64> = self
            .copper
            .iter()
            .map(|c| fine(c.thickness_um / 1000.0))
            .collect();
        let dielectrics: Vec<_> = self
            .dielectrics
            .iter()
            .map(|d| (d.kind, fine(d.thickness_mm), d.glass, d.dk.map(fine)))
            .collect();
        let bytes = serde_json::to_vec(&(copper, dielectrics)).unwrap_or_default();
        format!("{:016x}", crate::library::footprint_source::fnv(&bytes))
    }

    /// The fabricator's stackup the board has: the picked one, while the
    /// layers still are the ones picked.
    pub fn picked(&self) -> Option<&PickedStackup> {
        self.picked
            .as_ref()
            .filter(|p| p.layers == self.layers_digest())
    }

    /// These layers as picked from the fabricator's stackup `id`.
    pub fn picked_from(mut self, id: &str, name: &str) -> Self {
        self.picked = Some(PickedStackup {
            id: id.to_owned(),
            name: name.to_owned(),
            layers: self.layers_digest(),
        });
        self
    }

    /// This stackup without what its layers no longer say: a picked name,
    /// sheets that no longer give the Dk.
    pub fn settled(mut self) -> Self {
        if self.picked().is_none() {
            self.picked = None;
        }
        for d in &mut self.dielectrics {
            if d.calculated_from().is_none() {
                d.sheets.clear();
            }
        }
        self
    }

    /// The copper layer counts [`StackupPresets::preset`] offers, in order.
    pub fn preset_counts() -> impl Iterator<Item = usize> {
        (MIN_COPPER_LAYERS..=MAX_COPPER_LAYERS).step_by(2)
    }

    /// The copper layers top to bottom.
    pub fn layers(&self) -> Vec<CopperLayer> {
        let count = self.copper.len();
        (0..count)
            .map(|i| match i {
                0 => CopperLayer::Top,
                i if i + 1 == count => CopperLayer::Bottom,
                // Bounded by `check`: at most `MAX_COPPER_LAYERS`.
                i => CopperLayer::Inner(u8::try_from(i).unwrap_or(u8::MAX)),
            })
            .collect()
    }

    /// Whether the stackup has this copper layer.
    pub fn has(&self, layer: CopperLayer) -> bool {
        match layer {
            CopperLayer::Top | CopperLayer::Bottom => true,
            CopperLayer::Inner(n) => n >= 1 && usize::from(n) + 1 < self.copper.len(),
        }
    }

    /// The index of a layer in `copper`.
    pub fn index(&self, layer: CopperLayer) -> Option<usize> {
        if !self.has(layer) {
            return None;
        }
        Some(match layer {
            CopperLayer::Top => 0,
            CopperLayer::Inner(n) => usize::from(n),
            CopperLayer::Bottom => self.copper.len().checked_sub(1)?,
        })
    }

    /// Total dielectric thickness in mm, foils excluded. The laminate's
    /// height adds the inner foils ([`Self::laminate_mm`]).
    pub fn dielectric_mm(&self) -> f64 {
        self.dielectrics.iter().map(|d| d.thickness_mm).sum()
    }

    /// The top and bottom foils' thicknesses in µm (`None` without foils).
    pub fn outer_copper_um(&self) -> Option<(f64, f64)> {
        Some((
            self.copper.first()?.thickness_um,
            self.copper.last()?.thickness_um,
        ))
    }

    /// Whether both outer foils are `um` thick.
    pub fn outer_copper_is(&self, um: f64) -> bool {
        self.outer_copper_um()
            .is_some_and(|(top, bottom)| (top - um).abs() < 1e-6 && (bottom - um).abs() < 1e-6)
    }

    /// This stackup with both outer foils `um` thick (inner foils and
    /// plane marks unchanged), checked.
    pub fn with_outer_copper(&self, um: f64) -> Result<Stackup> {
        let mut next = self.clone();
        let count = next.copper.len();
        for (i, foil) in next.copper.iter_mut().enumerate() {
            if i == 0 || i + 1 == count {
                foil.thickness_um = um;
            }
        }
        let next = next.settled();
        next.check()?;
        Ok(next)
    }

    /// Finished board thickness over the outer copper, in mm (mask excluded).
    pub fn thickness_mm(&self) -> f64 {
        self.dielectric_mm()
            + self
                .copper
                .iter()
                .map(|c| c.thickness_um / 1000.0)
                .sum::<f64>()
    }

    /// The z of the bottom face of `dielectrics[index]`: every dielectric
    /// below it and every inner foil between them, with z = 0 at the bottom
    /// face of the lowest dielectric.
    fn dielectric_base(&self, index: usize) -> f64 {
        let n = self.copper.len();
        let below: f64 = self
            .dielectrics
            .iter()
            .skip(index + 1)
            .map(|d| d.thickness_mm)
            .sum();
        let foils: f64 = self
            .copper
            .iter()
            .enumerate()
            .filter(|(k, _)| *k > index && k + 1 < n)
            .map(|(_, c)| c.thickness_um / 1000.0)
            .sum();
        below + foils
    }

    /// The laminate's height in mm: every dielectric and every inner foil,
    /// from the bottom foil's face (z = 0) to the top foil's. With the outer
    /// foils it is the finished thickness, [`Self::thickness_mm`].
    pub fn laminate_mm(&self) -> f64 {
        self.dielectric_span(0).map_or(0.0, |(_, top)| top)
    }

    /// A copper layer's z range in mm, with z = 0 at the bottom face of the
    /// lowest dielectric. Every foil takes its own height, as a fabricator's
    /// stackup lists it: an inner foil lies on the dielectric below it,
    /// the bottom foil under z = 0 and the top foil on the laminate.
    pub fn copper_span(&self, layer: CopperLayer) -> Option<(f64, f64)> {
        let index = self.index(layer)?;
        let foil = self.copper.get(index)?.thickness_um / 1000.0;
        if layer == CopperLayer::Bottom {
            return Some((-foil, 0.0));
        }
        let below = if layer == CopperLayer::Top { 0 } else { index };
        let (_, top) = self.dielectric_span(below)?;
        Some((top, top + foil))
    }

    /// The z range in mm of `dielectrics[index]`.
    pub fn dielectric_span(&self, index: usize) -> Option<(f64, f64)> {
        let dielectric = self.dielectrics.get(index)?;
        let base = self.dielectric_base(index);
        Some((base, base + dielectric.thickness_mm))
    }

    /// The nominal physical wall saved on the board, independent of the
    /// process allowance in the fabrication profile's drill file settings.
    pub fn plating(&self) -> Result<&BarrelPlating> {
        let plating = self.barrel_plating.as_ref().ok_or_else(|| {
            Error::Invalid("The stackup is missing its nominal plated hole wall and source.".into())
        })?;
        plating.check()?;
        Ok(plating)
    }

    /// The picked name and the sheets are bounded, printable text.
    fn check_texts(&self) -> Result<()> {
        let printable = |text: &str| {
            !text.trim().is_empty()
                && text.len() <= crate::rules::MAX_PROFILE_NOTES
                && !text.chars().any(char::is_control)
        };
        let picked = self
            .picked
            .iter()
            .all(|p| printable(&p.id) && printable(&p.name) && printable(&p.layers));
        let sheets = self.dielectrics.iter().all(|d| {
            d.sheets.len() <= MAX_SHEETS && d.sheets.iter().all(|s| printable(&s.material))
        });
        if picked && sheets {
            Ok(())
        } else {
            Err(Error::Invalid(
                "a stackup's name and its sheets' materials need printable text".into(),
            ))
        }
    }

    /// The stackup rules: an even number of copper layers from
    /// [`MIN_COPPER_LAYERS`] to [`MAX_COPPER_LAYERS`], exactly one
    /// dielectric between neighbouring foils, finite thicknesses within
    /// fabrication ranges, Dk within [`DK_RANGE`] and the mask thickness
    /// within [`MASK_UM_RANGE`].
    pub fn check(&self) -> Result<()> {
        check_count(self.copper.len())?;
        self.plating()?;
        self.check_texts()?;
        if self.dielectrics.len() + 1 != self.copper.len() {
            return Err(Error::Invalid(
                "a stackup has one dielectric between each pair of copper layers".into(),
            ));
        }
        if !self
            .copper
            .iter()
            .all(|c| c.thickness_um.is_finite() && COPPER_UM_RANGE.contains(&c.thickness_um))
        {
            return Err(Error::Invalid("copper thickness must be 5–210 µm".into()));
        }
        if !self
            .dielectrics
            .iter()
            .all(|d| d.thickness_mm.is_finite() && DIELECTRIC_MM_RANGE.contains(&d.thickness_mm))
        {
            return Err(Error::Invalid(
                "dielectric thickness must be 0.025–3.2 mm".into(),
            ));
        }
        if !self
            .dielectrics
            .iter()
            .filter_map(|d| d.dk)
            .all(|dk| dk.is_finite() && DK_RANGE.contains(&dk))
        {
            return Err(Error::Invalid("dielectric Dk must be 1–20".into()));
        }
        if ![self.mask_um, self.mask_laminate_nm.to_mm_f64() * 1000.0]
            .iter()
            .all(|um| um.is_finite() && MASK_UM_RANGE.contains(um))
        {
            return Err(Error::Invalid(
                "soldermask thickness must be 5–100 µm".into(),
            ));
        }
        Ok(())
    }
}

/// The copper count rules. Odd counts are refused: fabricators laminate
/// copper in pairs (double-sided cores and foil), so an odd board is the
/// next even build with one layer etched away, priced as that build and
/// prone to warp from the unbalanced copper.
fn check_count(count: usize) -> Result<()> {
    if count < MIN_COPPER_LAYERS {
        return Err(Error::Invalid(format!(
            "a stackup has at least {MIN_COPPER_LAYERS} copper layers"
        )));
    }
    if count > MAX_COPPER_LAYERS {
        return Err(Error::Invalid(format!(
            "a stackup has at most {MAX_COPPER_LAYERS} copper layers"
        )));
    }
    if !count.is_multiple_of(2) {
        return Err(Error::Invalid(
            "a stackup has an even number of copper layers".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "stackup_tests.rs"]
mod tests;
