//! Native immutable-filesystem provider for Registry v3.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use auki_registry::{
    ClockRegistryEntry, DetectorRegistryEntry, DeviceModelRegistryEntry, FrameRegistryEntry,
    MapRegistryEntry, SensorRegistryEntry,
};
use auki_sdk::{AuthenticatedPeer, PeerId};

use super::endpoint::RegistryProvider;
use super::v3::{
    RegistryEntryEnvelope, RegistryKind, RegistryListEntry, RegistryRequest, RegistryResponse,
};

/// Read-only Registry v3 provider rooted at one application directory.
///
/// The root and owner cannot be changed after construction. Every Get is
/// resolved beneath that root for `local_peer_id`; malformed keys are rejected
/// before path construction, and decoded owner/id/content hashes are checked
/// again before bytes are exposed to an authenticated requester.
#[derive(Clone, Debug)]
pub struct FsRegistryProvider {
    app_root: PathBuf,
    local_peer_id: PeerId,
}

impl FsRegistryProvider {
    /// Bind one immutable application root to one local registry owner.
    pub fn new(app_root: impl Into<PathBuf>, local_peer_id: PeerId) -> Self {
        Self {
            app_root: app_root.into(),
            local_peer_id,
        }
    }

    /// Borrow the fixed application root.
    pub fn app_root(&self) -> &Path {
        &self.app_root
    }

    /// Borrow the fixed local registry owner.
    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    fn respond_to(&self, request: &RegistryRequest) -> RegistryResponse {
        match request {
            RegistryRequest::Get { kind, id, hash } => match self.read_envelope(*kind, id, hash) {
                Ok(entry) => RegistryResponse::Get { entry },
                Err(error) => RegistryResponse::Error {
                    reason: error.remote_reason().into(),
                },
            },
            RegistryRequest::List {
                kind: RegistryKind::DeviceModel,
            } => self.list_device_models(),
            RegistryRequest::List { .. } => RegistryResponse::Error {
                reason: "list is only implemented for device_model".into(),
            },
        }
    }

    fn read_envelope(
        &self,
        kind: RegistryKind,
        id: &str,
        hash: &str,
    ) -> Result<Option<RegistryEntryEnvelope>, FsRegistryReadError> {
        validate_key(id, hash)?;
        let owner = self.local_peer_id.to_string();
        match kind {
            RegistryKind::Sensor => {
                read_entry(&self.app_root, &owner, id, hash, auki_registry::read_sensor)
            }
            RegistryKind::Clock => {
                read_entry(&self.app_root, &owner, id, hash, auki_registry::read_clock)
            }
            RegistryKind::Frame => {
                read_entry(&self.app_root, &owner, id, hash, auki_registry::read_frame)
            }
            RegistryKind::Detector => read_entry(
                &self.app_root,
                &owner,
                id,
                hash,
                auki_registry::read_detector,
            ),
            RegistryKind::Map => {
                read_entry(&self.app_root, &owner, id, hash, auki_registry::read_map)
            }
            RegistryKind::DeviceModel => read_entry(
                &self.app_root,
                &owner,
                id,
                hash,
                auki_registry::read_device_model,
            ),
        }
    }

