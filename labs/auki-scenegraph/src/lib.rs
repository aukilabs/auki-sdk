//! Small QR scenegraphs, with optional Component publication. No detector or network dependency.
//! USD export is a deterministic projection, not a general USD composition engine.

pub mod conversion;
mod duplicate;
pub use auki_registry::CoordinateConvention;
pub use conversion::{FramedConventionMatrix, MapConventionConversion};
pub use duplicate::MapDuplicateTarget;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write;

#[cfg(feature = "components")]
pub mod alignment;
#[cfg(feature = "components")]
pub mod catalog;
#[cfg(feature = "components")]
pub mod component;
#[cfg(feature = "components")]
pub mod directory;
#[cfg(feature = "components")]
pub mod resolution;

pub const SNAPSHOT_SCHEMA: &str = "auki.scenegraph.qr-snapshot/v3";
pub const MAX_ANCHORS: usize = 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("invalid scenegraph: {0}")]
    Invalid(&'static str),
    #[error("snapshot exceeds the configured v1 size bound")]
    TooLarge,
    #[error(transparent)]
    Encoding(#[from] serde_json::Error),
}

/// USD-compatible stage up axis. Other Auki conventions need an explicit conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapFrame {
    pub handedness: Handedness,
    pub id: String,
    pub up_axis: UpAxis,
    pub meters_per_unit: f64,
    /// Complete named axes when known. Missing on legacy/custom maps: never infer it from up_axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convention: Option<CoordinateConvention>,
    /// Human-readable establishment of the origin and horizontal heading; not an alignment proof.
    pub origin_description: String,
}

impl MapFrame {
    /// Explicit named axes; currently only right-handed positive-Y/Z-up map conventions are representable.
    pub fn in_convention(
        id: impl Into<String>,
        origin_description: impl Into<String>,
        convention: CoordinateConvention,
        meters_per_unit: f64,
    ) -> Result<Self, SceneError> {
        let up_axis = named_map_up(convention)?;
        let frame = Self {
            id: id.into(),
            origin_description: origin_description.into(),
            handedness: Handedness::Right,
            up_axis,
            meters_per_unit,
            convention: Some(convention),
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), SceneError> {
        text(&self.id, 256)?;
        text(&self.origin_description, 4096)?;
        if let Some(convention) = self.convention
            && named_map_up(convention)? != self.up_axis
        {
            return Err(SceneError::Invalid(
                "named convention disagrees with map up axis",
            ));
        }
        if !self.meters_per_unit.is_finite() || self.meters_per_unit <= 0.0 {
            return Err(SceneError::Invalid(
                "meters_per_unit must be positive and finite",
            ));
        }
        Ok(())
    }

    /// Every v1 frame is right-handed; no implicit conversion or external alignment is asserted.
    pub fn z_up_meters(id: impl Into<String>, origin_description: impl Into<String>) -> Self {
        Self {
            handedness: Handedness::Right,
            id: id.into(),
            up_axis: UpAxis::Z,
            meters_per_unit: 1.0,
            convention: None,
            origin_description: origin_description.into(),
        }
    }
}

fn named_map_up(convention: CoordinateConvention) -> Result<UpAxis, SceneError> {
    use auki_registry::AxisDirection::Up;
    if convention.handedness() != auki_registry::Handedness::Right {
        return Err(SceneError::Invalid(
            "left-handed map conventions require reflection-capable poses",
        ));
    }
    let axes = convention.axes();
    if axes.y == Up {
        Ok(UpAxis::Y)
    } else if axes.z == Up {
        Ok(UpAxis::Z)
    } else {
        Err(SceneError::Invalid(
            "map convention requires a positive Y or Z up axis",
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapDefinition {
    pub map_id: String,
    pub name: Option<String>,
    /// Association with a place, not transport authority or spatial alignment.
    pub domain_reference: Option<String>,
    pub frame: MapFrame,
}

/// Explicitly labelled active rigid pose. Never infer either endpoint from context.
/// Translation uses map units. Rotation is a unit quaternion in w,x,y,z order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigidTransform {
    pub from_frame_id: String,
    pub to_frame_id: String,
    pub translation: [f64; 3],
    pub rotation_wxyz: [f64; 4],
}

impl RigidTransform {
    /// Explicit declaration that two named frames have coincident origins and axes.
    pub fn identity(from_frame_id: impl Into<String>, to_frame_id: impl Into<String>) -> Self {
        Self {
            from_frame_id: from_frame_id.into(),
            to_frame_id: to_frame_id.into(),
            translation: [0.0; 3],
            rotation_wxyz: [1.0, 0.0, 0.0, 0.0],
        }
    }
}

/// Centered QR frame: +X printed-right, +Y printed-up, +Z out of the front face.
/// Side length covers the encoded square and excludes the quiet zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QrAnchor {
    /// Stable physical-marker identity, independent of payload and USD prim path.
    pub anchor_id: String,
    pub payload: String,
    pub side_length_m: f64,
    pub pose_in_map: RigidTransform,
}

/// V1 is a scenegraph with one root and QR children directly under it.
/// Nested transforms, probabilistic placements and detector provenance are not invented here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenegraph {
    pub map: MapDefinition,
    pub anchors: BTreeMap<String, QrAnchor>,
}

fn text(value: &str, max: usize) -> Result<(), SceneError> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(SceneError::Invalid(
            "empty, oversized or control-character text",
        ));
    }
    Ok(())
}

