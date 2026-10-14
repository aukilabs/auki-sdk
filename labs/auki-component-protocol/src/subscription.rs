//! One request, followed by a bounded, wake-driven sequence of observations.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use auki_components::{
    BufferLimits, CursorRead, CursorStart, Envelope, Observation, ObservationEnd, OutputManifest,
    ProductManifest, ProductReference, RetainedProduct,
};
use auki_sdk::{AukiProtocolError, AuthenticatedRouteStream, Multiaddr, PeerId};
#[cfg(not(target_arch = "wasm32"))]
use futures::future::BoxFuture as ReceiveFuture;
#[cfg(target_arch = "wasm32")]
use futures::future::LocalBoxFuture as ReceiveFuture;
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, FutureExt, Stream, StreamExt, pin_mut};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::endpoint::{
    ComponentProtocolClient, ComponentProtocolError, ComponentProtocolOperation,
    EncodedObservation, NETWORK_TIMEOUT, OBSERVATION_EXCHANGE_TIMEOUT, ServiceError, ServiceState,
    close_stream, deadline, encode_observation,
};
use crate::wire::{
    OBSERVATION_STREAM_PROTOCOL_ID, ObservationStart, ObservationSubscriptionRequest, SourceGap,
    SubscriptionEventHeader, SubscriptionHeader, read_json, read_json_after_first, read_payload,
    write_json, write_payload,
};

enum SourceEvent {
    Observation(EncodedObservation),
    Gap(SourceGap),
    Closed(Option<ObservationEnd>),
}

type SourceEvents = Pin<Box<dyn Stream<Item = Result<SourceEvent, ServiceError>> + Send>>;

struct SourceCursor {
    next_sequence: u64,
    events: SourceEvents,
}

pub(super) struct SubscriptionSource {
    product: ProductManifest,
    manifest_hash: String,
    producer: OutputManifest,
    open: Box<dyn Fn(ObservationStart) -> SourceCursor + Send + Sync>,
    revoked: async_channel::Receiver<()>,
    _keep_open: async_channel::Sender<()>,
}

impl SubscriptionSource {
    pub fn new<T: Serialize + Send + Sync + 'static>(product: &RetainedProduct<T>) -> Self {
        let retained = product.clone();
        let (sender, revoked) = async_channel::bounded(1);
        Self {
            product: product.manifest.clone(),
            manifest_hash: product.manifest_hash.clone(),
            producer: product.producer.clone(),
            revoked,
            _keep_open: sender,
            open: Box::new(move |start| {
                let product = retained.clone();
                let cursor = product.buffer().subscribe(match start {
                    ObservationStart::FromSequence { sequence } => {
                        CursorStart::FromSequence(sequence)
                    }
                    ObservationStart::LatestExisting => CursorStart::Current,
                    ObservationStart::NewOnly => CursorStart::Latest,
                });
                let next_sequence = cursor.next_sequence();
                let events = futures::stream::unfold(Some(cursor), move |cursor| {
                    let product = product.clone();
                    async move {
                        let mut cursor = cursor?;
                        let event = match cursor.next_async().await {
                            CursorRead::Item(envelope) => {
                                encode_observation(envelope.payload.clone())
                                    .map(SourceEvent::Observation)
                            }
                            CursorRead::Gap(gap) => Ok(SourceEvent::Gap(SourceGap {
                                requested_sequence: gap.requested_sequence,
                                available_from: gap.available_from,
                            })),
                            CursorRead::Closed => {
                                return Some((Ok(SourceEvent::Closed(product.end_notice())), None));
                            }
                            CursorRead::Timeout => {
                                unreachable!("asynchronous cursors have no timer")
                            }
                        };
                        let next = event.is_ok().then_some(cursor);
                        Some((event, next))
                    }
                });
                SourceCursor {
                    next_sequence,
                    events: Box::pin(events),
                }
            }),
        }
    }

    pub fn revoke(&self) {
        self.revoked.close();
    }
}

struct ActiveSubscription<'a>(&'a AtomicUsize);

