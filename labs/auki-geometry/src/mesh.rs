//! Triangle-mesh ingest and raycast. Coordinates are metres, Y-up (OpenGL).
//!
//! Parse / triangulate / index / raycast here. Recast bake is `auki-navigation`.
//! Occupancy slice is `auki-raster`.

use crate::{GeometryError, Result};
use std::cmp::Ordering;

/// Metres, Y-up. Mesh-local — not [`auki_datatypes::pose::Vec3`] (f64 poses).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len <= f32::EPSILON {
            Self::default()
        } else {
            Self::new(self.x / len, self.y / len, self.z / len)
        }
    }

    pub fn dot(self, other: Self) -> f32 {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, rhs: f32) -> Self {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl std::ops::Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}

/// Ray–triangle hit. Distance is along the ray (metres).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub distance: f32,
    pub normal: Vec3,
}

/// Named OBJ object/group (`o` / `g`). Triangle indices into [`TriangleMesh::indices`].
#[derive(Clone, Debug, PartialEq)]
pub struct MeshGroup {
    pub name: String,
    /// Inclusive start / exclusive end into `TriangleMesh::indices`.
    pub triangle_range: std::ops::Range<usize>,
}

/// Indexed triangle soup. Faces are CCW when viewed from +Y for a floor.
#[derive(Clone, Debug, Default)]
pub struct TriangleMesh {
    pub vertices: Vec<Vec3>,
    pub indices: Vec<[u32; 3]>,
    pub groups: Vec<MeshGroup>,
}

impl TriangleMesh {
    /// Build a mesh from indexed triangles. `groups` may be empty.
    pub fn from_indexed(
        vertices: Vec<Vec3>,
        indices: Vec<[u32; 3]>,
        groups: Vec<MeshGroup>,
    ) -> Result<Self> {
        if vertices.is_empty() || indices.is_empty() {
            return Err(GeometryError::EmptyMesh);
        }
        for tri in &indices {
            for &i in tri {
                if i as usize >= vertices.len() {
                    return Err(GeometryError::InvalidIndex(format!(
                        "triangle index {i} out of range ({} verts)",
                        vertices.len()
                    )));
                }
            }
        }
        Ok(Self {
            vertices,
            indices,
            groups,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty() || self.indices.is_empty()
    }

    pub fn triangles(&self) -> impl Iterator<Item = (Vec3, Vec3, Vec3)> + '_ {
        self.indices.iter().map(|&[a, b, c]| {
            (
                self.vertices[a as usize],
                self.vertices[b as usize],
                self.vertices[c as usize],
            )
        })
    }

    /// Vertices belonging to a named group (segment-path sampling).
    pub fn group_vertices(&self, group: &MeshGroup) -> Vec<Vec3> {
        let mut out = Vec::new();
        for tri in &self.indices[group.triangle_range.clone()] {
            for &i in tri {
                out.push(self.vertices[i as usize]);
            }
        }
        out
    }

    /// Möller–Trumbore raycast. `direction` need not be unit. Hits sorted near→far.
    pub fn raycast(&self, origin: Vec3, direction: Vec3) -> Vec<RayHit> {
        let mut hits = Vec::new();
        for (a, b, c) in self.triangles() {
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

/// OBJ adapter → [`TriangleMesh`]. Fans n-gons to triangles. Tracks `o` / `g`.
pub fn parse_obj(text: &str) -> Result<TriangleMesh> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut groups = Vec::new();
    let mut current_name = String::from("default");
    let mut group_start = 0usize;

    let flush_group =
        |groups: &mut Vec<MeshGroup>, name: &str, start: usize, end: usize| {
            if end > start {
                groups.push(MeshGroup {
                    name: name.to_string(),
                    triangle_range: start..end,
                });
            }
        };

    for (line_no, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(tag) = parts.next() else {
            continue;
        };
        match tag {
            "v" => {
                let nums: Vec<f32> = parts
                    .take(3)
                    .map(|s| {
                        s.parse::<f32>().map_err(|_| {
                            GeometryError::ObjParse(format!(
                                "line {}: bad vertex float",
                                line_no + 1
                            ))
                        })
                    })
                    .collect::<Result<_>>()?;
                if nums.len() != 3 {
                    return Err(GeometryError::ObjParse(format!(
                        "line {}: vertex needs 3 components",
                        line_no + 1
                    )));
                }
                vertices.push(Vec3::new(nums[0], nums[1], nums[2]));
            }
            "o" | "g" => {
                flush_group(&mut groups, &current_name, group_start, indices.len());
                current_name = parts.next().unwrap_or("unnamed").to_string();
                group_start = indices.len();
            }
            "f" => {
                let face: Vec<u32> = parts
                    .map(|tok| {
                        let idx_str = tok.split('/').next().unwrap_or(tok);
                        let idx: i32 = idx_str.parse().map_err(|_| {
                            GeometryError::ObjParse(format!(
                                "line {}: bad face index",
                                line_no + 1
                            ))
                        })?;
                        let resolved = if idx < 0 {
                            (vertices.len() as i32 + idx + 1) as u32
                        } else {
                            idx as u32
                        };
                        if resolved == 0 || resolved as usize > vertices.len() {
                            return Err(GeometryError::ObjParse(format!(
                                "line {}: face index {idx} out of range",
                                line_no + 1
                            )));
                        }
                        Ok(resolved - 1)
                    })
                    .collect::<Result<_>>()?;
                if face.len() < 3 {
                    return Err(GeometryError::ObjParse(format!(
                        "line {}: face needs ≥3 indices",
                        line_no + 1
                    )));
                }
                for i in 1..face.len() - 1 {
                    indices.push([face[0], face[i], face[i + 1]]);
                }
            }
            _ => {}
        }
    }

    flush_group(&mut groups, &current_name, group_start, indices.len());

    if vertices.is_empty() || indices.is_empty() {
        return Err(GeometryError::EmptyMesh);
    }
    Ok(TriangleMesh {
        vertices,
        indices,
        groups,
    })
}
