use thiserror::Error;

#[derive(Debug, Error)]
pub enum DscError {
    #[error("empty triangle mesh")]
    EmptyMesh,
    #[error("invalid OBJ: {0}")]
    ObjParse(String),
    #[error("navmesh bake failed: {0}")]
    Bake(String),
    #[error("no walkable navmesh polygons after bake")]
    EmptyNavMesh,
    #[error("point off navmesh")]
    OffMesh,
    #[error("no path between waypoints")]
    NoPath,
    #[error("need at least two waypoints")]
    TooFewWaypoints,
    #[error("no mesh segments found")]
    NoSegments,
    #[error("{0}")]
    Other(String),
}
