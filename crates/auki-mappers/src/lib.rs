//! SDK-native map producers. Mappers consume SDK resources and streams and
//! produce MapUpdates; they deliberately have no robot or ROS dependency.

use auki_datatypes::map::{MapUpdate, VoxelChunkUpdate, VoxelDelta};
use auki_datatypes::point_cloud::Data as PointCloudData;
use auki_datatypes::pose::{SpatialTransform, Vec3};
use auki_registry::{PointField, PointFieldDataType, Rangefinder};
use std::collections::BTreeMap;

/// Safety ceiling for one ray from SDK input. It is intentionally far above
/// practical robot sensor ranges while preventing a finite-but-pathological
/// coordinate from turning into an effectively unbounded loop.
const MAX_RAY_STEPS_PER_POINT: usize = 1_000_000;

mod discovery;
mod runner;

pub use discovery::{
    VoxelMapperInputBindingError, VoxelMapperServiceConfig, VoxelMapperServiceError,
    VoxelMapperSourceQuery, VoxelMapperSourceSelectionError, VoxelMapperSources,
    run_sdk_voxel_mapper,
};

pub use runner::{
    LocalMapLogSink, MapSinkError, MapUpdateSink, MapperInput, MapperInputBindingError,
    MapperInputError, MapperStream, PoseAlignmentConfig, TimedSdkSample, VoxelMapperRunError,
    VoxelMapperRunReport, VoxelMapperRunner,
};

#[derive(Debug, Clone, Copy)]
pub struct Voxelizer {
    pub voxel_size_m: f64,
    pub chunk_dimension: u32,
}

impl Voxelizer {
    pub fn new(voxel_size_m: f64, chunk_dimension: u32) -> Result<Self, VoxelizerError> {
        if !voxel_size_m.is_finite() || voxel_size_m <= 0.0 || chunk_dimension == 0 {
            return Err(VoxelizerError::InvalidGrid);
        }
        Ok(Self {
            voxel_size_m,
            chunk_dimension,
        })
    }

    /// Bin points expressed in the map frame into additive occupancy evidence.
    /// A caller obtains these points only by decoding an SDK point-cloud sample.
    pub fn map_points(
        &self,
        points: impl IntoIterator<Item = Vec3>,
        occupancy_delta: f32,
    ) -> Result<MapUpdate, VoxelizerError> {
        self.map_weighted_points(points.into_iter().map(|point| (point, occupancy_delta)))
    }

    /// Transform SDK points from a sensor frame into the map frame before binning.
    pub fn map_sensor_points(
        &self,
        points: impl IntoIterator<Item = Vec3>,
        sensor_to_map: &SpatialTransform,
        occupancy_delta: f32,
    ) -> Result<MapUpdate, VoxelizerError> {
        let transform = ValidTransform::new(sensor_to_map)?;
        self.map_points(
            points
                .into_iter()
                .map(|point| transform.transform_point(point)),
            occupancy_delta,
        )
    }

    /// Produce free-space evidence along each sensor ray and occupied evidence
    /// at its endpoint. Both are additive, so independently produced updates
    /// commute when they reach a Map Log in different orders.
    pub fn map_sensor_rays(
        &self,
        points: impl IntoIterator<Item = Vec3>,
        sensor_to_map: &SpatialTransform,
        free_delta: f32,
        occupied_delta: f32,
    ) -> Result<MapUpdate, VoxelizerError> {
        if !free_delta.is_finite() || !occupied_delta.is_finite() {
            return Err(VoxelizerError::NonFiniteEvidence);
        }
        if free_delta >= 0.0 || occupied_delta <= 0.0 {
            return Err(VoxelizerError::InvalidEvidencePolarity);
        }
        let transform = ValidTransform::new(sensor_to_map)?;
        let origin = transform.translation;
        let mut all = Vec::new();
        for point in points {
            if ![point.x, point.y, point.z].into_iter().all(f64::is_finite) {
                return Err(VoxelizerError::NonFinitePoint);
            }
            let end = transform.transform_point(point);
            let dx = end.x - origin.x;
            let dy = end.y - origin.y;
            let dz = end.z - origin.z;
            let distance = dx.hypot(dy).hypot(dz);
            let ray_steps = (distance / self.voxel_size_m).floor();
            if !ray_steps.is_finite() || ray_steps > MAX_RAY_STEPS_PER_POINT as f64 {
                return Err(VoxelizerError::RayTooLong {
                    maximum: MAX_RAY_STEPS_PER_POINT,
                });
            }
            let steps = ray_steps as usize;
            for step in 0..steps {
                let f = step as f64 / steps.max(1) as f64;
                all.push((
                    Vec3 {
                        x: origin.x + dx * f,
                        y: origin.y + dy * f,
                        z: origin.z + dz * f,
                    },
                    free_delta,
                ));
            }
            all.push((end, occupied_delta));
        }
        self.map_weighted_points(all)
    }

