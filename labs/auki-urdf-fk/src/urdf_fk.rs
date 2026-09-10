//! URDF parsing and forward kinematics for articulated robots.
//!
//! Parse a URDF once, then walk the kinematic tree on each joint-angle frame
//! to produce per-link transforms for the consumer's renderer.
//!
//! ## Joint-angle ordering
//!
//! [`Model::resolve`] takes angles indexed by URDF declaration order, matching
//! [`Model::joint_names`]. Consumers must align incoming joint-angle streams
//! with that order, or use [`Model::resolve_with_joint_names`].
//!
//! ## RPY convention
//!
//! URDF `<origin rpy="r p y"/>` uses fixed-axis XYZ (extrinsic): roll about
//! world-X, then pitch about world-Y, then yaw about world-Z. This is
//! `Rz(yaw) * Ry(pitch) * Rx(roll)`. Axis-angle quaternions are composed
//! explicitly to avoid Euler-order ambiguity.

use std::collections::BTreeMap;
use std::path::Path;

use glam::{Mat4, Quat, Vec3};

/// Per-link transform produced by [`Model::resolve`]. Returned for
/// every link in the model, regardless of whether it has a visual
/// mesh — the browser uses those that do for rendering and ignores
/// the rest.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkTransform {
    /// URDF link name (matches the producer's `<link name="...">`).
    pub link_name: String,
    /// Visual mesh path relative to the URDF (e.g. `"meshes/Trunk.STL"`).
    /// `None` when the link has no `<visual>` block — common for
    /// kinematic chain joints whose physical body lives on the parent
    /// or child.
    pub mesh_path: Option<String>,
    /// Material RGBA from the URDF's `<material><color rgba="..."/>`,
    /// or `None` when no material is declared.
    pub color_rgba: Option<[f32; 4]>,
    /// 4×4 affine transform mapping points in this link's frame to the
    /// model's root frame, **column-major**. Three.js's
    /// `Matrix4.fromArray` consumes column-major directly.
    pub transform: [f32; 16],
}

/// Parse + FK errors.
#[derive(Debug)]
pub enum FkError {
    /// Couldn't read or parse the URDF file.
    UrdfParse(String),
    /// Resolving the tree found a joint pointing at a parent or child
    /// link that isn't declared in `<link>`. URDF spec violation.
    UnknownLink { joint: String, link: String },
    /// More than one root link — a URDF must have exactly one link
    /// that is no joint's child. Either the file is malformed or it
    /// describes a multi-robot scene we don't handle here.
    MultipleRoots(Vec<String>),
    /// `<robot>` declared zero links.
    NoRoot,
    /// [`Model::resolve`] was passed a joint-angle vector whose
    /// length doesn't match the model's joint count.
    JointCountMismatch { expected: usize, actual: usize },
    /// A named source vector referenced a joint that is not an active
    /// revolute / continuous joint in this URDF.
    UnknownJointName { joint: String },
}

impl std::fmt::Display for FkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FkError::UrdfParse(e) => write!(f, "URDF parse: {e}"),
            FkError::UnknownLink { joint, link } => {
                write!(f, "joint {joint:?} references undeclared link {link:?}")
            }
            FkError::MultipleRoots(roots) => write!(
                f,
                "URDF has {} root links (expected exactly 1): {roots:?}",
                roots.len()
            ),
            FkError::NoRoot => write!(f, "URDF declares no links"),
            FkError::JointCountMismatch { expected, actual } => write!(
                f,
                "joint count mismatch: model expects {expected} angles, got {actual}"
            ),
            FkError::UnknownJointName { joint } => {
                write!(f, "unknown active joint name {joint:?}")
            }
        }
    }
}

impl std::error::Error for FkError {}

