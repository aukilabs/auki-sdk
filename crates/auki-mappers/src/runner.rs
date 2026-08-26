//! Live SDK-stream orchestration for the voxel Mapper.

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::Arc,
    thread::{self, JoinHandle},
};

use auki_datatypes::{
    map::MapUpdate,
    point_cloud::Data as PointCloudData,
    pose::{Quat, SpatialTransform, Vec3},
};
use auki_registry::{LogRef, Rangefinder, RegistryRef, VoxelMap};
use futures::{FutureExt, Stream, StreamExt, future::BoxFuture};
use parking_lot::{Condvar, Mutex};

use crate::{
    VoxelMapperMapFrameBinding, VoxelPersistenceConfig, Voxelizer, VoxelizerError,
    persistence::VoxelPersistenceFilter,
};

/// One typed sample received from an SDK log stream.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedSdkSample<T> {
    /// Timestamp expressed in the input log's declared clock.
    pub timestamp_ns: i64,
    /// SDK payload decoded from the stream entry.
    pub payload: T,
}

/// Error surfaced by an SDK input stream.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct MapperInputError {
    /// SDK stream failure detail.
    pub detail: String,
}

impl MapperInputError {
    /// Construct an input failure from an SDK stream error.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// Peer-agnostic typed SDK stream consumed by a Mapper.
pub type MapperStream<T> =
    Pin<Box<dyn Stream<Item = Result<TimedSdkSample<T>, MapperInputError>> + Send>>;

/// An SDK stream bound to its canonical log identity.
pub struct MapperInput<T> {
    /// Source peer and resource identity. It may name any authenticated Domain peer.
    pub log_ref: LogRef,
    /// Clock in which every sample timestamp is expressed.
    pub clock: RegistryRef,
    /// Typed SDK samples from that log.
    pub samples: MapperStream<T>,
}

impl<T> MapperInput<T> {
    /// Bind a typed SDK stream to the catalog identity used to open it.
    pub fn new(log_ref: LogRef, clock: RegistryRef, samples: MapperStream<T>) -> Self {
        Self {
            log_ref,
            clock,
            samples,
        }
    }
}

impl<T: Send + 'static> MapperInput<T> {
    /// Bind an opened SDK subscription to the exact discovered log and clock.
    ///
    /// The accept-time manifest is checked before any payload is exposed to
    /// the Mapper. Sequence gaps are surfaced as input errors instead of
    /// silently producing an incomplete map.
    pub fn from_sdk_subscription(
        log_ref: LogRef,
        clock: RegistryRef,
        subscription: auki_network::stream_runtime::StreamSubscription<T>,
    ) -> Result<Self, MapperInputBindingError> {
        let manifest = &subscription.manifest;
        if manifest.resource_id != log_ref.resource_id {
            return Err(MapperInputBindingError::ResourceMismatch {
                expected: log_ref.resource_id,
                received: manifest.resource_id.clone(),
            });
        }
        let received_clock = RegistryRef {
            peer_id: manifest.clock_peer_id.clone(),
            id: manifest.clock_id.clone(),
            hash: manifest.clock_hash.clone(),
        };
        if received_clock != clock {
            return Err(MapperInputBindingError::ClockMismatch {
                expected: Box::new(clock),
                received: Box::new(received_clock),
            });
        }

        let samples = futures::stream::unfold(
            (subscription.entries, None::<u64>),
            |(mut entries, previous_sequence)| async move {
                let entry = entries.next().await?;
                let (sample, next_sequence) = match entry {
                    Ok(entry) => {
                        let sequence = entry.seq;
                        let sample = if previous_sequence
                            .is_some_and(|previous| sequence != previous.saturating_add(1))
                        {
                            Err(MapperInputError::new(format!(
                                "SDK stream sequence gap: received {sequence} after {}",
                                previous_sequence.expect("checked above")
                            )))
                        } else {
                            Ok(TimedSdkSample {
                                timestamp_ns: entry.timestamp_ns,
                                payload: entry.payload,
                            })
                        };
                        (sample, Some(sequence))
                    }
                    Err(error) => (
                        Err(MapperInputError::new(error.to_string())),
                        previous_sequence,
                    ),
                };
                Some((sample, (entries, next_sequence)))
            },
        );
        Ok(Self::new(log_ref, clock, Box::pin(samples)))
    }
}

/// An opened SDK stream does not match its discovered catalog identity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapperInputBindingError {
    /// The producer accepted a different resource than the selected log.
    #[error("SDK stream resource mismatch: expected {expected:?}, received {received:?}")]
    ResourceMismatch {
        /// Resource id selected through discovery.
        expected: String,
        /// Resource id committed by the accept-time stream manifest.
        received: String,
    },
    /// The producer accepted the resource with a different clock.
    #[error("SDK stream clock does not match the discovered log clock")]
    ClockMismatch {
        /// Exact clock selected through discovery.
        expected: Box<RegistryRef>,
        /// Clock committed by the accept-time stream manifest.
        received: Box<RegistryRef>,
    },
}

/// Failure writing a produced update to its chosen Map Log peer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{detail}")]
pub struct MapSinkError {
    /// Sink-specific failure detail.
    pub detail: String,
}

impl MapSinkError {
    /// Construct a sink failure without exposing transport internals to the
    /// Mapper.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// Peer-agnostic destination for Mapper output.
pub trait MapUpdateSink: Send + Sync {
    /// Canonical destination Map Log identity.
    fn log_ref(&self) -> &LogRef;

    /// Clock declared by the destination Map Log.
    fn clock_ref(&self) -> &RegistryRef;

    /// Validate whether this sink can correctly translate timestamps from the
    /// Mapper's aligned input clock into its destination clock.
    ///
    /// The default preserves the original same-clock behavior. Cross-peer
    /// sinks must override this together with [`Self::append_from`].
    fn validate_alignment_clock(&self, alignment_clock: &RegistryRef) -> Result<(), MapSinkError> {
        if alignment_clock == self.clock_ref() {
            Ok(())
        } else {
            Err(MapSinkError::new(
                "Map sink cannot translate the Mapper alignment clock into its destination clock",
            ))
        }
    }

    /// Append one update produced at `alignment_timestamp_ns` on the Mapper's
    /// aligned input clock. The sink owns the conversion or restamping into
    /// its declared destination Map Log clock.
    fn append_from<'a>(
        &'a self,
        alignment_clock: &'a RegistryRef,
        alignment_timestamp_ns: i64,
        update: &'a MapUpdate,
    ) -> BoxFuture<'a, Result<(), MapSinkError>>;
}

