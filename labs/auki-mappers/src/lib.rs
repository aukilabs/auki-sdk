//! SDK-native map producers. Mappers consume SDK resources and streams and
//! produce MapUpdates; they deliberately have no robot or ROS dependency.

mod camera;
#[cfg(feature = "runtime")]
mod persistence;

use auki_datatypes::map::{ColorEvidenceDelta, MapUpdate, VoxelChunkUpdate, VoxelDelta};
use auki_datatypes::point_cloud::Data as PointCloudData;
use auki_datatypes::pose::{SpatialTransform, Vec3};
use auki_registry::{PointField, PointFieldDataType, Rangefinder, VoxelColorModel};
use std::collections::BTreeMap;

/// Safety ceiling for one ray from SDK input. It is intentionally far above
/// practical robot sensor ranges while preventing a finite-but-pathological
/// coordinate from turning into an effectively unbounded loop.
const MAX_RAY_STEPS_PER_POINT: usize = 1_000_000;

#[cfg(feature = "runtime")]
mod discovery;
mod frame_alias;
#[cfg(feature = "runtime")]
mod runner;

pub use camera::{CameraCalibrationError, effective_camera_calibration};
#[cfg(feature = "runtime")]
pub use discovery::{
    VoxelMapperInputBindingError, VoxelMapperServiceConfig, VoxelMapperServiceError,
    VoxelMapperSourceQuery, VoxelMapperSourceSelectionError, VoxelMapperSources,
    run_sdk_voxel_mapper,
};
#[cfg(feature = "runtime")]
pub use persistence::VoxelPersistenceConfig;

pub use frame_alias::{FrameAliasError, ValidatedFrameAlias, VoxelMapperMapFrameBinding};

#[cfg(feature = "runtime")]
pub use runner::{
    LocalMapLogSink, MapSinkError, MapUpdateSink, MapperInput, MapperInputBindingError,
    MapperInputError, MapperStream, PoseAlignmentConfig, TimedSdkSample, VoxelMapperRunError,
    VoxelMapperRunReport, VoxelMapperRunner,
};

#[derive(Debug, Clone, Copy)]
pub struct Voxelizer {
    pub voxel_size_m: f64,
    pub chunk_dimension: u32,
    pub color_model: Option<VoxelColorModel>,
}

impl Voxelizer {
    pub fn new(voxel_size_m: f64, chunk_dimension: u32) -> Result<Self, VoxelizerError> {
        if !voxel_size_m.is_finite() || voxel_size_m <= 0.0 || chunk_dimension == 0 {
            return Err(VoxelizerError::InvalidGrid);
        }
        Ok(Self {
            voxel_size_m,
            chunk_dimension,
            color_model: None,
        })
    }

    /// Select the color evidence model declared by the destination Map.
    pub fn with_color_model(mut self, color_model: Option<VoxelColorModel>) -> Self {
        self.color_model = color_model;
        self
    }

    /// Bin points expressed in the map frame into additive occupancy evidence.
    /// A caller obtains these points only by decoding an SDK point-cloud sample.
    pub fn map_points(
        &self,
        points: impl IntoIterator<Item = Vec3>,
        occupancy_delta: f32,
    ) -> Result<MapUpdate, VoxelizerError> {
        self.map_weighted_points(
            points
                .into_iter()
                .map(|point| (point, occupancy_delta, None)),
        )
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
        self.map_sensor_samples(
            points.into_iter().map(|position| DecodedPoint {
                position,
                linear_rgb: None,
            }),
            sensor_to_map,
            free_delta,
            occupied_delta,
        )
    }