impl Drop for ActiveSubscription<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) async fn serve_subscription<S>(
    stream: &mut S,
    state: &ServiceState,
) -> Result<(), ServiceError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    state.active_subscriptions.fetch_add(1, Ordering::AcqRel);
    let _active = ActiveSubscription(&state.active_subscriptions);
    let request: ObservationSubscriptionRequest = deadline(
        ComponentProtocolOperation::Exchange,
        read_json(stream),
        NETWORK_TIMEOUT,
    )
    .await
    .map_err(service_timeout)??;
    let source = {
        let products = state.products.read().unwrap();
        products.get(&request.product.product_id).cloned()
    };
    let rejection = if request.product.peer_id != state.runtime.peer_id() {
        Some(("wrong_peer", "Product belongs to another peer"))
    } else if source.is_none() {
        Some(("unknown_product", "Product is not exported"))
    } else if !product_is_current(state, &request.product)
        || source
            .as_ref()
            .is_some_and(|export| export.source.manifest_hash != request.product.manifest_hash)
    {
        Some(("product_not_current", "Product Manifest is not current"))
    } else {
        None
    };
    if let Some((code, message)) = rejection {
        return deadline(
            ComponentProtocolOperation::Exchange,
            write_json(
                stream,
                &SubscriptionHeader::Rejected {
                    code: code.to_owned(),
                    message: message.to_owned(),
                },
            ),
            NETWORK_TIMEOUT,
        )
        .await
        .map_err(service_timeout)?
        .map_err(Into::into);
    }
    let exported = source.expect("validated export");
    let source = &exported.source;
    let mut cursor = (source.open)(request.start);
    deadline(
        ComponentProtocolOperation::Exchange,
        write_json(
            stream,
            &SubscriptionHeader::Accepted {
                product: Box::new(source.product.clone()),
                product_manifest_hash: source.manifest_hash.clone(),
                producer: Box::new(source.producer.clone()),
                next_sequence: cursor.next_sequence,
            },
        ),
        NETWORK_TIMEOUT,
    )
    .await
    .map_err(service_timeout)??;

    let (mut reader, mut writer) = stream.split();
    // FIN/drop cancels even while the source is idle or a payload write stalls.
    // No further client messages are valid on this one-request protocol.
    {
        let cancelled = async {
            let mut byte = [0];
            let _ = reader.read(&mut byte).await;
        }
        .fuse();
        let revoked = source.revoked.recv().fuse();
        let pump = async {
            while let Some(event) = cursor.events.next().await {
                if !product_is_current(state, &request.product) {
                    return send_unavailable(
                        &mut writer,
                        "product_not_current",
                        "Product was removed",
                    )
                    .await;
                }
                let event = match event {
                    Ok(event) => event,
                    Err(error) => {
                        return send_unavailable(&mut writer, &error.code, &error.message).await;
                    }
                };
                let terminal = matches!(event, SourceEvent::Closed(_));
                deadline(
                    ComponentProtocolOperation::Exchange,
                    send_event(&mut writer, event),
                    OBSERVATION_EXCHANGE_TIMEOUT,
                )
                .await
                .map_err(service_timeout)??;
                if terminal {
                    break;
                }
            }
            Ok(())
        }
        .fuse();
        pin_mut!(cancelled, revoked, pump);
        futures::select_biased! {
            () = cancelled => Ok(()),
            // A write may be partway through a frame: terminate the stream,
            // never splice an error frame into a partially written payload.
            _ = revoked => Ok(()),
            result = pump => result,
        }
    }
}

fn product_is_current(state: &ServiceState, reference: &ProductReference) -> bool {
    state
        .runtime
        .catalog()
        .product(&reference.product_id)
        .is_some_and(|entry| entry.manifest_hash == reference.manifest_hash)
}

fn service_timeout(error: ComponentProtocolError) -> ServiceError {
    ServiceError::new("timeout", error.to_string())
}

async fn send_unavailable<S: AsyncWrite + Unpin>(
    stream: &mut S,
    code: &str,
    message: &str,
) -> Result<(), ServiceError> {
    deadline(
        ComponentProtocolOperation::Exchange,
        write_json(
            stream,
            &SubscriptionEventHeader::Unavailable {
                code: code.to_owned(),
                message: message.to_owned(),
            },
        ),
        NETWORK_TIMEOUT,
    )
    .await
    .map_err(service_timeout)?
    .map_err(Into::into)
}

async fn send_event<S: AsyncWrite + Unpin>(
    stream: &mut S,
    event: SourceEvent,
) -> Result<(), ServiceError> {
    match event {
        SourceEvent::Observation(observation) => {
            write_json(
                stream,
                &SubscriptionEventHeader::Observation {
                    record: observation.header,
                },
            )
            .await?;
            write_payload(stream, &observation.payload).await?;
        }
        SourceEvent::Gap(gap) => write_json(stream, &SubscriptionEventHeader::Gap { gap }).await?,
        SourceEvent::Closed(end) => {
            write_json(
                stream,
                &SubscriptionEventHeader::Closed {
                    end: end.map(Box::new),
                },
            )
            .await?
        }
    }
    Ok(())
}

