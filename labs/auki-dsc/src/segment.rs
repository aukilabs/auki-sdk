//! Optimized segment path — port of robot-runner-kit segment_path / DSC findOptimizedSegmentPath.

use crate::error::DscError;
use crate::mesh::TriangleMesh;
use crate::nav::NavMesh;
use crate::types::{Extents, PathResult, Vec3, path_length};

/// Options for [`NavMesh::find_optimized_segment_path`].
#[derive(Clone, Debug)]
pub struct SegmentOpts {
    pub start: Option<Vec3>,
    pub end: Option<Vec3>,
    /// Sample spacing along each group's principal axis (metres). Default 3.0.
    pub interval: f32,
    /// Hold the last OBJ group as fixed end segment.
    pub fixed_end: bool,
    /// Optional axis-aligned filter on XZ.
    pub area: Option<AreaFilter>,
}

impl Default for SegmentOpts {
    fn default() -> Self {
        Self {
            start: None,
            end: None,
            interval: 3.0,
            fixed_end: false,
            area: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct AreaFilter {
    pub x_min: Option<f32>,
    pub x_max: Option<f32>,
    pub z_min: Option<f32>,
    pub z_max: Option<f32>,
}

impl AreaFilter {
    fn contains(&self, p: Vec3) -> bool {
        if self.x_min.is_some_and(|v| p.x < v) {
            return false;
        }
        if self.x_max.is_some_and(|v| p.x > v) {
            return false;
        }
        if self.z_min.is_some_and(|v| p.z < v) {
            return false;
        }
        if self.z_max.is_some_and(|v| p.z > v) {
            return false;
        }
        true
    }
}

struct Segment {
    points: Vec<Vec3>,
}

pub(crate) fn find_optimized_segment_path(
    nav: &NavMesh,
    mesh: &TriangleMesh,
    opts: SegmentOpts,
) -> Result<PathResult, DscError> {
    let mut segments = generate_segments(mesh, opts.area.as_ref(), opts.interval);
    if segments.is_empty() {
        return Err(DscError::NoSegments);
    }

    let extents = Extents::default();
    let mut full_path = Vec::new();

    let fixed_end_seg = if opts.fixed_end && segments.len() > 1 {
        Some(segments.pop().unwrap())
    } else {
        None
    };

    let mut current_exit = if let Some(start) = opts.start {
        let snapped = nav.find_closest_point(start, extents);
        full_path.push(snapped.adjusted);
        snapped.adjusted
    } else {
        let first = segments.remove(0);
        full_path.extend_from_slice(&first.points);
        *first.points.last().unwrap()
    };

    while !segments.is_empty() {
        let mut best_i = 0;
        let mut best_rev = false;
        let mut best_dist = f32::INFINITY;
        for (i, seg) in segments.iter().enumerate() {
            let fwd = path_dist(nav, current_exit, seg.points[0])?;
            let rev = path_dist(nav, current_exit, *seg.points.last().unwrap())?;
            let (d, rev_flag) = if rev < fwd { (rev, true) } else { (fwd, false) };
            if d < best_dist {
                best_dist = d;
                best_i = i;
                best_rev = rev_flag;
            }
        }
        let next = segments.remove(best_i);
        append_segment(nav, &mut full_path, &mut current_exit, &next, best_rev)?;
    }

    if let Some(end_seg) = fixed_end_seg {
        let fwd = path_dist(nav, current_exit, end_seg.points[0])?;
        let rev = path_dist(nav, current_exit, *end_seg.points.last().unwrap())?;
        append_segment(nav, &mut full_path, &mut current_exit, &end_seg, rev < fwd)?;
    }

    if let Some(end) = opts.end {
        let snapped = nav.find_closest_point(end, extents);
        let conn = nav.find_path(&[current_exit, snapped.adjusted])?;
        if conn.full.len() > 1 {
            full_path.extend_from_slice(&conn.full[1..]);
        }
    }

    Ok(PathResult {
        total_distance: path_length(&full_path),
        full: full_path,
        segments: Vec::new(),
        waypoints: Vec::new(),
    })
}

fn path_dist(nav: &NavMesh, a: Vec3, b: Vec3) -> Result<f32, DscError> {
    Ok(nav.find_path(&[a, b])?.total_distance)
}

fn append_segment(
    nav: &NavMesh,
    full_path: &mut Vec<Vec3>,
    current_exit: &mut Vec3,
    seg: &Segment,
    reversed: bool,
) -> Result<(), DscError> {
    let entry = if reversed {
        *seg.points.last().unwrap()
    } else {
        seg.points[0]
    };
    let exit = if reversed {
        seg.points[0]
    } else {
        *seg.points.last().unwrap()
    };

    let connection = nav.find_path(&[*current_exit, entry])?;
    if connection.full.len() > 1 {
        full_path.extend_from_slice(&connection.full[1..]);
    }

    let seg_points: Vec<Vec3> = if reversed {
        seg.points.iter().rev().copied().collect()
    } else {
        seg.points.clone()
    };
    if seg_points.len() > 1 {
        full_path.extend_from_slice(&seg_points[1..]);
    }
    *current_exit = exit;
    Ok(())
}

fn generate_segments(
    mesh: &TriangleMesh,
    area: Option<&AreaFilter>,
    interval: f32,
) -> Vec<Segment> {
    let mut out = Vec::new();
    for group in &mesh.groups {
        let mut verts = mesh.group_vertices(group);
        if let Some(area) = area {
            verts.retain(|v| area.contains(*v));
        }
        if verts.len() < 2 {
            continue;
        }
        let (wp1, wp2) = long_axis_endpoints(&verts);
        let points = interval_waypoints(wp1, wp2, interval)
            .into_iter()
            .map(|(x, z)| Vec3::new(x, 0.0, z))
            .collect();
        out.push(Segment { points });
    }
    out
}

fn long_axis_endpoints(vertices: &[Vec3]) -> ((f32, f32), (f32, f32)) {
    let n = vertices.len() as f32;
    let cx = vertices.iter().map(|v| v.x).sum::<f32>() / n;
    let cz = vertices.iter().map(|v| v.z).sum::<f32>() / n;

    let mut cxx = 0.0;
    let mut czz = 0.0;
    let mut cxz = 0.0;
    for v in vertices {
        let dx = v.x - cx;
        let dz = v.z - cz;
        cxx += dx * dx;
        czz += dz * dz;
        cxz += dx * dz;
    }
    let denom = (n - 1.0).max(1.0);
    cxx /= denom;
    czz /= denom;
    cxz /= denom;

    let trace = cxx + czz;
    let det = cxx * czz - cxz * cxz;
    let discriminant = (trace * trace / 4.0 - det).max(0.0);
    let lambda1 = trace / 2.0 + discriminant.sqrt();

    let (mut ex, mut ez) = if cxz != 0.0 {
        (lambda1 - czz, cxz)
    } else if cxx >= czz {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    let norm = (ex * ex + ez * ez).sqrt().max(1e-8);
    ex /= norm;
    ez /= norm;

    let mut min_proj = f32::INFINITY;
    let mut max_proj = f32::NEG_INFINITY;
    for v in vertices {
        let proj = (v.x - cx) * ex + (v.z - cz) * ez;
        min_proj = min_proj.min(proj);
        max_proj = max_proj.max(proj);
    }

    (
        (cx + min_proj * ex, cz + min_proj * ez),
        (cx + max_proj * ex, cz + max_proj * ez),
    )
}

fn interval_waypoints(wp1: (f32, f32), wp2: (f32, f32), interval: f32) -> Vec<(f32, f32)> {
    let dx = wp2.0 - wp1.0;
    let dz = wp2.1 - wp1.1;
    let distance = (dx * dx + dz * dz).sqrt();
    let segments = (distance / interval).floor() as i32;
    if segments < 1 {
        return vec![wp1, wp2];
    }
    let mut points = Vec::with_capacity(segments as usize + 1);
    for i in 0..=segments {
        let t = i as f32 / segments as f32;
        points.push((wp1.0 + t * dx, wp1.1 + t * dz));
    }
    points
}
