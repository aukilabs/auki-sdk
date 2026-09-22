//! NavMesh queries over a baked landmass archipelago.

use crate::bake::{BakeProfile, BakedNav, bake_navmesh, landmass_to_yup, yup_to_landmass};
use crate::error::NavError;
use crate::segment::{SegmentOpts, find_optimized_segment_path};
use crate::types::{
    Extents, OnMeshPoint, OptimizedPathResult, PathResult, PathSegment, RestrictResult, Vec3,
    drop_duplicate_joints, path_length, require_waypoints,
};
use auki_geometry::TriangleMesh;
use landmass::{
    Archipelago, ArchipelagoOptions, FromAgentRadius, Island, PathStep, PermittedAnimationLinks,
    PointSampleDistance3d, SampledPoint, Transform, XYZ,
};
use std::collections::HashMap;

/// Baked navigation mesh. [`Self::bake`] takes [`TriangleMesh`] only.
pub struct NavMesh {
    archipelago: Archipelago<XYZ>,
    agent_radius: f32,
}

impl NavMesh {
    pub fn bake(mesh: &TriangleMesh, profile: &BakeProfile) -> Result<Self, NavError> {
        let BakedNav {
            archipelago_mesh,
            agent_radius,
        } = bake_navmesh(mesh, profile)?;

        let mut archipelago =
            Archipelago::<XYZ>::new(ArchipelagoOptions::from_agent_radius(agent_radius));
        archipelago.add_island(Island::new(Transform::default(), archipelago_mesh));
        archipelago.update(1.0);

        Ok(Self {
            archipelago,
            agent_radius,
        })
    }

    fn sample_distance(extents: Extents) -> PointSampleDistance3d {
        PointSampleDistance3d {
            horizontal_distance: extents.x.max(extents.z),
            distance_above: extents.y,
            distance_below: extents.y,
            vertical_preference_ratio: 2.0,
            animation_link_max_vertical_distance: extents.y,
        }
    }

