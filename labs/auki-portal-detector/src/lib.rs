//! One addressable camera detector; size resolution is an internal asynchronous helper.
use auki_components::*;
use auki_qr_detector::{
    QR_DETECTIONS_SCHEMA, QrDetection, QrDetections, QrDetector, QrSourceFrame,
};
use auki_qr_mapper::{
    ResolvedPortal,
    local::{PortalMetadataSource, portal_id},
};
use auki_scenegraph::resolution::{PortalMaps, QrResolver, ResolvedQr};
use futures::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub const PORTAL_DETECTION_SCHEMA: &str = "auki.portal-detection/v2";
const CAPACITY: usize = 64;
const CACHE_CAPACITY: usize = 128;
const LOOKUPS: usize = 8;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PortalSize {
    Pending,
    LocalMaps {
        side_length_m: f64,
        candidates: Vec<ResolvedQr>,
    },
    Database {
        side_length_m: f64,
        portal_id: Uuid,
        domain_id: Uuid,
        updated_at: String,
    },
    Conflict {
        candidates: Vec<ResolvedQr>,
    },
    Unresolved {
        reason: String,
    },
}
/// Enrichment updates preserve the exact raw detection identity. Publication time
/// belongs to the configured processing clock, never the historical capture clock.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortalDetection {
    pub detection_product: ProductReference,
    pub detection_sequence: u64,
    pub detection_index: usize,
    pub source_frame: QrSourceFrame,
    pub capture_clock: auki_components::ClockReference,
    pub detection: QrDetection,
    pub size: PortalSize,
}
impl ContractType for PortalDetection {
    const DATATYPE: &'static str = PORTAL_DETECTION_SCHEMA;
}

struct Publisher {
    output: ConfiguredObservable<PortalDetection>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
    last_error: Arc<Mutex<Option<String>>>,
    publication_lock: Mutex<()>,
}
impl Publisher {
    fn publish(&self, value: PortalDetection) {
        let _guard = self.publication_lock.lock().unwrap();
        if let Err(error) = self.output.publish((self.clock)(), Arc::new(value)) {
            *self.last_error.lock().unwrap() = Some(error.to_string());
        }
    }
}

