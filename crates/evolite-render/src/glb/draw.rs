//! A model drawn by the processor, no GPU: the sandbox has no graphics (Win32k is off), and a
//! few hundred triangles do not need one. A z-buffer, flat faces (the files carry no normals:
//! Jembe's models are low-poly by design), corner colours, a small studio of lights and a sky
//! to reflect, so metal does not come out black. Drawn twice as large and scaled down, for
//! smooth edges; tone-mapped like three.js (ACES), which Jembe's own viewer uses.

use super::parse::{Material, Model, Triangle};

/// The camera's vertical field of view, as in Jembe's viewer.
const FOV_DEG: f32 = 35.0;
/// The camera a little above the model, looking slightly down.
const ELEVATION_DEG: f32 = 6.0;
const SUPERSAMPLE: usize = 2;

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}
fn normalize(a: V3) -> V3 {
    let len = dot(a, a).sqrt();
    if len > 0.0 {
        scale(a, 1.0 / len)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Where the model is: the centre and radius of a sphere around it.
pub(crate) struct Frame {
    center: V3,
    radius: f32,
}

impl Frame {
    pub(crate) fn of(model: &Model) -> Self {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in model.triangles.iter().flat_map(|t| t.p) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let center = scale(add(lo, hi), 0.5);
        let radius = model
            .triangles
            .iter()
            .flat_map(|t| t.p)
            .map(|p| dot(sub(p, center), sub(p, center)).sqrt())
            .fold(0.0f32, f32::max)
            .max(1e-6);
        Self { center, radius }
    }
}

/// The studio: a key light from the upper left, a fill, a rim from behind; the sky above and
/// a darker floor, seen in reflections and as soft ambient light.
const LIGHTS: [(V3, f32); 3] = [
    ([-0.45, 0.75, 0.5], 1.6),
    ([0.7, 0.25, 0.45], 0.55),
    ([0.0, 0.35, -1.0], 0.5),
];

fn sky(dir: V3) -> V3 {
    let up = (dir[1] * 0.5 + 0.5).clamp(0.0, 1.0);
    let floor = [0.22, 0.2, 0.26];
    let top = [1.0, 0.98, 1.02];
    // A bright band, as a softbox, makes the edges of metal catch light as they turn.
    let band = (-((dir[1] - 0.35) * 6.0).powi(2)).exp() * 0.9;
    add(add(scale(floor, 1.0 - up), scale(top, up)), [band; 3])
}

fn shade(material: &Material, albedo: V3, n: V3, view: V3) -> (V3, f32) {
    let base = mul(
        albedo,
        [material.base[0], material.base[1], material.base[2]],
    );
    let metallic = material.metallic;
    let rough = material.roughness.max(0.04);
    let f0 = add(scale([0.04; 3], 1.0 - metallic), scale(base, metallic));
    let shininess = (2.0 / (rough.powi(4) + 0.002) - 2.0).clamp(1.0, 2048.0);
    let mut diffuse = scale(sky(n), 0.35);
    let mut specular = [0.0; 3];
    for (dir, intensity) in LIGHTS {
        let l = normalize(dir);
        let nl = dot(n, l).max(0.0);
        diffuse = add(diffuse, [nl * intensity; 3]);
        let h = normalize(add(l, view));
        let spec = dot(n, h).max(0.0).powf(shininess) * (shininess + 8.0) / 25.0;
        specular = add(specular, scale(f0, spec * intensity * nl.max(0.15)));
    }
    // The sky in the surface: what makes metal look like metal.
    let r = sub(scale(n, 2.0 * dot(n, view)), view);
    let fresnel = (1.0 - dot(n, view).max(0.0)).powi(5);
    let reflect = add(f0, scale(sub([1.0; 3], f0), fresnel));
    let mirror = scale(mul(sky(r), reflect), 1.0 - rough * 0.75);
    let lit = add(
        add(mul(scale(base, 1.0 - metallic), diffuse), specular),
        mirror,
    );
    let emissive = material.emissive;
    let t = material.transmission;
    if t > 0.0 {
        // Light through the stone: its own colour, brighter where the sky shows through, and
        // sparkle on top. The faces stay partly see-through.
        let through = mul(base, add(scale(sky(scale(n, -1.0)), 0.55), [0.45; 3]));
        let colour = add(
            add(scale(lit, 1.0 - t), scale(through, t)),
            scale(specular, 1.5 * t),
        );
        return (add(colour, emissive), 1.0 - 0.35 * t);
    }
    (add(lit, emissive), material.base[3])
}

/// ACES filmic, as three.js, then sRGB.
fn tone(c: f32) -> f32 {
    let x = (c * 0.8).max(0.0);
    let mapped = (x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14);
    let v = mapped.clamp(0.0, 1.0);
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

struct Projected {
    s: [[f32; 2]; 3],
    z: [f32; 3],
    n: V3,
    view: V3,
}

/// A thread's drawing surface, kept from frame to frame: the samples and their depths, and
/// the rectangle the last frame touched, which is all that has to be cleared for the next.
pub(crate) struct Canvas {
    side: usize,
    color: Vec<[u8; 4]>,
    depth: Vec<f32>,
    dirty: Option<(usize, usize, usize, usize)>,
}

impl Canvas {
    pub(crate) fn new(side: usize) -> Self {
        let n = side * SUPERSAMPLE;
        Self {
            side,
            color: vec![[0; 4]; n * n],
            depth: vec![f32::MAX; n * n],
            dirty: None,
        }
    }

    fn clear(&mut self) {
        let n = self.side * SUPERSAMPLE;
        if let Some((x0, y0, x1, y1)) = self.dirty.take() {
            for y in y0..y1 {
                self.color[y * n + x0..y * n + x1].fill([0; 4]);
                self.depth[y * n + x0..y * n + x1].fill(f32::MAX);
            }
        }
    }

    fn touch(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        self.dirty = Some(match self.dirty {
            None => (x0, y0, x1, y1),
            Some((a, b, c, d)) => (a.min(x0), b.min(y0), c.max(x1), d.max(y1)),
        });
    }
}

/// A face's light, worked out once at each corner: the face is flat, so its normal and the
/// way it is seen are the same everywhere on it; only the corner colours differ, and the
/// light is spread between them. Tone-mapped and premultiplied, as the samples are kept.
fn corner_light(material: &Material, t: &Triangle, normal: V3, view: V3) -> [[f32; 4]; 3] {
    let mut out = [[0.0; 4]; 3];
    for k in 0..3 {
        if k > 0 && t.color[k] == t.color[0] {
            out[k] = out[0];
            continue;
        }
        let (rgb, alpha) = shade(material, t.color[k], normal, view);
        let a = alpha.clamp(0.0, 1.0);
        out[k] = [tone(rgb[0]) * a, tone(rgb[1]) * a, tone(rgb[2]) * a, a];
    }
    out
}

/// Fills one face into `canvas`: the edge functions and the depth are stepped pixel by
/// pixel (one addition each), not worked out again for every sample.
fn fill(canvas: &mut Canvas, material: &Material, t: &Triangle, p: &Projected, blend: bool) {
    let n_px = canvas.side * SUPERSAMPLE;
    let mut normal = p.n;
    if dot(normal, p.view) < 0.0 {
        if !material.double_sided && !blend {
            return;
        }
        normal = scale(normal, -1.0);
    }
    let [a, b, c] = p.s;
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if area.abs() < 1e-9 {
        return;
    }
    let x0 = a[0].min(b[0]).min(c[0]).floor().max(0.0) as usize;
    let x1 = (a[0].max(b[0]).max(c[0]).ceil().max(0.0) as usize).min(n_px);
    let y0 = a[1].min(b[1]).min(c[1]).floor().max(0.0) as usize;
    let y1 = (a[1].max(b[1]).max(c[1]).ceil().max(0.0) as usize).min(n_px);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let light = corner_light(material, t, normal, p.view);
    let flat = light[1] == light[0] && light[2] == light[0];
    let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let flat_color = light[0].map(byte);

    // w0 = (A0·x + B0·y + C0) / area, and the same for w1; w2 = 1 − w0 − w1.
    let inv = 1.0 / area;
    let (a0, b0, c0) = (
        (b[1] - c[1]) * inv,
        (c[0] - b[0]) * inv,
        (b[0] * c[1] - b[1] * c[0]) * inv,
    );
    let (a1, b1, c1) = (
        (c[1] - a[1]) * inv,
        (a[0] - c[0]) * inv,
        (c[0] * a[1] - c[1] * a[0]) * inv,
    );
    let [z0, z1, z2] = p.z;
    let mut touched = false;
    for y in y0..y1 {
        let py = y as f32 + 0.5;
        let px = x0 as f32 + 0.5;
        let mut w0 = a0 * px + b0 * py + c0;
        let mut w1 = a1 * px + b1 * py + c1;
        let row = y * n_px;
        for x in x0..x1 {
            let w2 = 1.0 - w0 - w1;
            if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                let z = w0 * z0 + w1 * z1 + w2 * z2;
                let i = row + x;
                if z < canvas.depth[i] {
                    touched = true;
                    let src = if flat {
                        flat_color
                    } else {
                        [0, 1, 2, 3]
                            .map(|k| byte(w0 * light[0][k] + w1 * light[1][k] + w2 * light[2][k]))
                    };
                    if blend {
                        // Premultiplied "over": the face, then what was behind it.
                        let keep = 255 - u16::from(src[3]);
                        let under = canvas.color[i];
                        canvas.color[i] = [0, 1, 2, 3].map(|k| {
                            (u16::from(src[k]) + (u16::from(under[k]) * keep + 127) / 255).min(255)
                                as u8
                        });
                    } else {
                        canvas.depth[i] = z;
                        canvas.color[i] = src;
                    }
                }
            }
            w0 += a0;
            w1 += a1;
        }
    }
    if touched {
        canvas.touch(x0, y0, x1, y1);
    }
}

/// The model turned by `angle` (radians, about the vertical axis), drawn into `canvas` and
/// scaled down to its side: premultiplied RGBA, transparent around it.
pub(crate) fn draw(model: &Model, frame: &Frame, angle: f32, canvas: &mut Canvas) -> Vec<u8> {
    canvas.clear();
    let side = canvas.side;
    let n_px = side * SUPERSAMPLE;
    let fov = FOV_DEG.to_radians();
    let focal = 1.0 / (fov * 0.5).tan();
    // Far enough that the whole sphere is in the picture, with a little room.
    let distance = frame.radius / (fov * 0.5).sin() * 1.04;
    let (sa, ca) = angle.sin_cos();
    let (se, ce) = ELEVATION_DEG.to_radians().sin_cos();
    let to_view = |p: V3| -> V3 {
        let p = sub(p, frame.center);
        // Turn about Y, then tilt as a camera above would see it.
        let x = p[0] * ca + p[2] * sa;
        let z = -p[0] * sa + p[2] * ca;
        let y = p[1];
        [x, y * ce - z * se, y * se + z * ce]
    };
    let project = |t: &Triangle| -> Option<Projected> {
        let v = t.p.map(to_view);
        let mut s = [[0.0; 2]; 3];
        let mut z = [0.0; 3];
        for k in 0..3 {
            let depth = distance - v[k][2];
            if depth <= frame.radius * 0.01 {
                return None;
            }
            let px = v[k][0] * focal / depth;
            let py = v[k][1] * focal / depth;
            s[k] = [
                (px * 0.5 + 0.5) * n_px as f32,
                (0.5 - py * 0.5) * n_px as f32,
            ];
            z[k] = depth;
        }
        let n = normalize(cross(sub(v[1], v[0]), sub(v[2], v[0])));
        let centre = scale(add(add(v[0], v[1]), v[2]), 1.0 / 3.0);
        let view = normalize(sub([0.0, 0.0, distance], centre));
        Some(Projected { s, z, n, view })
    };

    let material = |t: &Triangle| &model.materials[t.material.min(model.materials.len() - 1)];
    let mut see_through: Vec<(f32, &Triangle, Projected)> = Vec::new();
    for t in &model.triangles {
        let Some(p) = project(t) else { continue };
        let m = material(t);
        if m.transmission > 0.0 || m.base[3] < 1.0 {
            let far = p.z.iter().copied().fold(f32::MIN, f32::max);
            see_through.push((far, t, p));
        } else {
            fill(canvas, m, t, &p, false);
        }
    }
    // See-through faces after the solid ones, the farthest first.
    see_through.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, t, p) in &see_through {
        fill(canvas, material(t), t, p, true);
    }

    // Down to `side`: each pixel the mean of its samples, already premultiplied. Only the
    // part that was drawn on; the rest stays transparent.
    let mut out = vec![0u8; side * side * 4];
    let Some((x0, y0, x1, y1)) = canvas.dirty else {
        return out;
    };
    let samples = (SUPERSAMPLE * SUPERSAMPLE) as u32;
    for y in y0 / SUPERSAMPLE..y1.div_ceil(SUPERSAMPLE).min(side) {
        for x in x0 / SUPERSAMPLE..x1.div_ceil(SUPERSAMPLE).min(side) {
            let mut sum = [0u32; 4];
            for sy in 0..SUPERSAMPLE {
                let row = (y * SUPERSAMPLE + sy) * n_px + x * SUPERSAMPLE;
                for c in &canvas.color[row..row + SUPERSAMPLE] {
                    for k in 0..4 {
                        sum[k] += u32::from(c[k]);
                    }
                }
            }
            let o = (y * side + x) * 4;
            for k in 0..4 {
                out[o + k] = ((sum[k] + samples / 2) / samples) as u8;
            }
        }
    }
    out
}