    fn map_sensor_samples(
        &self,
        points: impl IntoIterator<Item = DecodedPoint>,
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
            let DecodedPoint {
                position,
                linear_rgb,
            } = point;
            if ![position.x, position.y, position.z]
                .into_iter()
                .all(f64::is_finite)
            {
                return Err(VoxelizerError::NonFinitePoint);
            }
            let end = transform.transform_point(position);
            // Walk crossed grid cells, rather than sampling at approximate metric
            // intervals (which can skip cells and can clear the hit's own cell).
            let start = [origin.x, origin.y, origin.z];
            let finish = [end.x, end.y, end.z];
            let mut cell = [
                self.grid_index(start[0])?,
                self.grid_index(start[1])?,
                self.grid_index(start[2])?,
            ];
            let target = [
                self.grid_index(finish[0])?,
                self.grid_index(finish[1])?,
                self.grid_index(finish[2])?,
            ];
            let maximum_steps: u64 = (0..3)
                .map(|i| (i64::from(target[i]) - i64::from(cell[i])).unsigned_abs())
                .sum();
            if maximum_steps > MAX_RAY_STEPS_PER_POINT as u64 {
                return Err(VoxelizerError::RayTooLong {
                    maximum: MAX_RAY_STEPS_PER_POINT,
                });
            }
            let mut step = [0i32; 3];
            let mut next = [f64::INFINITY; 3];
            let mut delta = [f64::INFINITY; 3];
            for i in 0..3 {
                let direction = finish[i] - start[i];
                if direction != 0. && cell[i] != target[i] {
                    step[i] = if direction > 0. { 1 } else { -1 };
                    let boundary = (f64::from(cell[i]) + if step[i] > 0 { 1. } else { 0. })
                        * self.voxel_size_m;
                    next[i] = (boundary - start[i]) / direction;
                    delta[i] = self.voxel_size_m / direction.abs();
                }
            }
            while cell != target {
                all.push((
                    Vec3 {
                        x: (f64::from(cell[0]) + 0.5) * self.voxel_size_m,
                        y: (f64::from(cell[1]) + 0.5) * self.voxel_size_m,
                        z: (f64::from(cell[2]) + 0.5) * self.voxel_size_m,
                    },
                    free_delta,
                    None,
                ));
                // Advance all axes at an exact edge/corner crossing together.
                let crossing = next.iter().copied().fold(f64::INFINITY, f64::min);
                for i in 0..3 {
                    if next[i] <= crossing && cell[i] != target[i] {
                        cell[i] += step[i];
                        next[i] += delta[i];
                    }
                    if cell[i] == target[i] {
                        next[i] = f64::INFINITY;
                    }
                }
            }
            all.push((end, occupied_delta, linear_rgb));
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
        let mut points = decode_xyz_rgb(&payload.data, layout)?;
        if self.color_model.is_none() {
            for point in &mut points {
                point.linear_rgb = None;
            }
        }
        self.map_sensor_samples(points, sensor_to_map, free_delta, occupied_delta)
    }

