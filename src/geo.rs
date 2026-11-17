//! Bedrock geometry (`.geo.json`, as Blockbench exports it) read into a bone tree of quads.
//!
//! No mod API types here, so the tests run with plain `cargo test` (see `tools/check`).
//! The space is the one GeckoLib draws Bedrock models in, so a model looks as it does in
//! Blockbench: Bedrock's x turned around, y up, the model's front towards -z, in texels
//! (16 to a block). Each bone keeps its pivot; its quads are relative to it, its own
//! rotation (radians, applied z, then y, then x) is left to the pose, as a `ModelPart` has it.
//!
//! Read: format 1.12.0 and later (`minecraft:geometry`) and the old 1.8/1.10 form
//! (`geometry.<name>` keys); cubes with box UV or per-face UV (`uv_size`, `uv_rotation`),
//! cube pivot and rotation, `inflate`, `mirror` (cube or bone); `poly_mesh` (triangles and
//! quads, plain or normalized UVs), locators. Texture meshes are skipped.

use crate::serde_json::{Map, Value};

/// A corner of a quad: position in its bone's frame (texels) and texture coordinates (0 to 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub u: f32,
    pub v: f32,
}

/// A face: four corners in drawing order (a triangle repeats its last corner) and its normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub vertices: [Vertex; 4],
    pub normal: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    /// Index of the parent bone in [`Model::bones`]; none for a root bone.
    pub parent: Option<usize>,
    /// The pivot, in model space (texels).
    pub pivot: [f32; 3],
    /// The bone's rest rotation in radians, applied z, then y, then x.
    pub rotation: [f32; 3],
    /// Faces relative to the pivot.
    pub quads: Vec<Quad>,
    /// Child bones, by index, in file order.
    pub children: Vec<usize>,
    /// Named points on the bone (particles come out of them), in model space at rest.
    pub locators: Vec<(String, [f32; 3])>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub identifier: String,
    pub texture_width: u32,
    pub texture_height: u32,
    /// Bones in file order; parents may come after their children in the file.
    pub bones: Vec<Bone>,
    /// Root bones, in file order.
    pub roots: Vec<usize>,
}

impl Model {
    pub fn bone(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name.eq_ignore_ascii_case(name))
    }
    /// The bone holding locator `name`, and the locator's place at rest.
    pub fn locator(&self, name: &str) -> Option<(usize, [f32; 3])> {
        self.bones.iter().enumerate().find_map(|(i, b)| {
            b.locators.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, p)| (i, *p))
        })
    }

    /// Where `point` (model space at rest, on bone `bone`) is with the bones posed: `pose(i)`
    /// gives bone i's added rotation (radians), added position (texels) and scale.
    pub fn posed_point(&self, bone: usize, point: [f32; 3], pose: &dyn Fn(usize) -> ([f32; 3], [f32; 3], [f32; 3])) -> [f32; 3] {
        let mut p = sub(point, self.bones[bone].pivot);
        let mut at = Some(bone);
        while let Some(i) = at {
            let b = &self.bones[i];
            let parent_pivot = b.parent.map_or([0.0; 3], |q| self.bones[q].pivot);
            let (rot, pos, scl) = pose(i);
            let scaled = [p[0] * scl[0], p[1] * scl[1], p[2] * scl[2]];
            let turned = rotate(scaled, add(b.rotation, rot));
            p = add(add(turned, sub(b.pivot, parent_pivot)), pos);
            at = b.parent;
        }
        p
    }

    pub fn quad_count(&self) -> usize {
        self.bones.iter().map(|b| b.quads.len()).sum()
    }
}

