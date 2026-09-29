//! Explicit convention conversion for the root-and-Portal scenegraph.
//!
//! Conversion changes coordinates, not physical placement. It keeps the physical
//! origin and requires an explicit axis rotation; up-axis metadata cannot choose
//! horizontal heading. Source and target conventions must be representable by
//! `MapFrame` (currently right-handed Y/Z-up with positive unit scales).

use crate::{MapFrame, MapSnapshot, RigidTransform, SceneError, Scenegraph, UpAxis};
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::{compose_spatial_transforms, spatial_transform_to_matrix4};
use serde::{Deserialize, Serialize};

/// Column-vector coordinate conversion: `p_to = matrix * [p_from, 1]`.
/// Unlike a rigid pose this matrix includes the change in length units.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FramedConventionMatrix {
    pub from_frame_id: String,
    pub to_frame_id: String,
    pub matrix: [[f64; 4]; 4],
}

/// Validated, immutable conversion between two explicitly declared map frames.
/// No physical translation is introduced and no alignment with another map is inferred.
#[derive(Clone, Debug)]
pub struct MapConventionConversion {
    source: MapFrame,
    target: MapFrame,
    rotation: SpatialTransform,
    scale: f64,
}

impl MapConventionConversion {
    /// Derive a conversion from enumerated conventions recorded on both frames.
    /// No convention is inferred for legacy frames that only declare an up axis.
    pub fn between_named_frames(source: MapFrame, target: MapFrame) -> Result<Self, SceneError> {
        source.validate()?;
        target.validate()?;
        let from = source.convention.ok_or(SceneError::Invalid(
            "source frame has no named convention; supply an explicit declaration or conversion",
        ))?;
        let to = target
            .convention
            .ok_or(SceneError::Invalid("target frame has no named convention"))?;
        let axes = auki_geometry::axis_convention_matrix(&from.axes(), &to.axes())
            .map_err(geometry_error)?;
        let matrix = [
            [axes[0][0], axes[0][1], axes[0][2], 0.],
            [axes[1][0], axes[1][1], axes[1][2], 0.],
            [axes[2][0], axes[2][1], axes[2][2], 0.],
            [0., 0., 0., 1.],
        ];
        let q = auki_geometry::spatial_transform_from_matrix4(matrix)
            .map_err(geometry_error)?
            .orientation
            .expect("matrix decomposition supplies orientation");
        let rotation = RigidTransform {
            from_frame_id: source.id.clone(),
            to_frame_id: target.id.clone(),
            translation: [0.; 3],
            rotation_wxyz: [q.w, q.x, q.y, q.z],
        };
        Self::new(source, target, rotation)
    }

    /// Derive the axis rotation from complete, caller-supplied registry frame
    /// conventions. IDs, handedness, up axes and units must match each map frame.
    /// Axis declarations specify convention, not alignment between physical origins.
    pub fn from_registry_frames(
        source: MapFrame,
        target: MapFrame,
        source_convention: &auki_registry::FrameRegistryEntry,
        target_convention: &auki_registry::FrameRegistryEntry,
    ) -> Result<Self, SceneError> {
        for (frame, convention) in [(&source, source_convention), (&target, target_convention)] {
            let up = match frame.up_axis {
                UpAxis::Y => convention.axes.y,
                UpAxis::Z => convention.axes.z,
            };
            if frame.convention.is_some_and(|named| {
                named.axes() != convention.axes || named.handedness() != convention.handedness
            }) || frame.id != convention.frame_id
                || convention.handedness != auki_registry::Handedness::Right
                || up != auki_registry::AxisDirection::Up
                || frame.meters_per_unit != auki_geometry::meters_per_unit(convention.units)
            {
                return Err(SceneError::Invalid(
                    "registry convention does not match map frame",
                ));
            }
        }
        // This geometry API validates the complete axes and declared handedness.
        let mut matrix = auki_geometry::convention_matrix(source_convention, target_convention)
            .map_err(geometry_error)?;
        let scale = source.meters_per_unit / target.meters_per_unit;
        for row in matrix.iter_mut().take(3) {
            for value in row.iter_mut().take(3) {
                *value /= scale;
            }
        }
        let pose = auki_geometry::spatial_transform_from_matrix4(matrix).map_err(geometry_error)?;
        let q = pose
            .orientation
            .expect("matrix decomposition supplies rotation");
        let rotation = RigidTransform {
            from_frame_id: source.id.clone(),
            to_frame_id: target.id.clone(),
            translation: [0.; 3],
            rotation_wxyz: [q.w, q.x, q.y, q.z],
        };
        Self::new(source, target, rotation)
    }

