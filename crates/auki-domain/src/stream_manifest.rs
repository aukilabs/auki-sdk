//! Registry-backed stream manifest construction for producers.
//!
//! `auki-network` treats [`StreamManifest`] as an already-formed wire
//! payload. This module is the domain-layer bridge that looks up the
//! producer's Sensor Registry entry, copies any committed frame reference,
//! and verifies the exact Frame Registry entry exists before an Accept
//! response is built.

use std::io;
use std::path::Path;

use auki_network::stream_protocol::StreamManifest;
use auki_registry::{SensorBody, read_frame, read_sensor};

/// Builds stream manifests from registry entries.
///
/// The builder performs no discovery and no directory scanning: callers
/// provide the exact sensor and clock ids/hashes they intend to publish, and
/// frame-bearing sensor bodies must already contain a resolvable
/// `(frame_id, frame_hash)` pair.
pub struct StreamManifestBuilder;

impl StreamManifestBuilder {
    /// Build a [`StreamManifest`] from the producer's local registry.
    ///
    /// `RgbCamera` and `PointCloud` sensor bodies copy their committed
    /// `frame_id` / `frame_hash` into the manifest after verifying the exact
    /// frame entry exists. `Audio` and `JointEncoders` are non-spatial and
    /// produce empty frame fields.
    pub fn from_registry(
        app_root: &Path,
        sensor_id: impl Into<String>,
        sensor_hash: impl Into<String>,
        clock_id: impl Into<String>,
        clock_hash: impl Into<String>,
    ) -> Result<StreamManifest, BuildStreamManifestError> {
        let sensor_id = sensor_id.into();
        let sensor_hash = sensor_hash.into();

        let entry = read_sensor(app_root, &sensor_id, &sensor_hash)?.ok_or_else(|| {
            BuildStreamManifestError::SensorEntryMissing {
                sensor_id: sensor_id.clone(),
                sensor_hash: sensor_hash.clone(),
            }
        })?;

        let (frame_id, frame_hash) = match entry.body {
            SensorBody::RgbCamera(b) => {
                spatial_frame_fields(sensor_id.clone(), b.frame_id, b.frame_hash)?
            }
            SensorBody::PointCloud(b) => {
                spatial_frame_fields(sensor_id.clone(), b.frame_id, b.frame_hash)?
            }
            SensorBody::Audio(_) | SensorBody::JointEncoders(_) => (String::new(), String::new()),
        };

        if !frame_id.is_empty() && read_frame(app_root, &frame_id, &frame_hash)?.is_none() {
            return Err(BuildStreamManifestError::FrameEntryMissing {
                frame_id,
                frame_hash,
            });
        }

        Ok(StreamManifest {
            sensor_id,
            sensor_hash,
            clock_id: clock_id.into(),
            clock_hash: clock_hash.into(),
            frame_id,
            frame_hash,
        })
    }
}

