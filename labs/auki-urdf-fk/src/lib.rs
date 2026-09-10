//! URDF parsing and forward kinematics for articulated robots.
//!
//! Parse an application-supplied robot description with [`Model::from_str`] or
//! [`Model::load`], then resolve link transforms for each joint-angle frame.
//! Robot descriptions and meshes are supplied separately by the application.

pub mod urdf_fk;

pub use urdf_fk::{BRACKETBOT_CAD_TO_BASE_LINK, FkError, LinkTransform, Model};