    /// The SDK-facing voxelizer entrypoint. `payload` and `layout` come from
    /// a Sensor Log and its pinned Rangefinder registry entry; `sensor_to_map`
    /// comes from the SDK pose/frame path.
    pub fn map_point_cloud(
        &self,
        payload: &PointCloudData,
        layout: &Rangefinder,
        sensor_to_map: &SpatialTransform,
        free_delta: f32,
        occupied_delta: f32,
    ) -> Result<MapUpdate, VoxelizerError> {
        self.map_sensor_rays(
            decode_xyz(&payload.data, layout)?,
            sensor_to_map,
            free_delta,
            occupied_delta,
        )
    }

    fn map_weighted_points(
        &self,
        points: impl IntoIterator<Item = (Vec3, f32)>,
    ) -> Result<MapUpdate, VoxelizerError> {
        let d = self.chunk_dimension as i32;
        let mut chunks: BTreeMap<(i32, i32, i32), Vec<VoxelDelta>> = BTreeMap::new();
        for (point, occupancy_delta) in points {
            if ![point.x, point.y, point.z].into_iter().all(f64::is_finite) {
                return Err(VoxelizerError::NonFinitePoint);
            }
            if !occupancy_delta.is_finite() {
                return Err(VoxelizerError::NonFiniteEvidence);
            }
            let x = self.grid_index(point.x)?;
            let y = self.grid_index(point.y)?;
            let z = self.grid_index(point.z)?;
            let (cx, lx) = (x.div_euclid(d), x.rem_euclid(d) as u32);
            let (cy, ly) = (y.div_euclid(d), y.rem_euclid(d) as u32);
            let (cz, lz) = (z.div_euclid(d), z.rem_euclid(d) as u32);
            chunks.entry((cx, cy, cz)).or_default().push(VoxelDelta {
                x: lx,
                y: ly,
                z: lz,
                occupancy_delta,
                semantics: vec![],
            });
        }
        Ok(MapUpdate {
            voxel_chunks: chunks
                .into_iter()
                .map(|((chunk_x, chunk_y, chunk_z), voxels)| VoxelChunkUpdate {
                    chunk_x,
                    chunk_y,
                    chunk_z,
                    voxels,
                })
                .collect(),
        })
    }

