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

pub struct SensorLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest. Public so `auki-domain` can build catalog rows; the
    /// sensor kind/type are derived from the peer's sensor registry at
    /// catalog-build time, not stored here. See #274 (D3).
    pub manifest: SensorLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
}

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
    ) -> crate::Result<(
        Vec<auki_logs::Entry<auki_datatypes::map::MapUpdate>>,
        tokio::sync::broadcast::Receiver<(i64, auki_datatypes::map::MapUpdate)>,
    )> {
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

pub struct DetectionLogHandle {
    pub resource_id: String,
    pub log_ref: LogRef,
    /// Full manifest, used for catalog row production.
    pub manifest: DetectionLogManifest,
    /// Head window spec, used for catalog row production.
    pub head_spec: HeadSpec,
    pub(crate) root: PathBuf,
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
}

pub struct MaterializedLogHandle {
    pub log_ref: LogRef,
}
