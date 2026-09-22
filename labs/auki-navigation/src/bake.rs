//! Recast bake profiles and poly-mesh → landmass conversion.
//!
//! Public API is Y-up. Landmass stores Z-up; conversion is only here / in nav.

use crate::error::NavError;
use crate::types::Vec3;
use auki_geometry::TriangleMesh;
use landmass::{NavigationMesh, ValidNavigationMesh, XYZ};
use rerecast::{Aabb3d, AreaType, BuildContoursFlags, HeightfieldBuilder, PolygonNavmesh, TriMesh};
use std::sync::Arc;

/// Recast bake knobs. Presets encode known client disagreement (Gotu vs DSC vs robots).
#[derive(Clone, Debug, PartialEq)]
pub struct BakeProfile {
    /// Walkable agent radius in metres (Recast erosion). Not restrict min-separation.
    pub walkable_radius: f32,
    pub cell_size: f32,
    pub cell_height: f32,
    pub agent_height: f32,
    pub walkable_climb: f32,
    /// Radians.
    pub walkable_slope_angle: f32,
    pub max_simplification_error: f32,
    pub min_region_area: u16,
    pub merge_region_area: u16,
    pub max_edge_len: u16,
    pub detail_sample_dist: f32,
    pub detail_sample_max_error: f32,
    pub max_vertices_per_polygon: u16,
}

impl BakeProfile {
    /// Browser Recast (Gotu / cactus-search): radius 0.05, cs=ch=0.05.
    pub fn gotu() -> Self {
        Self {
            walkable_radius: 0.05,
            cell_size: 0.05,
            cell_height: 0.05,
            ..Self::dsc_defaults(0.05)
        }
    }

    /// DSC Deno bake cells (cs=0.05, ch=0.01). `agent_radius` is **metres** → voxels
    /// via `(m / cs).ceil()`. That is the correct contract.
    ///
    /// Legacy DSC HTTP / `@recast-navigation` bug: passes `walkableRadius` straight into
    /// Recast's int voxel field with no m→vx convert, so `radius: 0.05` eroded **0** cells
    /// (`int(0.05)==0`). Only use [`Self::dsc`]`(0.0)` when you intentionally need to
    /// bit-match that broken bake. Prefer real metres (`0.05`, `0.4`, …).
    ///
    /// Use [`Self::restrict_bake`] for restrict-style bake (no erosion).
    pub fn dsc(agent_radius: f32) -> Self {
        Self::dsc_defaults(agent_radius)
    }

    /// Robots: DSC cells + 0.4 m radius + taller climb/height.
    pub fn robot_r40() -> Self {
        Self {
            walkable_radius: 0.40,
            agent_height: 1.6,
            walkable_climb: 0.4,
            ..Self::dsc_defaults(0.40)
        }
    }

    /// DSC restrict bake (walkable_radius 0).
    pub fn restrict_bake() -> Self {
        Self::dsc(0.0)
    }

    fn dsc_defaults(agent_radius: f32) -> Self {
        Self {
            walkable_radius: agent_radius,
            cell_size: 0.05,
            cell_height: 0.01,
            agent_height: 2.0,
            walkable_climb: 0.9,
            walkable_slope_angle: 45.0_f32.to_radians(),
            max_simplification_error: 0.5,
            min_region_area: 2,
            merge_region_area: 8,
            max_edge_len: 30,
            detail_sample_dist: 2.0,
            detail_sample_max_error: 0.5,
            max_vertices_per_polygon: 6,
        }
    }
}

/// OpenGL Y-up → landmass XYZ (Z-up): `(x, y, z) → (x, z, y)`.
#[inline]
pub(crate) fn yup_to_landmass(v: Vec3) -> landmass::Vec3 {
    landmass::Vec3::new(v.x, v.z, v.y)
}

#[inline]
pub(crate) fn landmass_to_yup(v: landmass::Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y)
}

pub(crate) struct BakedNav {
    pub archipelago_mesh: Arc<ValidNavigationMesh<XYZ>>,
    pub agent_radius: f32,
}

