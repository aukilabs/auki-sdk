//! Detector-agnostic Camera Sensor Log runners.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use auki_datatypes::camera::CameraFrame;
use auki_datatypes::detection::DetectionFrame;
use auki_logs::{Log, TailIter};
use auki_registry::{Camera, LogRef, RegistryRef};
use futures::{Stream, StreamExt};
use thiserror::Error;
use tokio::sync::watch;

use crate::{DetectionCadence, DetectionLogHandle, SensorLogHandle};

/// One detector-specific result before the SDK stamps shared provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectorOutput {
    pub r#type: String,
    pub data: Vec<u8>,
}

/// Transport-neutral input provenance for a camera frame stream.
///
/// Local logs and live remote subscriptions both bind to the same immutable
/// Sensor Log identity, Sensor Registry entry, and clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInputBinding {
    pub log_ref: LogRef,
    pub sensor: RegistryRef,
    pub clock: RegistryRef,
}

impl From<&SensorLogHandle> for CameraInputBinding {
    fn from(input: &SensorLogHandle) -> Self {
        Self {
            log_ref: input.log_ref.clone(),
            sensor: input.manifest.sensor.clone(),
            clock: input.manifest.clock.clone(),
        }
    }
}

/// One timestamped camera sample from any local or remote frame source.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraFrameSample {
    pub timestamp_ns: i64,
    pub frame: Arc<CameraFrame>,
}

/// Bounded, transport-neutral fanout for one camera stream.
///
/// Every subscriber receives cheap clones of the same reference-counted
/// frame. Slow subscribers skip overwritten frames instead of blocking the
/// publisher or other detectors.
#[derive(Clone)]
pub struct CameraFrameHub {
    sender: tokio::sync::broadcast::Sender<CameraFrameSample>,
    lagged_frames: Arc<AtomicU64>,
}

