//! What one part preview shows, read from library records: the package's
//! body (its part file at the package's stored values, else the envelope
//! box), the footprint's lands and drawings, and the first symbol unit as
//! core resolves it. Cheap: no evaluation happens here, asset bytes are
//! shared, and the result is plain data a worker can take.
use sparksmith_core::geometry::{Bounds, Point, Shape, Transform3};
use sparksmith_core::ids::{BindingRef, ComponentRef, ContentHash, DefinitionKind, PartRef};
use sparksmith_core::library::bundle::{AssetBytes, local_library};
use sparksmith_core::library::graphics::{DrawingLayer, Item};
use sparksmith_core::library::package_model::{PackageModel, PartValue};
use sparksmith_core::library::{Bundle, FootprintDefinition, Library, PackedAsset};
use sparksmith_core::materials::MaterialId;
use sparksmith_core::model::Project;
use sparksmith_core::schematic::{self, DefinitionUnit, PlacedUnit};
use std::collections::BTreeMap;

/// Curves of pads and drawings are flattened to this chord error (mm).
const CHORD_MM: f64 = 0.01;

/// The 3D body a preview draws.
#[derive(Clone, Debug)]
pub enum Body {
    /// A parametric Solidcraft part file at the package's values, placed
    /// by the package's model placement.
    PartFile {
        bytes: AssetBytes,
        values: BTreeMap<String, PartValue>,
        placement: Transform3,
    },
    /// The package envelope as a box: an approximation, labelled so.
    Envelope {
        size_mm: [f64; 3],
        material: Option<MaterialId>,
        /// Why the envelope stands in for a body.
        note: String,
    },
    /// Nothing to draw in 3D, with the reason.
    Absent { reason: String },
}

/// A footprint as the thumbnail paints it, in millimetres, y up.
#[derive(Clone, Debug, Default)]
pub struct FootprintDrawing {
    /// Copper land outlines.
    pub pads: Vec<Vec<Point>>,
    /// Drilled hole outlines.
    pub holes: Vec<Vec<Point>>,
    /// Silkscreen, courtyard and assembly strokes with their pen width.
    pub lines: Vec<(Shape, f64, DrawingLayer)>,
    pub bounds: Option<Bounds>,
}

/// Everything one preview needs.
#[derive(Clone, Debug)]
pub struct PreviewSource {
    pub body: Body,
    pub footprint: Option<FootprintDrawing>,
    pub symbol: Option<PlacedUnit>,
}

impl PreviewSource {
    /// The root component (or a Part's component) of `bundle`, with the
    /// part file from the bundle's packed assets.
    pub fn from_bundle(bundle: &Bundle) -> Result<Self, String> {
        let local = local_library(&bundle.records).map_err(|e| e.to_string())?;
        let root = local
            .lineages
            .iter()
            .find(|l| l.lineage == bundle.root.lineage)
            .map(|l| l.local)
            .ok_or_else(|| format!("{}: the bundle lacks its root record", bundle.name))?;
        let kind = bundle
            .records
            .iter()
            .find(|r| r.lineage == bundle.root.lineage && r.revision == bundle.root.revision)
            .map(|r| r.kind)
            .ok_or_else(|| format!("{}: the bundle lacks its root record", bundle.name))?;
        let library = &local.library;
        let (component, binding) = match kind {
            DefinitionKind::Component => (ComponentRef::new(root, bundle.root.revision), None),
            DefinitionKind::Part => {
                let part = library
                    .part(PartRef::new(root, bundle.root.revision))
                    .ok_or("the Part record does not resolve")?;
                (part.definition.component, Some(part.definition.binding))
            }
            other => return Err(format!("a {other:?} is not a part to preview")),
        };
        Self::of(library, &bundle.assets, component, binding)
    }

    /// A component revision the project holds, with the project's packed
    /// assets.
    pub fn from_project(project: &Project, component: ComponentRef) -> Result<Self, String> {
        let library = &project.library;
        Self::of(library, &library.assets, component, None)
    }

