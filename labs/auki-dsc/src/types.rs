//! Shared value types. Field names follow DSC JSON (snake_cased).

use crate::error::DscError;

/// Metres, Y-up (domain OBJ / OpenGL). No silent frame conversion.
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

    pub fn distance(self, other: Self) -> f32 {
        (self - other).length()
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

/// Half-extents for closest-point queries. DSC default: `(100, 10, 100)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extents {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Default for Extents {
    fn default() -> Self {
        Self {
            x: 100.0,
            y: 10.0,
            z: 100.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OnMeshPoint {
    pub original: Vec3,
    pub adjusted: Vec3,
    pub is_off_mesh: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathSegment {
    pub start: OnMeshPoint,
    pub end: OnMeshPoint,
    pub path: Vec<Vec3>,
    pub distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathResult {
    pub full: Vec<Vec3>,
    pub segments: Vec<PathSegment>,
    pub total_distance: f32,
    pub waypoints: Vec<OnMeshPoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OptimizedPathResult {
    pub path: PathResult,
    pub waypoint_indices: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RestrictResult {
    pub original: Vec3,
    pub restricted: Vec3,
    pub direction: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub distance: f32,
    pub normal: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrossSection {
    pub png: Vec<u8>,
    pub map_yaml: String,
    pub width: u32,
    pub height: u32,
    pub resolution: f32,
    pub origin_x: f32,
    pub origin_z: f32,
}

pub(crate) fn path_length(points: &[Vec3]) -> f32 {
    points.windows(2).map(|w| w[0].distance(w[1])).sum()
}

pub(crate) fn drop_duplicate_joints(points: &mut Vec<Vec3>) {
    if points.is_empty() {
        return;
    }
    let mut out = Vec::with_capacity(points.len());
    out.push(points[0]);
    for p in points.iter().skip(1) {
        if out.last().map(|last| last.distance(*p) > 1e-5).unwrap_or(true) {
            out.push(*p);
        }
    }
    *points = out;
}

pub(crate) fn require_waypoints(waypoints: &[Vec3]) -> Result<(), DscError> {
    if waypoints.len() < 2 {
        Err(DscError::TooFewWaypoints)
    } else {
        Ok(())
    }
}