/// Reads every geometry in a `.geo.json` file.
pub fn parse(bytes: &[u8]) -> Result<Vec<Model>, String> {
    let root: Value = crate::serde_json::from_slice(bytes).map_err(|e| format!("not JSON: {e}"))?;
    let root = root.as_object().ok_or("the file is not a JSON object")?;
    let mut models = Vec::new();
    if let Some(list) = root.get("minecraft:geometry") {
        let list = list.as_array().ok_or("minecraft:geometry is not a list")?;
        for g in list {
            let g = g.as_object().ok_or("a geometry is not an object")?;
            let d = g.get("description").and_then(Value::as_object);
            let id = d.and_then(|d| d.get("identifier")).and_then(Value::as_str).unwrap_or("geometry.unknown");
            let w = d.and_then(|d| d.get("texture_width")).and_then(Value::as_f64).unwrap_or(64.0);
            let h = d.and_then(|d| d.get("texture_height")).and_then(Value::as_f64).unwrap_or(64.0);
            models.push(read_geometry(id, w as u32, h as u32, g).map_err(|e| format!("{id}: {e}"))?);
        }
    } else {
        // 1.8.0 / 1.10.0: { "geometry.name": { "texturewidth": .., "bones": [..] } }
        for (key, g) in root {
            if !key.starts_with("geometry.") {
                continue;
            }
            let g = g.as_object().ok_or_else(|| format!("{key} is not an object"))?;
            let w = g.get("texturewidth").and_then(Value::as_f64).unwrap_or(64.0);
            let h = g.get("textureheight").and_then(Value::as_f64).unwrap_or(64.0);
            let id = key.split(':').next().unwrap_or(key);
            models.push(read_geometry(id, w as u32, h as u32, g).map_err(|e| format!("{key}: {e}"))?);
        }
    }
    if models.is_empty() {
        return Err("no geometry in the file".into());
    }
    Ok(models)
}

fn read_geometry(id: &str, tw: u32, th: u32, g: &Map<String, Value>) -> Result<Model, String> {
    if tw == 0 || th == 0 {
        return Err("texture size is 0".into());
    }
    let mut model = Model {
        identifier: id.to_string(),
        texture_width: tw,
        texture_height: th,
        bones: Vec::new(),
        roots: Vec::new(),
    };
    let empty = Vec::new();
    let raw = g.get("bones").and_then(Value::as_array).unwrap_or(&empty);
    let mut parents = Vec::new();
    for b in raw {
        let b = b.as_object().ok_or("a bone is not an object")?;
        let name = b.get("name").and_then(Value::as_str).ok_or("a bone has no name")?.to_string();
        if model.bone(&name).is_some() {
            return Err(format!("two bones are named {name}"));
        }
        let p = vec3(b.get("pivot"))?;
        let r = vec3(b.get("rotation"))?;
        let pivot = [-p[0], p[1], p[2]];
        let rotation = [(-r[0]).to_radians(), (-r[1]).to_radians(), r[2].to_radians()];
        let mirror = b.get("mirror").and_then(Value::as_bool).unwrap_or(false);
        let inflate = b.get("inflate").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let mut quads = Vec::new();
        if let Some(cubes) = b.get("cubes").and_then(Value::as_array) {
            for (i, c) in cubes.iter().enumerate() {
                let c = c.as_object().ok_or("a cube is not an object")?;
                cube(c, mirror, inflate, tw as f32, th as f32, &mut quads)
                    .map_err(|e| format!("bone {name}, cube {i}: {e}"))?;
            }
        }
        if let Some(mesh) = b.get("poly_mesh").and_then(Value::as_object) {
            poly_mesh(mesh, tw as f32, th as f32, &mut quads).map_err(|e| format!("bone {name}, poly_mesh: {e}"))?;
        }
        for q in &mut quads {
            for v in &mut q.vertices {
                v.pos = sub(v.pos, pivot);
            }
        }
        let mut locators = Vec::new();
        if let Some(map) = b.get("locators").and_then(Value::as_object) {
            for (lname, l) in map {
                // [x, y, z] or { "offset": [x, y, z], "rotation": .. }
                let at = if l.is_array() { vec3(Some(l))? } else { vec3(l.get("offset"))? };
                locators.push((lname.clone(), [-at[0], at[1], at[2]]));
            }
        }
        parents.push(b.get("parent").and_then(Value::as_str).map(str::to_string));
        model.bones.push(Bone { name, parent: None, pivot, rotation, quads, children: Vec::new(), locators });
    }
    for (i, parent) in parents.into_iter().enumerate() {
        match parent.and_then(|p| model.bone(&p)) {
            Some(p) if p != i => {
                model.bones[i].parent = Some(p);
                model.bones[p].children.push(i);
            }
            // An unknown parent makes a root bone, as Blockbench and GeckoLib load it.
            _ => model.roots.push(i),
        }
    }
    // A parent loop leaves bones no root reaches: refuse rather than drop them silently.
    let mut seen = vec![false; model.bones.len()];
    let mut stack = model.roots.clone();
    while let Some(i) = stack.pop() {
        if !std::mem::replace(&mut seen[i], true) {
            stack.extend(&model.bones[i].children);
        }
    }
    if let Some(i) = seen.iter().position(|s| !s) {
        return Err(format!("bone {} is in a parent loop", model.bones[i].name));
    }
    Ok(model)
}