/// First-stint sink: durably append to a Map Log on the same SDK process.
pub struct LocalMapLogSink {
    handle: auki_session::MapLogHandle,
    destination_now_ns: Option<Arc<dyn Fn() -> i64 + Send + Sync>>,
}

impl LocalMapLogSink {
    /// Take ownership of a locally registered Map Log handle.
    pub fn new(handle: auki_session::MapLogHandle) -> Self {
        Self {
            handle,
            destination_now_ns: None,
        }
    }

    /// Append updates using timestamps sampled from the destination Map Log's
    /// own clock. This is the correct local sink when Mapper inputs belong to
    /// another peer's clock and no explicit time transform is available. The
    /// stored timestamp represents MapUpdate production time, not the sensor's
    /// original observation time.
    pub fn retimestamped(
        handle: auki_session::MapLogHandle,
        destination_now_ns: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            handle,
            destination_now_ns: Some(Arc::new(destination_now_ns)),
        }
    }

    /// Borrow the underlying local handle for replay or diagnostics.
    pub fn handle(&self) -> &auki_session::MapLogHandle {
        &self.handle
    }
}

impl MapUpdateSink for LocalMapLogSink {
    fn log_ref(&self) -> &LogRef {
        &self.handle.log_ref
    }

    fn clock_ref(&self) -> &RegistryRef {
        &self.handle.manifest.clock
    }

    fn validate_alignment_clock(&self, alignment_clock: &RegistryRef) -> Result<(), MapSinkError> {
        if self.destination_now_ns.is_some() || alignment_clock == self.clock_ref() {
            Ok(())
        } else {
            Err(MapSinkError::new(
                "LocalMapLogSink requires the input clock to match the Map Log clock; use LocalMapLogSink::retimestamped for cross-clock input",
            ))
        }
    }

    fn append_from<'a>(
        &'a self,
        alignment_clock: &'a RegistryRef,
        alignment_timestamp_ns: i64,
        update: &'a MapUpdate,
    ) -> BoxFuture<'a, Result<(), MapSinkError>> {
        let destination_timestamp_ns = match &self.destination_now_ns {
            Some(destination_now_ns) => destination_now_ns(),
            None if alignment_clock == self.clock_ref() => alignment_timestamp_ns,
            None => {
                return futures::future::ready(Err(MapSinkError::new(
                    "LocalMapLogSink received an untranslatable input clock",
                )))
                .boxed();
            }
        };
        let handle = self.handle.clone();
        let update = update.clone();
        async move {
            tokio::task::spawn_blocking(move || handle.append(destination_timestamp_ns, &update))
                .await
                .map_err(|error| MapSinkError::new(format!("Map Log append task failed: {error}")))?
                .map_err(|error| MapSinkError::new(error.to_string()))
        }
        .boxed()
    }
}

/// Timestamp alignment behavior for point clouds and poses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoseAlignmentConfig {
    /// Maximum number of point clouds retained while waiting for a bracketing
    /// pose sample. This bounds memory if a pose source stalls.
    pub maximum_pending_point_clouds: usize,
    /// Maximum pose samples retained while point clouds lag or pause. The
    /// oldest pose is discarded once this bound is exceeded.
    pub maximum_buffered_poses: usize,
}

impl Default for PoseAlignmentConfig {
    fn default() -> Self {
        Self {
            maximum_pending_point_clouds: 32,
            maximum_buffered_poses: 256,
        }
    }
}

/// Continuous point-cloud/pose Mapper configured entirely with SDK metadata.
#[derive(Debug, Clone)]
pub struct VoxelMapperRunner {
    voxelizer: Voxelizer,
    point_layout: Rangefinder,
    free_delta: f32,
    occupied_delta: f32,
    persistence: Option<VoxelPersistenceConfig>,
    alignment: PoseAlignmentConfig,
}

impl VoxelMapperRunner {
    /// Build a runner from discovered SDK registry contracts.
    ///
    /// `pose_from_frame` and `pose_to_frame` describe the pose log selected
    /// for this Mapper. They must bind the point-cloud sensor frame to the Map
    /// frame exactly; ids without matching content hashes are rejected.
    pub fn from_sdk_contract(
        point_layout: Rangefinder,
        pose_from_frame: RegistryRef,
        pose_to_frame: RegistryRef,
        map: &VoxelMap,
        free_delta: f32,
        occupied_delta: f32,
        alignment: PoseAlignmentConfig,
    ) -> Result<Self, VoxelMapperRunError> {
        Self::from_sdk_contract_with_frame_binding(
            point_layout,
            pose_from_frame,
            VoxelMapperMapFrameBinding::Exact(pose_to_frame),
            map,
            free_delta,
            occupied_delta,
            alignment,
        )
    }

