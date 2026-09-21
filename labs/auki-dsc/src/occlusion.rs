//! Occlusion raycast (Möller–Trumbore) and Y-plane cross-section raster.

use crate::mesh::TriangleMesh;
use crate::types::{CrossSection, RayHit, Vec3};
use image::{ImageBuffer, ImageFormat, Luma};
use std::cmp::Ordering;
use std::io::Cursor;

/// Triangle soup for occlusion queries (not the navmesh).
pub struct OcclusionMesh {
    triangles: Vec<(Vec3, Vec3, Vec3)>,
}

impl OcclusionMesh {
    pub fn from_mesh(mesh: TriangleMesh) -> Self {
        Self {
            triangles: mesh.triangles().collect(),
        }
    }

    /// Raycast along `direction` (need not be unit). Hits sorted by distance ascending.
    pub fn raycast(&self, origin: Vec3, direction: Vec3) -> Vec<RayHit> {
        let mut hits = Vec::new();
        for &(a, b, c) in &self.triangles {
            if let Some(hit) = ray_triangle(origin, direction, a, b, c) {
                hits.push(hit);
            }
        }
        hits.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(Ordering::Equal)
        });
        hits
    }

    /// Slice at `y = height`, rasterize at `ppm` pixels/metre → PNG + map.yaml.
    pub fn cross_section(&self, height: f32, ppm: f32) -> CrossSection {
        let segments = plane_segments(&self.triangles, height);
        rasterize_segments(&segments, ppm)
    }
}

fn ray_triangle(
    origin: Vec3,
    direction: Vec3,
    v0: Vec3,
    v1: Vec3,
    v2: Vec3,
) -> Option<RayHit> {
    const EPS: f32 = 1e-7;
    let edge1 = v1 - v0;
    let edge2 = v2 - v0;
    let h = direction.cross(edge2);
    let a = edge1.dot(h);
    if a > -EPS && a < EPS {
        return None;
    }
    let f = 1.0 / a;
    let s = origin - v0;
    let u = f * s.dot(h);
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(edge1);
    let v = f * direction.dot(q);
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = f * edge2.dot(q);
    if t < EPS {
        return None;
    }
    let point = origin + direction * t;
    let normal = edge1.cross(edge2).normalized();
    Some(RayHit {
        point,
        distance: t * direction.length(),
        normal,
    })
}

fn plane_segments(tris: &[(Vec3, Vec3, Vec3)], height: f32) -> Vec<(Vec3, Vec3)> {
    let mut out = Vec::new();
    for &(a, b, c) in tris {
        let mut pts = Vec::new();
        edge_plane_hit(a, b, height, &mut pts);
        edge_plane_hit(b, c, height, &mut pts);
        edge_plane_hit(c, a, height, &mut pts);
        if pts.len() >= 2 {
            out.push((pts[0], pts[1]));
        }
    }
    out
}

fn edge_plane_hit(a: Vec3, b: Vec3, height: f32, out: &mut Vec<Vec3>) {
    let da = a.y - height;
    let db = b.y - height;
    if da * db > 0.0 {
        return;
    }
    if da.abs() < 1e-8 && db.abs() < 1e-8 {
        out.push(a);
        out.push(b);
        return;
    }
    if da.abs() < 1e-8 {
        out.push(a);
        return;
    }
    if db.abs() < 1e-8 {
        out.push(b);
        return;
    }
    let t = da / (da - db);
    out.push(a + (b - a) * t);
}

fn rasterize_segments(segments: &[(Vec3, Vec3)], ppm: f32) -> CrossSection {
    if segments.is_empty() {
        let png = encode_png(1, 1, &[255]);
        return CrossSection {
            png,
            map_yaml: map_yaml(1, 1, ppm, 0.0, 0.0),
            width: 1,
            height: 1,
            resolution: 1.0 / ppm,
            origin_x: 0.0,
            origin_z: 0.0,
        };
    }

    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &(a, b) in segments {
        for p in [a, b] {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_z = min_z.min(p.z);
            max_z = max_z.max(p.z);
        }
    }
    let pad = 1.0 / ppm;
    min_x -= pad;
    max_x += pad;
    min_z -= pad;
    max_z += pad;

    let width = ((max_x - min_x) * ppm).ceil().max(1.0) as u32;
    let height = ((max_z - min_z) * ppm).ceil().max(1.0) as u32;
    let mut pixels = vec![255u8; (width * height) as usize];

    for &(a, b) in segments {
        draw_line(
            &mut pixels, width, height, min_x, min_z, ppm, a.x, a.z, b.x, b.z,
        );
    }

    CrossSection {
        png: encode_png(width, height, &pixels),
        map_yaml: map_yaml(width, height, ppm, min_x, min_z),
        width,
        height,
        resolution: 1.0 / ppm,
        origin_x: min_x,
        origin_z: min_z,
    }
}

fn draw_line(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    origin_x: f32,
    origin_z: f32,
    ppm: f32,
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
) {
    let mut x0i = ((x0 - origin_x) * ppm).round() as i32;
    let mut z0i = ((z0 - origin_z) * ppm).round() as i32;
    let x1i = ((x1 - origin_x) * ppm).round() as i32;
    let z1i = ((z1 - origin_z) * ppm).round() as i32;
    let dx = (x1i - x0i).abs();
    let sx = if x0i < x1i { 1 } else { -1 };
    let dy = -(z1i - z0i).abs();
    let sy = if z0i < z1i { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0i >= 0 && z0i >= 0 && (x0i as u32) < width && (z0i as u32) < height {
            pixels[(z0i as u32 * width + x0i as u32) as usize] = 0;
        }
        if x0i == x1i && z0i == z1i {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0i += sx;
        }
        if e2 <= dx {
            err += dx;
            z0i += sy;
        }
    }
}

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let img: ImageBuffer<Luma<u8>, Vec<u8>> =
        ImageBuffer::from_raw(width, height, pixels.to_vec()).expect("png buffer size");
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).expect("png encode");
    buf.into_inner()
}

fn map_yaml(width: u32, height: u32, ppm: f32, origin_x: f32, origin_z: f32) -> String {
    let resolution = 1.0 / ppm;
    format!(
        "image: map.png\nresolution: {resolution}\norigin: [{origin_x}, {origin_z}, 0.0]\nnegate: 0\noccupied_thresh: 0.65\nfree_thresh: 0.196\nwidth: {width}\nheight: {height}\n"
    )
}