impl ComponentProtocolClient {
    /// Subscribe once; later `next()` calls read pushed data, not new requests.
    /// The caller drives reception in its own async task. No worker is spawned.
    #[allow(clippy::too_many_arguments)]
    pub async fn subscribe_product_exact<T>(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        product: ProductReference,
        start: ObservationStart,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<RemoteProductSubscription<T>, ComponentProtocolError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        validate_peer(&product, remote_peer_id)?;
        subscribe_opened(
            ObservationSubscriptionRequest { product, start },
            self.protocols
                .open_exact(remote_peer_id, route, OBSERVATION_STREAM_PROTOCOL_ID),
            limits,
            retained_size,
        )
        .await
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn subscribe_product<T>(
        &self,
        remote_peer_id: PeerId,
        product: ProductReference,
        start: ObservationStart,
        limits: BufferLimits,
        retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
    ) -> Result<RemoteProductSubscription<T>, ComponentProtocolError>
    where
        T: DeserializeOwned + Send + Sync + 'static,
    {
        validate_peer(&product, remote_peer_id)?;
        subscribe_opened(
            ObservationSubscriptionRequest { product, start },
            self.protocols
                .open(remote_peer_id, OBSERVATION_STREAM_PROTOCOL_ID),
            limits,
            retained_size,
        )
        .await
    }
}

fn validate_peer(product: &ProductReference, peer: PeerId) -> Result<(), ComponentProtocolError> {
    if product.peer_id != peer.to_string() {
        return Err(ComponentProtocolError::InvalidRequest(
            "Product peer must match authenticated destination".into(),
        ));
    }
    Ok(())
}

async fn subscribe_opened<T>(
    request: ObservationSubscriptionRequest,
    opening: impl Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
    limits: BufferLimits,
    retained_size: impl Fn(&T) -> usize + Send + Sync + 'static,
) -> Result<RemoteProductSubscription<T>, ComponentProtocolError>
where
    T: DeserializeOwned + Send + Sync + 'static,
{
    let mut stream = deadline(ComponentProtocolOperation::Open, opening, NETWORK_TIMEOUT).await??;
    let header: SubscriptionHeader = deadline(
        ComponentProtocolOperation::Exchange,
        async {
            write_json(&mut stream, &request).await?;
            read_json(&mut stream).await
        },
        NETWORK_TIMEOUT,
    )
    .await??;
    let SubscriptionHeader::Accepted {
        product,
        product_manifest_hash,
        producer,
        next_sequence,
    } = header
    else {
        let SubscriptionHeader::Rejected { code, message } = header else {
            unreachable!()
        };
        return Err(ComponentProtocolError::RemoteRejected { code, message });
    };
    if product.peer_id != request.product.peer_id
        || product.product_id != request.product.product_id
        || product_manifest_hash != request.product.manifest_hash
        || matches!(request.start, ObservationStart::FromSequence { sequence } if sequence != next_sequence)
    {
        return Err(ComponentProtocolError::InvalidResponse(
            "subscription does not match requested Product/cursor".into(),
        ));
    }
    let product = RetainedProduct::imported_buffer(
        *product,
        product_manifest_hash,
        *producer,
        limits,
        retained_size,
    )
    .map_err(|error| ComponentProtocolError::Import(error.to_string()))?;
    Ok(RemoteProductSubscription {
        product,
        stream: Some(stream),
        pending: None,
        pending_observation: None,
        next_sequence,
        finished: false,
    })
}

/// Data is retained before `Observation` is returned. `Closed(None)` means the
/// capture closed without a producer end notice; it is not a reconfiguration.
#[derive(Debug)]
pub enum RemoteObservationEvent<T> {
    Observation(Observation<T>),
    Gap(SourceGap),
    Closed(Option<ObservationEnd>),
}

type Receive<T> = ReceiveFuture<
    'static,
    (
        AuthenticatedRouteStream,
        Result<RemoteObservationEvent<T>, ComponentProtocolError>,
    ),
>;

/// Host-driven, cancellation-safe receiver for one exact remote Buffer Product.
///
/// Dropping or closing it closes local readers and cancels the remote handler.
/// Transport failure terminates this relationship; reconnect/rebind is explicit.
#[must_use = "drive next() in your async task; dropping the subscription cancels it"]
pub struct RemoteProductSubscription<T> {
    product: RetainedProduct<T>,
    stream: Option<AuthenticatedRouteStream>,
    pending: Option<Receive<T>>,
    pending_observation: Option<Observation<T>>,
    next_sequence: u64,
    finished: bool,
}

impl<T: DeserializeOwned + Send + Sync + 'static> RemoteProductSubscription<T> {
    pub fn product(&self) -> &RetainedProduct<T> {
        &self.product
    }
    pub fn next_sequence(&self) -> u64 {
        self.next_sequence
    }
    pub fn is_closed(&self) -> bool {
        self.finished
    }
    pub fn end_notice(&self) -> Option<ObservationEnd> {
        self.product.end_notice()
    }

