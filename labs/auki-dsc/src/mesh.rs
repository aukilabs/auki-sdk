//! Triangle mesh ingest. Coordinates are metres, Y-up (OpenGL / domain OBJ).

use crate::error::DscError;
use crate::types::Vec3;

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

    /// Vertices belonging to a named group (for segment-path sampling).
    pub fn group_vertices(&self, group: &MeshGroup) -> Vec<Vec3> {
        let mut out = Vec::new();
        for tri in &self.indices[group.triangle_range.clone()] {
            for &i in tri {
                out.push(self.vertices[i as usize]);
            }
        }
        out
    }
}

/// Parse Wavefront OBJ text (vertices + triangle faces; fan for n-gons).
/// Tracks `o` / `g` names for segment path.
pub fn parse_obj(text: &str) -> Result<TriangleMesh, DscError> {
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
                            DscError::ObjParse(format!(
                                "line {}: bad vertex float",
                                line_no + 1
                            ))
                        })
                    })
                    .collect::<Result<_, _>>()?;
                if nums.len() != 3 {
                    return Err(DscError::ObjParse(format!(
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
                            DscError::ObjParse(format!(
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
                            return Err(DscError::ObjParse(format!(
                                "line {}: face index {idx} out of range",
                                line_no + 1
                            )));
                        }
                        Ok(resolved - 1)
                    })
                    .collect::<Result<_, _>>()?;
                if face.len() < 3 {
                    return Err(DscError::ObjParse(format!(
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
        return Err(DscError::EmptyMesh);
    }
    Ok(TriangleMesh {
        vertices,
        indices,
        groups,
    })
}
