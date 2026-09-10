//! UniFFI facade over [`auki_urdf_fk`] for native fleet articulation.

use std::sync::Arc;

use auki_urdf_fk::{LinkTransform, Model};

use crate::{AukiSdkError, operation_error};

/// One link transform from FK resolve (column-major 4×4).
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AukiUrdfLinkTransform {
    pub link_name: String,
    pub mesh_path: Option<String>,
    pub color_rgba: Option<Vec<f32>>,
    pub transform: Vec<f32>,
}

impl From<LinkTransform> for AukiUrdfLinkTransform {
    fn from(link: LinkTransform) -> Self {
        Self {
            link_name: link.link_name,
            mesh_path: link.mesh_path,
            color_rgba: link.color_rgba.map(|rgba| rgba.to_vec()),
            transform: link.transform.to_vec(),
        }
    }
}

/// Parsed URDF held for repeated `resolve` calls.
#[derive(uniffi::Object)]
pub struct AukiUrdfModel {
    inner: Model,
}

#[uniffi::export]
impl AukiUrdfModel {
    /// Parse URDF XML (`Model::from_str`).
    #[uniffi::constructor]
    pub fn from_xml(xml: String) -> Result<Arc<Self>, AukiSdkError> {
        Model::from_str(&xml)
            .map(|inner| Arc::new(Self { inner }))
            .map_err(|error| operation_error("parse URDF", error))
    }

    pub fn joint_count(&self) -> u32 {
        self.inner.joint_count() as u32
    }

    pub fn robot_name(&self) -> String {
        self.inner.robot_name().to_owned()
    }

    /// FK for `angles` indexed by URDF active-joint declaration order.
    pub fn resolve(&self, angles: Vec<f32>) -> Result<Vec<AukiUrdfLinkTransform>, AukiSdkError> {
        self.inner
            .resolve(&angles)
            .map(|links| links.into_iter().map(AukiUrdfLinkTransform::from).collect())
            .map_err(|error| operation_error("resolve URDF FK", error))
    }

    /// Rest pose with all joint angles at zero.
    pub fn resolve_identity_pose(&self) -> Vec<AukiUrdfLinkTransform> {
        self.inner
            .resolve_identity_pose()
            .into_iter()
            .map(AukiUrdfLinkTransform::from)
            .collect()
    }
}