    /// Wait for the next pushed event, retaining any observation first.
    /// A cancelled wait preserves partial framing for the next call. On local
    /// retention failure, adjust limits and retry: the same observation remains
    /// pending and this method does not read another event or advance its cursor.
    pub async fn next(
        &mut self,
    ) -> Result<Option<RemoteObservationEvent<T>>, ComponentProtocolError> {
        if self.finished {
            return Ok(None);
        }
        if self.pending_observation.is_none() {
            if self.pending.is_none() {
                let mut stream = self
                    .stream
                    .take()
                    .expect("active subscription owns its stream");
                self.pending = Some(Box::pin(async move {
                    let event = receive_event(&mut stream).await;
                    (stream, event)
                }));
            }
            let (stream, event) = self.pending.as_mut().expect("pending receive").await;
            self.pending = None;
            self.stream = Some(stream);
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    self.finish();
                    return Err(error);
                }
            };
            match event {
                RemoteObservationEvent::Observation(observation) => {
                    if observation.output != self.product.manifest.producer
                        || observation.sequence != self.next_sequence
                    {
                        self.finish();
                        return Err(ComponentProtocolError::InvalidResponse(
                            "observation violates the exact source/cursor".into(),
                        ));
                    }
                    self.pending_observation = Some(observation);
                }
                RemoteObservationEvent::Gap(gap) => {
                    if gap.requested_sequence != self.next_sequence
                        || gap.available_from <= gap.requested_sequence
                    {
                        self.finish();
                        return Err(ComponentProtocolError::InvalidResponse(
                            "invalid subscription gap".into(),
                        ));
                    }
                    self.next_sequence = gap.available_from;
                    return Ok(Some(RemoteObservationEvent::Gap(gap)));
                }
                RemoteObservationEvent::Closed(end) => {
                    let closed = end
                        .clone()
                        .map(|notice| self.product.close_with_notice(notice))
                        .transpose();
                    self.finish();
                    closed.map_err(|error| {
                        ComponentProtocolError::InvalidResponse(error.to_string())
                    })?;
                    return Ok(Some(RemoteObservationEvent::Closed(end)));
                }
            }
        }
        let observation = self
            .pending_observation
            .as_ref()
            .expect("pending observation");
        self.product
            .buffer()
            .append_shared(Arc::new(Envelope::new(
                observation.sequence,
                observation.timestamp_ns,
                observation.clone(),
            )))
            .map_err(|error| ComponentProtocolError::Import(error.to_string()))?;
        self.next_sequence = observation.sequence.saturating_add(1);
        Ok(Some(RemoteObservationEvent::Observation(
            self.pending_observation.take().unwrap(),
        )))
    }

    /// Cancel, release pending data, and close local readers. No remote producer
    /// end is invented. An in-flight read is dropped; an idle stream is closed.
    pub async fn close(&mut self) -> Result<(), ComponentProtocolError> {
        let stream = self.stream.take();
        self.finish();
        if let Some(mut stream) = stream {
            close_stream(&mut stream).await?;
        }
        Ok(())
    }

    fn finish(&mut self) {
        self.finished = true;
        self.stream = None;
        self.pending = None;
        self.pending_observation = None;
        self.product.buffer().close();
    }
}

impl<T> Drop for RemoteProductSubscription<T> {
    fn drop(&mut self) {
        self.product.buffer().close();
    }
}