/// Parsed URDF model + topology. Cheap to build (~ms for the K1 22-DoF
/// URDF), expensive to parse (XML + serde), so consumers should `load`
/// once at boot and call [`Self::resolve`] per incoming pose frame.
/// BracketBot `chopped_urdf_v1` Draco/CAD verts share a world frame where
/// +Y is right (wheel track), −Z is toward the head. BBOS `base_link` is
/// +X right, +Y forward, +Z up. This rigid map (Rz(π/2)·Rx(π)) puts the
/// assembled CAD into `base_link` without re-applying per-link FK origins
/// (which explode the arms).
pub const BRACKETBOT_CAD_TO_BASE_LINK: Mat4 = Mat4::from_cols_array(&[
    0.0, 1.0, 0.0, 0.0, //
    1.0, 0.0, 0.0, 0.0, //
    0.0, 0.0, -1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
]);

pub struct Model {
    /// `<robot name="…">` (lowercased by some packs; preserved as parsed).
    robot_name: String,
    /// URDF root link's name (the link no joint declares as its
    /// child). For K1: `"Trunk"`.
    root_link: String,
    /// All modeled joints in URDF declaration order, including fixed
    /// joints. The angle vector passed to [`Self::resolve`] is
    /// parallel only to the revolute / continuous subset surfaced by
    /// [`Self::joint_names`].
    joints: Vec<JointDef>,
    /// All links keyed by name, with visual mesh + colour metadata.
    links: BTreeMap<String, LinkDef>,
    /// Adjacency: parent-link name → indices into `self.joints` of the
    /// joints whose `parent_link` is that link. Built once at load,
    /// reused per `resolve` so the FK walk doesn't re-scan
    /// `self.joints` per link.
    children_of: BTreeMap<String, Vec<usize>>,
}

struct JointDef {
    name: String,
    child_link: String,
    /// Fixed transform from `parent_link`'s frame to the joint frame,
    /// composed from the URDF's `<origin xyz="..." rpy="..."/>`.
    origin: Mat4,
    /// Rotation axis in the joint's own frame. Used only when
    /// `angle_index` is `Some`; fixed joints carry `Vec3::ZERO`.
    axis: Vec3,
    /// Index into the caller-provided angle vector for revolute /
    /// continuous joints. `None` for fixed joints, which preserve
    /// their static origin transform without consuming an angle.
    angle_index: Option<usize>,
}

struct LinkDef {
    mesh_path: Option<String>,
    color_rgba: Option<[f32; 4]>,
}

impl Model {
    /// Parse a URDF file from disk and build the FK-ready model.
    pub fn load(path: &Path) -> Result<Self, FkError> {
        let robot = urdf_rs::read_file(path).map_err(|e| FkError::UrdfParse(e.to_string()))?;
        Self::from_robot(&robot)
    }

