//! A preview body's triangles: the part file evaluated by Solidcraft at the
//! package's stored values (each body's display mesh and its mapped
//! material's base colour, metallic and roughness, moved by the package's
//! model placement), or the envelope box. Runs on the preview worker.
use super::raster::{MeshPart, Model};
use super::source::Body;
use sparksmith_core::materials::MaterialId;
use sparksmith_solidcraft::models::{look, part_file};

/// The neutral look of a body whose material is unknown.
const GREY: ([f32; 3], f32, f32) = ([0.18, 0.18, 0.19], 0.0, 0.55);

/// What the 3D view shows, for the pane's caption.
pub fn label(body: &Body) -> String {
    match body {
        Body::PartFile { .. } => "3D body".into(),
        Body::Envelope { note, .. } => note.clone(),
        Body::Absent { reason } => reason.clone(),
    }
}

fn material(id: Option<MaterialId>) -> ([f32; 3], f32, f32) {
    id.and_then(|m| look::definition(m).ok()).map_or(GREY, |d| {
        (
            d.base_linear.map(|c| c as f32),
            d.metallic as f32,
            d.roughness as f32,
        )
    })
}

/// The triangles `body` draws; an error says why there are none.
pub fn model(body: &Body) -> Result<Model, String> {
    match body {
        Body::Absent { reason } => Err(reason.clone()),
        Body::Envelope {
            size_mm,
            material: m,
            ..
        } => {
            let (colour, metallic, roughness) = material(*m);
            Model::boxed(*size_mm, colour, metallic, roughness)
        }
        Body::PartFile {
            bytes,
            values,
            placement,
        } => {
            let part = part_file::evaluate(bytes.as_slice(), values).map_err(|e| e.to_string())?;
            let r = placement.rotation;
            let t = placement.translation;
            let turn = |v: [f64; 3]| {
                [0, 1, 2].map(|i| (r[i][0] * v[0] + r[i][1] * v[1] + r[i][2] * v[2]) as f32)
            };
            let mut parts = Vec::new();
            for body in &part.bodies {
                let Some(state) = part
                    .state()
                    .bodies
                    .iter()
                    .find(|b| b.definition_name() == body.name || b.name == body.name)
                else {
                    continue;
                };
                let mesh = state.mesh();
                let (colour, metallic, roughness) = material(body.material);
                parts.push(MeshPart {
                    positions: mesh
                        .positions
                        .iter()
                        .map(|p| {
                            let q = turn([p.x, p.y, p.z]);
                            [q[0] + t[0] as f32, q[1] + t[1] as f32, q[2] + t[2] as f32]
                        })
                        .collect(),
                    normals: mesh.normals.iter().map(|n| turn([n.x, n.y, n.z])).collect(),
                    triangles: mesh.triangles.clone(),
                    colour,
                    metallic,
                    roughness,
                });
            }
            Model::new(parts)
        }
    }
}