/// Camera input and both outputs belong to this single Component. No PortalSizer
/// or child detector Component is registered. Host controls Product retention/export.
pub struct PortalDetector {
    component: Component,
    input: Option<ConfiguredBufferInput<VideoFrame>>,
    raw: ConfiguredObservable<QrDetections>,
    raw_capture: BufferProductCapture<QrDetections>,
    portals: ConfiguredObservable<PortalDetection>,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
    publisher: Arc<Publisher>,
    last_error: Arc<Mutex<Option<String>>>,
}
impl PortalDetector {
    #[allow(clippy::too_many_arguments)]
    pub fn bind<S: PortalMetadataSource + Send + Sync + 'static>(
        runtime: &ComponentRuntime,
        component_id: &str,
        publication_id: &str,
        camera: &RetainedProduct<VideoFrame>,
        maps: PortalMaps,
        metadata: S,
        domain: Uuid,
        publication_clock: auki_components::ClockReference,
        clock: impl Fn() -> u64 + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let PayloadContract::Camera(contract) = &camera.producer.payload else {
            return Err("expected camera Product".into());
        };
        if contract.datatype != VideoFrame::DATATYPE
            || contract.schema != "auki.video-frame/v1"
            || !matches!(contract.encoding.as_str(), "rgb8" | "jpeg")
            || contract.width == 0
            || contract.height == 0
            || camera
                .producer
                .spatial_frame_id
                .as_ref()
                .is_none_or(|s| s.is_empty())
            || auki_components::clock::validate_clock(&publication_clock).is_err()
        {
            return Err("invalid camera or publication clock contract".into());
        }
        let contract = contract.clone();
        let component = runtime
            .component(
                ComponentSpec::new(component_id)
                    .product_input(ProductInputContract {
                        name: "frames".into(),
                        form: ProductForm::Buffer,
                        datatype: VideoFrame::DATATYPE.into(),
                        schema: "auki.video-frame/v1".into(),
                        exposure: Exposure::Cluster,
                    })
                    .observable(ObservableContract {
                        name: "detections".into(),
                        datatype: QrDetections::DATATYPE.into(),
                        schema: QR_DETECTIONS_SCHEMA.into(),
                        access: vec![ObservationAccess::FollowNew],
                        exposure: Exposure::Cluster,
                    })
                    .observable(ObservableContract {
                        name: "portals".into(),
                        datatype: PORTAL_DETECTION_SCHEMA.into(),
                        schema: PORTAL_DETECTION_SCHEMA.into(),
                        access: vec![ObservationAccess::FollowNew],
                        exposure: Exposure::Cluster,
                    }),
            )
            .map_err(|e| e.to_string())?;
        let raw = component
            .configured_observable::<QrDetections>(
                ConfiguredObservableSpec::new(
                    "detections",
                    format!("{publication_id}/detections"),
                    camera.producer.clock.clone(),
                    PayloadContract::Structured(StructuredPayloadContract {
                        modality: "qr".into(),
                        datatype: QrDetections::DATATYPE.into(),
                        schema: QR_DETECTIONS_SCHEMA.into(),
                        observes: "camera".into(),
                        unit: None,
                    }),
                )
                .in_spatial_frame(camera.producer.spatial_frame_id.clone().unwrap()),
            )
            .map_err(|e| e.to_string())?;
        let portals = component
            .configured_observable::<PortalDetection>(
                ConfiguredObservableSpec::new(
                    "portals",
                    format!("{publication_id}/portals"),
                    publication_clock,
                    PayloadContract::Structured(StructuredPayloadContract {
                        modality: "portal".into(),
                        datatype: PORTAL_DETECTION_SCHEMA.into(),
                        schema: PORTAL_DETECTION_SCHEMA.into(),
                        observes: "camera portal detections and size provenance".into(),
                        unit: None,
                    }),
                )
                .in_spatial_frame(camera.producer.spatial_frame_id.clone().unwrap()),
            )
            .map_err(|e| e.to_string())?;
        let last_error = Arc::new(Mutex::new(None));
        let publisher = Arc::new(Publisher {
            output: portals.clone(),
            clock: Box::new(clock),
            last_error: last_error.clone(),
            publication_lock: Mutex::new(()),
        });
        let cancellation = CancellationToken::new();
        let (send, recv) = mpsc::channel::<PortalDetection>(CAPACITY);
        let detector = Mutex::new(QrDetector::default());
        let raw_output = raw.clone();
        let source = camera.reference();
        let producer = camera.manifest.producer.clone();
        let capture_clock = camera.producer.clock.clone();
        let ready = Arc::new((
            Mutex::new(None::<Result<ProductReference, String>>),
            std::sync::Condvar::new(),
        ));
        let input_ready = ready.clone();
        let overflow = publisher.clone();
        let port = InputPort::<Observation<VideoFrame>>::try_new("detect-portals", move |entry| {
            let (lock, wake) = &*input_ready;
            let mut state = lock.lock().map_err(|e| e.to_string())?;
            while state.is_none() {
                state = wake.wait(state).map_err(|e| e.to_string())?;
            }
            let detection_product = state.as_ref().unwrap().clone()?;
            drop(state);
            let observation = &entry.payload;
            let frame = &observation.payload;
            if observation.output != producer
                || frame.width != contract.width
                || frame.height != contract.height
                || frame.encoding != contract.encoding
            {
                return Err("camera observation contract mismatch".into());
            }
            let mut scanner = detector.lock().map_err(|e| e.to_string())?;
            let mut found = match frame.encoding.as_str() {
                "rgb8" => scanner.detect_rgb8(
                    &frame.bytes,
                    frame.width as usize,
                    frame.height as usize,
                    frame.width as usize * 3,
                ),
                _ => scanner.detect_jpeg(&frame.bytes, frame.width as usize, frame.height as usize),
            }
            .map_err(|e| e.to_string())?;
            drop(scanner);
            if found.codes.len() > 256 {
                return Err("too many detections".into());
            }
            let source_frame = QrSourceFrame {
                camera_product: source.clone(),
                sequence: observation.sequence,
                timestamp_ns: observation.timestamp_ns,
            };
            found.source_frame = Some(source_frame.clone());
            let (published, _) = raw_output
                .publish(observation.timestamp_ns, Arc::new(found))
                .map_err(|e| e.to_string())?;
            for (index, code) in published.payload.codes.iter().enumerate() {
                let record = PortalDetection {
                    detection_product: detection_product.clone(),
                    detection_sequence: published.sequence,
                    detection_index: index,
                    source_frame: source_frame.clone(),
                    capture_clock: capture_clock.clone(),
                    detection: code.clone(),
                    size: PortalSize::Pending,
                };
                if let Err(error) = send.try_send(record) {
                    let mut record = error.into_inner();
                    record.size = PortalSize::Unresolved {
                        reason: "enrichment queue unavailable or full".into(),
                    };
                    overflow.publish(record);
                }
            }
            Ok(())
        });
        let input = component
            .configured_buffer_input("frames", camera, CursorStart::FromSequence(0), &port)
            .map_err(|e| e.to_string())?;
        // Input must exist before exposure; do not process buffered frames until
        // retention has been installed. Wake the reader on every initialization outcome.
        let capture = component.expose().map_err(|e| e.to_string()).and_then(|_| {
            runtime
                .capture_buffer(
                    format!("{publication_id}/raw"),
                    &raw,
                    BufferLimits::entries(CAPACITY),
                    serde_size,
                )
                .map_err(|e| e.to_string())
        });
        {
            let (lock, wake) = &*ready;
            *lock.lock().unwrap() = Some(
                capture
                    .as_ref()
                    .map(|c| c.product().reference())
                    .map_err(Clone::clone),
            );
            wake.notify_all();
        }
        let raw_capture = capture?;
        let cancel = cancellation.clone();
        let errors = last_error.clone();
        let worker_publisher = publisher.clone();
        let worker = std::thread::Builder::new()
            .name("portal-enrichment".into())
            .spawn(move || {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt.block_on(enrich(
                        recv,
                        maps,
                        Arc::new(metadata),
                        domain,
                        worker_publisher,
                        cancel,
                    )),
                    Err(e) => *errors.lock().unwrap() = Some(e.to_string()),
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            component,
            input: Some(input),
            raw,
            raw_capture,
            portals,
            cancellation,
            worker: Some(worker),
            publisher,
            last_error,
        })
    }
    pub fn component(&self) -> &Component {
        &self.component
    }
    pub fn detections(&self) -> &ConfiguredObservable<QrDetections> {
        &self.raw
    }
    pub fn detection_product(&self) -> RetainedProduct<QrDetections> {
        self.raw_capture.product()
    }
    pub fn portals(&self) -> &ConfiguredObservable<PortalDetection> {
        &self.portals
    }
    pub fn input(&self) -> Option<&ConfiguredBufferInput<VideoFrame>> {
        self.input.as_ref()
    }
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }
    pub fn close(&mut self) {
        if self.worker.is_none() {
            return;
        }
        self.cancellation.cancel();
        drop(self.input.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let capture_time = self
            .raw_capture
            .product()
            .latest_existing()
            .ok()
            .flatten()
            .map_or(0, |o| o.timestamp_ns);
        if let Err(e) = self.raw.end(
            capture_time,
            ObservationEndReason::Reconfigured { replacement: None },
        ) {
            *self.last_error.lock().unwrap() = Some(e.to_string());
        }
        if let Err(e) = self.portals.end(
            (self.publisher.clock)(),
            ObservationEndReason::Reconfigured { replacement: None },
        ) {
            *self.last_error.lock().unwrap() = Some(e.to_string());
        }
        self.raw_capture.cancel();
    }
}
impl Drop for PortalDetector {
    fn drop(&mut self) {
        self.close();
    }
}
fn serde_size(d: &QrDetections) -> usize {
    d.codes.iter().map(|c| c.payload.len() + 512).sum::<usize>() + 1024
}