    /// Parse a URDF from a string. Useful for tests.
    #[allow(
        clippy::should_implement_trait,
        reason = "Preserve the upstream Model::from_str API when relocating the crate."
    )]
    pub fn from_str(s: &str) -> Result<Self, FkError> {
        let robot = urdf_rs::read_from_string(s).map_err(|e| FkError::UrdfParse(e.to_string()))?;
        Self::from_robot(&robot)
    }

    fn from_robot(robot: &urdf_rs::Robot) -> Result<Self, FkError> {
        let mut links = BTreeMap::new();
        for link in &robot.links {
            // First <visual> wins. K1's URDF uses multi-visual on Trunk
            // for the body + the K1 logo overlay; we render the body
            // and ignore the decorative overlay (the alternative,
            // emitting a synthetic per-overlay link, would balloon the
            // wire payload for cosmetic gain we don't need yet).
            let visual = link.visual.first();
            let mesh_path = visual.and_then(|v| match &v.geometry {
                urdf_rs::Geometry::Mesh { filename, .. } => Some(filename.clone()),
                _ => None,
            });
            let color_rgba = visual
                .and_then(|v| v.material.as_ref())
                .and_then(|m| m.color.as_ref())
                .map(|c| {
                    let [r, g, b, a] = c.rgba.0;
                    [r as f32, g as f32, b as f32, a as f32]
                });
            links.insert(
                link.name.clone(),
                LinkDef {
                    mesh_path,
                    color_rgba,
                },
            );
        }

        let mut joints = Vec::with_capacity(robot.joints.len());
        let mut children_of: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut active_joint_count = 0_usize;
        for j in &robot.joints {
            let angle_index = match j.joint_type {
                urdf_rs::JointType::Revolute | urdf_rs::JointType::Continuous => {
                    let idx = active_joint_count;
                    active_joint_count += 1;
                    Some(idx)
                }
                // Fixed, and prismatic/floating/planar at zero extension:
                // keep them in the tree so manufacturer packs with prismatic
                // arm slides (BracketBot lj0/rj0) stay single-rooted. We do
                // not consume angle slots until translation DoF is modeled.
                urdf_rs::JointType::Fixed
                | urdf_rs::JointType::Prismatic
                | urdf_rs::JointType::Floating
                | urdf_rs::JointType::Planar
                | urdf_rs::JointType::Spherical => None,
            };
            let parent_link = j.parent.link.clone();
            let child_link = j.child.link.clone();
            if !links.contains_key(&parent_link) {
                return Err(FkError::UnknownLink {
                    joint: j.name.clone(),
                    link: parent_link,
                });
            }
            if !links.contains_key(&child_link) {
                return Err(FkError::UnknownLink {
                    joint: j.name.clone(),
                    link: child_link,
                });
            }
            let [x, y, z] = j.origin.xyz.0;
            let [roll, pitch, yaw] = j.origin.rpy.0;
            let translation = Vec3::new(x as f32, y as f32, z as f32);
            // Fixed-axis XYZ (extrinsic): roll first about world-X,
            // then pitch about world-Y, then yaw about world-Z.
            // Equivalent to the matrix product Rz·Ry·Rx, hence the
            // quaternion order qz·qy·qx.
            let rotation = Quat::from_axis_angle(Vec3::Z, yaw as f32)
                * Quat::from_axis_angle(Vec3::Y, pitch as f32)
                * Quat::from_axis_angle(Vec3::X, roll as f32);
            let origin = Mat4::from_rotation_translation(rotation, translation);
            let axis = if angle_index.is_some() {
                let [ax, ay, az] = j.axis.xyz.0;
                Vec3::new(ax as f32, ay as f32, az as f32).normalize_or_zero()
            } else {
                Vec3::ZERO
            };
            let idx = joints.len();
            joints.push(JointDef {
                name: j.name.clone(),
                child_link,
                origin,
                axis,
                angle_index,
            });
            children_of.entry(parent_link).or_default().push(idx);
        }

        // Find the root link. URDF rule: exactly one link is no
        // joint's child.
        let mut child_links: BTreeMap<&str, ()> = BTreeMap::new();
        for j in &joints {
            child_links.insert(&j.child_link, ());
        }
        let roots: Vec<String> = links
            .keys()
            .filter(|name| !child_links.contains_key(name.as_str()))
            .cloned()
            .collect();
        let root_link = match roots.len() {
            1 => roots.into_iter().next().unwrap(),
            0 => return Err(FkError::NoRoot),
            _ => return Err(FkError::MultipleRoots(roots)),
        };

        Ok(Self {
            robot_name: robot.name.clone(),
            root_link,
            joints,
            links,
            children_of,
        })
    }

    /// `<robot name>` from the URDF.
    pub fn robot_name(&self) -> &str {
        &self.robot_name
    }

    /// Active joint names in URDF declaration order. Fixed joints are
    /// traversed by FK but omitted here because they do not consume
    /// entries from the angle vector passed to [`Self::resolve`].
    pub fn joint_names(&self) -> Vec<&str> {
        self.joints
            .iter()
            .filter(|j| j.angle_index.is_some())
            .map(|j| j.name.as_str())
            .collect()
    }

    /// Number of revolute / continuous joints — the expected length of
    /// `angles` in [`Self::resolve`]. Fixed joints are not counted.
    /// For K1 22-DoF: 22.
    pub fn joint_count(&self) -> usize {
        self.joints
            .iter()
            .filter(|j| j.angle_index.is_some())
            .count()
    }

    /// URDF root link name — the frame all returned transforms are in.
    pub fn root_link(&self) -> &str {
        &self.root_link
    }

    /// All link names. Ordered alphabetically (the underlying storage
    /// is a `BTreeMap`).
    pub fn link_names(&self) -> Vec<&str> {
        self.links.keys().map(String::as_str).collect()
    }

    /// Walk the kinematic tree from `root_link` outward, applying each
    /// joint's fixed `origin` transform composed with its axis-angle
    /// rotation, and emit one [`LinkTransform`] per link in the model.
    /// Output order is the depth-first traversal order (root first), which is
    /// stable across calls.
    pub fn resolve(&self, angles: &[f32]) -> Result<Vec<LinkTransform>, FkError> {
        let expected_angles = self.joint_count();
        if angles.len() != expected_angles {
            return Err(FkError::JointCountMismatch {
                expected: expected_angles,
                actual: angles.len(),
            });
        }

        let mut out = Vec::with_capacity(self.links.len());
        let mut stack: Vec<(String, Mat4)> = vec![(self.root_link.clone(), Mat4::IDENTITY)];

        while let Some((link_name, link_to_root)) = stack.pop() {
            let link = &self.links[&link_name];
            out.push(LinkTransform {
                link_name: link_name.clone(),
                mesh_path: link.mesh_path.clone(),
                color_rgba: link.color_rgba,
                transform: link_to_root.to_cols_array(),
            });

            // For every joint whose parent is this link, compose the
            // joint's origin with its axis-angle rotation, multiply
            // into the accumulated link transform, and push the child.
            // Iterating in declaration order (the `Vec<usize>` we
            // built at load) keeps traversal deterministic.
            if let Some(child_indices) = self.children_of.get(&link_name) {
                // Push in reverse so the *first*-declared child is
                // popped *first* — matches the URDF's textual order
                // top-down, which is the natural reading for tests
                // and for any future link-list serialization.
                for &j_idx in child_indices.iter().rev() {
                    let j = &self.joints[j_idx];
                    let joint_rotation = match j.angle_index {
                        Some(angle_idx) => {
                            Mat4::from_quat(Quat::from_axis_angle(j.axis, angles[angle_idx]))
                        }
                        None => Mat4::IDENTITY,
                    };
                    let child_to_root = link_to_root * j.origin * joint_rotation;
                    stack.push((j.child_link.clone(), child_to_root));
                }
            }
        }

        Ok(out)
    }

    /// Same depth-first link order as [`Self::resolve`], but every link shares one rigid
    /// rest transform (usually identity).
    ///
    /// Manufacturer packs (BracketBot) leave mesh vertices in a shared
    /// CAD/world frame. Applying FK joint origins then double-places geometry
    /// and "explodes" the arms. Dig-only / no-`joint_encoders` rest pose should
    /// use this; live joint streams still call [`Self::resolve`].
    ///
    /// For `chopped_urdf_v1`, the shared transform is
    /// [`BRACKETBOT_CAD_TO_BASE_LINK`] so CAD verts land in BBOS `base_link`.
    pub fn resolve_identity_pose(&self) -> Vec<LinkTransform> {
        let zeros = vec![0.0_f32; self.joint_count()];
        let Ok(mut links) = self.resolve(&zeros) else {
            // joint_count always matches zeros; resolve only fails on mismatch.
            return Vec::new();
        };
        let rest = if self.robot_name == "chopped_urdf_v1" {
            BRACKETBOT_CAD_TO_BASE_LINK
        } else {
            Mat4::IDENTITY
        };
        let rest = rest.to_cols_array();
        for link in &mut links {
            link.transform = rest;
        }
        links
    }

    /// Resolve FK from a source vector indexed by explicit active
    /// joint names instead of this URDF's complete active-joint order.
    /// Joints omitted by the source default to zero radians.
    pub fn resolve_with_joint_names(
        &self,
        joint_names: &[&str],
        angles: &[f32],
    ) -> Result<Vec<LinkTransform>, FkError> {
        let joint_names: Vec<Option<&str>> = joint_names.iter().copied().map(Some).collect();
        self.resolve_with_optional_joint_names(&joint_names, angles)
    }

    /// Resolve FK from a source vector indexed by optional target URDF
    /// joint names. `None` source slots are accepted and ignored.
    pub fn resolve_with_optional_joint_names(
        &self,
        joint_names: &[Option<&str>],
        angles: &[f32],
    ) -> Result<Vec<LinkTransform>, FkError> {
        if joint_names.len() != angles.len() {
            return Err(FkError::JointCountMismatch {
                expected: joint_names.len(),
                actual: angles.len(),
            });
        }

        let mut expanded = vec![0.0_f32; self.joint_count()];
        for (joint_name, angle) in joint_names.iter().zip(angles) {
            let Some(joint_name) = joint_name else {
                continue;
            };
            let Some(angle_index) = self.joints.iter().find_map(|joint| {
                if joint.name == *joint_name {
                    joint.angle_index
                } else {
                    None
                }
            }) else {
                return Err(FkError::UnknownJointName {
                    joint: (*joint_name).to_string(),
                });
            };
            expanded[angle_index] = *angle;
        }

        self.resolve(&expanded)
    }
}