pub(crate) fn bake_navmesh(
    mesh: &TriangleMesh,
    profile: &BakeProfile,
) -> Result<BakedNav, NavError> {
    if mesh.is_empty() {
        return Err(NavError::EmptyMesh);
    }

    let mut trimesh = TriMesh {
        vertices: mesh
            .vertices
            .iter()
            .map(|v| glam::Vec3A::new(v.x, v.y, v.z))
            .collect(),
        indices: mesh
            .indices
            .iter()
            .map(|&[a, b, c]| glam::UVec3::new(a, b, c))
            .collect(),
        area_types: vec![AreaType::NOT_WALKABLE; mesh.indices.len()],
    };

    // Domain OBJs sometimes wind floors CW when viewed from +Y. Recast only
    // marks upward-facing tris walkable — flip downward ones (DSC Python does this).
    for tri in &mut trimesh.indices {
        let a = trimesh.vertices[tri.x as usize];
        let b = trimesh.vertices[tri.y as usize];
        let c = trimesh.vertices[tri.z as usize];
        let normal = (b - a).cross(c - a);
        if normal.y < 0.0 {
            std::mem::swap(&mut tri.y, &mut tri.z);
        }
    }

    trimesh.mark_walkable_triangles(profile.walkable_slope_angle);

    let aabb = trimesh.compute_aabb().ok_or(NavError::EmptyMesh)?;
    // Recast needs vertical room above the floor for agent height / climb filters.
    let pad = profile.cell_size * 2.0;
    let aabb = Aabb3d {
        min: glam::Vec3::new(aabb.min.x - pad, aabb.min.y - pad, aabb.min.z - pad),
        max: glam::Vec3::new(
            aabb.max.x + pad,
            aabb.max.y + profile.agent_height + profile.walkable_climb + pad,
            aabb.max.z + pad,
        ),
    };

    let cs = profile.cell_size;
    let ch = profile.cell_height;
    let walkable_radius_vx = if profile.walkable_radius <= 0.0 {
        0u16
    } else {
        (profile.walkable_radius / cs).ceil() as u16
    };
    let walkable_height = (profile.agent_height / ch).ceil().max(3.0) as u16;
    let walkable_climb = (profile.walkable_climb / ch).floor() as u16;

    let mut heightfield = HeightfieldBuilder {
        aabb,
        cell_size: cs,
        cell_height: ch,
    }
    .build()
    .map_err(|e| NavError::Bake(e.to_string()))?;

    heightfield
        .rasterize_triangles(&trimesh, walkable_climb)
        .map_err(|e| NavError::Bake(e.to_string()))?;

    heightfield.filter_low_hanging_walkable_obstacles(walkable_climb);
    heightfield.filter_ledge_spans(walkable_height, walkable_climb);
    heightfield.filter_walkable_low_height_spans(walkable_height);

    let mut compact = heightfield
        .into_compact(walkable_height, walkable_climb)
        .map_err(|e| NavError::Bake(e.to_string()))?;

    compact.erode_walkable_area(walkable_radius_vx);
    compact.build_distance_field();
    compact
        .build_regions(0, profile.min_region_area, profile.merge_region_area)
        .map_err(|e| NavError::Bake(e.to_string()))?;

    let contours = compact.build_contours(
        profile.max_simplification_error,
        profile.max_edge_len,
        BuildContoursFlags::default(),
    );

    let poly_mesh = contours
        .into_polygon_mesh(profile.max_vertices_per_polygon)
        .map_err(|e| NavError::Bake(e.to_string()))?;

    if poly_mesh.polygon_count() == 0 {
        return Err(NavError::EmptyNavMesh);
    }

    let nav = poly_mesh_to_landmass(&poly_mesh)?;
    let validated = nav
        .validate()
        .map_err(|e| NavError::Bake(format!("landmass validate: {e:?}")))?;

    Ok(BakedNav {
        archipelago_mesh: Arc::new(validated),
        agent_radius: profile.walkable_radius.max(0.05),
    })
}

fn poly_mesh_to_landmass(poly: &PolygonNavmesh) -> Result<NavigationMesh<XYZ>, NavError> {
    let cs = poly.cell_size;
    let ch = poly.cell_height;
    let orig = poly.aabb.min;

    let vertices: Vec<landmass::Vec3> = poly
        .vertices
        .iter()
        .map(|v| {
            let yup = Vec3::new(
                orig.x + v.x as f32 * cs,
                orig.y + v.y as f32 * ch,
                orig.z + v.z as f32 * cs,
            );
            yup_to_landmass(yup)
        })
        .collect();

    let nvp = poly.max_vertices_per_polygon as usize;
    let mut polygons = Vec::with_capacity(poly.polygon_count());
    let mut polygon_type_indices = Vec::with_capacity(poly.polygon_count());

    for chunk in poly.polygons.chunks_exact(nvp) {
        let mut poly_idxs = Vec::new();
        for &idx in chunk {
            if idx == PolygonNavmesh::NO_INDEX {
                break;
            }
            poly_idxs.push(idx as usize);
        }
        if poly_idxs.len() < 3 {
            continue;
        }
        // Remap (x,y,z)→(x,z,y) flips the ground-plane handedness relative to
        // landmass's Z-up CCW expectation — reverse winding.
        poly_idxs.reverse();
        polygons.push(poly_idxs);
        polygon_type_indices.push(0);
    }

    if polygons.is_empty() {
        return Err(NavError::EmptyNavMesh);
    }

    Ok(NavigationMesh {
        vertices,
        polygons,
        polygon_type_indices,
        height_mesh: None,
    })
}
