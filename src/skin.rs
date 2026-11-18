//! A player's own texture on a model without the texture atlas: every texel of every face becomes
//! a small quad of its colour. The game can't load a picture while it runs, so the model is
//! drawn in coloured quads instead; it looks the same, texel for texel. No mod API types here.

use crate::geo::{Model, Quad};

/// A texel's quad: corners in its bone's frame (texels), and its colour 0xRRGGBB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Texel {
    pub corners: [[f32; 3]; 4],
    pub rgb: u32,
}

/// An RGBA picture.
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    pub rgba: &'a [u8],
}

impl Image<'_> {
    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y.min(self.height - 1) * self.width + x.min(self.width - 1)) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Every bone's texel quads, in bone order. Faces whose corners don't make a parallelogram (mesh
/// triangles) get one quad of their texels' average colour.
pub fn texels(model: &Model, image: &Image<'_>) -> Vec<Vec<Texel>> {
    model.bones.iter().map(|b| b.quads.iter().flat_map(|q| face(q, image)).collect()).collect()
}

fn rgb(p: [u8; 4]) -> u32 {
    (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32
}

fn face(q: &Quad, img: &Image<'_>) -> Vec<Texel> {
    let v = q.vertices;
    let (w, h) = (img.width as f32, img.height as f32);
    let p0 = v[0].pos;
    let es = sub(v[1].pos, p0);
    let et = sub(v[3].pos, p0);
    let parallelogram = (0..3).all(|i| (v[0].pos[i] + v[2].pos[i] - v[1].pos[i] - v[3].pos[i]).abs() < 1e-3);
    // UV (in texels) as a function of (s, t) along the face's edges 0→1 and 0→3.
    let uv0 = [v[0].u * w, v[0].v * h];
    let us = [v[1].u * w - uv0[0], v[1].v * h - uv0[1]];
    let ut = [v[3].u * w - uv0[0], v[3].v * h - uv0[1]];
    let det = us[0] * ut[1] - us[1] * ut[0];
    let (umin, umax) = (v.iter().map(|c| c.u * w).fold(f32::MAX, f32::min), v.iter().map(|c| c.u * w).fold(f32::MIN, f32::max));
    let (vmin, vmax) = (v.iter().map(|c| c.v * h).fold(f32::MAX, f32::min), v.iter().map(|c| c.v * h).fold(f32::MIN, f32::max));
    if !parallelogram || det.abs() < 1e-6 {
        // One quad, the average colour of the face's texels.
        let (mut sum, mut n) = ([0u32; 3], 0u32);
        for y in vmin.floor().max(0.0) as u32..(vmax.ceil() as u32).clamp(1, img.height) {
            for x in umin.floor().max(0.0) as u32..(umax.ceil() as u32).clamp(1, img.width) {
                let p = img.pixel(x, y);
                if p[3] >= 128 {
                    sum = [sum[0] + p[0] as u32, sum[1] + p[1] as u32, sum[2] + p[2] as u32];
                    n += 1;
                }
            }
        }
        if n == 0 {
            return Vec::new();
        }
        let c = [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255];
        return vec![Texel { corners: [v[0].pos, v[1].pos, v[2].pos, v[3].pos], rgb: rgb(c) }];
    }
    // (s, t) for a point (u, v) in texels: the inverse of the 2x2 map.
    let st = |u: f32, vv: f32| {
        let (du, dv) = (u - uv0[0], vv - uv0[1]);
        ((du * ut[1] - dv * ut[0]) / det, (us[0] * dv - us[1] * du) / det)
    };
    let at = |s: f32, t: f32| [p0[0] + es[0] * s + et[0] * t, p0[1] + es[1] * s + et[1] * t, p0[2] + es[2] * s + et[2] * t];
    let mut out = Vec::new();
    let (x0, x1) = (umin.floor().max(0.0) as u32, (umax.ceil() as u32).min(img.width));
    let (y0, y1) = (vmin.floor().max(0.0) as u32, (vmax.ceil() as u32).min(img.height));
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.pixel(x, y);
            if p[3] < 128 {
                continue;
            }
            // The texel's rectangle cut to the face's, then into (s, t).
            let (ua, ub) = ((x as f32).max(umin), ((x + 1) as f32).min(umax));
            let (va, vb) = ((y as f32).max(vmin), ((y + 1) as f32).min(vmax));
            if ub - ua < 1e-4 || vb - va < 1e-4 {
                continue;
            }
            let corners_st = [st(ua, va), st(ub, va), st(ub, vb), st(ua, vb)];
            let smin = corners_st.iter().map(|c| c.0).fold(f32::MAX, f32::min).max(0.0);
            let smax = corners_st.iter().map(|c| c.0).fold(f32::MIN, f32::max).min(1.0);
            let tmin = corners_st.iter().map(|c| c.1).fold(f32::MAX, f32::min).max(0.0);
            let tmax = corners_st.iter().map(|c| c.1).fold(f32::MIN, f32::max).min(1.0);
            if smax - smin < 1e-5 || tmax - tmin < 1e-5 {
                continue;
            }
            out.push(Texel { corners: [at(smin, tmin), at(smax, tmin), at(smax, tmax), at(smin, tmax)], rgb: rgb(p) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cube_in_texels() {
        let m = &crate::geo::parse(br#"{"minecraft:geometry":[{"description":{"identifier":"geometry.c","texture_width":8,"texture_height":8},
            "bones":[{"name":"b","cubes":[{"origin":[0,0,0],"size":[2,2,2],"uv":[0,0]}]}]}]}"#)
        .unwrap()[0];
        // All red but one clear texel on the north face (box UV: x 2..4, y 2..4).
        let mut rgba = vec![0u8; 8 * 8 * 4];
        for p in rgba.chunks_mut(4) {
            p.copy_from_slice(&[255, 0, 0, 255]);
        }
        let i = (2 * 8 + 2) * 4;
        rgba[i + 3] = 0;
        let t = texels(m, &Image { width: 8, height: 8, rgba: &rgba });
        assert_eq!(t[0].len(), 6 * 4 - 1);
        assert!(t[0].iter().all(|q| q.rgb == 0xFF0000));
        // The quads cover the cube's faces exactly: total area 6 faces of 2x2.
        let area: f32 = t[0]
            .iter()
            .map(|q| {
                let a = sub(q.corners[1], q.corners[0]);
                let b = sub(q.corners[3], q.corners[0]);
                let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt()
            })
            .sum();
        assert!((area - 23.0).abs() < 1e-3, "{area}");
    }

    #[test]
    fn triangles_get_an_average() {
        let m = &crate::geo::parse(br#"{"minecraft:geometry":[{"description":{"identifier":"geometry.t","texture_width":2,"texture_height":2},
            "bones":[{"name":"m","poly_mesh":{"normalized_uvs":true,"positions":[[0,0,0],[1,0,0],[0,1,0]],"normals":[[0,0,-1]],"uvs":[[0,0],[1,0],[0,1]],"polys":[[[0,0,0],[1,0,1],[2,0,2]]]}}]}]}"#)
        .unwrap()[0];
        let rgba = [0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255];
        let t = texels(m, &Image { width: 2, height: 2, rgba: &rgba });
        assert_eq!(t[0].len(), 1);
        assert_eq!(t[0][0].rgb, 0x7F7F7F);
    }
}
