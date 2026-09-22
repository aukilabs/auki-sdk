//! Recast bake + landmass path / snap / optimize / restrict.
//!
//! Input geometry is [`auki_geometry::TriangleMesh`]. Raycast lives on
//! that mesh. Occupancy slice is [`auki-raster`](../auki-raster). Domain
//! assets (`navmesh_v1`, `occlusionmesh_v1`) are consumer loaders.
//!
//! Coordinates are metres, **Y-up** (OpenGL). Landmass is Z-up; conversion
//! happens only at the bake/query boundary.

mod bake;
mod error;
mod nav;
mod segment;
mod types;

pub use bake::BakeProfile;
pub use error::NavError;
pub use nav::NavMesh;
pub use segment::{AreaFilter, SegmentOpts};
pub use types::{
    Extents, OnMeshPoint, OptimizedPathResult, PathResult, PathSegment, RestrictResult, Vec3,
};