    fn of(
        library: &Library,
        assets: &[PackedAsset],
        component: ComponentRef,
        binding: Option<BindingRef>,
    ) -> Result<Self, String> {
        let comp = library
            .component(component)
            .ok_or("the component record does not resolve")?;
        let binding = binding
            .or_else(|| comp.definition.implementations.first().copied())
            .and_then(|b| library.binding(b));
        let package = binding.and_then(|b| library.package(b.definition.package));
        let footprint = binding
            .and_then(|b| b.definition.footprints.first().copied())
            .and_then(|f| library.footprint(f));
        let body = match package {
            Some(p) => body_of(library, assets, &p.definition),
            None => Body::Absent {
                reason: "it has no package".into(),
            },
        };
        let symbol = comp
            .definition
            .representations
            .first()
            .and_then(|s| library.symbol(*s))
            .and_then(|s| {
                let unit = s.definition.units.first()?.id;
                schematic::definition_unit(
                    library,
                    DefinitionUnit {
                        symbol: &s.definition,
                        unit,
                        component: &comp.definition,
                        component_name: &comp.meta.name,
                        physical: binding
                            .zip(package)
                            .map(|(b, p)| (&b.definition, &p.definition)),
                    },
                )
                .ok()
            });
        Ok(Self {
            body,
            footprint: footprint.map(|f| drawing(&f.definition)),
            symbol,
        })
    }
}

fn body_of(
    library: &Library,
    assets: &[PackedAsset],
    package: &sparksmith_core::library::PackageDefinition,
) -> Body {
    let size_mm = package.envelope_nm.map(|v| v.to_mm_f64());
    let envelope = |material: Option<MaterialId>, note: &str| Body::Envelope {
        size_mm,
        material,
        note: note.to_owned(),
    };
    match &package.model {
        PackageModel::PartFile(pf) => {
            let Some(model) = library.model(pf.model) else {
                return envelope(None, "Envelope: the part file record is missing");
            };
            match asset(assets, model.definition.asset.hash) {
                Some(bytes) => Body::PartFile {
                    bytes,
                    values: pf.values.clone(),
                    placement: package.placement.transform(),
                },
                None => envelope(None, "Envelope: the part file is not packed"),
            }
        }
        PackageModel::Envelope { material } => envelope(Some(*material), "Envelope (approximate)"),
        PackageModel::Library(_) => envelope(
            None,
            "Envelope: previews draw part files; this model is another format",
        ),
        PackageModel::NoBody => Body::Absent {
            reason: "The package declares no 3D body".into(),
        },
    }
}

fn asset(assets: &[PackedAsset], hash: ContentHash) -> Option<AssetBytes> {
    assets
        .iter()
        .find(|a| a.hash == hash)
        .map(|a| a.data.clone())
}

/// The footprint's lands, holes and drawings in millimetres.
pub fn drawing(footprint: &FootprintDefinition) -> FootprintDrawing {
    let mut out = FootprintDrawing::default();
    let mut bounds: Option<Bounds> = None;
    let mut grow = |b: Bounds| bounds = Some(bounds.map_or(b, |a| a.union(b)));
    for pad in &footprint.pads {
        let ring = pad.outline(CHORD_MM);
        if ring.len() >= 3 {
            grow(pad.bounds());
            out.pads.push(ring);
        }
        if let Some(hole) = pad.hole_shape() {
            out.holes.push(hole.flatten(CHORD_MM));
        }
    }
    for graphic in &footprint.graphics {
        if let Item::Shape {
            shape, width_nm, ..
        } = &graphic.item
        {
            let shape = shape.to_mm();
            let width = width_nm.to_mm_f64();
            grow(shape.bounds(width));
            out.lines.push((shape, width, graphic.layer));
        }
    }
    out.bounds = bounds;
    out
}