fn local_size(maps: &PortalMaps, payload: &str) -> Result<Option<PortalSize>, String> {
    let candidates = maps.resolve(payload).map_err(|e| e.to_string())?;
    if candidates.is_empty() {
        return Ok(None);
    }
    for qr in &candidates {
        qr.validate().map_err(|e| e.to_string())?;
    }
    let size = candidates[0].anchor.side_length_m;
    if candidates.iter().any(|qr| qr.anchor.side_length_m != size) {
        Ok(Some(PortalSize::Conflict { candidates }))
    } else {
        Ok(Some(PortalSize::LocalMaps {
            side_length_m: size,
            candidates,
        }))
    }
}
fn resolved_size(result: &Result<ResolvedPortal, String>, domain: Uuid) -> PortalSize {
    match result {
        Ok(portal) => PortalSize::Database {
            side_length_m: portal.side_length_m(),
            portal_id: portal.metadata().id,
            domain_id: domain,
            updated_at: portal.metadata().updated_at.to_string(),
        },
        Err(reason) => PortalSize::Unresolved {
            reason: reason.clone(),
        },
    }
}
async fn enrich<S: PortalMetadataSource + Send + Sync + 'static>(
    mut recv: mpsc::Receiver<PortalDetection>,
    maps: PortalMaps,
    metadata: Arc<S>,
    domain: Uuid,
    publisher: Arc<Publisher>,
    cancellation: CancellationToken,
) {
    let mut pending: HashMap<String, Vec<PortalDetection>> = HashMap::new();
    let mut cache: HashMap<String, (Instant, Result<ResolvedPortal, String>)> = HashMap::new();
    let mut lookups: FuturesUnordered<
        futures::future::LocalBoxFuture<'static, (String, Result<ResolvedPortal, String>)>,
    > = FuturesUnordered::new();
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            Some((payload, result)) = lookups.next(), if !lookups.is_empty() => {
                let records = pending.remove(&payload).unwrap_or_default();
                if cache.len() >= CACHE_CAPACITY
                    && let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone()) { cache.remove(&oldest); }
                let ttl = if result.is_ok() { Duration::from_secs(60) } else { Duration::from_secs(5) };
                cache.insert(payload.clone(), (Instant::now() + ttl, result.clone()));
                for mut record in records {
                    record.size = match local_size(&maps, &payload) {
                        Ok(Some(local)) => local,
                        Ok(None) => resolved_size(&result, domain),
                        Err(reason) => PortalSize::Unresolved { reason },
                    };
                    publisher.publish(record);
                }
            },
            record = recv.recv() => {
                let Some(mut record) = record else { break; };
                let payload = record.detection.payload.clone();
                match local_size(&maps, &payload) {
                    Ok(Some(local)) => { record.size = local; publisher.publish(record); continue; }
                    Err(reason) => { record.size = PortalSize::Unresolved { reason }; publisher.publish(record); continue; }
                    Ok(None) => {}
                }
                let id = match portal_id(&payload) {
                    Ok(id) => id,
                    Err(error) => { record.size = PortalSize::Unresolved { reason: error.to_string() }; publisher.publish(record); continue; }
                };
                if let Some((expiry, result)) = cache.get(&payload)
                    && *expiry > Instant::now() { record.size = resolved_size(result, domain); publisher.publish(record); continue; }
                if pending.values().map(Vec::len).sum::<usize>() >= CAPACITY || (!pending.contains_key(&payload) && pending.len() >= LOOKUPS) {
                    record.size = PortalSize::Unresolved { reason: "pending lookup capacity exceeded".into() }; publisher.publish(record); continue;
                }
                publisher.publish(record.clone());
                if let Some(waiters) = pending.get_mut(&payload) { waiters.push(record); continue; }
                pending.insert(payload.clone(), vec![record]);
                let source = metadata.clone();
                let cancel = cancellation.child_token();
                lookups.push(Box::pin(async move {
                    let result = tokio::time::timeout(Duration::from_secs(10), source.resolve(domain, &id, &cancel)).await;
                    cancel.cancel();
                    let result = match result {
                        Ok(Ok(portal)) if id.matches(portal.metadata().id, &portal.metadata().short_id) => Ok(portal),
                        Ok(Ok(_)) => Err("metadata Portal identity mismatch".into()),
                        Ok(Err(error)) => Err(error.to_string()),
                        Err(_) => Err("Portal size lookup timed out".into()),
                    };
                    (payload, result)
                }));
            }
        }
    }
}