    /// `axis_rotation` must be a labelled, zero-translation unit-quaternion
    /// rotation from `source.id` to `target.id`, mapping source up to target up.
    /// The length scale comes exclusively from the two frame declarations.
    pub fn new(
        source: MapFrame,
        target: MapFrame,
        axis_rotation: RigidTransform,
    ) -> Result<Self, SceneError> {
        source.validate()?;
        target.validate()?;
        if source.id == target.id {
            return Err(SceneError::Invalid(
                "conversion requires distinct frame IDs",
            ));
        }
        if axis_rotation.from_frame_id != source.id || axis_rotation.to_frame_id != target.id {
            return Err(SceneError::Invalid(
                "conversion rotation has mismatched frame endpoints",
            ));
        }
        if axis_rotation.translation != [0.; 3] {
            return Err(SceneError::Invalid(
                "convention conversion must preserve the physical origin",
            ));
        }
        let q = axis_rotation.rotation_wxyz;
        if !q.iter().all(|v| v.is_finite())
            || (q.iter().map(|v| v * v).sum::<f64>() - 1.).abs() > 1e-9
        {
            return Err(SceneError::Invalid(
                "conversion rotation must be a finite unit quaternion",
            ));
        }
        let rotation = numeric(&axis_rotation);
        let matrix = spatial_transform_to_matrix4(&rotation).map_err(geometry_error)?;
        if let (Some(from), Some(to)) = (source.convention, target.convention) {
            let expected = auki_geometry::axis_convention_matrix(&from.axes(), &to.axes())
                .map_err(geometry_error)?;
            for i in 0..3 {
                for j in 0..3 {
                    if (matrix[i][j] - expected[i][j]).abs() > 1e-9 {
                        return Err(SceneError::Invalid(
                            "axis rotation disagrees with named conventions",
                        ));
                    }
                }
            }
        }
        let up_index = |up| match up {
            UpAxis::Y => 1,
            UpAxis::Z => 2,
        };
        let source_up = up_index(source.up_axis);
        let target_up = up_index(target.up_axis);
        for (row, values) in matrix.iter().take(3).enumerate() {
            let expected = if row == target_up { 1. } else { 0. };
            if (values[source_up] - expected).abs() > 1e-9 {
                return Err(SceneError::Invalid(
                    "axis rotation disagrees with declared up axes",
                ));
            }
        }
        let scale = source.meters_per_unit / target.meters_per_unit;
        if !scale.is_finite() || scale <= 0. {
            return Err(SceneError::Invalid(
                "unit conversion scale overflows or underflows",
            ));
        }
        Ok(Self {
            source,
            target,
            rotation,
            scale,
        })
    }

    pub fn source_frame(&self) -> &MapFrame {
        &self.source
    }
    pub fn target_frame(&self) -> &MapFrame {
        &self.target
    }

    /// Full source-coordinate to target-coordinate mapping, including unit scale.
    /// A scale change cannot be represented by a `RigidTransform` alone.
    pub fn coordinate_transform(&self) -> FramedConventionMatrix {
        let mut matrix = spatial_transform_to_matrix4(&self.rotation)
            .expect("constructor validated the immutable rotation");
        for row in matrix.iter_mut().take(3) {
            for value in row.iter_mut().take(3) {
                *value *= self.scale;
            }
        }
        FramedConventionMatrix {
            from_frame_id: self.source.id.clone(),
            to_frame_id: self.target.id.clone(),
            matrix,
        }
    }

    /// Return a new map artifact. The source is untouched, Portal identities and
    /// physical sizes in meters are preserved, and poses target the new frame.
    /// Portal local axes remain printed-right/up/out; only the map side rotates.
    pub fn convert_scenegraph(
        &self,
        source: &Scenegraph,
        target_map_id: impl Into<String>,
    ) -> Result<Scenegraph, SceneError> {
        source.validate()?;
        if source.map.frame != self.source {
            return Err(SceneError::Invalid(
                "source map frame does not match conversion declaration",
            ));
        }
        let target_map_id = target_map_id.into();
        if source.map.map_id == target_map_id {
            return Err(SceneError::Invalid(
                "converted map requires a distinct map ID",
            ));
        }
        let mut converted = source.clone();
        converted.map.map_id = target_map_id;
        converted.map.frame = self.target.clone();
        for anchor in converted.anchors.values_mut() {
            // Compose on the map/target side, not a two-sided basis conjugation:
            // the canonical Portal frame must remain unchanged.
            let pose = compose_spatial_transforms(&numeric(&anchor.pose_in_map), &self.rotation)
                .map_err(geometry_error)?;
            let t = pose.translation.expect("composition supplies translation");
            let q = pose.orientation.expect("composition supplies orientation");
            anchor.pose_in_map.translation = [t.x * self.scale, t.y * self.scale, t.z * self.scale];
            anchor.pose_in_map.rotation_wxyz = [q.w, q.x, q.y, q.z];
            anchor.pose_in_map.to_frame_id = self.target.id.clone();
            // side_length_m is physical, not expressed in map units. USDA derives
            // local mesh coordinates from side_length_m / target.meters_per_unit.
        }
        converted.validate()?;
        Ok(converted)
    }

    /// Validate the complete source artifact and regenerate canonical USDA.
    /// Publication and its clock/sequence remain the host's responsibility.
    pub fn convert_snapshot(
        &self,
        source: &MapSnapshot,
        target_map_id: impl Into<String>,
    ) -> Result<MapSnapshot, SceneError> {
        source.validate()?;
        MapSnapshot::new(self.convert_scenegraph(&source.scenegraph, target_map_id)?)
    }
}

fn geometry_error(_: auki_geometry::GeometryError) -> SceneError {
    SceneError::Invalid("invalid geometry during convention conversion")
}

fn numeric(pose: &RigidTransform) -> SpatialTransform {
    let [x, y, z] = pose.translation;
    let [w, qx, qy, qz] = pose.rotation_wxyz;
    SpatialTransform {
        translation: Some(Vec3 { x, y, z }),
        orientation: Some(Quat {
            w,
            x: qx,
            y: qy,
            z: qz,
        }),
    }
}