// ─── tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_joints_preserve_mount_links_without_consuming_angles() {
        let urdf = r#"<?xml version="1.0"?>
<robot name="fixed_mount_test">
  <link name="root"/>
  <link name="mount"/>
  <link name="tip"/>
  <joint name="root_to_mount" type="fixed">
    <origin xyz="1 2 3" rpy="0 0 0"/>
    <parent link="root"/>
    <child link="mount"/>
  </joint>
  <joint name="mount_to_tip" type="revolute">
    <origin xyz="0 0 1" rpy="0 0 0"/>
    <parent link="mount"/>
    <child link="tip"/>
    <axis xyz="0 0 1"/>
    <limit lower="-1" upper="1" effort="1" velocity="1"/>
  </joint>
</robot>"#;
        let model = Model::from_str(urdf).unwrap();
        assert_eq!(model.joint_count(), 1);
        assert_eq!(model.joint_names(), vec!["mount_to_tip"]);

        let transforms = model.resolve(&[0.0]).unwrap();
        let mount = transforms
            .iter()
            .find(|t| t.link_name == "mount")
            .expect("fixed child link should be traversed");
        let tip = transforms
            .iter()
            .find(|t| t.link_name == "tip")
            .expect("revolute child link should be traversed");
        let mount_m = Mat4::from_cols_array(&mount.transform);
        let tip_m = Mat4::from_cols_array(&tip.transform);
        assert!(
            mount_m
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5)
        );
        assert!(
            tip_m
                .w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(1.0, 2.0, 4.0), 1e-5)
        );
    }

    #[test]
    fn prismatic_joints_keep_tree_connected_at_zero_extension() {
        // BracketBot manufacturer pack: shoulders hang off arm_base via
        // prismatic lj0/rj0. Skipping those joints used to invent extra roots.
        let urdf = r#"<?xml version="1.0"?>
<robot name="prismatic_bridge">
  <link name="root"/>
  <link name="arm_base"/>
  <link name="shoulder"/>
  <joint name="root_to_base" type="fixed">
    <origin xyz="0 0 0" rpy="0 0 0"/>
    <parent link="root"/>
    <child link="arm_base"/>
  </joint>
  <joint name="lj0" type="prismatic">
    <origin xyz="0.1 0 0" rpy="0 0 0"/>
    <parent link="arm_base"/>
    <child link="shoulder"/>
    <axis xyz="1 0 0"/>
    <limit lower="0" upper="0.2" effort="1" velocity="1"/>
  </joint>
</robot>"#;
        let model = Model::from_str(urdf).unwrap();
        assert_eq!(model.root_link(), "root");
        assert_eq!(model.joint_count(), 0);
        let transforms = model.resolve(&[]).unwrap();
        assert_eq!(transforms.len(), 3);
        let shoulder = transforms
            .iter()
            .find(|t| t.link_name == "shoulder")
            .expect("prismatic child reachable");
        let m = Mat4::from_cols_array(&shoulder.transform);
        assert!(
            m.w_axis
                .truncate()
                .abs_diff_eq(Vec3::new(0.1, 0.0, 0.0), 1e-5)
        );

        let identity = model.resolve_identity_pose();
        assert_eq!(identity.len(), transforms.len());
        for link in &identity {
            let m = Mat4::from_cols_array(&link.transform);
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6));
        }
    }

    #[test]
    fn chopped_urdf_identity_pose_applies_cad_to_base() {
        let model = Model::from_str(
            r#"<robot name="chopped_urdf_v1">
  <link name="root"/>
  <link name="base_extrusion__base_extrusion">
    <visual><geometry><mesh filename="draco/Base.glb"/></geometry></visual>
  </link>
  <joint name="fixed_node_to_root_joint" type="fixed">
    <origin xyz="0 0 0" rpy="3.14159 0 0"/>
    <parent link="root"/>
    <child link="base_extrusion__base_extrusion"/>
  </joint>
</robot>"#,
        )
        .expect("chopped stub");
        let links = model.resolve_identity_pose();
        assert!(!links.is_empty());
        for link in &links {
            let m = Mat4::from_cols_array(&link.transform);
            assert!(
                m.abs_diff_eq(BRACKETBOT_CAD_TO_BASE_LINK, 1e-5),
                "link {} transform {:?}",
                link.link_name,
                link.transform
            );
        }
        // Head-ish CAD point (0,0,-1) → base +Z.
        let p = BRACKETBOT_CAD_TO_BASE_LINK.transform_point3(Vec3::new(0.0, 0.0, -1.0));
        assert!(p.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-5));
        // Right-wheel-ish CAD (0,+y,0) → base +X.
        let p = BRACKETBOT_CAD_TO_BASE_LINK.transform_point3(Vec3::new(0.0, 0.2, 0.0));
        assert!(p.abs_diff_eq(Vec3::new(0.2, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn fixed_axis_xyz_rpy_composition() {
        // Tiny synthetic URDF exercising rpy composition independent of
        // K1. roll=π/2 (X), pitch=0, yaw=0: the joint frame's Y axis
        // should land on world-Z.
        let urdf = r#"<?xml version="1.0"?>
<robot name="test">
  <link name="A"/>
  <link name="B"/>
  <joint name="j" type="revolute">
    <origin xyz="0 0 0" rpy="1.5707963 0 0"/>
    <parent link="A"/>
    <child link="B"/>
    <axis xyz="0 0 1"/>
    <limit lower="-1" upper="1" effort="1" velocity="1"/>
  </joint>
</robot>"#;
        let model = Model::from_str(urdf).unwrap();
        let transforms = model.resolve(&[0.0]).unwrap();
        let b = transforms.iter().find(|t| t.link_name == "B").unwrap();
        let m = Mat4::from_cols_array(&b.transform);
        // Original Y axis (0,1,0) of frame B sits at +Z in frame A.
        let y_in_a = m.transform_vector3(Vec3::Y);
        assert!(
            y_in_a.abs_diff_eq(Vec3::Z, 1e-5),
            "rpy=(π/2,0,0) should map Y→Z, got {y_in_a:?}"
        );
    }
}