fn vec3(v: Option<&Value>) -> Result<[f32; 3], String> {
    let Some(v) = v else { return Ok([0.0; 3]) };
    let a = v.as_array().filter(|a| a.len() == 3).ok_or("expected [x, y, z]")?;
    let mut out = [0.0; 3];
    for (o, x) in out.iter_mut().zip(a) {
        // Molang strings in rest poses are not supported: read them as 0.
        *o = x.as_f64().unwrap_or(0.0) as f32;
    }
    Ok(out)
}

fn vec2(v: Option<&Value>) -> Result<Option<[f32; 2]>, String> {
    let Some(v) = v else { return Ok(None) };
    let a = v.as_array().filter(|a| a.len() == 2).ok_or("expected [u, v]")?;
    Ok(Some([a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32]))
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Face {
    West,
    East,
    North,
    South,
    Up,
    Down,
}

impl Face {
    const ALL: [Face; 6] = [Face::West, Face::East, Face::North, Face::South, Face::Up, Face::Down];
    fn key(self) -> &'static str {
        match self {
            Face::West => "west",
            Face::East => "east",
            Face::North => "north",
            Face::South => "south",
            Face::Up => "up",
            Face::Down => "down",
        }
    }
    fn normal(self) -> [f32; 3] {
        match self {
            Face::West => [-1.0, 0.0, 0.0],
            Face::East => [1.0, 0.0, 0.0],
            Face::North => [0.0, 0.0, -1.0],
            Face::South => [0.0, 0.0, 1.0],
            Face::Up => [0.0, 1.0, 0.0],
            Face::Down => [0.0, -1.0, 0.0],
        }
    }
}

