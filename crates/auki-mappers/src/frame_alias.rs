//! Explicitly validated identity-only aliases between equivalent SDK frames.

use auki_registry::{FrameRegistryEntry, RegistryRef};

/// A source frame may be rebound to an independently owned target identity
/// only when both registry entries declare the exact same coordinate
/// convention. This does not transform coordinates; it only makes that
/// identity-only relationship explicit and auditable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedFrameAlias {
    source: RegistryRef,
    target: RegistryRef,
}

/// Explicit relationship between a pose destination and the Map frame used
/// while constructing a voxel Mapper runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoxelMapperMapFrameBinding {
    /// The pose destination is the exact Map frame.
    Exact(RegistryRef),
    /// The pose destination is explicitly rebound to an equivalent Map-owned
    /// frame.
    Aliased(ValidatedFrameAlias),
}

impl VoxelMapperMapFrameBinding {
    pub(crate) fn pose_frame(&self) -> &RegistryRef {
        match self {
            Self::Exact(frame) => frame,
            Self::Aliased(alias) => alias.source(),
        }
    }

    pub(crate) fn matches_map(&self, map_frame: &RegistryRef) -> bool {
        match self {
            Self::Exact(frame) => frame == map_frame,
            Self::Aliased(alias) => alias.target() == map_frame,
        }
    }
}

impl ValidatedFrameAlias {
    /// Validate both content-addressed identities and their coordinate
    /// conventions before constructing an alias.
    pub fn new(
        source: RegistryRef,
        source_entry: &FrameRegistryEntry,
        target: RegistryRef,
        target_entry: &FrameRegistryEntry,
    ) -> Result<Self, FrameAliasError> {
        if source != registry_ref(source_entry) {
            return Err(FrameAliasError::SourceIdentityMismatch);
        }
        if target != registry_ref(target_entry) {
            return Err(FrameAliasError::TargetIdentityMismatch);
        }
        if source_entry.handedness != target_entry.handedness
            || source_entry.axes != target_entry.axes
            || source_entry.units != target_entry.units
        {
            return Err(FrameAliasError::ConventionMismatch);
        }
        Ok(Self { source, target })
    }

    /// Frame identity carried by the selected pose destination.
    pub fn source(&self) -> &RegistryRef {
        &self.source
    }

    /// Equivalent frame identity owned by the Map publisher.
    pub fn target(&self) -> &RegistryRef {
        &self.target
    }
}

fn registry_ref(entry: &FrameRegistryEntry) -> RegistryRef {
    RegistryRef {
        peer_id: entry.peer_id.clone(),
        id: entry.frame_id.clone(),
        hash: entry.hash(),
    }
}

/// A requested frame alias is not an identity-preserving rebind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameAliasError {
    /// The supplied source reference does not identify the supplied entry.
    #[error("source frame reference does not match its registry entry")]
    SourceIdentityMismatch,
    /// The supplied target reference does not identify the supplied entry.
    #[error("target frame reference does not match its registry entry")]
    TargetIdentityMismatch,
    /// Aliasing would require a coordinate conversion rather than a rename.
    #[error("source and target frame conventions are not equivalent")]
    ConventionMismatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(entry: &FrameRegistryEntry) -> RegistryRef {
        registry_ref(entry)
    }

    #[test]
    fn accepts_equivalent_conventions_with_independent_owners() {
        let source = FrameRegistryEntry::ros_body("bracketbot", "local_world");
        let target = FrameRegistryEntry::ros_body("park", "voxel/world");
        let alias =
            ValidatedFrameAlias::new(reference(&source), &source, reference(&target), &target)
                .unwrap();
        assert_eq!(alias.source().peer_id, "bracketbot");
        assert_eq!(alias.target().peer_id, "park");
    }

    #[test]
    fn rejects_convention_and_content_identity_mismatches() {
        let source = FrameRegistryEntry::ros_body("bracketbot", "local_world");
        let target = FrameRegistryEntry::opengl("park", "voxel/world");
        assert_eq!(
            ValidatedFrameAlias::new(reference(&source), &source, reference(&target), &target,),
            Err(FrameAliasError::ConventionMismatch)
        );

        let mut stale = reference(&source);
        stale.hash = "stale".into();
        assert_eq!(
            ValidatedFrameAlias::new(stale, &source, reference(&source), &source),
            Err(FrameAliasError::SourceIdentityMismatch)
        );
    }
}