    fn map_weighted_points(
        &self,
        points: impl IntoIterator<Item = (Vec3, f32, Option<[f32; 3]>)>,
    ) -> Result<MapUpdate, VoxelizerError> {
        let d = self.chunk_dimension as i32;
        let mut chunks: BTreeMap<(i32, i32, i32), Vec<VoxelDelta>> = BTreeMap::new();
        for (point, occupancy_delta, linear_rgb) in points {
            if ![point.x, point.y, point.z].into_iter().all(f64::is_finite) {
                return Err(VoxelizerError::NonFinitePoint);
            }
            if !occupancy_delta.is_finite() {
                return Err(VoxelizerError::NonFiniteEvidence);
            }
            if linear_rgb.is_some_and(|color| {
                !color
                    .into_iter()
                    .all(|channel| channel.is_finite() && (0.0..=1.0).contains(&channel))
            }) {
                return Err(VoxelizerError::InvalidColor);
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
                color: linear_rgb.map(|[red, green, blue]| ColorEvidenceDelta {
                    red_sum_delta: red,
                    green_sum_delta: green,
                    blue_sum_delta: blue,
                    weight_delta: 1.0,
                }),
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
            checkpoint: None,
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
    #[error("point-cloud color fields must be uint8 r/g/b or packed float32 rgb")]
    UnsupportedColorLayout,
    #[error("point-cloud payload is truncated")]
    TruncatedPoint,
    #[error("point-cloud coordinates must be finite")]
    NonFinitePoint,
    #[error("occupancy evidence must be finite")]
    NonFiniteEvidence,
    #[error("point-cloud color must contain finite linear RGB channels in [0, 1]")]
    InvalidColor,
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
    Ok(decode_xyz_rgb(data, layout)?
        .into_iter()
        .map(|point| point.position)
        .collect())
}

/// One decoded SDK point-cloud sample. Color is linear-light RGB when the
/// Rangefinder declares a supported source color layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecodedPoint {
    pub position: Vec3,
    pub linear_rgb: Option<[f32; 3]>,
}

/// Decode XYZ and optional source color using only the pinned SDK Rangefinder
/// contract. Supported color layouts match common PointCloud2 producers:
/// separate scalar uint8 `r`/`g`/`b`, or packed scalar float32 `rgb`.
pub fn decode_xyz_rgb(
    data: &[u8],
    layout: &Rangefinder,
) -> Result<Vec<DecodedPoint>, VoxelizerError> {
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
    let find = |name: &str| layout.fields.iter().find(|field| field.name == name);
    let separate = match (find("r"), find("g"), find("b")) {
        (None, None, None) => None,
        (Some(red), Some(green), Some(blue))
            if [red, green, blue]
                .into_iter()
                .all(|field| field.datatype == PointFieldDataType::Uint8 && field.count == 1) =>
        {
            Some((red, green, blue))
        }
        _ => return Err(VoxelizerError::UnsupportedColorLayout),
    };
    let packed = find("rgb");
    if packed.is_some_and(|field| field.datatype != PointFieldDataType::Float32 || field.count != 1)
    {
        return Err(VoxelizerError::UnsupportedColorLayout);
    }
    if layout.point_step == 0 || !data.len().is_multiple_of(layout.point_step as usize) {
        return Err(VoxelizerError::TruncatedPoint);
    }
    let color = |point: &[u8]| -> Result<Option<[f32; 3]>, VoxelizerError> {
        let srgb = if let Some((red, green, blue)) = separate {
            let channel = |field: &PointField| {
                point
                    .get(field.offset as usize)
                    .copied()
                    .ok_or(VoxelizerError::TruncatedPoint)
            };
            Some([channel(red)?, channel(green)?, channel(blue)?])
        } else if let Some(field) = packed {
            let start = field.offset as usize;
            let bytes: [u8; 4] = point
                .get(start..start + 4)
                .ok_or(VoxelizerError::TruncatedPoint)?
                .try_into()
                .unwrap();
            let bits = if layout.is_bigendian {
                u32::from_be_bytes(bytes)
            } else {
                u32::from_le_bytes(bytes)
            };
            Some([
                ((bits >> 16) & 0xff) as u8,
                ((bits >> 8) & 0xff) as u8,
                (bits & 0xff) as u8,
            ])
        } else {
            None
        };
        Ok(srgb.map(|channels| channels.map(|channel| srgb_to_linear(channel as f32 / 255.0))))
    };
    data.chunks_exact(layout.point_step as usize)
        .map(|point| {
            Ok(DecodedPoint {
                position: Vec3 {
                    x: read(x, point)?,
                    y: read(y, point)?,
                    z: read(z, point)?,
                },
                linear_rgb: color(point)?,
            })
        })
        .collect()
}

fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
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

    fn identity_pose() -> SpatialTransform {
        SpatialTransform {
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
        }
    }

    fn xyz_fields() -> Vec<PointField> {
        [("x", 0), ("y", 4), ("z", 8)]
            .into_iter()
            .map(|(name, offset)| PointField {
                name: name.into(),
                offset,
                datatype: PointFieldDataType::Float32,
                count: 1,
            })
            .collect()
    }

    fn color_layout(fields: Vec<PointField>, point_step: u32) -> Rangefinder {
        Rangefinder {
            r#type: "point_cloud".into(),
            fields,
            point_step,
            is_bigendian: false,
            frame_rate_hz: 30,
            frame: RegistryRef {
                peer_id: "bracketbot".into(),
                id: "head".into(),
                hash: "frame-hash".into(),
            },
        }
    }

    #[test]
    fn decodes_separate_and_packed_rgb_to_linear_light() {
        let mut separate_fields = xyz_fields();
        separate_fields.extend([("r", 12), ("g", 13), ("b", 14)].map(|(name, offset)| {
            PointField {
                name: name.into(),
                offset,
                datatype: PointFieldDataType::Uint8,
                count: 1,
            }
        }));
        let mut separate_bytes = Vec::new();
        separate_bytes.extend_from_slice(&1.5_f32.to_le_bytes());
        separate_bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        separate_bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        separate_bytes.extend_from_slice(&[255, 128, 0]);
        let separate =
            decode_xyz_rgb(&separate_bytes, &color_layout(separate_fields, 15)).unwrap()[0];
        assert_eq!(separate.position.x, 1.5);
        let [red, green, blue] = separate.linear_rgb.unwrap();
        assert_eq!(red, 1.0);
        assert!((green - 0.215_860_53).abs() < 1e-6);
        assert_eq!(blue, 0.0);

        let mut packed_fields = xyz_fields();
        packed_fields.push(PointField {
            name: "rgb".into(),
            offset: 12,
            datatype: PointFieldDataType::Float32,
            count: 1,
        });
        let mut packed_bytes = separate_bytes[..12].to_vec();
        packed_bytes.extend_from_slice(&0x00ff_8000_u32.to_le_bytes());
        let packed = decode_xyz_rgb(&packed_bytes, &color_layout(packed_fields, 16)).unwrap()[0];
        assert_eq!(packed.linear_rgb, separate.linear_rgb);
    }

    #[test]
    fn colored_map_writes_endpoint_color_while_occupancy_only_map_drops_it() {
        let mut fields = xyz_fields();
        fields.extend(
            [("r", 12), ("g", 13), ("b", 14)].map(|(name, offset)| PointField {
                name: name.into(),
                offset,
                datatype: PointFieldDataType::Uint8,
                count: 1,
            }),
        );
        let layout = color_layout(fields, 15);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&1.5_f32.to_le_bytes());
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        bytes.extend_from_slice(&0.0_f32.to_le_bytes());
        bytes.extend_from_slice(&[255, 0, 0]);
        let payload = PointCloudData { data: bytes };

        let colored = Voxelizer::new(1.0, 64)
            .unwrap()
            .with_color_model(Some(VoxelColorModel::AdditiveLinearRgbEvidence))
            .map_point_cloud(&payload, &layout, &identity_pose(), -0.2, 0.8)
            .unwrap();
        let voxels = colored
            .voxel_chunks
            .iter()
            .flat_map(|chunk| &chunk.voxels)
            .collect::<Vec<_>>();
        assert_eq!(
            voxels.iter().filter(|voxel| voxel.color.is_some()).count(),
            1
        );
        assert_eq!(
            voxels.last().unwrap().color.as_ref().unwrap().red_sum_delta,
            1.0
        );

        let occupancy_only = Voxelizer::new(1.0, 64)
            .unwrap()
            .map_point_cloud(&payload, &layout, &identity_pose(), -0.2, 0.8)
            .unwrap();
        assert!(
            occupancy_only
                .voxel_chunks
                .iter()
                .flat_map(|chunk| &chunk.voxels)
                .all(|voxel| voxel.color.is_none())
        );
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
    #[test]
    fn diagonal_ray_visits_crossed_cells_once_and_never_clears_the_endpoint() {
        let mut pose = identity_pose();
        pose.translation = Some(Vec3 {
            x: 0.9,
            y: 0.1,
            z: 0.1,
        });
        let update = Voxelizer::new(1., 16)
            .unwrap()
            .map_sensor_rays(
                [Vec3 {
                    x: 1.2,
                    y: 1.8,
                    z: 0.,
                }],
                &pose,
                -1.,
                1.,
            )
            .unwrap();
        let cells: Vec<_> = update
            .voxel_chunks
            .iter()
            .flat_map(|c| c.voxels.iter().map(|v| (v.x, v.y, v.z, v.occupancy_delta)))
            .collect();
        assert_eq!(
            cells,
            vec![
                (0, 0, 0, -1.),
                (1, 0, 0, -1.),
                (1, 1, 0, -1.),
                (2, 1, 0, 1.)
            ]
        );
        let same = Voxelizer::new(1., 16)
            .unwrap()
            .map_sensor_rays(
                [Vec3 {
                    x: 0.01,
                    y: 0.01,
                    z: 0.,
                }],
                &pose,
                -1.,
                1.,
            )
            .unwrap();
        assert_eq!(same.voxel_chunks[0].voxels.len(), 1);
        assert_eq!(same.voxel_chunks[0].voxels[0].occupancy_delta, 1.);
    }
    #[test]
    fn ray_traversal_handles_negative_cells_and_exact_corner_crossings() {
        let mut pose = identity_pose();
        pose.translation = Some(Vec3 {
            x: 0.1,
            y: 0.1,
            z: 0.1,
        });
        let voxelizer = Voxelizer::new(1., 16).unwrap();
        let negative = voxelizer
            .map_sensor_rays(
                [Vec3 {
                    x: -2.2,
                    y: 0.,
                    z: 0.,
                }],
                &pose,
                -1.,
                1.,
            )
            .unwrap();
        let cells: BTreeMap<_, _> = negative
            .voxel_chunks
            .iter()
            .flat_map(|c| {
                c.voxels
                    .iter()
                    .map(move |v| (c.chunk_x * 16 + v.x as i32, v.occupancy_delta))
            })
            .collect();
        assert_eq!(
            cells,
            BTreeMap::from([(-3, 1.), (-2, -1.), (-1, -1.), (0, -1.)])
        );
        let diagonal = voxelizer
            .map_sensor_rays(
                [Vec3 {
                    x: 2.,
                    y: 2.,
                    z: 2.,
                }],
                &pose,
                -1.,
                1.,
            )
            .unwrap();
        let cells: Vec<_> = diagonal
            .voxel_chunks
            .iter()
            .flat_map(|c| c.voxels.iter().map(|v| (v.x, v.y, v.z, v.occupancy_delta)))
            .collect();
        assert_eq!(cells, vec![(0, 0, 0, -1.), (1, 1, 1, -1.), (2, 2, 2, 1.)]);
    }
}