async fn receive_event<T: DeserializeOwned>(
    stream: &mut AuthenticatedRouteStream,
) -> Result<RemoteObservationEvent<T>, ComponentProtocolError> {
    // Idle producers are valid. Once a frame starts, its full transfer is bounded.
    let mut first = [0];
    stream
        .read_exact(&mut first)
        .await
        .map_err(|error| ComponentProtocolError::Wire(error.to_string()))?;
    deadline(
        ComponentProtocolOperation::Exchange,
        async {
            let header: SubscriptionEventHeader = read_json_after_first(stream, first[0]).await?;
            match header {
                SubscriptionEventHeader::Observation { record } => {
                    if record.payload_encoding != "application/json" {
                        return Err(ComponentProtocolError::InvalidResponse(
                            "unsupported payload encoding".into(),
                        ));
                    }
                    let payload = read_payload(stream, record.payload_bytes).await?;
                    let payload = serde_json::from_slice(&payload)
                        .map_err(|error| ComponentProtocolError::Codec(error.to_string()))?;
                    Ok(RemoteObservationEvent::Observation(Observation {
                        output: record.output,
                        sequence: record.sequence,
                        timestamp_ns: record.timestamp_ns,
                        payload: Arc::new(payload),
                    }))
                }
                SubscriptionEventHeader::Gap { gap } => Ok(RemoteObservationEvent::Gap(gap)),
                SubscriptionEventHeader::Closed { end } => {
                    Ok(RemoteObservationEvent::Closed(end.map(|end| *end)))
                }
                SubscriptionEventHeader::Unavailable { code, message } => {
                    Err(ComponentProtocolError::RemoteRejected { code, message })
                }
            }
        },
        OBSERVATION_EXCHANGE_TIMEOUT,
    )
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_components::{
        ComponentRuntime, ComponentSpec, ConfiguredObservableSpec, Exposure, GaugePayloadContract,
        ObservableContract, ObservationAccess, PayloadContract,
    };
    use futures::executor::block_on;

    #[test]
    fn slow_source_cursors_do_not_pin_history_and_report_eviction_independently() {
        block_on(async {
            let runtime = ComponentRuntime::new("source");
            let component = runtime
                .component(ComponentSpec::new("gauge").observable(ObservableContract {
                    name: "level".into(),
                    datatype: "float64".into(),
                    schema: "test.level/v1".into(),
                    access: vec![ObservationAccess::FollowNew],
                    exposure: Exposure::Cluster,
                }))
                .unwrap();
            let output = component
                .configured_observable::<f64>(ConfiguredObservableSpec::new(
                    "level",
                    "level-1",
                    "clock",
                    PayloadContract::Gauge(GaugePayloadContract {
                        datatype: "float64".into(),
                        schema: "test.level/v1".into(),
                        observes: "test".into(),
                        unit: "percent".into(),
                    }),
                ))
                .unwrap();
            component.expose().unwrap();
            let capture = runtime
                .capture_buffer("history", &output, BufferLimits::entries(2), |_| 8)
                .unwrap();
            let source = SubscriptionSource::new(&capture.product());
            let mut slow = (source.open)(ObservationStart::NewOnly);
            let mut fast = (source.open)(ObservationStart::NewOnly);
            for sequence in 0..10 {
                output
                    .publish(sequence + 1, Arc::new(sequence as f64))
                    .unwrap();
                let SourceEvent::Observation(observation) =
                    fast.events.next().await.unwrap().unwrap()
                else {
                    panic!("fast observer lost data");
                };
                assert_eq!(observation.header.sequence, sequence);
                assert_eq!(
                    capture.product().buffer().range().entries,
                    (sequence + 1).min(2) as usize
                );
            }
            let SourceEvent::Gap(gap) = slow.events.next().await.unwrap().unwrap() else {
                panic!("slow observer must report eviction");
            };
            assert_eq!(
                gap,
                SourceGap {
                    requested_sequence: 0,
                    available_from: 8
                }
            );
            for sequence in [8, 9] {
                let SourceEvent::Observation(observation) =
                    slow.events.next().await.unwrap().unwrap()
                else {
                    panic!("missing retained tail");
                };
                assert_eq!(observation.header.sequence, sequence);
            }
            capture.cancel();
            assert!(matches!(
                slow.events.next().await.unwrap().unwrap(),
                SourceEvent::Closed(None)
            ));
            assert!(slow.events.next().await.is_none());
            assert!(matches!(
                fast.events.next().await.unwrap().unwrap(),
                SourceEvent::Closed(None)
            ));
        });
    }
}
