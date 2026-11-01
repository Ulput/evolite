//! A binary glTF (`.glb`) read into triangles, as little of the format as a still model needs
//! and nothing that reaches outside the file: the one buffer is the file's own BIN chunk (a
//! `uri` — a path, a URL, even `data:` — is refused), no images or textures, no animations,
//! no sparse accessors, no extension the drawing does not know. Every offset and count is
//! checked against the chunk before it is read, and the whole stays within [`MAX_TRIANGLES`].

use serde::Deserialize;

use crate::frames::RenderError;

/// More than any NFT needs (Jembe's ring has about 400), little enough to draw in time.
pub(crate) const MAX_TRIANGLES: usize = 100_000;
const MAX_NODES: usize = 1_000;
const MAX_DEPTH: usize = 32;

/// Extensions a file may require: the drawing understands them, or loses nothing without them.
const KNOWN: [&str; 7] = [
    "KHR_mesh_quantization",
    "KHR_materials_transmission",
    "KHR_materials_volume",
    "KHR_materials_ior",
    "KHR_materials_dispersion",
    "KHR_materials_emissive_strength",
    "KHR_materials_unlit",
];

fn invalid(what: &'static str) -> RenderError {
    RenderError::Invalid(what)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Material {
    /// Linear RGBA.
    pub base: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    /// 0: opaque; towards 1 the light goes through (glass, a gem).
    pub transmission: f32,
    pub double_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base: [1.0; 4],
            metallic: 1.0,
            roughness: 1.0,
            emissive: [0.0; 3],
            transmission: 0.0,
            double_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Triangle {
    /// Positions in the model's space, every node transform applied.
    pub p: [[f32; 3]; 3],
    /// Linear RGB of each corner: `COLOR_0`, or white.
    pub color: [[f32; 3]; 3],
    pub material: usize,
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Model {
    pub triangles: Vec<Triangle>,
    pub materials: Vec<Material>,
}

// The JSON, as far as it is read. Unknown fields are ignored; what is read is checked.

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct Gltf {
    #[serde(default)]
    extensions_required: Vec<String>,
    scene: Option<usize>,
    #[serde(default)]
    scenes: Vec<SceneJson>,
    #[serde(default)]
    nodes: Vec<NodeJson>,
    #[serde(default)]
    meshes: Vec<MeshJson>,
    #[serde(default)]
    materials: Vec<MaterialJson>,
    #[serde(default)]
    accessors: Vec<AccessorJson>,
    #[serde(default)]
    buffer_views: Vec<ViewJson>,
    #[serde(default)]
    buffers: Vec<BufferJson>,
}

#[derive(Deserialize)]
struct SceneJson {
    #[serde(default)]
    nodes: Vec<usize>,
}

#[derive(Deserialize)]
struct NodeJson {
    mesh: Option<usize>,
    #[serde(default)]
    children: Vec<usize>,
    matrix: Option<[f32; 16]>,
    translation: Option<[f32; 3]>,
    rotation: Option<[f32; 4]>,
    scale: Option<[f32; 3]>,
}

#[derive(Deserialize)]
struct MeshJson {
    #[serde(default)]
    primitives: Vec<PrimitiveJson>,
}

#[derive(Deserialize)]
struct PrimitiveJson {
    attributes: std::collections::BTreeMap<String, usize>,
    indices: Option<usize>,
    material: Option<usize>,
    mode: Option<u32>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct MaterialJson {
    #[serde(default)]
    pbr_metallic_roughness: PbrJson,
    emissive_factor: Option<[f32; 3]>,
    #[serde(default)]
    double_sided: bool,
    #[serde(default)]
    extensions: ExtensionsJson,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PbrJson {
    base_color_factor: Option<[f32; 4]>,
    metallic_factor: Option<f32>,
    roughness_factor: Option<f32>,
}

#[derive(Deserialize, Default)]
struct ExtensionsJson {
    #[serde(rename = "KHR_materials_transmission")]
    transmission: Option<TransmissionJson>,
    #[serde(rename = "KHR_materials_emissive_strength")]
    emissive_strength: Option<EmissiveStrengthJson>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransmissionJson {
    transmission_factor: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmissiveStrengthJson {
    emissive_strength: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AccessorJson {
    buffer_view: Option<usize>,
    #[serde(default)]
    byte_offset: usize,
    component_type: u32,
    #[serde(default)]
    normalized: bool,
    count: usize,
    #[serde(rename = "type")]
    kind: String,
    sparse: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewJson {
    buffer: usize,
    #[serde(default)]
    byte_offset: usize,
    byte_length: usize,
    byte_stride: Option<usize>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BufferJson {
    uri: Option<String>,
    byte_length: usize,
}

/// The JSON and BIN chunks of a GLB container, checked against its own lengths.
fn chunks(bytes: &[u8]) -> Result<(&[u8], &[u8]), RenderError> {
    let u32_at = |at: usize| -> Result<usize, RenderError> {
        let b = bytes.get(at..at + 4).ok_or(invalid("not a GLB"))?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    if !bytes.starts_with(b"glTF") || u32_at(4)? != 2 || u32_at(8)? != bytes.len() {
        return Err(invalid("not a GLB"));
    }
    let mut at = 12;
    let mut json = None;
    let mut bin: &[u8] = &[];
    while at < bytes.len() {
        let len = u32_at(at)?;
        let kind = u32_at(at + 4)?;
        let body = bytes
            .get(at + 8..(at + 8).checked_add(len).ok_or(invalid("not a GLB"))?)
            .ok_or(invalid("not a GLB"))?;
        match kind {
            0x4E4F_534A if json.is_none() => json = Some(body),
            0x004E_4942 if bin.is_empty() => bin = body,
            // Unknown chunks are skipped, as the format says.
            _ => {}
        }
        at += 8 + len.next_multiple_of(4);
    }
    Ok((json.ok_or(invalid("not a GLB"))?, bin))
}

/// Where an accessor's elements are: offset, stride and size of one, all inside `bin`.
struct View<'a> {
    bin: &'a [u8],
    start: usize,
    stride: usize,
    component: u32,
    width: usize,
    normalized: bool,
    count: usize,
}

fn component_size(component: u32) -> Result<usize, RenderError> {
    Ok(match component {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(invalid("not a GLB")),
    })
}

impl<'a> View<'a> {
    fn of(gltf: &Gltf, bin: &'a [u8], index: usize, width: usize) -> Result<Self, RenderError> {
        let a = gltf.accessors.get(index).ok_or(invalid("not a GLB"))?;
        let kind_width = match a.kind.as_str() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => return Err(RenderError::Unsupported),
        };
        if a.sparse.is_some() || kind_width < width {
            return Err(RenderError::Unsupported);
        }
        let view = gltf
            .buffer_views
            .get(a.buffer_view.ok_or(RenderError::Unsupported)?)
            .ok_or(invalid("not a GLB"))?;
        if view.buffer != 0 {
            return Err(RenderError::Unsupported);
        }
        let size = component_size(a.component_type)?;
        let element = size * kind_width;
        let stride = view.byte_stride.unwrap_or(element);
        if stride < element || stride > 252 {
            return Err(invalid("not a GLB"));
        }
        let view_end = view
            .byte_offset
            .checked_add(view.byte_length)
            .filter(|end| *end <= bin.len())
            .ok_or(invalid("not a GLB"))?;
        let start = view.byte_offset + a.byte_offset;
        if a.count > 0 {
            let last = start
                .checked_add(
                    stride
                        .checked_mul(a.count - 1)
                        .ok_or(invalid("not a GLB"))?,
                )
                .and_then(|l| l.checked_add(element))
                .ok_or(invalid("not a GLB"))?;
            if last > view_end {
                return Err(invalid("not a GLB"));
            }
        }
        Ok(Self {
            bin,
            start,
            stride,
            component: a.component_type,
            width,
            normalized: a.normalized,
            count: a.count,
        })
    }

    /// Element `i`, the first `width` components, as floats (normalized when the file says).
    fn get(&self, i: usize) -> [f32; 4] {
        let size = component_size(self.component).unwrap_or(4);
        let mut out = [0.0; 4];
        for (c, value) in out.iter_mut().enumerate().take(self.width) {
            let at = self.start + i * self.stride + c * size;
            let b = &self.bin[at..at + size];
            *value = match (self.component, self.normalized) {
                (5120, true) => (b[0] as i8 as f32 / 127.0).max(-1.0),
                (5120, false) => b[0] as i8 as f32,
                (5121, true) => b[0] as f32 / 255.0,
                (5121, false) => b[0] as f32,
                (5122, n) => {
                    let v = i16::from_le_bytes([b[0], b[1]]) as f32;
                    if n { (v / 32767.0).max(-1.0) } else { v }
                }
                (5123, n) => {
                    let v = u16::from_le_bytes([b[0], b[1]]) as f32;
                    if n { v / 65535.0 } else { v }
                }
                (5125, _) => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            };
        }
        out
    }

    fn index(&self, i: usize) -> usize {
        let at = self.start + i * self.stride;
        let b = &self.bin[at..];
        match self.component {
            5121 => b[0] as usize,
            5123 => u16::from_le_bytes([b[0], b[1]]) as usize,
            _ => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize,
        }
    }
}

type Mat4 = [[f32; 4]; 4];

const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// Column-major, as glTF writes them: `m[column][row]`.
fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (c, column) in out.iter_mut().enumerate() {
        for (r, value) in column.iter_mut().enumerate() {
            *value = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

fn local(node: &NodeJson) -> Mat4 {
    if let Some(m) = node.matrix {
        let mut out = [[0.0; 4]; 4];
        for (i, v) in m.iter().enumerate() {
            out[i / 4][i % 4] = *v;
        }
        return out;
    }
    let [tx, ty, tz] = node.translation.unwrap_or([0.0; 3]);
    let [x, y, z, w] = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let [sx, sy, sz] = node.scale.unwrap_or([1.0; 3]);
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y + z * w),
            2.0 * (x * z - y * w),
        ],
        [
            2.0 * (x * y - z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z + x * w),
        ],
        [
            2.0 * (x * z + y * w),
            2.0 * (y * z - x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    [
        [r[0][0] * sx, r[0][1] * sx, r[0][2] * sx, 0.0],
        [r[1][0] * sy, r[1][1] * sy, r[1][2] * sy, 0.0],
        [r[2][0] * sz, r[2][1] * sz, r[2][2] * sz, 0.0],
        [tx, ty, tz, 1.0],
    ]
}

fn apply(m: &Mat4, p: [f32; 4]) -> [f32; 3] {
    let mut out = [0.0; 3];
    for (r, value) in out.iter_mut().enumerate() {
        *value = m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r];
    }
    out
}

fn material(json: &MaterialJson) -> Material {
    let pbr = &json.pbr_metallic_roughness;
    let clamp = |v: f32| {
        if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let strength = json
        .extensions
        .emissive_strength
        .as_ref()
        .and_then(|e| e.emissive_strength)
        .filter(|s| s.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 10.0);
    Material {
        base: pbr.base_color_factor.unwrap_or([1.0; 4]).map(clamp),
        metallic: clamp(pbr.metallic_factor.unwrap_or(1.0)),
        roughness: clamp(pbr.roughness_factor.unwrap_or(1.0)),
        emissive: json
            .emissive_factor
            .unwrap_or([0.0; 3])
            .map(|e| clamp(e) * strength),
        transmission: clamp(
            json.extensions
                .transmission
                .as_ref()
                .and_then(|t| t.transmission_factor)
                .unwrap_or(0.0),
        ),
        double_sided: json.double_sided,
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Model, RenderError> {
    let (json, bin) = chunks(bytes)?;
    let gltf: Gltf = serde_json::from_slice(json).map_err(|_| invalid("not a GLB"))?;
    if gltf
        .extensions_required
        .iter()
        .any(|e| !KNOWN.contains(&e.as_str()))
    {
        return Err(RenderError::Unsupported);
    }
    // One buffer, the file's own: nothing is fetched or read from anywhere else.
    match gltf.buffers.as_slice() {
        [] => {}
        [only] if only.uri.is_none() && only.byte_length <= bin.len() => {}
        _ => return Err(RenderError::Unsupported),
    }
    if gltf.nodes.len() > MAX_NODES {
        return Err(RenderError::Limit("model size"));
    }
    let mut model = Model {
        triangles: Vec::new(),
        materials: gltf.materials.iter().map(material).collect(),
    };
    // The default material, after the file's own.
    model.materials.push(Material::default());
    let default_material = model.materials.len() - 1;

    let roots: Vec<usize> = match gltf.scene.or((!gltf.scenes.is_empty()).then_some(0)) {
        Some(s) => gltf
            .scenes
            .get(s)
            .ok_or(invalid("not a GLB"))?
            .nodes
            .clone(),
        None => Vec::new(),
    };
    // Depth first, each node at most once: a cycle in `children` cannot loop.
    let mut visited = vec![false; gltf.nodes.len()];
    let mut stack: Vec<(usize, Mat4, usize)> = roots.iter().map(|n| (*n, IDENTITY, 0)).collect();
    while let Some((n, parent, depth)) = stack.pop() {
        let node = gltf.nodes.get(n).ok_or(invalid("not a GLB"))?;
        if std::mem::replace(&mut visited[n], true) || depth > MAX_DEPTH {
            return Err(invalid("not a GLB"));
        }
        let world = mul(&parent, &local(node));
        for child in &node.children {
            stack.push((*child, world, depth + 1));
        }
        let Some(mesh) = node.mesh else { continue };
        let mesh = gltf.meshes.get(mesh).ok_or(invalid("not a GLB"))?;
        for primitive in &mesh.primitives {
            // Triangles only; points and lines are not drawn.
            if primitive.mode.unwrap_or(4) != 4 {
                continue;
            }
            let position = primitive
                .attributes
                .get("POSITION")
                .ok_or(invalid("not a GLB"))?;
            let positions = View::of(&gltf, bin, *position, 3)?;
            let colors = match primitive.attributes.get("COLOR_0") {
                Some(c) => Some(View::of(&gltf, bin, *c, 3)?),
                None => None,
            };
            if colors.as_ref().is_some_and(|c| c.count < positions.count) {
                return Err(invalid("not a GLB"));
            }
            let indices = match primitive.indices {
                Some(i) => Some(View::of(&gltf, bin, i, 1)?),
                None => None,
            };
            if indices.as_ref().is_some_and(|i| i.component == 5126) {
                return Err(invalid("not a GLB"));
            }
            let corners = indices.as_ref().map_or(positions.count, |i| i.count);
            let material = primitive
                .material
                .filter(|m| *m < gltf.materials.len())
                .unwrap_or(default_material);
            if model.triangles.len() + corners / 3 > MAX_TRIANGLES {
                return Err(RenderError::Limit("model size"));
            }
            for t in 0..corners / 3 {
                let mut tri = Triangle {
                    p: [[0.0; 3]; 3],
                    color: [[1.0; 3]; 3],
                    material,
                };
                for k in 0..3 {
                    let v = match &indices {
                        Some(i) => i.index(t * 3 + k),
                        None => t * 3 + k,
                    };
                    if v >= positions.count {
                        return Err(invalid("not a GLB"));
                    }
                    let [x, y, z, _] = positions.get(v);
                    tri.p[k] = apply(&world, [x, y, z, 1.0]);
                    if let Some(c) = &colors {
                        let [r, g, b, _] = c.get(v);
                        tri.color[k] = [r, g, b];
                    }
                }
                if tri.p.iter().flatten().all(|v| v.is_finite()) {
                    model.triangles.push(tri);
                }
            }
        }
    }
    if model.triangles.is_empty() {
        return Err(invalid("no frames"));
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GLB from a JSON and a BIN chunk, padded as the format wants.
    pub(crate) fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
        let mut out = b"glTF".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend((total as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(json);
        if !bin.is_empty() {
            out.extend((bin.len() as u32).to_le_bytes());
            out.extend(0x004E_4942u32.to_le_bytes());
            out.extend(bin);
        }
        out
    }

    /// One triangle, float positions.
    pub(crate) fn triangle(extra: &str) -> Vec<u8> {
        let mut bin = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            bin.extend(v.to_le_bytes());
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],"meshes":[{{"primitives":[{{"attributes":{{"POSITION":0}}}}]}}],"accessors":[{{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}}],"bufferViews":[{{"buffer":0,"byteLength":36}}],"buffers":[{{"byteLength":36}}]{extra}}}"#
        );
        glb(&json, &bin)
    }

    fn ring() -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../evolite-core/assets/nft/diamond-ring-3d.glb"),
        )
        .unwrap()
    }

    #[test]
    fn the_jembe_ring_is_read_whole() {
        let model = parse(&ring()).unwrap();
        // 936 / 3 + 84 / 3 + 216 / 3 + 72 / 3 triangles, four primitives.
        assert_eq!(model.triangles.len(), 312 + 28 + 72 + 24);
        // Quantized positions, scaled by the node: ±59 / 127 × 2.4.
        let max_x = model
            .triangles
            .iter()
            .flat_map(|t| t.p)
            .map(|p| p[0])
            .fold(f32::MIN, f32::max);
        assert!((max_x - 81.0 / 127.0 * 2.4).abs() < 0.01, "{max_x}");
        // The stone lets light through; its faces have colours of their own.
        let mut stone = model.triangles.iter().filter(|t| t.material == 1);
        assert_eq!(model.materials[1].transmission, 0.75);
        assert!(stone.any(|t| t.color.iter().any(|c| *c != [1.0; 3])));
    }

    #[test]
    fn nothing_outside_the_file_is_followed() {
        let with_uri = triangle("").into_iter().collect::<Vec<u8>>();
        assert!(parse(&with_uri).is_ok());
        let json = r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"file:///C:/Users/x/secret.bin","byteLength":36}]}"#;
        assert_eq!(parse(&glb(json, &[0; 36])), Err(RenderError::Unsupported));
        let json = r#"{"asset":{"version":"2.0"},"buffers":[{"uri":"data:application/octet-stream;base64,AAAA","byteLength":3}]}"#;
        assert_eq!(parse(&glb(json, &[])), Err(RenderError::Unsupported));
    }

    #[test]
    fn an_unknown_required_extension_is_refused() {
        let required = r#","extensionsRequired":["EXT_meshopt_compression"]"#;
        assert_eq!(parse(&triangle(required)), Err(RenderError::Unsupported));
    }

    #[test]
    fn offsets_past_the_buffer_and_cycles_are_refused() {
        // The accessor claims more elements than the view holds.
        let mut bytes = triangle("");
        let text = String::from_utf8_lossy(&bytes).replace(r#""count":3"#, r#""count":9"#);
        bytes = text.into_bytes();
        assert!(parse(&bytes).is_err());
        // A node that is its own child.
        let json =
            r#"{"asset":{"version":"2.0"},"scenes":[{"nodes":[0]}],"nodes":[{"children":[0]}]}"#;
        assert!(parse(&glb(json, &[])).is_err());
        // Lengths that do not add up.
        let mut cut = triangle("");
        cut.truncate(cut.len() - 4);
        assert!(parse(&cut).is_err());
    }
}
