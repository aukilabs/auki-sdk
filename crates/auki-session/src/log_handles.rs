//! Log handles returned by Session::register_*_log.

use auki_manifests::{
    DetectionLogManifest, MapLogManifest, PoseLogManifest, PoseWriterMode, SensorLogManifest,
    TimeTransformLogManifest,
};
use auki_registry::LogRef;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::log_specs::HeadSpec;

/// A consistent persisted-log snapshot paired with its live append receiver.
pub type LogSnapshot<T> = (
    Vec<auki_logs::Entry<T>>,
    tokio::sync::broadcast::Receiver<(i64, T)>,
);

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest used by local readers and protocol adapters.
    ///
    /// Sensor kind/type are resolved from the peer's Sensor Registry when a
    /// catalog row is projected; they are not duplicated here.
    pub manifest: SensorLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

#[derive(Clone)]
pub struct MapLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    pub manifest: MapLogManifest,
    pub head_spec: HeadSpec,
    root: PathBuf,
    writer: Arc<Mutex<auki_logs::Log<auki_datatypes::map::MapUpdate>>>,
    updates: tokio::sync::broadcast::Sender<(i64, auki_datatypes::map::MapUpdate)>,
}

impl MapLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }

    /// Total bytes currently persisted beneath this Map Log's storage root.
    /// The writer is flushed first, so diagnostics include the durable tail.
    pub fn persisted_bytes(&self) -> crate::Result<u64> {
        self.writer.lock().flush()?;
        Ok(directory_bytes(&self.root)?)
    }

    /// Append one mergeable map update. The timestamp belongs to the map log's
    /// declared clock; callers may be a local or remote-input Mapper.
    pub fn append(
        &self,
        timestamp_ns: i64,
        update: &auki_datatypes::map::MapUpdate,
    ) -> crate::Result<()> {
        let mut writer = self.writer.lock();
        writer.append(timestamp_ns, update)?;
        writer.flush()?;
        // Durability precedes visibility. A missing receiver is expected when
        // nobody currently subscribes and is not an append failure.
        let _ = self.updates.send((timestamp_ns, update.clone()));
        Ok(())
    }

    /// Read the materialized log entries currently persisted on this peer.
    pub fn entries(&self) -> crate::Result<Vec<auki_logs::Entry<auki_datatypes::map::MapUpdate>>> {
        self.writer.lock().flush()?;
        Ok(auki_logs::Log::<auki_datatypes::map::MapUpdate>::read(&self.root)?.entries()?)
    }

    /// Subscribe at the current live end. Entries appended after this call
    /// are delivered after their durable append succeeds.
    pub fn subscribe(
        &self,
    ) -> tokio::sync::broadcast::Receiver<(i64, auki_datatypes::map::MapUpdate)> {
        self.updates.subscribe()
    }

    /// Atomically capture persisted history and subscribe to subsequent
    /// updates. Holding the writer lock across both operations prevents the
    /// replay/live boundary from losing or duplicating an append.
    pub fn snapshot_and_subscribe(
        &self,
    ) -> crate::Result<LogSnapshot<auki_datatypes::map::MapUpdate>> {
        let mut writer = self.writer.lock();
        writer.flush()?;
        let entries =
            auki_logs::Log::<auki_datatypes::map::MapUpdate>::read(&self.root)?.entries()?;
        let receiver = self.updates.subscribe();
        Ok((entries, receiver))
    }

    pub(crate) fn with_writer(
        resource_id: String,
        log_ref: LogRef,
        manifest: MapLogManifest,
        head_spec: HeadSpec,
        root: PathBuf,
        writer: Arc<Mutex<auki_logs::Log<auki_datatypes::map::MapUpdate>>>,
        updates: tokio::sync::broadcast::Sender<(i64, auki_datatypes::map::MapUpdate)>,
    ) -> Self {
        Self {
            resource_id,
            log_ref,
            manifest,
            head_spec,
            root,
            writer,
            updates,
        }
    }
}

fn directory_bytes(root: &Path) -> std::io::Result<u64> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
    }
    Ok(total)
}

impl SensorLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct PoseLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: PoseLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    /// Writer mode (derived from spec).
    pub writer_mode: PoseWriterMode,
    pub(crate) root: PathBuf,
}

impl PoseLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct TimeTransformLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: TimeTransformLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

impl TimeTransformLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Clone)]
pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: DetectionLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
    writer: Arc<Mutex<auki_logs::Log<auki_datatypes::detection::DetectionFrame>>>,
    entries: tokio::sync::broadcast::Sender<(i64, auki_datatypes::detection::DetectionFrame)>,
}

impl DetectionLogHandle {
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }
    pub fn log_ref(&self) -> &LogRef {
        &self.log_ref
    }
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Append one detector output and publish it to live subscribers only
    /// after the durable write succeeds.
    pub fn append(
        &self,
        timestamp_ns: i64,
        frame: &auki_datatypes::detection::DetectionFrame,
    ) -> Result<(), auki_logs::Error> {
        let mut writer = self.writer.lock();
        writer.append(timestamp_ns, frame)?;
        writer.flush()?;
        let _ = self.entries.send((timestamp_ns, frame.clone()));
        Ok(())
    }

    /// Subscribe at the current live end.
    pub fn subscribe(
        &self,
    ) -> tokio::sync::broadcast::Receiver<(i64, auki_datatypes::detection::DetectionFrame)> {
        self.entries.subscribe()
    }

    /// Atomically capture persisted history and subscribe to future entries.
    pub fn snapshot_and_subscribe(
        &self,
    ) -> Result<LogSnapshot<auki_datatypes::detection::DetectionFrame>, auki_logs::Error> {
        let mut writer = self.writer.lock();
        writer.flush()?;
        let history =
            auki_logs::Log::<auki_datatypes::detection::DetectionFrame>::read(&self.root)?
                .entries()?;
        let receiver = self.entries.subscribe();
        Ok((history, receiver))
    }

    pub(crate) fn with_writer(
        resource_id: String,
        log_ref: LogRef,
        manifest: DetectionLogManifest,
        head_spec: HeadSpec,
        root: PathBuf,
        writer: Arc<Mutex<auki_logs::Log<auki_datatypes::detection::DetectionFrame>>>,
        entries: tokio::sync::broadcast::Sender<(i64, auki_datatypes::detection::DetectionFrame)>,
    ) -> Self {
        Self {
            resource_id,
            log_ref,
            manifest,
            head_spec,
            root,
            writer,
            entries,
        }
    }
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use auki_datatypes::map::MapUpdate;
    use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
    use tempfile::tempdir;

    use crate::{FrameDef, HeadSpec, MapLogSpec, Peer};

    #[test]
    fn map_append_is_durable_when_it_becomes_visible() {
        let tmp = tempdir().unwrap();
        let peer = Peer::new("galbot", "test-app").with_storage_root(tmp.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(0.05),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: Vec::new(),
                }),
            )
            .unwrap();
        let handle = session
            .register_map_log(MapLogSpec {
                map,
                clock: session.monotonic_clock(),
                head: HeadSpec::Fixed,
                segment_duration: Duration::from_secs(1),
                retention: Duration::ZERO,
            })
            .unwrap();
        let mut updates = handle.subscribe();

        handle.append(42, &MapUpdate::default()).unwrap();

        assert_eq!(updates.try_recv().unwrap().0, 42);
        let persisted = auki_logs::Log::<MapUpdate>::read(&handle.root)
            .unwrap()
            .entries()
            .unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].timestamp_ns, 42);
    }
}