    /// Build from SDK contracts while permitting an explicit identity-only
    /// alias from the pose destination to the Map's independently owned frame.
    pub fn from_sdk_contract_with_frame_binding(
        point_layout: Rangefinder,
        pose_from_frame: RegistryRef,
        map_frame_binding: VoxelMapperMapFrameBinding,
        map: &VoxelMap,
        free_delta: f32,
        occupied_delta: f32,
        alignment: PoseAlignmentConfig,
    ) -> Result<Self, VoxelMapperRunError> {
        if point_layout.r#type != "point_cloud" {
            return Err(VoxelMapperRunError::UnsupportedSensorType(
                point_layout.r#type.clone(),
            ));
        }
        if point_layout.frame != pose_from_frame {
            return Err(VoxelMapperRunError::PointFrameMismatch {
                point_frame: Box::new(point_layout.frame.clone()),
                pose_from_frame: Box::new(pose_from_frame),
            });
        }
        if !map_frame_binding.matches_map(&map.frame) {
            return Err(VoxelMapperRunError::MapFrameMismatch {
                pose_to_frame: Box::new(map_frame_binding.pose_frame().clone()),
                map_frame: Box::new(map.frame.clone()),
            });
        }
        Self::new(
            Voxelizer::new(map.voxel_size_m.0, map.chunk_dimension)?
                .with_color_model(map.color_model),
            point_layout,
            free_delta,
            occupied_delta,
            alignment,
        )
    }

    /// Configure the live runner. `point_layout` is the exact Rangefinder
    /// Registry body pinned by the input point-cloud catalog row.
    pub fn new(
        voxelizer: Voxelizer,
        point_layout: Rangefinder,
        free_delta: f32,
        occupied_delta: f32,
        alignment: PoseAlignmentConfig,
    ) -> Result<Self, VoxelMapperRunError> {
        if !free_delta.is_finite()
            || free_delta >= 0.0
            || !occupied_delta.is_finite()
            || occupied_delta <= 0.0
            || alignment.maximum_pending_point_clouds == 0
            || alignment.maximum_buffered_poses < 2
        {
            return Err(VoxelMapperRunError::InvalidConfiguration);
        }
        Ok(Self {
            voxelizer,
            point_layout,
            free_delta,
            occupied_delta,
            persistence: None,
            alignment,
        })
    }

    /// Enable time-based hysteresis. Raw point density is normalized to one
    /// observation per voxel per frame; only confirmed state transitions are
    /// written to the Map Log.
    pub fn with_persistence(
        mut self,
        persistence: VoxelPersistenceConfig,
    ) -> Result<Self, VoxelMapperRunError> {
        if !persistence.validate() {
            return Err(VoxelMapperRunError::InvalidConfiguration);
        }
        self.persistence = Some(persistence);
        Ok(self)
    }

    /// Run until the point-cloud input ends. Pose and point-cloud sources may
    /// belong to any peers; the sink independently chooses the writer peer.
    /// The pose stream must contain SDK-resolved transforms from the
    /// Rangefinder frame into the Map frame. Point clouds are held until two
    /// pose samples bracket their timestamp, then translation is linearly
    /// interpolated and orientation is SLERPed.
    pub async fn run<S: MapUpdateSink>(
        &self,
        mut point_clouds: MapperInput<PointCloudData>,
        mut poses: MapperInput<SpatialTransform>,
        sink: &S,
    ) -> Result<VoxelMapperRunReport, VoxelMapperRunError> {
        if point_clouds.clock != poses.clock {
            return Err(VoxelMapperRunError::InputClockMismatch {
                point_cloud_clock: Box::new(point_clouds.clock.clone()),
                pose_clock: Box::new(poses.clock.clone()),
            });
        }
        sink.validate_alignment_clock(&point_clouds.clock)
            .map_err(VoxelMapperRunError::Sink)?;
        let mut report = VoxelMapperRunReport {
            point_cloud_source: point_clouds.log_ref.clone(),
            pose_source: poses.log_ref.clone(),
            map_destination: sink.log_ref().clone(),
            alignment_clock: point_clouds.clock.clone(),
            map_clock: sink.clock_ref().clone(),
            point_clouds_received: 0,
            poses_received: 0,
            poses_dropped_for_backpressure: 0,
            map_updates_written: 0,
            point_clouds_without_pose: 0,
            point_clouds_dropped_for_backpressure: 0,
            point_clouds_dropped_for_worker_backpressure: 0,
        };
        let mut pose_buffer = PoseBuffer::default();
        let mut pending = VecDeque::<TimedSdkSample<PointCloudData>>::new();
        let mut points_done = false;
        let mut poses_done = false;
        let mut last_point_timestamp = None;
        let jobs = Arc::new(LatestVoxelJobSlot::default());
        let worker_jobs = Arc::clone(&jobs);
        let (updates_tx, mut updates_rx) = tokio::sync::mpsc::channel(1);
        let voxelizer = self.voxelizer;
        let point_layout = self.point_layout.clone();
        let free_delta = self.free_delta;
        let occupied_delta = self.occupied_delta;
        let persistence = self.persistence;
        let worker = thread::Builder::new()
            .name("auki-voxel-mapper".into())
            .spawn(move || {
                run_voxel_worker(
                    voxelizer,
                    point_layout,
                    free_delta,
                    occupied_delta,
                    persistence,
                    worker_jobs,
                    updates_tx,
                )
            })
            .map_err(|_| VoxelMapperRunError::WorkerSpawn)?;
        let mut worker = MapperWorkerGuard::new(Arc::clone(&jobs), worker);
        let mut alignment_done = false;

        loop {
            resolve_pending(
                &mut pending,
                &mut pose_buffer,
                poses_done,
                &jobs,
                &mut report,
            )?;

            if !alignment_done && points_done && (pending.is_empty() || poses_done) {
                report.point_clouds_without_pose += pending.len() as u64;
                pending.clear();
                jobs.finish();
                alignment_done = true;
            }

            tokio::select! {
                point = point_clouds.samples.next(), if !points_done && !alignment_done => {
                    match point {
                        Some(Ok(sample)) => {
                            if last_point_timestamp.is_some_and(|last| sample.timestamp_ns < last) {
                                return Err(VoxelMapperRunError::OutOfOrderPointCloud {
                                    previous: last_point_timestamp.unwrap(),
                                    received: sample.timestamp_ns,
                                });
                            }
                            last_point_timestamp = Some(sample.timestamp_ns);
                            report.point_clouds_received += 1;
                            pending.push_back(sample);
                            if pending.len() > self.alignment.maximum_pending_point_clouds {
                                pending.pop_front();
                                report.point_clouds_dropped_for_backpressure += 1;
                            }
                        }
                        Some(Err(error)) => {
                            worker.cancel();
                            worker.join().await?;
                            return Err(VoxelMapperRunError::PointCloudInput(error));
                        }
                        None => points_done = true,
                    }
                }
                pose = poses.samples.next(), if !poses_done && !alignment_done => {
                    match pose {
                        Some(Ok(sample)) => {
                            report.poses_received += 1;
                            if pose_buffer.push(sample, self.alignment.maximum_buffered_poses)? {
                                report.poses_dropped_for_backpressure += 1;
                            }
                        }
                        Some(Err(error)) => {
                            worker.cancel();
                            worker.join().await?;
                            return Err(VoxelMapperRunError::PoseInput(error));
                        }
                        None => poses_done = true,
                    }
                }
                update = updates_rx.recv() => match update {
                    Some(Ok(update)) => {
                        if let Err(error) = sink
                            .append_from(&report.alignment_clock, update.timestamp_ns, &update.update)
                            .await
                        {
                            worker.cancel();
                            worker.join().await?;
                            return Err(VoxelMapperRunError::Sink(error));
                        }
                        report.map_updates_written += 1;
                    }
                    Some(Err(error)) => {
                        worker.cancel();
                        worker.join().await?;
                        return Err(VoxelMapperRunError::Voxelizer(error));
                    }
                    None if alignment_done => {
                        worker.join().await?;
                        return Ok(report);
                    }
                    None => {
                        worker.join().await?;
                        return Err(VoxelMapperRunError::WorkerStopped);
                    }
                }
            }
        }
    }
}