impl QrAnchor {
    pub fn validate_in_map(&self, map: &MapDefinition) -> Result<(), SceneError> {
        self.validate()?;
        if self.pose_in_map.to_frame_id != map.frame.id {
            return Err(SceneError::Invalid(
                "anchor destination frame differs from map frame",
            ));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), SceneError> {
        text(&self.pose_in_map.from_frame_id, 256)?;
        text(&self.pose_in_map.to_frame_id, 256)?;
        text(&self.anchor_id, 256)?;
        text(&self.payload, 4096)?;
        if !self.side_length_m.is_finite() || self.side_length_m <= 0.0 {
            return Err(SceneError::Invalid("QR size must be positive and finite"));
        }
        if !self
            .pose_in_map
            .translation
            .iter()
            .chain(self.pose_in_map.rotation_wxyz.iter())
            .all(|v| v.is_finite())
        {
            return Err(SceneError::Invalid("pose must be finite"));
        }
        let norm: f64 = self.pose_in_map.rotation_wxyz.iter().map(|v| v * v).sum();
        if (norm - 1.0).abs() > 1e-9 {
            return Err(SceneError::Invalid("rotation must be a unit quaternion"));
        }
        Ok(())
    }
}

impl Scenegraph {
    pub fn new(map: MapDefinition) -> Result<Self, SceneError> {
        let scene = Self {
            map,
            anchors: BTreeMap::new(),
        };
        scene.validate()?;
        Ok(scene)
    }

    pub fn validate(&self) -> Result<(), SceneError> {
        text(&self.map.map_id, 256)?;
        self.map.frame.validate()?;
        for value in [&self.map.name, &self.map.domain_reference]
            .into_iter()
            .flatten()
        {
            text(value, 256)?;
        }
        let scale = self.map.frame.meters_per_unit;
        if self.anchors.len() > MAX_ANCHORS {
            return Err(SceneError::TooLarge);
        }
        for (id, anchor) in &self.anchors {
            if id != &anchor.anchor_id {
                return Err(SceneError::Invalid("anchor key and identity disagree"));
            }
            anchor.validate_in_map(&self.map)?;
            if !(anchor.side_length_m / scale).is_finite() {
                return Err(SceneError::Invalid("QR size overflows map units"));
            }
        }
        Ok(())
    }

    /// Generate a self-contained ASCII USD stage. No references, payload assets or external files.
    /// User identifiers are attributes; deterministic hex prim names prevent path injection.
    pub fn to_usda(&self) -> Result<String, SceneError> {
        self.export_usda(false)
    }

    /// Inspection artifact with white, double-sided encoded-size portal squares.
    /// This is not the canonical snapshot representation and must not replace its USDA.
    pub fn to_usda_with_portal_geometry(&self) -> Result<String, SceneError> {
        self.export_usda(true)
    }