impl CameraFrameHub {
    /// Create a hub retaining at most `capacity` frames per subscriber.
    ///
    /// A capacity of one provides strict latest-frame behavior.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "camera frame hub capacity must be positive");
        let (sender, _) = tokio::sync::broadcast::channel(capacity);
        Self {
            sender,
            lagged_frames: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Publish a frame and return the number of active subscribers.
    pub fn publish(&self, sample: CameraFrameSample) -> usize {
        self.sender.send(sample).unwrap_or(0)
    }

    /// Subscribe to future frames.
    ///
    /// If this subscriber falls behind, overwritten frames are counted and
    /// skipped; lag is not treated as a terminal source error.
    pub fn subscribe(
        &self,
    ) -> impl Stream<Item = std::result::Result<CameraFrameSample, String>> + Send + 'static {
        let receiver = self.sender.subscribe();
        let lagged_frames = Arc::clone(&self.lagged_frames);
        futures::stream::unfold(
            (receiver, lagged_frames),
            |(mut receiver, lagged_frames)| async move {
                loop {
                    match receiver.recv().await {
                        Ok(sample) => return Some((Ok(sample), (receiver, lagged_frames))),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            lagged_frames.fetch_add(count, Ordering::Relaxed);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        )
    }

    /// Total overwritten frames observed across all subscribers.
    pub fn lagged_frames(&self) -> u64 {
        self.lagged_frames.load(Ordering::Relaxed)
    }
}

/// Detector implementation consumed by the generic Camera Sensor Log runner.
///
/// Cadence, source timestamps, sensor provenance, log I/O, and task lifecycle
/// are owned by [`DetectorTask`], not by individual detector implementations.
pub trait CameraDetector: Send + 'static {
    fn process(
        &mut self,
        frame: &CameraFrame,
        camera: &Camera,
    ) -> std::result::Result<Vec<DetectorOutput>, String>;
}

/// Errors produced while binding or running a detector instance.
#[derive(Debug, Error)]
pub enum DetectorRunnerError {
    #[error("input Sensor Log reference does not match the Detection Log manifest")]
    InputLogMismatch,
    #[error("input Sensor Registry reference does not match the Detection Log manifest")]
    InputSensorMismatch,
    #[error("input Sensor Log clock does not match the Detection Log manifest")]
    InputClockMismatch,
    #[error("detector cadence period must be positive")]
    InvalidCadence,
    #[error("log: {0}")]
    Log(#[from] auki_logs::Error),
    #[error("detector: {0}")]
    Detector(String),
    #[error("input stream: {0}")]
    InputStream(String),
    #[error("streaming detector task requires a Tokio runtime")]
    NoAsyncRuntime,
    #[error("detector task panicked")]
    TaskPanicked,
}

fn validate_binding(
    input: &CameraInputBinding,
    output: &DetectionLogHandle,
) -> std::result::Result<(), DetectorRunnerError> {
    if output.manifest.input_log != input.log_ref {
        return Err(DetectorRunnerError::InputLogMismatch);
    }
    if output.manifest.input_sensor != input.sensor {
        return Err(DetectorRunnerError::InputSensorMismatch);
    }
    if output.manifest.clock != input.clock {
        return Err(DetectorRunnerError::InputClockMismatch);
    }
    if matches!(
        output.manifest.cadence,
        DetectionCadence::Periodic { period_ns: 0 }
    ) {
        return Err(DetectorRunnerError::InvalidCadence);
    }
    Ok(())
}

struct DetectorPipeline<D> {
    detector: D,
    camera: Camera,
    output: Log<DetectionFrame>,
    cadence: DetectionCadence,
    sensor_hash: String,
    last_processed_ns: Option<i64>,
}

impl<D: CameraDetector> DetectorPipeline<D> {
    fn open(
        detector: D,
        camera: Camera,
        output: &DetectionLogHandle,
    ) -> std::result::Result<Self, DetectorRunnerError> {
        let output_manifest = serde_json::to_value(&output.manifest)
            .expect("DetectionLogManifest serializes to JSON");
        Ok(Self {
            detector,
            camera,
            output: Log::<DetectionFrame>::open(output.root(), output_manifest)?,
            cadence: output.manifest.cadence,
            sensor_hash: output.manifest.input_sensor.hash.clone(),
            last_processed_ns: None,
        })
    }

    fn process(
        &mut self,
        sample: CameraFrameSample,
    ) -> std::result::Result<(), DetectorRunnerError> {
        if !cadence_accepts(self.cadence, self.last_processed_ns, sample.timestamp_ns) {
            return Ok(());
        }

        let detections = self
            .detector
            .process(sample.frame.as_ref(), &self.camera)
            .map_err(DetectorRunnerError::Detector)?;
        self.last_processed_ns = Some(sample.timestamp_ns);
        for detection in detections {
            self.output.append(
                sample.timestamp_ns,
                &DetectionFrame {
                    data: detection.data,
                    sensor_hash: self.sensor_hash.clone(),
                    r#type: detection.r#type,
                },
            )?;
        }
        self.output.flush()?;
        Ok(())
    }
}

/// A running local detector instance.
///
/// Dropping the task requests shutdown. Call [`Self::shutdown`] when the caller
/// needs to wait for the worker and observe its terminal result.
pub struct DetectorTask {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<std::result::Result<(), DetectorRunnerError>>>,
}

impl DetectorTask {
    /// Tail `input`, run `detector` at the cadence declared by `output`, and
    /// append SDK [`DetectionFrame`] records to `output`.
    pub fn start<D: CameraDetector>(
        detector: D,
        camera: Camera,
        input: &SensorLogHandle,
        output: &DetectionLogHandle,
    ) -> std::result::Result<Self, DetectorRunnerError> {
        validate_binding(&CameraInputBinding::from(input), output)?;

        // Establish the input EOF before the worker starts so frames appended
        // immediately after `start` cannot race past the subscription.
        let tail = Log::<CameraFrame>::tail(input.root())?;
        let pipeline = DetectorPipeline::open(detector, camera, output)?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);

        let worker = thread::spawn(move || run_local_loop(pipeline, tail, &worker_stop));
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    pub fn request_shutdown(&self) {
        self.stop.store(true, Ordering::Release);
    }

    pub fn shutdown(mut self) -> std::result::Result<(), DetectorRunnerError> {
        self.request_shutdown();
        self.join_worker()
    }

    fn join_worker(&mut self) -> std::result::Result<(), DetectorRunnerError> {
        match self.worker.take() {
            Some(worker) => worker
                .join()
                .map_err(|_| DetectorRunnerError::TaskPanicked)?,
            None => Ok(()),
        }
    }
}

impl Drop for DetectorTask {
    fn drop(&mut self) {
        self.request_shutdown();
    }
}

/// A detector instance consuming an asynchronous camera frame stream.
///
/// This is the transport-neutral path used by live remote subscriptions.
/// Callers map their transport entries into [`CameraFrameSample`] values.
pub struct StreamingDetectorTask {
    stop: watch::Sender<bool>,
    worker: Option<tokio::task::JoinHandle<std::result::Result<(), DetectorRunnerError>>>,
}

impl StreamingDetectorTask {
    /// Consume `frames`, run `detector` at the output manifest's cadence, and
    /// append Detection Log records with the bound sensor provenance.
    pub fn start<D, S>(
        detector: D,
        camera: Camera,
        input: CameraInputBinding,
        frames: S,
        output: &DetectionLogHandle,
    ) -> std::result::Result<Self, DetectorRunnerError>
    where
        D: CameraDetector,
        S: Stream<Item = std::result::Result<CameraFrameSample, String>> + Send + 'static,
    {
        validate_binding(&input, output)?;
        tokio::runtime::Handle::try_current().map_err(|_| DetectorRunnerError::NoAsyncRuntime)?;
        let pipeline = DetectorPipeline::open(detector, camera, output)?;
        let (stop, stop_rx) = watch::channel(false);
        let worker = tokio::spawn(run_stream_loop(pipeline, Box::pin(frames), stop_rx));
        Ok(Self {
            stop,
            worker: Some(worker),
        })
    }

    pub fn request_shutdown(&self) {
        let _ = self.stop.send(true);
    }

    pub async fn shutdown(mut self) -> std::result::Result<(), DetectorRunnerError> {
        self.request_shutdown();
        match self.worker.take() {
            Some(worker) => worker
                .await
                .map_err(|_| DetectorRunnerError::TaskPanicked)?,
            None => Ok(()),
        }
    }
}

impl Drop for StreamingDetectorTask {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(worker) = &self.worker {
            worker.abort();
        }
    }
}

fn run_local_loop<D: CameraDetector>(
    mut pipeline: DetectorPipeline<D>,
    mut input: TailIter<CameraFrame>,
    stop: &AtomicBool,
) -> std::result::Result<(), DetectorRunnerError> {
    while !stop.load(Ordering::Acquire) {
        let Some(entry) = input.try_next()? else {
            thread::sleep(Duration::from_millis(5));
            continue;
        };
        pipeline.process(CameraFrameSample {
            timestamp_ns: entry.timestamp_ns,
            frame: Arc::new(entry.payload),
        })?;
    }
    Ok(())
}

async fn run_stream_loop<D: CameraDetector>(
    mut pipeline: DetectorPipeline<D>,
    mut frames: std::pin::Pin<
        Box<dyn Stream<Item = std::result::Result<CameraFrameSample, String>> + Send>,
    >,
    mut stop: watch::Receiver<bool>,
) -> std::result::Result<(), DetectorRunnerError> {
    loop {
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
            sample = frames.next() => match sample {
                Some(Ok(sample)) => pipeline.process(sample)?,
                Some(Err(error)) => return Err(DetectorRunnerError::InputStream(error)),
                None => return Ok(()),
            }
        }
    }
}

fn cadence_accepts(
    cadence: DetectionCadence,
    last_processed_ns: Option<i64>,
    timestamp_ns: i64,
) -> bool {
    match cadence {
        DetectionCadence::EveryFrame => true,
        DetectionCadence::Periodic { period_ns } => last_processed_ns.is_none_or(|last| {
            timestamp_ns.saturating_sub(last) >= i64::try_from(period_ns).unwrap_or(i64::MAX)
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_frame_accepts_every_timestamp() {
        assert!(cadence_accepts(DetectionCadence::EveryFrame, None, 10));
        assert!(cadence_accepts(DetectionCadence::EveryFrame, Some(10), 10));
    }

    #[test]
    fn periodic_cadence_is_anchored_to_last_processed_source_timestamp() {
        let cadence = DetectionCadence::Periodic { period_ns: 1_000 };
        assert!(cadence_accepts(cadence, None, 5_000));
        assert!(!cadence_accepts(cadence, Some(5_000), 5_999));
        assert!(cadence_accepts(cadence, Some(5_000), 6_000));
        assert!(!cadence_accepts(cadence, Some(5_000), 4_000));
    }

    #[tokio::test]
    async fn frame_hub_fans_out_shared_frames_and_skips_lag() {
        let hub = CameraFrameHub::new(1);
        let mut first = Box::pin(hub.subscribe());
        let mut second = Box::pin(hub.subscribe());
        let frame = Arc::new(CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![7; 16],
        });

        assert_eq!(
            hub.publish(CameraFrameSample {
                timestamp_ns: 1,
                frame: Arc::clone(&frame),
            }),
            2
        );
        let first_sample = first.next().await.unwrap().unwrap();
        let second_sample = second.next().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(&first_sample.frame, &second_sample.frame));

        for timestamp_ns in [2, 3] {
            hub.publish(CameraFrameSample {
                timestamp_ns,
                frame: Arc::clone(&frame),
            });
        }
        assert_eq!(first.next().await.unwrap().unwrap().timestamp_ns, 3);
        assert_eq!(hub.lagged_frames(), 1);
    }
}