fn resolve_pending(
    pending: &mut VecDeque<TimedSdkSample<PointCloudData>>,
    poses: &mut PoseBuffer,
    poses_done: bool,
    jobs: &LatestVoxelJobSlot,
    report: &mut VoxelMapperRunReport,
) -> Result<(), VoxelMapperRunError> {
    loop {
        let Some(point_cloud) = pending.front() else {
            return Ok(());
        };
        let pose = match poses.resolve(point_cloud.timestamp_ns)? {
            PoseResolution::Ready(pose) => pose,
            PoseResolution::BeforeAvailableRange => {
                pending.pop_front();
                report.point_clouds_without_pose += 1;
                continue;
            }
            PoseResolution::Waiting if !poses_done => return Ok(()),
            PoseResolution::Waiting => {
                pending.pop_front();
                report.point_clouds_without_pose += 1;
                continue;
            }
        };
        let point_cloud = pending.pop_front().expect("front checked above");
        match jobs.submit(VoxelJob {
            timestamp_ns: point_cloud.timestamp_ns,
            point_cloud: point_cloud.payload,
            pose,
        }) {
            Some(true) => report.point_clouds_dropped_for_worker_backpressure += 1,
            Some(false) => {}
            None => return Err(VoxelMapperRunError::WorkerStopped),
        }
    }
}

struct VoxelJob {
    timestamp_ns: i64,
    point_cloud: PointCloudData,
    pose: SpatialTransform,
}

struct VoxelWorkerOutput {
    timestamp_ns: i64,
    update: MapUpdate,
}

#[derive(Default)]
struct LatestVoxelJobSlot {
    state: Mutex<LatestVoxelJobState>,
    ready: Condvar,
}

#[derive(Default)]
struct LatestVoxelJobState {
    pending: Option<VoxelJob>,
    closed: bool,
}

impl LatestVoxelJobSlot {
    fn submit(&self, job: VoxelJob) -> Option<bool> {
        let mut state = self.state.lock();
        if state.closed {
            return None;
        }
        let replaced = state.pending.replace(job).is_some();
        self.ready.notify_one();
        Some(replaced)
    }

    fn receive(&self) -> Option<VoxelJob> {
        let mut state = self.state.lock();
        loop {
            if let Some(job) = state.pending.take() {
                return Some(job);
            }
            if state.closed {
                return None;
            }
            self.ready.wait(&mut state);
        }
    }

    fn finish(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        self.ready.notify_all();
    }

    fn cancel(&self) {
        let mut state = self.state.lock();
        state.pending = None;
        state.closed = true;
        self.ready.notify_all();
    }
}

fn run_voxel_worker(
    voxelizer: Voxelizer,
    point_layout: Rangefinder,
    free_delta: f32,
    occupied_delta: f32,
    persistence: Option<VoxelPersistenceConfig>,
    jobs: Arc<LatestVoxelJobSlot>,
    updates: tokio::sync::mpsc::Sender<Result<VoxelWorkerOutput, VoxelizerError>>,
) {
    let mut persistence =
        persistence.map(|config| VoxelPersistenceFilter::new(config, occupied_delta));
    while let Some(job) = jobs.receive() {
        let result = voxelizer
            .map_point_cloud(
                &job.point_cloud,
                &point_layout,
                &job.pose,
                free_delta,
                occupied_delta,
            )
            .map(|update| {
                let update = match &mut persistence {
                    Some(filter) => filter.apply(job.timestamp_ns, update),
                    None => update,
                };
                VoxelWorkerOutput {
                    timestamp_ns: job.timestamp_ns,
                    update,
                }
            });
        if result
            .as_ref()
            .is_ok_and(|output| output.update.voxel_chunks.is_empty())
        {
            continue;
        }
        let failed = result.is_err();
        if updates.blocking_send(result).is_err() || failed {
            jobs.cancel();
            return;
        }
    }
}

struct MapperWorkerGuard {
    jobs: Arc<LatestVoxelJobSlot>,
    worker: Option<JoinHandle<()>>,
}

impl MapperWorkerGuard {
    fn new(jobs: Arc<LatestVoxelJobSlot>, worker: JoinHandle<()>) -> Self {
        Self {
            jobs,
            worker: Some(worker),
        }
    }

    fn cancel(&self) {
        self.jobs.cancel();
    }

    async fn join(&mut self) -> Result<(), VoxelMapperRunError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || worker.join())
            .await
            .map_err(|_| VoxelMapperRunError::WorkerPanicked)?
            .map_err(|_| VoxelMapperRunError::WorkerPanicked)
    }
}

impl Drop for MapperWorkerGuard {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[derive(Default)]
struct PoseBuffer {
    samples: VecDeque<TimedSdkSample<SpatialTransform>>,
}

enum PoseResolution {
    Ready(SpatialTransform),
    BeforeAvailableRange,
    Waiting,
}

impl PoseBuffer {
    fn push(
        &mut self,
        sample: TimedSdkSample<SpatialTransform>,
        maximum_buffered_poses: usize,
    ) -> Result<bool, VoxelMapperRunError> {
        validate_pose(&sample.payload)?;
        if let Some(last) = self.samples.back() {
            if sample.timestamp_ns < last.timestamp_ns {
                return Err(VoxelMapperRunError::OutOfOrderPose {
                    previous: last.timestamp_ns,
                    received: sample.timestamp_ns,
                });
            }
            if sample.timestamp_ns == last.timestamp_ns {
                self.samples.pop_back();
            }
        }
        self.samples.push_back(sample);
        let dropped = self.samples.len() > maximum_buffered_poses;
        if dropped {
            self.samples.pop_front();
        }
        Ok(dropped)
    }