    fn export_usda(&self, geometry: bool) -> Result<String, SceneError> {
        self.validate()?;
        let q = |s: &str| serde_json::to_string(s).expect("string serialization");
        let mut out = format!(
            "#usda 1.0\n(\n    defaultPrim = \"Map\"\n    upAxis = \"{:?}\"\n    metersPerUnit = {}\n)\n\ndef Xform \"Map\"\n{{\n    custom string auki:mapId = {}\n    custom string auki:rootFrameId = {}\n    custom string auki:handedness = \"right\"\n    custom string auki:originDescription = {}\n",
            self.map.frame.up_axis,
            self.map.frame.meters_per_unit,
            q(&self.map.map_id),
            q(&self.map.frame.id),
            q(&self.map.frame.origin_description)
        );
        if let Some(convention) = self.map.frame.convention {
            writeln!(
                out,
                "    custom token auki:coordinateConvention = {}",
                q(convention.as_str())
            )
            .unwrap();
        }
        if let Some(name) = &self.map.name {
            writeln!(out, "    custom string auki:name = {}", q(name)).unwrap();
        }
        if let Some(domain) = &self.map.domain_reference {
            writeln!(out, "    custom string auki:domain = {}", q(domain)).unwrap();
        }
        for anchor in self.anchors.values() {
            let path: String = anchor
                .anchor_id
                .as_bytes()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            let [x, y, z] = anchor.pose_in_map.translation;
            let [w, qx, qy, qz] = anchor.pose_in_map.rotation_wxyz;
            writeln!(out, "    def Xform \"QR_{path}\"\n    {{\n        custom string auki:kind = \"qr_anchor\"\n        custom string auki:anchorId = {}\n        custom string auki:fromFrameId = {}\n        custom string auki:toFrameId = {}\n        custom string auki:qr:payload = {}\n        custom double auki:qr:sideLengthMeters = {}\n        custom token auki:qr:frameConvention = \"center_xRight_yUp_zOut\"\n        double3 xformOp:translate = ({x}, {y}, {z})\n        quatd xformOp:orient = ({w}, {qx}, {qy}, {qz})\n        uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]", q(&anchor.anchor_id), q(&anchor.pose_in_map.from_frame_id), q(&anchor.pose_in_map.to_frame_id), q(&anchor.payload), anchor.side_length_m).unwrap();
            if geometry {
                let h = anchor.side_length_m / self.map.frame.meters_per_unit / 2.;
                writeln!(out, "        def Mesh \"PortalSquare\"\n        {{\n            point3f[] points = [(-{h}, -{h}, 0), ({h}, -{h}, 0), ({h}, {h}, 0), (-{h}, {h}, 0)]\n            int[] faceVertexCounts = [4]\n            int[] faceVertexIndices = [0, 1, 2, 3]\n            uniform token subdivisionScheme = \"none\"\n            uniform bool doubleSided = true\n            color3f[] primvars:displayColor = [(1, 1, 1)]\n        }}").unwrap();
            }
            out.push_str("    }\n");
        }
        out.push_str("}\n");
        Ok(out)
    }
}

/// Complete typed state plus its derived USD representation; never independently edited.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSnapshot {
    pub scenegraph: Scenegraph,
    pub usda: String,
}

impl MapSnapshot {
    pub fn new(scenegraph: Scenegraph) -> Result<Self, SceneError> {
        let usda = scenegraph.to_usda()?;
        let snapshot = Self { scenegraph, usda };
        if snapshot.encoded_size() > MAX_SNAPSHOT_BYTES {
            return Err(SceneError::TooLarge);
        }
        Ok(snapshot)
    }
    /// Validate after decoding data from an untrusted producer.
    pub fn validate(&self) -> Result<(), SceneError> {
        if self.usda != self.scenegraph.to_usda()? {
            return Err(SceneError::Invalid("USD does not match typed scenegraph"));
        }
        if self.encoded_size() > MAX_SNAPSHOT_BYTES {
            return Err(SceneError::TooLarge);
        }
        Ok(())
    }
    pub fn encoded_size(&self) -> usize {
        serde_json::to_vec(self).map_or(usize::MAX, |bytes| bytes.len())
    }
}
