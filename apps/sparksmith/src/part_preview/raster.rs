//! A small CPU rasteriser for the part preview: an orthographic three-quarter
//! view from above of a few triangle meshes, turned about the vertical axis,
//! with per-vertex lighting from each body's base colour, metallic and
//! roughness, a soft contact shadow and a transparent background. Pure and
//! deterministic: the same model and angle give the same pixels. Rendered
//! at twice the output size and box-filtered down.
use egui::ColorImage;

/// Triangles one preview model may hold; a larger part is refused with
/// its count rather than stalling the worker.
pub const MAX_TRIANGLES: usize = 2_000_000;
/// Camera elevation above the board plane.
const ELEVATION_DEG: f32 = 28.0;
/// Share of the half image the model's bounding sphere fills.
const FILL: f32 = 0.9;
/// Peak opacity of the contact shadow.
const SHADOW: f32 = 0.30;

/// One body: triangles with per-vertex normals and one look.
#[derive(Clone, Debug, Default)]
pub struct MeshPart {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
    /// Linear RGB.
    pub colour: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
}

/// The bodies of one part, framed for the turntable.
#[derive(Clone, Debug)]
pub struct Model {
    pub parts: Vec<MeshPart>,
    center: [f32; 3],
    radius: f32,
    /// The lowest z and the half extents in x and y about `center`.
    ground: f32,
    half: [f32; 2],
}