    fn resolve(&mut self, timestamp_ns: i64) -> Result<PoseResolution, VoxelMapperRunError> {
        while self.samples.len() >= 2 && self.samples[1].timestamp_ns <= timestamp_ns {
            self.samples.pop_front();
        }
        let Some(before) = self.samples.front() else {
            return Ok(PoseResolution::Waiting);
        };
        if timestamp_ns < before.timestamp_ns {
            return Ok(PoseResolution::BeforeAvailableRange);
        }
        if timestamp_ns == before.timestamp_ns {
            return Ok(PoseResolution::Ready(before.payload));
        }
        let Some(after) = self.samples.get(1) else {
            return Ok(PoseResolution::Waiting);
        };
        Ok(PoseResolution::Ready(interpolate_pose(
            before,
            after,
            timestamp_ns,
        )?))
    }
}

fn validate_pose(pose: &SpatialTransform) -> Result<(), VoxelMapperRunError> {
    let translation = pose
        .translation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    let orientation = pose
        .orientation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    if ![
        translation.x,
        translation.y,
        translation.z,
        orientation.x,
        orientation.y,
        orientation.z,
        orientation.w,
    ]
    .into_iter()
    .all(f64::is_finite)
    {
        return Err(VoxelMapperRunError::NonFinitePose);
    }
    let norm_squared = orientation.x * orientation.x
        + orientation.y * orientation.y
        + orientation.z * orientation.z
        + orientation.w * orientation.w;
    if norm_squared <= f64::EPSILON {
        return Err(VoxelMapperRunError::ZeroQuaternion);
    }
    Ok(())
}

fn interpolate_pose(
    before: &TimedSdkSample<SpatialTransform>,
    after: &TimedSdkSample<SpatialTransform>,
    timestamp_ns: i64,
) -> Result<SpatialTransform, VoxelMapperRunError> {
    let span = i128::from(after.timestamp_ns) - i128::from(before.timestamp_ns);
    if span <= 0 {
        return Err(VoxelMapperRunError::OutOfOrderPose {
            previous: before.timestamp_ns,
            received: after.timestamp_ns,
        });
    }
    let offset = i128::from(timestamp_ns) - i128::from(before.timestamp_ns);
    let t = offset as f64 / span as f64;
    let before_translation = before
        .payload
        .translation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    let after_translation = after
        .payload
        .translation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    let before_orientation = before
        .payload
        .orientation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    let after_orientation = after
        .payload
        .orientation
        .as_ref()
        .ok_or(VoxelMapperRunError::IncompletePose)?;
    Ok(SpatialTransform {
        translation: Some(Vec3 {
            x: lerp(before_translation.x, after_translation.x, t),
            y: lerp(before_translation.y, after_translation.y, t),
            z: lerp(before_translation.z, after_translation.z, t),
        }),
        orientation: Some(slerp(before_orientation, after_orientation, t)?),
    })
}

fn lerp(before: f64, after: f64, t: f64) -> f64 {
    before + (after - before) * t
}

fn slerp(before: &Quat, after: &Quat, t: f64) -> Result<Quat, VoxelMapperRunError> {
    let mut a = normalize_quaternion(before)?;
    let mut b = normalize_quaternion(after)?;
    let mut dot = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    if dot < 0.0 {
        dot = -dot;
        b = Quat {
            x: -b.x,
            y: -b.y,
            z: -b.z,
            w: -b.w,
        };
    }
    let result = if dot > 0.9995 {
        Quat {
            x: lerp(a.x, b.x, t),
            y: lerp(a.y, b.y, t),
            z: lerp(a.z, b.z, t),
            w: lerp(a.w, b.w, t),
        }
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();
        let before_weight = ((1.0 - t) * theta).sin() / sin_theta;
        let after_weight = (t * theta).sin() / sin_theta;
        Quat {
            x: a.x * before_weight + b.x * after_weight,
            y: a.y * before_weight + b.y * after_weight,
            z: a.z * before_weight + b.z * after_weight,
            w: a.w * before_weight + b.w * after_weight,
        }
    };
    a = normalize_quaternion(&result)?;
    Ok(a)
}

fn normalize_quaternion(quaternion: &Quat) -> Result<Quat, VoxelMapperRunError> {
    let norm = (quaternion.x * quaternion.x
        + quaternion.y * quaternion.y
        + quaternion.z * quaternion.z
        + quaternion.w * quaternion.w)
        .sqrt();
    if !norm.is_finite() {
        return Err(VoxelMapperRunError::NonFinitePose);
    }
    if norm <= f64::EPSILON {
        return Err(VoxelMapperRunError::ZeroQuaternion);
    }
    Ok(Quat {
        x: quaternion.x / norm,
        y: quaternion.y / norm,
        z: quaternion.z / norm,
        w: quaternion.w / norm,
    })
}

/// Counters and exact peer/resource identities from one completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoxelMapperRunReport {
    /// Point-cloud input selected from the SDK catalog.
    pub point_cloud_source: LogRef,
    /// Pose input selected from the SDK catalog.
    pub pose_source: LogRef,
    /// Map Log destination selected independently of both sources.
    pub map_destination: LogRef,
    /// Exact shared input clock used for point-cloud/pose interpolation.
    pub alignment_clock: RegistryRef,
    /// Clock declared by the destination Map Log. It may differ from the
    /// alignment clock when the sink converts or restamps updates.
    pub map_clock: RegistryRef,
    /// Point-cloud samples received from the SDK.
    pub point_clouds_received: u64,
    /// Pose samples received from the SDK.
    pub poses_received: u64,
    /// Oldest pose samples discarded when their configured buffer bound was
    /// exceeded.
    pub poses_dropped_for_backpressure: u64,
    /// Durable MapUpdate appends completed.
    pub map_updates_written: u64,
    /// Point clouds that could not be bracketed by pose samples.
    pub point_clouds_without_pose: u64,
    /// Oldest pending point clouds discarded when the configured bound was
    /// exceeded.
    pub point_clouds_dropped_for_backpressure: u64,
    /// Ready, pose-aligned point clouds replaced by a fresher job while the
    /// blocking voxel worker was busy.
    pub point_clouds_dropped_for_worker_backpressure: u64,
}