    fn grid_index(&self, coordinate_m: f64) -> Result<i32, VoxelizerError> {
        let index = (coordinate_m / self.voxel_size_m).floor();
        if index < f64::from(i32::MIN) || index > f64::from(i32::MAX) {
            return Err(VoxelizerError::CoordinateOutOfRange);
        }
        Ok(index as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VoxelizerError {
    #[error("invalid voxel grid")]
    InvalidGrid,
    #[error("point cloud is missing {0} coordinate")]
    MissingCoordinate(&'static str),
    #[error("point-cloud coordinates must be scalar float32 fields")]
    UnsupportedCoordinateLayout,
    #[error("point-cloud payload is truncated")]
    TruncatedPoint,
    #[error("point-cloud coordinates must be finite")]
    NonFinitePoint,
    #[error("occupancy evidence must be finite")]
    NonFiniteEvidence,
    #[error("free-space evidence must be negative and occupied evidence must be positive")]
    InvalidEvidencePolarity,
    #[error("point-cloud coordinate lies outside the supported signed voxel grid")]
    CoordinateOutOfRange,
    #[error("sensor ray exceeds the safety limit of {maximum} voxel steps")]
    RayTooLong { maximum: usize },
    #[error("SDK pose must contain a finite translation and non-zero finite quaternion")]
    InvalidPose,
}

/// Decode normalized SDK point-cloud payload bytes using the Rangefinder
/// registry contract. This is intentionally not a ROS or robot API.
pub fn decode_xyz(data: &[u8], layout: &Rangefinder) -> Result<Vec<Vec3>, VoxelizerError> {
    let field = |name: &'static str| -> Result<&PointField, VoxelizerError> {
        layout
            .fields
            .iter()
            .find(|field| field.name == name)
            .ok_or(VoxelizerError::MissingCoordinate(name))
    };
    let (x, y, z) = (field("x")?, field("y")?, field("z")?);
    let read = |field: &PointField, point: &[u8]| -> Result<f64, VoxelizerError> {
        if field.datatype != PointFieldDataType::Float32 || field.count != 1 {
            return Err(VoxelizerError::UnsupportedCoordinateLayout);
        }
        let start = field.offset as usize;
        let bytes: [u8; 4] = point
            .get(start..start + 4)
            .ok_or(VoxelizerError::TruncatedPoint)?
            .try_into()
            .unwrap();
        Ok(if layout.is_bigendian {
            f32::from_be_bytes(bytes)
        } else {
            f32::from_le_bytes(bytes)
        } as f64)
    };
    if layout.point_step == 0 || !data.len().is_multiple_of(layout.point_step as usize) {
        return Err(VoxelizerError::TruncatedPoint);
    }
    data.chunks_exact(layout.point_step as usize)
        .map(|point| {
            Ok(Vec3 {
                x: read(x, point)?,
                y: read(y, point)?,
                z: read(z, point)?,
            })
        })
        .collect()
}

struct ValidTransform {
    translation: Vec3,
    orientation: auki_datatypes::pose::Quat,
}

impl ValidTransform {
    fn new(transform: &SpatialTransform) -> Result<Self, VoxelizerError> {
        let translation = transform
            .translation
            .as_ref()
            .ok_or(VoxelizerError::InvalidPose)?;
        let orientation = transform
            .orientation
            .as_ref()
            .ok_or(VoxelizerError::InvalidPose)?;
        if ![translation.x, translation.y, translation.z]
            .into_iter()
            .all(f64::is_finite)
        {
            return Err(VoxelizerError::InvalidPose);
        }
        let norm = (orientation.x * orientation.x
            + orientation.y * orientation.y
            + orientation.z * orientation.z
            + orientation.w * orientation.w)
            .sqrt();
        if !norm.is_finite() || norm <= f64::EPSILON {
            return Err(VoxelizerError::InvalidPose);
        }
        Ok(Self {
            translation: *translation,
            orientation: auki_datatypes::pose::Quat {
                x: orientation.x / norm,
                y: orientation.y / norm,
                z: orientation.z / norm,
                w: orientation.w / norm,
            },
        })
    }

    fn transform_point(&self, p: Vec3) -> Vec3 {
        let q = &self.orientation;
        let tr = &self.translation;
        let (x, y, z, w) = (q.x, q.y, q.z, q.w);
        let ix = w * p.x + y * p.z - z * p.y;
        let iy = w * p.y + z * p.x - x * p.z;
        let iz = w * p.z + x * p.y - y * p.x;
        let iw = -x * p.x - y * p.y - z * p.z;
        Vec3 {
            x: ix * w + iw * -x + iy * -z - iz * -y + tr.x,
            y: iy * w + iw * -y + iz * -x - ix * -z + tr.y,
            z: iz * w + iw * -z + ix * -y - iy * -x + tr.z,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_registry::{PointField, RegistryRef};
    #[test]
    fn bins_negative_points_into_signed_chunks() {
        let v = Voxelizer::new(1.0, 64).unwrap();
        let u = v
            .map_points(
                [Vec3 {
                    x: -1.0,
                    y: 0.0,
                    z: 64.0,
                }],
                1.0,
            )
            .unwrap();
        assert_eq!(u.voxel_chunks[0].chunk_x, -1);
        assert_eq!(u.voxel_chunks[0].voxels[0].x, 63);
    }

    #[test]
    fn decodes_normalized_sdk_xyz_payload() {
        let layout = Rangefinder {
            r#type: "point_cloud".into(),
            fields: vec![
                PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "y".into(),
                    offset: 4,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
                PointField {
                    name: "z".into(),
                    offset: 8,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                },
            ],
            point_step: 12,
            is_bigendian: false,
            frame_rate_hz: 30,
            frame: RegistryRef {
                peer_id: "galbot".into(),
                id: "lidar".into(),
                hash: "hash".into(),
            },
        };
        let bytes = [
            1.5_f32.to_le_bytes(),
            (-2.0_f32).to_le_bytes(),
            3.25_f32.to_le_bytes(),
        ]
        .concat();
        assert_eq!(decode_xyz(&bytes, &layout).unwrap()[0].y, -2.0);
    }

    #[test]
    fn rejects_non_finite_points_and_invalid_poses_without_panicking() {
        let voxelizer = Voxelizer::new(1.0, 64).unwrap();
        assert_eq!(
            voxelizer.map_points(
                [Vec3 {
                    x: f64::NAN,
                    y: 0.0,
                    z: 0.0,
                }],
                0.8,
            ),
            Err(VoxelizerError::NonFinitePoint)
        );
        assert_eq!(
            voxelizer.map_sensor_rays(
                [Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                }],
                &SpatialTransform {
                    translation: None,
                    orientation: None,
                },
                -0.2,
                0.8,
            ),
            Err(VoxelizerError::InvalidPose)
        );
        assert_eq!(
            voxelizer.map_sensor_rays(
                [Vec3 {
                    x: 1.0,
                    y: 0.0,
                    z: 0.0,
                }],
                &SpatialTransform {
                    translation: Some(Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    }),
                    orientation: Some(auki_datatypes::pose::Quat {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    }),
                },
                0.2,
                0.8,
            ),
            Err(VoxelizerError::InvalidEvidencePolarity)
        );
        assert_eq!(
            voxelizer.map_points(
                [Vec3 {
                    x: f64::from(i32::MAX) + 1.0,
                    y: 0.0,
                    z: 0.0,
                }],
                0.8,
            ),
            Err(VoxelizerError::CoordinateOutOfRange)
        );
        assert_eq!(
            voxelizer.map_sensor_rays(
                [Vec3 {
                    x: 1_000_001.0,
                    y: 0.0,
                    z: 0.0,
                }],
                &SpatialTransform {
                    translation: Some(Vec3 {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    }),
                    orientation: Some(auki_datatypes::pose::Quat {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    }),
                },
                -0.2,
                0.8,
            ),
            Err(VoxelizerError::RayTooLong {
                maximum: MAX_RAY_STEPS_PER_POINT
            })
        );
    }
}
