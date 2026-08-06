//! Registry-backed stream manifest construction for producers.
//!
//! `auki-network` treats [`StreamManifest`] as an already-formed wire
//! payload. This module is the domain-layer bridge that looks up the
//! producer's Sensor Registry entry, copies and verifies its committed frame
//! reference when the sensor is spatial, and leaves frame fields empty for
//! explicitly non-spatial sensor kinds such as Scalar.
//!
//! Spatial sensor bodies carry a `frame: RegistryRef` with `peer_id`. Scalar
//! bodies deliberately do not invent a coordinate frame.

use std::io;
use std::path::Path;

use auki_network::stream_protocol::StreamManifest;
use auki_registry::{SensorBody, read_frame, read_sensor};

/// Builds stream manifests from registry entries.
///
/// The builder performs no discovery and no directory scanning: callers
/// provide the exact sensor and clock ids/hashes they intend to publish, and
/// every spatial sensor body must already contain a resolvable
/// `frame: RegistryRef`.
pub struct StreamManifestBuilder;

impl StreamManifestBuilder {
    /// Build a [`StreamManifest`] from the producer's local registry.
    ///
    /// Spatial sensor bodies contribute `frame_id` / `frame_hash` from their
    /// `frame: RegistryRef`; non-spatial Scalar sensors leave both empty. The `peer_id`
    /// used for the sensor read is `sensor_peer_id`; the `peer_id` used
    /// for the frame read is the `peer_id` embedded in `body.frame`.
    pub fn from_registry(
        app_root: &Path,
        sensor_peer_id: impl AsRef<str>,
        sensor_id: impl Into<String>,
        sensor_hash: impl Into<String>,
        clock_id: impl Into<String>,
        clock_hash: impl Into<String>,
    ) -> Result<StreamManifest, BuildStreamManifestError> {
        let sensor_peer_id = sensor_peer_id.as_ref();
        let sensor_id = sensor_id.into();
        let sensor_hash = sensor_hash.into();

        let entry =
            read_sensor(app_root, sensor_peer_id, &sensor_id, &sensor_hash)?.ok_or_else(|| {
                BuildStreamManifestError::SensorEntryMissing {
                    sensor_id: sensor_id.clone(),
                    sensor_hash: sensor_hash.clone(),
                }
            })?;

        let frame_ref = match &entry.body {
            SensorBody::Camera(b) => Some(&b.frame),
            SensorBody::Rangefinder(b) => Some(&b.frame),
            SensorBody::Rf(b) => Some(&b.frame),
            SensorBody::Audio(b) => Some(&b.frame),
            SensorBody::JointEncoders(b) => Some(&b.frame),
            SensorBody::Scalar(_) => None,
        };

        let (frame_id, frame_hash) = if let Some(frame_ref) = frame_ref {
            let frame_id = frame_ref.id.clone();
            let frame_hash = frame_ref.hash.clone();
            let frame_peer_id = frame_ref.peer_id.clone();

            if frame_id.is_empty() {
                return Err(BuildStreamManifestError::FrameIdMissing { sensor_id });
            }
            if frame_hash.is_empty() {
                return Err(BuildStreamManifestError::FrameHashMissing {
                    sensor_id,
                    frame_id,
                });
            }
            if read_frame(app_root, &frame_peer_id, &frame_id, &frame_hash)?.is_none() {
                return Err(BuildStreamManifestError::FrameEntryMissing {
                    frame_id,
                    frame_hash,
                });
            }
            (frame_id, frame_hash)
        } else {
            (String::new(), String::new())
        };

        Ok(StreamManifest {
            resource_id: sensor_id.clone(),
            sensor_id,
            sensor_hash,
            clock_peer_id: sensor_peer_id.to_string(),
            clock_id: clock_id.into(),
            clock_hash: clock_hash.into(),
            frame_id,
            frame_hash,
            ..Default::default()
        })
    }
}