/// Failure from live Mapper orchestration.
#[derive(Debug, thiserror::Error)]
pub enum VoxelMapperRunError {
    /// Evidence or pending-buffer configuration is invalid.
    #[error("invalid voxel Mapper runner configuration")]
    InvalidConfiguration,
    /// The dedicated blocking worker could not be started.
    #[error("failed to start voxel Mapper worker")]
    WorkerSpawn,
    /// The dedicated blocking worker panicked.
    #[error("voxel Mapper worker panicked")]
    WorkerPanicked,
    /// The dedicated blocking worker stopped before input alignment ended.
    #[error("voxel Mapper worker stopped unexpectedly")]
    WorkerStopped,
    /// The selected Rangefinder does not produce SDK point-cloud payloads.
    #[error("voxel Mapper requires a point_cloud Rangefinder, received {0:?}")]
    UnsupportedSensorType(String),
    /// The selected pose does not start at the point-cloud sensor frame.
    #[error("pose source frame does not match the point-cloud frame")]
    PointFrameMismatch {
        /// Exact frame declared by the Rangefinder Registry body.
        point_frame: Box<RegistryRef>,
        /// Exact source frame declared by the Pose Log manifest.
        pose_from_frame: Box<RegistryRef>,
    },
    /// The selected pose does not end at the Map frame.
    #[error("pose destination frame does not match the Map frame")]
    MapFrameMismatch {
        /// Exact destination frame declared by the Pose Log manifest.
        pose_to_frame: Box<RegistryRef>,
        /// Exact frame declared by the Map Registry body.
        map_frame: Box<RegistryRef>,
    },
    /// Point-cloud and pose inputs do not use one exact SDK clock reference.
    #[error("point-cloud and pose input clocks must match exactly")]
    InputClockMismatch {
        /// Point-cloud log clock.
        point_cloud_clock: Box<RegistryRef>,
        /// Pose log clock.
        pose_clock: Box<RegistryRef>,
    },
    /// Point-cloud SDK stream ended with an error.
    #[error("point-cloud input: {0}")]
    PointCloudInput(MapperInputError),
    /// Pose SDK stream ended with an error.
    #[error("pose input: {0}")]
    PoseInput(MapperInputError),
    /// Point-cloud timestamps moved backwards.
    #[error("point-cloud timestamps out of order: {received} after {previous}")]
    OutOfOrderPointCloud {
        /// Prior timestamp.
        previous: i64,
        /// Backwards timestamp.
        received: i64,
    },
    /// Pose timestamps moved backwards.
    #[error("pose timestamps out of order: {received} after {previous}")]
    OutOfOrderPose {
        /// Prior timestamp.
        previous: i64,
        /// Backwards timestamp.
        received: i64,
    },
    /// Pose payload omitted translation or orientation.
    #[error("pose is missing translation or orientation")]
    IncompletePose,
    /// Pose contains NaN or infinity.
    #[error("pose contains non-finite values")]
    NonFinitePose,
    /// Pose quaternion has zero magnitude.
    #[error("pose quaternion has zero magnitude")]
    ZeroQuaternion,
    /// Point-cloud decoding or voxelization failed.
    #[error("voxelizer: {0}")]
    Voxelizer(#[from] VoxelizerError),
    /// Destination Map Log append failed.
    #[error("map sink: {0}")]
    Sink(MapSinkError),
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use auki_registry::{PointField, PointFieldDataType, RegistryRef};

    struct RecordingSink {
        destination: LogRef,
        clock: RegistryRef,
        updates: Mutex<Vec<(i64, MapUpdate)>>,
    }

    impl MapUpdateSink for RecordingSink {
        fn log_ref(&self) -> &LogRef {
            &self.destination
        }

        fn clock_ref(&self) -> &RegistryRef {
            &self.clock
        }

        fn append_from<'a>(
            &'a self,
            _alignment_clock: &'a RegistryRef,
            timestamp_ns: i64,
            update: &'a MapUpdate,
        ) -> BoxFuture<'a, Result<(), MapSinkError>> {
            self.updates
                .lock()
                .unwrap()
                .push((timestamp_ns, update.clone()));
            futures::future::ready(Ok(())).boxed()
        }
    }

    fn log(peer: &str, resource: &str) -> LogRef {
        LogRef {
            source_peer_id: peer.into(),
            resource_id: resource.into(),
        }
    }

    fn clock() -> RegistryRef {
        RegistryRef {
            peer_id: "clock-peer".into(),
            id: "session/monotonic".into(),
            hash: "clock-hash".into(),
        }
    }

    fn layout() -> Rangefinder {
        Rangefinder {
            r#type: "point_cloud".into(),
            fields: ["x", "y", "z"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| PointField {
                    name: name.into(),
                    offset: (index * 4) as u32,
                    datatype: PointFieldDataType::Float32,
                    count: 1,
                })
                .collect(),
            point_step: 12,
            is_bigendian: false,
            frame_rate_hz: 30,
            frame: RegistryRef {
                peer_id: "sensor-peer".into(),
                id: "lidar".into(),
                hash: "frame-hash".into(),
            },
        }
    }

    fn pose(x: f64) -> SpatialTransform {
        SpatialTransform {
            translation: Some(Vec3 { x, y: 0.0, z: 0.0 }),
            orientation: Some(Quat {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                w: 1.0,
            }),
        }
    }

    fn point(x: f32) -> PointCloudData {
        PointCloudData {
            data: [
                x.to_le_bytes(),
                0.0_f32.to_le_bytes(),
                0.0_f32.to_le_bytes(),
            ]
            .concat(),
        }
    }

    #[test]
    fn aligned_worker_queue_keeps_only_the_latest_job() {
        let jobs = LatestVoxelJobSlot::default();
        assert_eq!(
            jobs.submit(VoxelJob {
                timestamp_ns: 1,
                point_cloud: point(1.0),
                pose: pose(1.0),
            }),
            Some(false)
        );
        assert_eq!(
            jobs.submit(VoxelJob {
                timestamp_ns: 2,
                point_cloud: point(2.0),
                pose: pose(2.0),
            }),
            Some(true)
        );
        jobs.finish();

        assert_eq!(jobs.receive().unwrap().timestamp_ns, 2);
        assert!(jobs.receive().is_none());
    }

    #[test]
    fn aligned_worker_queue_cancellation_discards_pending_job() {
        let jobs = LatestVoxelJobSlot::default();
        jobs.submit(VoxelJob {
            timestamp_ns: 1,
            point_cloud: point(1.0),
            pose: pose(1.0),
        });
        jobs.cancel();

        assert!(jobs.receive().is_none());
    }

    #[test]
    fn pose_buffer_is_bounded_and_extreme_timestamps_interpolate() {
        let mut buffer = PoseBuffer::default();
        assert!(
            !buffer
                .push(
                    TimedSdkSample {
                        timestamp_ns: i64::MIN,
                        payload: pose(0.0),
                    },
                    2,
                )
                .unwrap()
        );
        assert!(
            !buffer
                .push(
                    TimedSdkSample {
                        timestamp_ns: 0,
                        payload: pose(5.0),
                    },
                    2,
                )
                .unwrap()
        );
        assert!(
            buffer
                .push(
                    TimedSdkSample {
                        timestamp_ns: i64::MAX,
                        payload: pose(10.0),
                    },
                    2,
                )
                .unwrap()
        );
        assert_eq!(buffer.samples.len(), 2);
        let PoseResolution::Ready(resolved) = buffer.resolve(i64::MAX / 2).unwrap() else {
            panic!("remaining extreme timestamps must bracket the sample")
        };
        assert!(resolved.translation.unwrap().x > 7.0);

        let midpoint = interpolate_pose(
            &TimedSdkSample {
                timestamp_ns: i64::MIN,
                payload: pose(0.0),
            },
            &TimedSdkSample {
                timestamp_ns: i64::MAX,
                payload: pose(10.0),
            },
            0,
        )
        .unwrap();
        assert!((midpoint.translation.unwrap().x - 5.0).abs() < 1e-9);
    }

    fn sdk_manifest(
        resource_id: &str,
        clock: &RegistryRef,
    ) -> auki_network::stream_protocol::StreamManifest {
        auki_network::stream_protocol::StreamManifest {
            resource_id: resource_id.into(),
            clock_peer_id: clock.peer_id.clone(),
            clock_id: clock.id.clone(),
            clock_hash: clock.hash.clone(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn sdk_subscription_binding_validates_identity_and_detects_sequence_gaps() {
        use auki_network::stream_runtime::{StreamEntry, StreamSubscription};

        let selected_clock = clock();
        let subscription = StreamSubscription {
            manifest: sdk_manifest("points", &selected_clock),
            entries: Box::pin(futures::stream::iter(vec![
                Ok(StreamEntry {
                    timestamp_ns: 10,
                    seq: 41,
                    payload: point(1.0),
                }),
                Ok(StreamEntry {
                    timestamp_ns: 20,
                    seq: 43,
                    payload: point(2.0),
                }),
            ])),
        };
        let mut input = MapperInput::from_sdk_subscription(
            log("sensor-peer", "points"),
            selected_clock,
            subscription,
        )
        .unwrap();

        let first = input.samples.next().await.unwrap().unwrap();
        assert_eq!(first.timestamp_ns, 10);
        let gap = input.samples.next().await.unwrap().unwrap_err();
        assert!(gap.detail.contains("received 43 after 41"));
    }

    #[test]
    fn sdk_subscription_binding_rejects_wrong_accept_manifest() {
        use auki_network::stream_runtime::StreamSubscription;

        let selected_clock = clock();
        let subscription = StreamSubscription::<PointCloudData> {
            manifest: sdk_manifest("other-points", &selected_clock),
            entries: Box::pin(futures::stream::empty()),
        };
        let result = MapperInput::from_sdk_subscription(
            log("sensor-peer", "points"),
            selected_clock,
            subscription,
        );

        assert!(matches!(
            result,
            Err(MapperInputBindingError::ResourceMismatch { .. })
        ));
    }

    #[test]
    fn sdk_contract_requires_exact_sensor_pose_and_map_frames() {
        use auki_registry::{FiniteF64, VoxelValueModel};

        let point_layout = layout();
        let sensor_frame = point_layout.frame.clone();
        let map_frame = RegistryRef {
            peer_id: "map-peer".into(),
            id: "world".into(),
            hash: "world-frame-hash".into(),
        };
        let map = VoxelMap {
            frame: map_frame.clone(),
            voxel_size_m: FiniteF64(0.25),
            chunk_dimension: 32,
            value_model: VoxelValueModel::AdditiveOccupancyEvidence,
            color_model: None,
            semantic_classes: vec![],
        };

        let runner = VoxelMapperRunner::from_sdk_contract(
            point_layout.clone(),
            sensor_frame.clone(),
            map_frame.clone(),
            &map,
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        assert_eq!(runner.voxelizer.voxel_size_m, 0.25);
        assert_eq!(runner.voxelizer.chunk_dimension, 32);

        let mut stale_sensor_frame = sensor_frame;
        stale_sensor_frame.hash = "stale-frame-hash".into();
        assert!(matches!(
            VoxelMapperRunner::from_sdk_contract(
                point_layout,
                stale_sensor_frame,
                map_frame,
                &map,
                -0.2,
                0.8,
                PoseAlignmentConfig::default(),
            ),
            Err(VoxelMapperRunError::PointFrameMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn runner_interpolates_sdk_pose_and_writes_independent_sink_peer() {
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let points = MapperInput::new(
            log("sensor-peer", "lidar-points"),
            clock(),
            Box::pin(futures::stream::iter(vec![Ok(TimedSdkSample {
                timestamp_ns: 5,
                payload: point(0.0),
            })])),
        );
        let poses = MapperInput::new(
            log("pose-peer", "world->lidar"),
            clock(),
            Box::pin(futures::stream::iter(vec![
                Ok(TimedSdkSample {
                    timestamp_ns: 0,
                    payload: pose(0.0),
                }),
                Ok(TimedSdkSample {
                    timestamp_ns: 10,
                    payload: pose(10.0),
                }),
            ])),
        );
        let sink = RecordingSink {
            destination: log("map-peer", "occupancy"),
            clock: clock(),
            updates: Mutex::default(),
        };

        let report = runner.run(points, poses, &sink).await.unwrap();
        assert_eq!(report.point_cloud_source.source_peer_id, "sensor-peer");
        assert_eq!(report.pose_source.source_peer_id, "pose-peer");
        assert_eq!(report.map_destination.source_peer_id, "map-peer");
        assert_eq!(report.map_updates_written, 1);
        let updates = sink.updates.lock().unwrap();
        assert_eq!(updates[0].0, 5);
        assert!(
            updates[0]
                .1
                .voxel_chunks
                .iter()
                .flat_map(|chunk| &chunk.voxels)
                .any(|voxel| voxel.x == 5 && voxel.occupancy_delta > 0.0)
        );
    }

    #[tokio::test]
    async fn runner_reports_point_cloud_without_bracketing_pose() {
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let points = MapperInput::new(
            log("p1", "points"),
            clock(),
            Box::pin(futures::stream::iter(vec![Ok(TimedSdkSample {
                timestamp_ns: 1,
                payload: point(0.0),
            })])),
        );
        let poses = MapperInput::new(
            log("p2", "pose"),
            clock(),
            Box::pin(futures::stream::iter(vec![Ok(TimedSdkSample {
                timestamp_ns: 2,
                payload: pose(0.0),
            })])),
        );
        let sink = RecordingSink {
            destination: log("p3", "map"),
            clock: clock(),
            updates: Mutex::default(),
        };
        let report = runner.run(points, poses, &sink).await.unwrap();
        assert_eq!(report.point_clouds_without_pose, 1);
        assert_eq!(report.map_updates_written, 0);
    }

    #[tokio::test]
    async fn runner_rejects_inputs_from_different_clocks() {
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let points = MapperInput::new(
            log("point-peer", "points"),
            clock(),
            Box::pin(futures::stream::empty()),
        );
        let mut pose_clock = clock();
        pose_clock.hash = "different-clock-hash".into();
        let poses = MapperInput::new(
            log("pose-peer", "pose"),
            pose_clock,
            Box::pin(futures::stream::empty()),
        );
        let sink = RecordingSink {
            destination: log("map-peer", "map"),
            clock: clock(),
            updates: Mutex::default(),
        };

        assert!(matches!(
            runner.run(points, poses, &sink).await,
            Err(VoxelMapperRunError::InputClockMismatch { .. })
        ));
        assert!(sink.updates.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_map_sink_persists_runner_output() {
        use std::time::Duration;

        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        use auki_session::{FrameDef, HeadSpec, MapLogSpec, Peer};

        let temporary = tempfile::tempdir().unwrap();
        let peer =
            Peer::new("map-peer", "mapper").with_storage_root(temporary.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(1.0),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        let sink = LocalMapLogSink::new(
            session
                .register_map_log(MapLogSpec {
                    map,
                    clock: session.monotonic_clock(),
                    head: HeadSpec::Fixed,
                    segment_duration: Duration::from_secs(1),
                    retention: Duration::ZERO,
                })
                .unwrap(),
        );
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let points = MapperInput::new(
            log("sensor-peer", "points"),
            session.monotonic_clock(),
            Box::pin(futures::stream::iter(vec![Ok(TimedSdkSample {
                timestamp_ns: 5,
                payload: point(0.0),
            })])),
        );
        let poses = MapperInput::new(
            log("pose-peer", "pose"),
            session.monotonic_clock(),
            Box::pin(futures::stream::iter(vec![
                Ok(TimedSdkSample {
                    timestamp_ns: 0,
                    payload: pose(0.0),
                }),
                Ok(TimedSdkSample {
                    timestamp_ns: 10,
                    payload: pose(10.0),
                }),
            ])),
        );

        let report = runner.run(points, poses, &sink).await.unwrap();
        assert_eq!(report.map_updates_written, 1);
        let persisted = sink.handle().entries().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].timestamp_ns, 5);
    }

    #[tokio::test]
    async fn retimestamped_local_sink_writes_on_its_destination_clock() {
        use std::time::Duration;

        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        use auki_session::{FrameDef, HeadSpec, MapLogSpec, Peer};

        let temporary = tempfile::tempdir().unwrap();
        let peer = Peer::new("park-peer", "park").with_storage_root(temporary.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "voxel/world",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(1.0),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        let map_clock = session.monotonic_clock();
        let sink = LocalMapLogSink::retimestamped(
            session
                .register_map_log(MapLogSpec {
                    map,
                    clock: map_clock.clone(),
                    head: HeadSpec::Fixed,
                    segment_duration: Duration::from_secs(1),
                    retention: Duration::ZERO,
                })
                .unwrap(),
            || 42,
        );
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let input_clock = clock();
        let points = MapperInput::new(
            log("bracketbot", "head_pointcloud"),
            input_clock.clone(),
            Box::pin(futures::stream::iter(vec![Ok(TimedSdkSample {
                timestamp_ns: 5,
                payload: point(0.0),
            })])),
        );
        let poses = MapperInput::new(
            log("bracketbot", "base_link->local_world"),
            input_clock.clone(),
            Box::pin(futures::stream::iter(vec![
                Ok(TimedSdkSample {
                    timestamp_ns: 0,
                    payload: pose(0.0),
                }),
                Ok(TimedSdkSample {
                    timestamp_ns: 10,
                    payload: pose(10.0),
                }),
            ])),
        );

        let report = runner.run(points, poses, &sink).await.unwrap();
        assert_eq!(report.alignment_clock, input_clock);
        assert_eq!(report.map_clock, map_clock);
        assert_eq!(report.map_updates_written, 1);
        let persisted = sink.handle().entries().unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].timestamp_ns, 42);
    }

    #[tokio::test]
    async fn same_clock_local_sink_rejects_foreign_input_clock() {
        use std::time::Duration;

        use auki_registry::{FiniteF64, MapBody, VoxelMap, VoxelValueModel};
        use auki_session::{FrameDef, HeadSpec, MapLogSpec, Peer};

        let temporary = tempfile::tempdir().unwrap();
        let peer =
            Peer::new("map-peer", "mapper").with_storage_root(temporary.path().to_path_buf());
        let session = peer.start_session().unwrap();
        let frame = peer.register_frame("world", FrameDef::ros_body()).unwrap();
        let map = peer
            .register_map(
                "occupancy",
                MapBody::Voxel(VoxelMap {
                    frame,
                    voxel_size_m: FiniteF64(1.0),
                    chunk_dimension: 64,
                    value_model: VoxelValueModel::AdditiveOccupancyEvidence,
                    color_model: None,
                    semantic_classes: vec![],
                }),
            )
            .unwrap();
        let sink = LocalMapLogSink::new(
            session
                .register_map_log(MapLogSpec {
                    map,
                    clock: session.monotonic_clock(),
                    head: HeadSpec::Fixed,
                    segment_duration: Duration::from_secs(1),
                    retention: Duration::ZERO,
                })
                .unwrap(),
        );
        let runner = VoxelMapperRunner::new(
            Voxelizer::new(1.0, 64).unwrap(),
            layout(),
            -0.2,
            0.8,
            PoseAlignmentConfig::default(),
        )
        .unwrap();
        let foreign_clock = clock();
        let points = MapperInput::new(
            log("sensor-peer", "points"),
            foreign_clock.clone(),
            Box::pin(futures::stream::empty()),
        );
        let poses = MapperInput::new(
            log("pose-peer", "pose"),
            foreign_clock,
            Box::pin(futures::stream::empty()),
        );

        assert!(matches!(
            runner.run(points, poses, &sink).await,
            Err(VoxelMapperRunError::Sink(_))
        ));
        assert!(sink.handle().entries().unwrap().is_empty());
    }
}
