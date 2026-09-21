//! Local domain spatial compute: Recast bake + landmass queries + occlusion.
//!
//! Coordinates are metres, **Y-up** (domain OBJ / OpenGL). Landmass is Z-up;
//! conversion happens only at the bake/query boundary inside this crate.

mod bake;
mod error;
mod mesh;
mod nav;
mod occlusion;
mod segment;
mod types;

pub use bake::BakeProfile;
pub use error::DscError;
pub use mesh::{MeshGroup, TriangleMesh, parse_obj};
pub use nav::NavMesh;
pub use occlusion::OcclusionMesh;
pub use segment::{AreaFilter, SegmentOpts};
pub use types::{
    CrossSection, Extents, OnMeshPoint, OptimizedPathResult, PathResult, PathSegment, RayHit,
    RestrictResult, Vec3,
};