/// Errors returned while building a registry-backed stream manifest.
#[derive(Debug, thiserror::Error)]
pub enum BuildStreamManifestError {
    /// The exact sensor entry requested by the producer is not present.
    #[error("sensor registry entry missing for ({sensor_id:?}, {sensor_hash:?})")]
    SensorEntryMissing {
        /// Sensor Registry id requested by the producer.
        sensor_id: String,
        /// Sensor Registry hash requested by the producer.
        sensor_hash: String,
    },
    /// A sensor body had an empty frame id.
    #[error("frame id missing for sensor {sensor_id:?}")]
    FrameIdMissing {
        /// Sensor Registry id whose body was incomplete.
        sensor_id: String,
    },
    /// A sensor body had a frame id but an empty frame hash.
    #[error("frame hash missing for sensor {sensor_id:?} frame {frame_id:?}")]
    FrameHashMissing {
        /// Sensor Registry id whose body was incomplete.
        sensor_id: String,
        /// Frame id present on the sensor body.
        frame_id: String,
    },
    /// The sensor references a frame entry that is not on disk.
    #[error("frame registry entry missing for ({frame_id:?}, {frame_hash:?})")]
    FrameEntryMissing {
        /// Frame Registry id referenced by the sensor body.
        frame_id: String,
        /// Frame Registry hash referenced by the sensor body.
        frame_hash: String,
    },
    /// Underlying registry I/O failed.
    #[error("io: {0}")]
    Io(#[source] io::Error),
    /// Underlying registry JSON/id validation failed.
    #[error("registry: {0}")]
    Registry(#[source] auki_registry::Error),
}

impl From<auki_registry::Error> for BuildStreamManifestError {
    fn from(err: auki_registry::Error) -> Self {
        match err {
            auki_registry::Error::Io(err) => BuildStreamManifestError::Io(err),
            err => BuildStreamManifestError::Registry(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use auki_registry::{
        FrameRegistryEntry, PointField, PointFieldDataType, Rangefinder, RegistryRef, Scalar,
        SensorRegistryEntry, WriteOutcome, write_frame, write_sensor,
    };

    const PEER_ID: &str = "K1-AABBCCDDEEFF";
    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";

    fn write_frame_fixture(app_root: &Path) -> String {
        let frame = FrameRegistryEntry::ros_optical(PEER_ID, FRAME_ID);
        match write_frame(app_root, &frame).unwrap() {
            WriteOutcome::Created(hash) | WriteOutcome::AlreadyExists(hash) => hash,
        }
    }

    fn rangefinder_sensor(frame_hash: impl Into<String>) -> SensorRegistryEntry {
        SensorRegistryEntry {
            peer_id: PEER_ID.into(),
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::Rangefinder(Rangefinder {
                r#type: "point_cloud".into(),
                fields: vec![PointField {
                    name: "x".into(),
                    offset: 0,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                }],
                point_step: 4,
                is_bigendian: false,
                frame_rate_hz: 10,
                frame: RegistryRef {
                    peer_id: PEER_ID.into(),
                    id: FRAME_ID.into(),
                    hash: frame_hash.into(),
                },
            }),
        }
    }

    fn write_sensor_bypassing_validation(app_root: &Path, entry: &SensorRegistryEntry) -> String {
        let hash = entry.hash();
        let path =
            auki_layout::sensor_entry_path(app_root, &entry.peer_id, &entry.sensor_id, &hash);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, entry.canonical_bytes()).unwrap();
        hash
    }

    #[test]
    fn from_registry_builds_spatial_manifest_with_frame_fields() {
        let dir = tempfile::tempdir().unwrap();
        let frame_hash = write_frame_fixture(dir.path());
        let entry = rangefinder_sensor(frame_hash.clone());
        let sensor_hash = write_sensor(dir.path(), &entry).unwrap().hash().to_string();

        let manifest = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            &sensor_hash,
            "K1-AABBCCDDEEFF/monotonic",
            "clock-hash",
        )
        .unwrap();

        assert_eq!(manifest.sensor_id, entry.sensor_id);
        assert_eq!(manifest.resource_id, entry.sensor_id);
        assert_eq!(manifest.sensor_hash, sensor_hash);
        assert_eq!(manifest.clock_peer_id, PEER_ID);
        assert_eq!(manifest.clock_id, "K1-AABBCCDDEEFF/monotonic");
        assert_eq!(manifest.clock_hash, "clock-hash");
        assert_eq!(manifest.frame_id, FRAME_ID);
        assert_eq!(manifest.frame_hash, frame_hash);
    }

    #[test]
    fn from_registry_builds_non_spatial_scalar_manifest_without_frame_fields() {
        let dir = tempfile::tempdir().unwrap();
        let entry = SensorRegistryEntry {
            peer_id: PEER_ID.into(),
            sensor_id: "K1-AABBCCDDEEFF/battery_charge".into(),
            body: SensorBody::Scalar(Scalar {
                r#type: "battery_charge".into(),
                unit: "percent".into(),
                expected_rate_hz: 1,
            }),
        };
        let sensor_hash = write_sensor(dir.path(), &entry).unwrap().hash().to_string();

        let manifest = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            &sensor_hash,
            "K1-AABBCCDDEEFF/monotonic",
            "clock-hash",
        )
        .unwrap();

        assert_eq!(manifest.sensor_id, entry.sensor_id);
        assert_eq!(manifest.sensor_hash, sensor_hash);
        assert!(manifest.frame_id.is_empty());
        assert!(manifest.frame_hash.is_empty());
    }

    #[test]
    fn from_registry_errors_when_sensor_entry_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            "missing/sensor",
            "missing-hash",
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            BuildStreamManifestError::SensorEntryMissing { .. }
        ));
    }

    #[test]
    fn from_registry_errors_when_frame_id_missing() {
        let dir = tempfile::tempdir().unwrap();
        let mut entry = rangefinder_sensor("frame-hash");
        match &mut entry.body {
            SensorBody::Rangefinder(body) => body.frame.id.clear(),
            _ => unreachable!(),
        }
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            sensor_hash,
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            BuildStreamManifestError::FrameIdMissing { .. }
        ));
    }

    #[test]
    fn from_registry_errors_when_frame_hash_missing() {
        let dir = tempfile::tempdir().unwrap();
        let entry = rangefinder_sensor("");
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            sensor_hash,
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            BuildStreamManifestError::FrameHashMissing { .. }
        ));
    }

    #[test]
    fn from_registry_errors_when_frame_entry_missing() {
        let dir = tempfile::tempdir().unwrap();
        let entry = rangefinder_sensor("not-on-disk");
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            sensor_hash,
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(
            err,
            BuildStreamManifestError::FrameEntryMissing { .. }
        ));
    }

    #[test]
    fn from_registry_surfaces_registry_errors() {
        let dir = tempfile::tempdir().unwrap();
        let entry = rangefinder_sensor("frame-hash");
        let sensor_hash = entry.hash();
        let path = auki_layout::sensor_entry_path(
            dir.path(),
            &entry.peer_id,
            &entry.sensor_id,
            &sensor_hash,
        );
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"{").unwrap();

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            PEER_ID,
            &entry.sensor_id,
            sensor_hash,
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(err, BuildStreamManifestError::Registry(_)));
    }
}