fn cube(
    c: &Map<String, Value>,
    bone_mirror: bool,
    bone_inflate: f32,
    tw: f32,
    th: f32,
    out: &mut Vec<Quad>,
) -> Result<(), String> {
    let mirror = c.get("mirror").and_then(Value::as_bool).unwrap_or(bone_mirror);
    let inflate = c.get("inflate").and_then(Value::as_f64).map(|x| x as f32).unwrap_or(bone_inflate);
    let size = vec3(c.get("size"))?;
    let o = vec3(c.get("origin"))?;
    let origin = [-(o[0] + size[0]), o[1], o[2]];
    let r = vec3(c.get("rotation"))?;
    let rotation = [(-r[0]).to_radians(), (-r[1]).to_radians(), r[2].to_radians()];
    let p = vec3(c.get("pivot"))?;
    let pivot = [-p[0], p[1], p[2]];

    // The eight corners, named as GeckoLib's VertexSet names them.
    let (x0, y0, z0) = (origin[0] - inflate, origin[1] - inflate, origin[2] - inflate);
    let (x1, y1, z1) = (origin[0] + size[0] + inflate, origin[1] + size[1] + inflate, origin[2] + size[2] + inflate);
    let blb = [x0, y0, z0];
    let brb = [x0, y0, z1];
    let tlb = [x0, y1, z0];
    let trb = [x0, y1, z1];
    let tlf = [x1, y1, z0];
    let trf = [x1, y1, z1];
    let blf = [x1, y0, z0];
    let brf = [x1, y0, z1];
    let corners = |face: Face| -> [[f32; 3]; 4] {
        match face {
            Face::West => [trb, tlb, blb, brb],
            Face::East => [tlf, trf, brf, blf],
            Face::North => [tlb, tlf, blf, blb],
            Face::South => [trf, trb, brb, brf],
            Face::Up => [trb, trf, tlf, tlb],
            Face::Down => [blb, blf, brf, brb],
        }
    };

    let uv = c.get("uv");
    let box_uv = match uv {
        None => Some([0.0, 0.0]),
        Some(Value::Array(_)) => vec2(uv)?,
        Some(Value::Object(_)) => None,
        Some(_) => return Err("uv is neither [u, v] nor per-face".into()),
    };
    for face in Face::ALL {
        if zero_size(size, face) {
            continue;
        }
        // (u, v, width, height, rotation in quarter turns), in texels.
        let rect = match box_uv {
            Some([u, v]) => {
                let (sx, sy, sz) = (size[0].floor(), size[1].floor(), size[2].floor());
                let (a, b, w, h) = match face {
                    Face::West => (u + sz + sx, v + sz, sz, sy),
                    Face::East => (u, v + sz, sz, sy),
                    Face::North => (u + sz, v + sz, sx, sy),
                    Face::South => (u + sz + sx + sz, v + sz, sx, sy),
                    Face::Up => (u + sz, v, sx, sz),
                    Face::Down => (u + sz + sx, v + sz, sx, -sz),
                };
                (a, b, w, h, 0)
            }
            None => {
                let Some(f) = uv.and_then(|m| m.get(face.key())).and_then(Value::as_object) else {
                    continue; // a face left out of per-face UV is not drawn
                };
                let [u, v] = vec2(f.get("uv"))?.unwrap_or([0.0, 0.0]);
                let [w, h] = vec2(f.get("uv_size"))?.unwrap_or([0.0, 0.0]);
                let turns = f.get("uv_rotation").and_then(Value::as_i64).unwrap_or(0).rem_euclid(360) / 90;
                (u, v, w, h, turns)
            }
        };
        // With per-face UV a mirrored cube swaps up and down too; with box UV only west and east.
        let geometry_face = match (face, mirror, box_uv.is_some()) {
            (Face::West, true, _) => Face::East,
            (Face::East, true, _) => Face::West,
            (Face::Up, true, false) => Face::Down,
            (Face::Down, true, false) => Face::Up,
            (f, _, _) => f,
        };
        let pos = corners(geometry_face);
        let mut normal = face.normal();
        let (u0, v0, w, h, turns) = rect;
        let (mut ua, va, mut ub, vb) = (u0 / tw, v0 / th, (u0 + w) / tw, (v0 + h) / th);
        if !mirror {
            std::mem::swap(&mut ua, &mut ub);
        } else {
            normal[0] = -normal[0];
        }
        let uvs = match turns {
            0 => [[ua, va], [ub, va], [ub, vb], [ua, vb]],
            1 => [[ub, va], [ub, vb], [ua, vb], [ua, va]],
            2 => [[ub, vb], [ua, vb], [ua, va], [ub, va]],
            _ => [[ua, vb], [ua, va], [ub, va], [ub, vb]],
        };
        let mut vertices = [Vertex { pos: [0.0; 3], u: 0.0, v: 0.0 }; 4];
        for i in 0..4 {
            vertices[i] = Vertex { pos: rotate_about(pos[i], rotation, pivot), u: uvs[i][0], v: uvs[i][1] };
        }
        out.push(Quad { vertices, normal: rotate(normal, rotation) });
    }
    Ok(())
}

fn zero_size(size: [f32; 3], face: Face) -> bool {
    let x_axis = matches!(face, Face::West | Face::East);
    let y_axis = matches!(face, Face::Up | Face::Down);
    let z_axis = matches!(face, Face::North | Face::South);
    (size[0] == 0.0 && !x_axis) || (size[1] == 0.0 && !y_axis) || (size[2] == 0.0 && !z_axis)
}