fn spatial_frame_fields(
    sensor_id: String,
    frame_id: String,
    frame_hash: String,
) -> Result<(String, String), BuildStreamManifestError> {
    if frame_id.is_empty() {
        return Err(BuildStreamManifestError::FrameIdMissing { sensor_id });
    }
    if frame_hash.is_empty() {
        return Err(BuildStreamManifestError::FrameHashMissing {
            sensor_id,
            frame_id,
        });
    }
    Ok((frame_id, frame_hash))
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
    /// A frame-bearing sensor body had no frame id.
    #[error("frame id missing for spatial sensor {sensor_id:?}")]
    FrameIdMissing {
        /// Sensor Registry id whose body was incomplete.
        sensor_id: String,
    },
    /// A frame-bearing sensor body had a frame id but no frame hash.
    #[error("frame hash missing for spatial sensor {sensor_id:?} frame {frame_id:?}")]
    FrameHashMissing {
        /// Sensor Registry id whose body was incomplete.
        sensor_id: String,
        /// Frame id present on the sensor body.
        frame_id: String,
    },
    /// The frame-bearing sensor references a frame entry that is not on disk.
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
        Audio, FrameRegistryEntry, PointCloud, PointField, PointFieldDataType, SensorRegistryEntry,
        WriteOutcome, write_frame, write_sensor,
    };

    const FRAME_ID: &str = "K1-AABBCCDDEEFF/head_left_cam_optical";

    fn write_frame_fixture(app_root: &Path) -> String {
        let frame = FrameRegistryEntry::ros_optical(FRAME_ID);
        match write_frame(app_root, &frame).unwrap() {
            WriteOutcome::Created(hash) | WriteOutcome::AlreadyExists(hash) => hash,
        }
    }

    fn point_cloud_sensor(frame_hash: impl Into<String>) -> SensorRegistryEntry {
        SensorRegistryEntry {
            sensor_id: "K1-AABBCCDDEEFF/head_depth_points".into(),
            body: SensorBody::PointCloud(PointCloud {
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
                frame_rate_hz: 10,
                frame_id: FRAME_ID.into(),
                frame_hash: frame_hash.into(),
            }),
        }
    }

    fn audio_sensor() -> SensorRegistryEntry {
        SensorRegistryEntry {
            sensor_id: "K1-AABBCCDDEEFF/head_array".into(),
            body: SensorBody::Audio(Audio {
                sample_rate_hz: 48_000,
                channels: 4,
                sample_format: "pcm_s16le".into(),
                channel_layout: "n_channel".into(),
            }),
        }
    }

    fn write_sensor_bypassing_validation(app_root: &Path, entry: &SensorRegistryEntry) -> String {
        let hash = entry.hash();
        let path = auki_layout::sensor_entry_path(app_root, &entry.sensor_id, &hash);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, entry.canonical_bytes()).unwrap();
        hash
    }

    #[test]
    fn from_registry_builds_spatial_manifest_with_frame_fields() {
        let dir = tempfile::tempdir().unwrap();
        let frame_hash = write_frame_fixture(dir.path());
        let entry = point_cloud_sensor(frame_hash.clone());
        let sensor_hash = write_sensor(dir.path(), &entry).unwrap().hash().to_string();

        let manifest = StreamManifestBuilder::from_registry(
            dir.path(),
            &entry.sensor_id,
            &sensor_hash,
            "K1-AABBCCDDEEFF/monotonic",
            "clock-hash",
        )
        .unwrap();

        assert_eq!(manifest.sensor_id, entry.sensor_id);
        assert_eq!(manifest.sensor_hash, sensor_hash);
        assert_eq!(manifest.clock_id, "K1-AABBCCDDEEFF/monotonic");
        assert_eq!(manifest.clock_hash, "clock-hash");
        assert_eq!(manifest.frame_id, FRAME_ID);
        assert_eq!(manifest.frame_hash, frame_hash);
    }

    #[test]
    fn from_registry_builds_non_spatial_manifest_with_empty_frame_fields() {
        let dir = tempfile::tempdir().unwrap();
        let entry = audio_sensor();
        let sensor_hash = write_sensor(dir.path(), &entry).unwrap().hash().to_string();

        let manifest = StreamManifestBuilder::from_registry(
            dir.path(),
            &entry.sensor_id,
            &sensor_hash,
            "K1-AABBCCDDEEFF/audio_clock",
            "clock-hash",
        )
        .unwrap();

        assert_eq!(manifest.frame_id, "");
        assert_eq!(manifest.frame_hash, "");
    }

    #[test]
    fn from_registry_errors_when_sensor_entry_missing() {
        let dir = tempfile::tempdir().unwrap();
        let err = StreamManifestBuilder::from_registry(
            dir.path(),
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
        let mut entry = point_cloud_sensor("frame-hash");
        match &mut entry.body {
            SensorBody::PointCloud(body) => body.frame_id.clear(),
            _ => unreachable!(),
        }
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
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
        let entry = point_cloud_sensor("");
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
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
        let entry = point_cloud_sensor("not-on-disk");
        let sensor_hash = write_sensor_bypassing_validation(dir.path(), &entry);

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
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
        let entry = point_cloud_sensor("frame-hash");
        let sensor_hash = entry.hash();
        let path = auki_layout::sensor_entry_path(dir.path(), &entry.sensor_id, &sensor_hash);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"{").unwrap();

        let err = StreamManifestBuilder::from_registry(
            dir.path(),
            &entry.sensor_id,
            sensor_hash,
            "clock",
            "clock-hash",
        )
        .unwrap_err();

        assert!(matches!(err, BuildStreamManifestError::Registry(_)));
    }
}