impl Model {
    /// Frame `parts` (millimetres, z up); `None` when nothing is drawable.
    pub fn new(mut parts: Vec<MeshPart>) -> Result<Self, String> {
        parts.retain(|p| {
            !p.triangles.is_empty()
                && p.positions.len() == p.normals.len()
                && p.triangles
                    .iter()
                    .flatten()
                    .all(|i| (*i as usize) < p.positions.len())
        });
        let triangles: usize = parts.iter().map(|p| p.triangles.len()).sum();
        if triangles == 0 {
            return Err("the body has no triangles to draw".into());
        }
        if triangles > MAX_TRIANGLES {
            return Err(format!(
                "the body has {triangles} triangles; a preview draws at most {MAX_TRIANGLES}"
            ));
        }
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in parts.iter().flat_map(|p| &p.positions) {
            if p.iter().any(|v| !v.is_finite()) {
                return Err("the body has a point that is not finite".into());
            }
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        let center = [0, 1, 2].map(|k| (min[k] + max[k]) / 2.0);
        let radius = parts
            .iter()
            .flat_map(|p| &p.positions)
            .map(|p| {
                let d = [0, 1, 2].map(|k| p[k] - center[k]);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .fold(0.0f32, f32::max)
            .max(1e-3);
        Ok(Self {
            parts,
            center,
            radius,
            ground: min[2] - center[2],
            half: [(max[0] - min[0]) / 2.0, (max[1] - min[1]) / 2.0],
        })
    }

    /// A box of `size` millimetres standing on z = 0, centred in x and y.
    pub fn boxed(
        size: [f64; 3],
        colour: [f32; 3],
        metallic: f32,
        roughness: f32,
    ) -> Result<Self, String> {
        let [l, w, h] = size.map(|v| v.max(0.01) as f32);
        let (x, y) = (l / 2.0, w / 2.0);
        let mut part = MeshPart {
            colour,
            metallic,
            roughness,
            ..MeshPart::default()
        };
        let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
            (
                [0.0, 0.0, 1.0],
                [[-x, -y, h], [x, -y, h], [x, y, h], [-x, y, h]],
            ),
            (
                [0.0, 0.0, -1.0],
                [[-x, y, 0.0], [x, y, 0.0], [x, -y, 0.0], [-x, -y, 0.0]],
            ),
            (
                [1.0, 0.0, 0.0],
                [[x, -y, 0.0], [x, y, 0.0], [x, y, h], [x, -y, h]],
            ),
            (
                [-1.0, 0.0, 0.0],
                [[-x, y, 0.0], [-x, -y, 0.0], [-x, -y, h], [-x, y, h]],
            ),
            (
                [0.0, 1.0, 0.0],
                [[x, y, 0.0], [-x, y, 0.0], [-x, y, h], [x, y, h]],
            ),
            (
                [0.0, -1.0, 0.0],
                [[-x, -y, 0.0], [x, -y, 0.0], [x, -y, h], [-x, -y, h]],
            ),
        ];
        for (normal, corners) in faces {
            let base = part.positions.len() as u32;
            part.positions.extend(corners);
            part.normals.extend([normal; 4]);
            part.triangles
                .extend([[base, base + 1, base + 2], [base, base + 2, base + 3]]);
        }
        Self::new(vec![part])
    }

    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.triangles.len()).sum()
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if l > 1e-12 {
        v.map(|c| c / l)
    } else {
        [0.0, 0.0, 1.0]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn encode(linear: f32) -> u8 {
    let c = linear.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

/// The lit colour of a surface with view-space normal `n` (right, up,
/// depth; the viewer looks along +depth).
fn shade(part: &MeshPart, n: [f32; 3]) -> [f32; 3] {
    let key = normalize([-0.45, 0.75, -0.5]);
    let fill = normalize([0.65, 0.15, -0.45]);
    let half = normalize([key[0], key[1], key[2] - 1.0]);
    let m = part.metallic.clamp(0.0, 1.0);
    let r = part.roughness.clamp(0.04, 1.0);
    let diffuse = 0.26 + 0.64 * dot(n, key).max(0.0) + 0.2 * dot(n, fill).max(0.0);
    let sky = 0.35 + 0.5 * n[1].max(0.0);
    let gloss = (1.0 - r) * (1.0 - r);
    let spec = (0.06 + 1.5 * gloss) * dot(n, half).max(0.0).powf(4.0 + 220.0 * gloss * (1.0 - r));
    [0, 1, 2].map(|k| {
        let base = part.colour[k];
        let f0 = 0.04 + (base - 0.04) * m;
        base * diffuse * (1.0 - 0.75 * m) + base * m * sky + f0 * spec
    })
}

/// The model turned `angle` radians about the vertical axis, `size` pixels
/// square, unmultiplied RGBA with a transparent background.
pub fn render(model: &Model, angle: f32, size: usize) -> ColorImage {
    let size = size.clamp(8, 1024);
    let n = size * 2;
    let (sin_e, cos_e) = ELEVATION_DEG.to_radians().sin_cos();
    let (sin_a, cos_a) = angle.sin_cos();
    let scale = n as f32 / 2.0 * FILL / model.radius;
    let mid = n as f32 / 2.0;
    // Model-frame point to (x px, y px, depth), larger depth farther.
    let view = |p: [f32; 3]| {
        let q = [0, 1, 2].map(|k| p[k] - model.center[k]);
        let x = q[0] * cos_a - q[1] * sin_a;
        let y = q[0] * sin_a + q[1] * cos_a;
        [x, y * sin_e + q[2] * cos_e, y * cos_e - q[2] * sin_e]
    };
    let turn = |v: [f32; 3]| {
        let x = v[0] * cos_a - v[1] * sin_a;
        let y = v[0] * sin_a + v[1] * cos_a;
        [x, y * sin_e + v[2] * cos_e, y * cos_e - v[2] * sin_e]
    };
    let mut depth = vec![f32::INFINITY; n * n];
    let mut colour = vec![[0.0f32; 3]; n * n];
    let mut screen: Vec<[f32; 3]> = Vec::new();
    let mut lit: Vec<[f32; 3]> = Vec::new();
    for part in &model.parts {
        screen.clear();
        lit.clear();
        for (p, nrm) in part.positions.iter().zip(&part.normals) {
            let v = view(*p);
            screen.push([mid + v[0] * scale, mid - v[1] * scale, v[2]]);
            let mut s = normalize(turn(*nrm));
            // Two-sided: a face turned away is lit as its front.
            if s[2] > 0.0 {
                s = s.map(|c| -c);
            }
            lit.push(shade(part, s));
        }
        for tri in &part.triangles {
            let [a, b, c] = tri.map(|i| i as usize);
            let (pa, pb, pc) = (screen[a], screen[b], screen[c]);
            let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
            if area.abs() < 1e-9 {
                continue;
            }
            let x0 = pa[0].min(pb[0]).min(pc[0]).floor().max(0.0) as usize;
            let x1 =
                (pa[0].max(pb[0]).max(pc[0]).ceil() as isize).clamp(0, n as isize - 1) as usize;
            let y0 = pa[1].min(pb[1]).min(pc[1]).floor().max(0.0) as usize;
            let y1 =
                (pa[1].max(pb[1]).max(pc[1]).ceil() as isize).clamp(0, n as isize - 1) as usize;
            if x0 > x1 || y0 > y1 {
                continue;
            }
            let inv = 1.0 / area;
            for y in y0..=y1 {
                let py = y as f32 + 0.5;
                for x in x0..=x1 {
                    let px = x as f32 + 0.5;
                    let w0 = ((pb[0] - px) * (pc[1] - py) - (pb[1] - py) * (pc[0] - px)) * inv;
                    let w1 = ((pc[0] - px) * (pa[1] - py) - (pc[1] - py) * (pa[0] - px)) * inv;
                    let w2 = 1.0 - w0 - w1;
                    if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                        continue;
                    }
                    let z = w0 * pa[2] + w1 * pb[2] + w2 * pc[2];
                    let at = y * n + x;
                    if z < depth[at] {
                        depth[at] = z;
                        colour[at] =
                            [0, 1, 2].map(|k| w0 * lit[a][k] + w1 * lit[b][k] + w2 * lit[c][k]);
                    }
                }
            }
        }
    }
    // The contact shadow: an ellipse round the ground footprint's image.
    let corners = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|[sx, sy]| {
        let p = [
            model.center[0] + sx * model.half[0] * 1.08,
            model.center[1] + sy * model.half[1] * 1.08,
            model.center[2] + model.ground,
        ];
        let v = view(p);
        [mid + v[0] * scale, mid - v[1] * scale]
    });
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for c in corners {
        for k in 0..2 {
            lo[k] = lo[k].min(c[k]);
            hi[k] = hi[k].max(c[k]);
        }
    }
    let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];
    let radii = [
        ((hi[0] - lo[0]) / 2.0).max(2.0),
        ((hi[1] - lo[1]) / 2.0).max(2.0),
    ];
    let mut pixels = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let mut sum = [0.0f32; 3];
            let mut covered = 0u32;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let at = (2 * y + dy) * n + 2 * x + dx;
                if depth[at].is_finite() {
                    covered += 1;
                    for k in 0..3 {
                        sum[k] += colour[at][k];
                    }
                }
            }
            let c = covered as f32 / 4.0;
            let ex = ((2 * x + 1) as f32 - centre[0]) / radii[0];
            let ey = ((2 * y + 1) as f32 - centre[1]) / radii[1];
            let s = SHADOW * (1.0 - smoothstep(0.55, 1.2, (ex * ex + ey * ey).sqrt()));
            let alpha = c + s * (1.0 - c);
            if alpha <= 0.0 {
                pixels.extend([0, 0, 0, 0]);
                continue;
            }
            // Shadow is black: only the model's light adds colour.
            let rgb = [0, 1, 2].map(|k| {
                let model = if covered > 0 {
                    sum[k] / covered as f32
                } else {
                    0.0
                };
                encode(model * c / alpha)
            });
            pixels.extend([
                rgb[0],
                rgb[1],
                rgb[2],
                (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            ]);
        }
    }
    ColorImage::from_rgba_unmultiplied([size, size], &pixels)
}