fn poly_mesh(m: &Map<String, Value>, tw: f32, th: f32, out: &mut Vec<Quad>) -> Result<(), String> {
    let normalized = m.get("normalized_uvs").and_then(Value::as_bool).unwrap_or(false);
    let list = |key: &str, n: usize| -> Result<Vec<Vec<f32>>, String> {
        let Some(a) = m.get(key).and_then(Value::as_array) else { return Ok(Vec::new()) };
        a.iter()
            .map(|p| {
                let p = p.as_array().filter(|p| p.len() == n).ok_or_else(|| format!("{key}: expected {n} numbers"))?;
                Ok(p.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
            })
            .collect()
    };
    let positions = list("positions", 3)?;
    let normals = list("normals", 3)?;
    let uvs = list("uvs", 2)?;
    let polys: Vec<Vec<[usize; 3]>> = match m.get("polys") {
        Some(Value::Array(a)) => a
            .iter()
            .map(|poly| {
                let poly = poly.as_array().filter(|p| p.len() == 3 || p.len() == 4).ok_or("a poly has 3 or 4 corners")?;
                poly.iter()
                    .map(|c| {
                        let c = c.as_array().filter(|c| c.len() == 3).ok_or("a corner is [position, normal, uv]")?;
                        let i = |k: usize| c[k].as_u64().map(|x| x as usize).ok_or("a corner index is not a number");
                        Ok([i(0)?, i(1)?, i(2)?])
                    })
                    .collect::<Result<Vec<_>, &str>>()
            })
            .collect::<Result<_, &str>>()?,
        // "tri_list" / "quad_list": every position, normal and uv in order.
        Some(Value::String(kind)) => {
            let n = if kind == "quad_list" { 4 } else { 3 };
            (0..positions.len() / n).map(|p| (0..n).map(|k| [p * n + k; 3]).collect()).collect()
        }
        _ => return Err("polys missing".into()),
    };
    for poly in polys {
        let mut vertices = [Vertex { pos: [0.0; 3], u: 0.0, v: 0.0 }; 4];
        let mut normal = [0.0; 3];
        // Bedrock's x turned around reverses the winding: walk the corners backwards.
        for (slot, k) in (0..4).zip((0..poly.len()).rev().chain(std::iter::once(0))) {
            let [pi, ni, ui] = poly[k];
            let p = positions.get(pi).ok_or("position index out of range")?;
            let uv = uvs.get(ui).ok_or("uv index out of range")?;
            let (u, v) = if normalized { (uv[0], 1.0 - uv[1]) } else { (uv[0] / tw, 1.0 - uv[1] / th) };
            vertices[slot] = Vertex { pos: [-p[0], p[1], p[2]], u, v };
            if let Some(n) = normals.get(ni) {
                normal = add(normal, [-n[0], n[1], n[2]]);
            }
        }
        if poly.len() == 3 {
            vertices[3] = vertices[2];
        }
        let len = dot(normal, normal).sqrt();
        out.push(Quad { vertices, normal: if len > 0.0 { scale(normal, 1.0 / len) } else { face_normal(&vertices) } });
    }
    Ok(())
}

fn face_normal(v: &[Vertex; 4]) -> [f32; 3] {
    let n = cross(sub(v[1].pos, v[0].pos), sub(v[2].pos, v[0].pos));
    let len = dot(n, n).sqrt();
    if len > 0.0 { scale(n, 1.0 / len) } else { [0.0, 1.0, 0.0] }
}

/// `p` turned by `r` (radians; x first, then y, then z, as `rotationZYX`) about `pivot`.
pub fn rotate_about(p: [f32; 3], r: [f32; 3], pivot: [f32; 3]) -> [f32; 3] {
    add(rotate(sub(p, pivot), r), pivot)
}

/// `v` turned by x, then y, then z (the matrix Rz·Ry·Rx).
pub fn rotate(v: [f32; 3], r: [f32; 3]) -> [f32; 3] {
    let [mut x, mut y, mut z] = v;
    if r[0] != 0.0 {
        let (s, c) = r[0].sin_cos();
        (y, z) = (y * c - z * s, y * s + z * c);
    }
    if r[1] != 0.0 {
        let (s, c) = r[1].sin_cos();
        (x, z) = (x * c + z * s, -x * s + z * c);
    }
    if r[2] != 0.0 {
        let (s, c) = r[2].sin_cos();
        (x, y) = (x * c - y * s, x * s + y * c);
    }
    [x, y, z]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn humanoid_box_uv() {
        let m = &parse(br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.t","texture_width":64,"texture_height":64},
            "bones":[{"name":"body","pivot":[0,24,0],"cubes":[{"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]}]},
                     {"name":"head","parent":"body","pivot":[0,24,0],"cubes":[{"origin":[-4,24,-4],"size":[8,8,8],"uv":[0,0]}]},
                     {"name":"rightArm","parent":"body","pivot":[-5,22,0],"rotation":[10,0,0],"cubes":[{"origin":[-8,12,-2],"size":[4,12,4],"uv":[40,16]}]}]}]}"#)
            .unwrap()[0];
        assert_eq!(m.roots, vec![0]);
        assert_eq!(m.bones[0].children, vec![1, 2]);
        assert_eq!(m.quad_count(), 18);
        // Bedrock x is turned around: the right arm's pivot at -5 is at +5 here.
        assert_eq!(m.bones[2].pivot, [5.0, 22.0, 0.0]);
        assert!(close(m.bones[2].rotation[0], -10f32.to_radians()));
        // The head's north (front) face: GeckoLib's box layout, u swapped when not mirrored.
        let north = m.bones[1].quads[2];
        assert_eq!(north.normal, [0.0, 0.0, -1.0]);
        assert!(close(north.vertices[0].u, 16.0 / 64.0) && close(north.vertices[1].u, 8.0 / 64.0));
        assert!(close(north.vertices[0].v, 8.0 / 64.0) && close(north.vertices[2].v, 16.0 / 64.0));
        // Relative to the pivot: the head spans y 0..8 above it, x -4..4.
        let ys: Vec<f32> = m.bones[1].quads.iter().flat_map(|q| q.vertices.iter().map(|v| v.pos[1])).collect();
        assert!(ys.iter().all(|&y| (0.0..=8.0).contains(&y)));
    }

    #[test]
    fn per_face_uv_skips_missing_faces_and_rotates() {
        let m = &parse(br#"{"format_version":"1.21.0","minecraft:geometry":[{"description":{"identifier":"geometry.f","texture_width":32,"texture_height":32},
            "bones":[{"name":"b","cubes":[{"origin":[0,0,0],"size":[2,2,2],"uv":{"north":{"uv":[0,0],"uv_size":[2,2]},"up":{"uv":[4,4],"uv_size":[2,2],"uv_rotation":90}}}]}]}]}"#)
            .unwrap()[0];
        assert_eq!(m.bones[0].quads.len(), 2);
        let up = m.bones[0].quads[1];
        // Quarter turn: the first corner takes the second corner's place.
        assert!(close(up.vertices[0].u, 4.0 / 32.0) && close(up.vertices[0].v, 4.0 / 32.0));
    }

    #[test]
    fn cube_rotation_about_its_pivot() {
        let m = &parse(br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.r","texture_width":16,"texture_height":16},
            "bones":[{"name":"b","cubes":[{"origin":[0,0,0],"size":[1,1,1],"pivot":[0,0,0],"rotation":[0,0,90],"uv":[0,0]}]}]}]}"#)
            .unwrap()[0];
        // x turned around: the cube spans x -1..0; a quarter turn about z through the origin
        // takes it to y -1..0.
        for q in &m.bones[0].quads {
            for v in q.vertices {
                assert!(v.pos[1] <= 1e-4 && v.pos[1] >= -1.0 - 1e-4, "{:?}", v.pos);
            }
        }
    }

    #[test]
    fn old_format_and_poly_mesh() {
        let m = &parse(br#"{"format_version":"1.10.0","geometry.old:geometry.parent":{"texturewidth":16,"textureheight":16,
            "bones":[{"name":"m","poly_mesh":{"normalized_uvs":true,"positions":[[0,0,0],[1,0,0],[0,1,0]],"normals":[[0,0,1]],"uvs":[[0,0],[1,0],[0,1]],
            "polys":[[[0,0,0],[1,0,1],[2,0,2]]]}}]}}"#)
            .unwrap()[0];
        assert_eq!(m.identifier, "geometry.old");
        let q = m.bones[0].quads[0];
        assert_eq!(q.vertices[3], q.vertices[2]);
        assert_eq!(q.vertices[0].pos, [0.0, 1.0, 0.0]);
        assert_eq!(q.normal, [0.0, 0.0, 1.0]);
    }

    #[test]
    fn parent_loop_is_refused() {
        let e = parse(br#"{"minecraft:geometry":[{"description":{"identifier":"geometry.l"},
            "bones":[{"name":"a","parent":"b"},{"name":"b","parent":"a"}]}]}"#)
            .unwrap_err();
        assert!(e.contains("parent loop"), "{e}");
    }
}