    fn sample(&self, p: Vec3, extents: Extents) -> Result<SampledPoint<'_, XYZ>, NavError> {
        self.archipelago
            .sample_point(yup_to_landmass(p), &Self::sample_distance(extents))
            .map_err(|_| NavError::OffMesh)
    }

    /// Closest point on the navmesh within `extents` half-size (DSC default `(100,10,100)`).
    pub fn find_closest_point(&self, p: Vec3, extents: Extents) -> OnMeshPoint {
        match self.sample(p, extents) {
            Ok(sampled) => OnMeshPoint {
                original: p,
                adjusted: landmass_to_yup(sampled.point()),
                is_off_mesh: false,
            },
            Err(_) => OnMeshPoint {
                original: p,
                adjusted: p,
                is_off_mesh: true,
            },
        }
    }

    fn path_between_sampled(
        &self,
        start: &SampledPoint<'_, XYZ>,
        end: &SampledPoint<'_, XYZ>,
    ) -> Result<Vec<Vec3>, NavError> {
        let steps = self
            .archipelago
            .find_path(start, end, &HashMap::new(), PermittedAnimationLinks::All)
            .map_err(|_| NavError::NoPath)?;

        let mut points = Vec::new();
        for step in steps {
            match step {
                PathStep::Waypoint(p) => points.push(landmass_to_yup(p)),
                PathStep::AnimationLink {
                    start_point,
                    end_point,
                    ..
                } => {
                    points.push(landmass_to_yup(start_point));
                    points.push(landmass_to_yup(end_point));
                }
            }
        }
        drop_duplicate_joints(&mut points);
        Ok(points)
    }

    /// Snap waypoints then path between consecutive snaps.
    pub fn find_path(&self, waypoints: &[Vec3]) -> Result<PathResult, NavError> {
        require_waypoints(waypoints)?;
        let extents = Extents::default();

        let on_mesh: Vec<OnMeshPoint> = waypoints
            .iter()
            .map(|&p| self.find_closest_point(p, extents))
            .collect();
        if on_mesh.iter().any(|p| p.is_off_mesh) {
            return Err(NavError::OffMesh);
        }

        let mut segments = Vec::new();
        let mut full = Vec::new();

        for window in on_mesh.windows(2) {
            let start = window[0];
            let end = window[1];
            let start_s = self.sample(start.adjusted, extents)?;
            let end_s = self.sample(end.adjusted, extents)?;
            let path = self.path_between_sampled(&start_s, &end_s)?;
            let distance = path_length(&path);

            if full.is_empty() {
                full.extend_from_slice(&path);
            } else if path.len() > 1 {
                full.extend_from_slice(&path[1..]);
            }

            segments.push(PathSegment {
                start,
                end,
                path,
                distance,
            });
        }

        drop_duplicate_joints(&mut full);
        Ok(PathResult {
            full,
            total_distance: segments.iter().map(|s| s.distance).sum(),
            segments,
            waypoints: on_mesh,
        })
    }

    /// Greedy nearest-neighbor over navmesh path length. Optional fixed end waypoint.
    pub fn find_optimized_path(
        &self,
        waypoints: &[Vec3],
        fixed_end: bool,
    ) -> Result<OptimizedPathResult, NavError> {
        require_waypoints(waypoints)?;
        let extents = Extents::default();
        let on_mesh: Vec<OnMeshPoint> = waypoints
            .iter()
            .map(|&p| self.find_closest_point(p, extents))
            .collect();
        if on_mesh.iter().any(|p| p.is_off_mesh) {
            return Err(NavError::OffMesh);
        }

        let n = on_mesh.len();
        let mut order = Vec::with_capacity(n);
        let mut remaining: Vec<usize> = (0..n).collect();
        order.push(remaining.remove(0));

        let fixed_end_idx = if fixed_end && n > 1 {
            Some(remaining.pop().unwrap())
        } else {
            None
        };

        while !remaining.is_empty() {
            let current = order[order.len() - 1];
            let current_s = self.sample(on_mesh[current].adjusted, extents)?;

            let mut best_i = 0;
            let mut best_dist = f32::INFINITY;
            for (ri, &cand) in remaining.iter().enumerate() {
                let end_s = self.sample(on_mesh[cand].adjusted, extents)?;
                let path = self.path_between_sampled(&current_s, &end_s)?;
                let d = path_length(&path);
                if d < best_dist {
                    best_dist = d;
                    best_i = ri;
                }
            }
            order.push(remaining.remove(best_i));
        }

        if let Some(end_idx) = fixed_end_idx {
            order.push(end_idx);
        }

        let ordered: Vec<Vec3> = order.iter().map(|&i| on_mesh[i].adjusted).collect();
        let path = self.find_path(&ordered)?;
        Ok(OptimizedPathResult {
            path,
            waypoint_indices: order,
        })
    }

    /// Closest point; if distance &lt; `min_separation`, push away from target (DSC semantics).
    pub fn restrict(&self, target: Vec3, min_separation: f32) -> RestrictResult {
        let extents = Extents::default();
        let closest = self.find_closest_point(target, extents);
        let mut restricted = closest.adjusted;
        let delta = target - restricted;
        let distance = delta.length();
        let direction = if distance > 1e-8 {
            delta.normalized()
        } else {
            Vec3::default()
        };

        if distance < min_separation && direction.length() > 0.0 {
            let move_by = min_separation - distance;
            restricted = restricted - direction * move_by;
        }

        RestrictResult {
            original: target,
            restricted,
            direction,
        }
    }

    /// Per-OBJ-group principal-axis sampling + NN between reversible segments.
    pub fn find_optimized_segment_path(
        &self,
        mesh: &TriangleMesh,
        opts: SegmentOpts,
    ) -> Result<PathResult, NavError> {
        find_optimized_segment_path(self, mesh, opts)
    }

    pub fn agent_radius(&self) -> f32 {
        self.agent_radius
    }
}