    fn list_device_models(&self) -> RegistryResponse {
        let owner = self.local_peer_id.to_string();
        match auki_registry::list_device_models(&self.app_root, &owner) {
            Ok(entries) => {
                let entries = entries
                    .into_iter()
                    .map(|entry| {
                        validate_key(&entry.id, &entry.hash).map(|()| RegistryListEntry {
                            id: entry.id,
                            hash: entry.hash,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>();
                match entries {
                    Ok(entries) => RegistryResponse::List { entries },
                    Err(_) => RegistryResponse::Error {
                        reason: "list failed".into(),
                    },
                }
            }
            Err(auki_registry::Error::RegistryListLimit) => RegistryResponse::Error {
                reason: "list too large".into(),
            },
            Err(_) => RegistryResponse::Error {
                reason: "list failed".into(),
            },
        }
    }
}

impl RegistryProvider for FsRegistryProvider {
    fn respond(
        &self,
        _requester: &AuthenticatedPeer,
        request: &RegistryRequest,
    ) -> RegistryResponse {
        self.respond_to(request)
    }
}

fn validate_key(id: &str, hash: &str) -> Result<(), FsRegistryReadError> {
    auki_registry::validate_registry_id(id).map_err(|_| FsRegistryReadError::InvalidRequest)?;
    if !auki_registry::is_registry_entry_hash(hash) {
        return Err(FsRegistryReadError::InvalidRequest);
    }
    Ok(())
}

fn read_entry<T>(
    app_root: &Path,
    owner: &str,
    id: &str,
    expected_hash: &str,
    read: impl FnOnce(&Path, &str, &str, &str) -> auki_registry::Result<Option<T>>,
) -> Result<Option<RegistryEntryEnvelope>, FsRegistryReadError>
where
    T: FsRegistryEntry,
{
    let Some(entry) = read(app_root, owner, id, expected_hash).map_err(map_registry_error)? else {
        return Ok(None);
    };
    if entry.owner_peer_id() != owner || entry.registry_id() != id {
        return Err(FsRegistryReadError::Integrity);
    }
    let canonical_bytes = entry.canonical_bytes();
    let actual_hash = auki_hash::hash_jcs_bytes(&canonical_bytes);
    if actual_hash != expected_hash {
        return Err(FsRegistryReadError::Integrity);
    }
    let canonical_json =
        String::from_utf8(canonical_bytes).map_err(|_| FsRegistryReadError::Integrity)?;
    Ok(Some(RegistryEntryEnvelope {
        kind: T::KIND,
        id: id.into(),
        hash: expected_hash.into(),
        canonical_json,
    }))
}

fn map_registry_error(error: auki_registry::Error) -> FsRegistryReadError {
    match error {
        auki_registry::Error::IdMismatch { .. } => FsRegistryReadError::Integrity,
        _ => FsRegistryReadError::Storage,
    }
}

trait FsRegistryEntry {
    const KIND: RegistryKind;

    fn owner_peer_id(&self) -> &str;
    fn registry_id(&self) -> &str;
    fn canonical_bytes(&self) -> Vec<u8>;
}

macro_rules! impl_fs_registry_entry {
    ($entry:ty, $kind:expr, $id:ident) => {
        impl FsRegistryEntry for $entry {
            const KIND: RegistryKind = $kind;

            fn owner_peer_id(&self) -> &str {
                &self.peer_id
            }

            fn registry_id(&self) -> &str {
                &self.$id
            }

            fn canonical_bytes(&self) -> Vec<u8> {
                <$entry>::canonical_bytes(self)
            }
        }
    };
}

impl_fs_registry_entry!(SensorRegistryEntry, RegistryKind::Sensor, sensor_id);
impl_fs_registry_entry!(ClockRegistryEntry, RegistryKind::Clock, clock_id);
impl_fs_registry_entry!(FrameRegistryEntry, RegistryKind::Frame, frame_id);
impl_fs_registry_entry!(DetectorRegistryEntry, RegistryKind::Detector, detector_id);
impl_fs_registry_entry!(MapRegistryEntry, RegistryKind::Map, map_id);
impl_fs_registry_entry!(
    DeviceModelRegistryEntry,
    RegistryKind::DeviceModel,
    device_model_id
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FsRegistryReadError {
    InvalidRequest,
    Integrity,
    Storage,
}

impl FsRegistryReadError {
    fn remote_reason(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid registry request",
            Self::Integrity => "registry entry integrity check failed",
            Self::Storage => "registry read failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use auki_p2p::Identity;
    use auki_registry::{
        DeviceModelBody, DeviceModelFormat, DeviceModelRegistryEntry, FrameRegistryEntry,
    };

    use super::*;

    fn peer(seed: u8) -> PeerId {
        Identity::from_ed25519_seed(&[seed; 32]).peer_id()
    }

    #[test]
    fn gets_only_exact_content_owned_by_the_fixed_peer() {
        let root = tempfile::tempdir().unwrap();
        let owner = peer(1);
        let entry = FrameRegistryEntry::ros_body(owner.to_string(), "base");
        let hash = auki_registry::write_frame(root.path(), &entry)
            .unwrap()
            .hash()
            .to_owned();
        let provider = FsRegistryProvider::new(root.path(), owner);

        let response =
            provider.respond_to(&RegistryRequest::get(RegistryKind::Frame, "base", &hash));
        let RegistryResponse::Get { entry: Some(found) } = response else {
            panic!("expected exact local entry");
        };
        assert_eq!(found.kind, RegistryKind::Frame);
        assert_eq!(found.id, "base");
        assert_eq!(found.hash, hash);
        assert_eq!(
            found.canonical_json,
            String::from_utf8(entry.canonical_bytes()).unwrap()
        );

        assert_eq!(
            provider.respond_to(&RegistryRequest::get(
                RegistryKind::Frame,
                "missing",
                "0".repeat(32),
            )),
            RegistryResponse::Get { entry: None }
        );
    }

    #[test]
    fn rejects_unsafe_keys_and_planted_owner_or_hash_mismatches() {
        let root = tempfile::tempdir().unwrap();
        let owner = peer(2);
        let provider = FsRegistryProvider::new(root.path(), owner);

        assert_eq!(
            provider.respond_to(&RegistryRequest::get(
                RegistryKind::Frame,
                "../outside",
                "0".repeat(32),
            )),
            RegistryResponse::Error {
                reason: "invalid registry request".into(),
            }
        );

        let other = FrameRegistryEntry::ros_body(peer(3).to_string(), "base");
        let other_bytes = other.canonical_bytes();
        let other_hash = auki_hash::hash_jcs_bytes(&other_bytes);
        let planted = root
            .path()
            .join("registries/frames")
            .join(owner.to_string())
            .join("base")
            .join(format!("{other_hash}.json"));
        std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
        std::fs::write(&planted, other_bytes).unwrap();
        assert_eq!(
            provider.respond_to(&RegistryRequest::get(
                RegistryKind::Frame,
                "base",
                &other_hash,
            )),
            RegistryResponse::Error {
                reason: "registry entry integrity check failed".into(),
            }
        );

        let original = FrameRegistryEntry::ros_body(owner.to_string(), "world");
        let original_hash = auki_hash::hash_jcs_bytes(&original.canonical_bytes());
        let replacement = FrameRegistryEntry::opengl(owner.to_string(), "world");
        let planted = root
            .path()
            .join("registries/frames")
            .join(owner.to_string())
            .join("world")
            .join(format!("{original_hash}.json"));
        std::fs::create_dir_all(planted.parent().unwrap()).unwrap();
        std::fs::write(&planted, replacement.canonical_bytes()).unwrap();
        assert_eq!(
            provider.respond_to(&RegistryRequest::get(
                RegistryKind::Frame,
                "world",
                &original_hash,
            )),
            RegistryResponse::Error {
                reason: "registry entry integrity check failed".into(),
            }
        );
    }

    #[test]
    fn lists_only_device_model_tips_for_the_fixed_peer() {
        let root = tempfile::tempdir().unwrap();
        let owner = peer(4);
        let urdf_sha256 = auki_registry::put_blob(root.path(), b"<robot name='g1'/>").unwrap();
        let entry = DeviceModelRegistryEntry {
            peer_id: owner.to_string(),
            device_model_id: "unitree/g1".into(),
            body: DeviceModelBody {
                model_id: "g1".into(),
                format: DeviceModelFormat::Urdf {
                    urdf_sha256,
                    meshes: Vec::new(),
                },
                root_convention: Some("ros_body".into()),
            },
        };
        let hash = auki_registry::write_device_model(root.path(), &entry)
            .unwrap()
            .hash()
            .to_owned();
        let provider = FsRegistryProvider::new(root.path(), owner);

        assert_eq!(
            provider.respond_to(&RegistryRequest::list(RegistryKind::DeviceModel)),
            RegistryResponse::List {
                entries: vec![RegistryListEntry {
                    id: "unitree/g1".into(),
                    hash,
                }],
            }
        );
        assert_eq!(
            provider.respond_to(&RegistryRequest::list(RegistryKind::Frame)),
            RegistryResponse::Error {
                reason: "list is only implemented for device_model".into(),
            }
        );
    }
}
