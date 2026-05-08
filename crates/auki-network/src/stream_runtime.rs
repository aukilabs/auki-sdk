//! Typed `Stream<T>` Rust API on top of [`crate::stream_protocol`]'s wire
//! primitives — [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! deliverables #2 + #3, lifted to multi-`T` dispatch by
//! [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) Batch 1.
//!
//! Producer side: [`StreamProvider`] — a callable [`ClusterRuntime`]
//! invokes per inbound substream. The callable returns a [`StreamDispatch`]
//! variant that pairs the typed source-stream with the [`AcceptInfo`]
//! metadata, or [`StreamDispatch::Decline`] with a typed reason. The
//! dispatch enum is *closed* over the SDK-supported `T`s (`JpegFrame`,
//! `PointCloudFrame` today; new variants added per coordinated
//! SDK + consumer release). Each substream is mono-`T`; the producer's
//! callback decides which `T` based on `request.sensor_id`.
//!
//! Consumer side: [`ClusterRuntime::open_stream`] returns a typed
//! [`StreamSubscription<T>`] containing the [`AcceptInfo`] from the
//! producer plus a [`futures::Stream`] of [`Result<ConsumerFrame<T>,
//! StreamError>`] items. The iterator yields one [`Err`] as a final
//! item describing how the stream ended (graceful end-of-stream,
//! connection lost, protocol error) and then returns `None`. The
//! consumer side stays generic over `T` — the consumer statically
//! knows which `T` it expects per call.
//!
//! ## Layered on top of `ClusterRuntime`
//!
//! Per grimsby's "share the swarm" decision — the same [`ClusterRuntime`]
//! that runs ansuz's cluster orchestration drives this protocol too. One
//! libp2p stack, one swarm, one driver task. `ClusterRuntime::spawn`
//! takes a `stream_provider` argument; `ClusterRuntime::open_stream`
//! opens outbound subscriptions through the same swarm. No second
//! Noise handshake, no second connection per peer pair.
//!
//! ## Per-call `T` on the producer side (Dagaz D1)
//!
//! grimsby v1 pinned `T = JpegFrame` at `ClusterRuntime::spawn` time —
//! the producer's callback was `StreamProvider<JpegFrame>`, returning
//! `StreamDecision<JpegFrame>`. Dagaz lifts that pinning so a single
//! daemon can serve multiple `T`s (camera + pointcloud, today). The
//! producer dispatches on `request.sensor_id` and returns a
//! [`StreamDispatch`] variant matching whichever `T` that sensor emits.
//! New `T` = new `StreamDispatch` variant + a coordinated SDK-consumer
//! release; on the wire each substream stays purely typed end-to-end.

use crate::cluster_runtime::ClusterRuntime;
use crate::stream_protocol::{
    AcceptInfo, DeclineReason, EndReason, JpegFrame, PointCloudFrame, PoseStreamFrameWire,
    STREAM_PROTOCOL, StreamMessage, StreamProtocolError, StreamRequest, read_message,
    write_message,
};
use futures::{Stream, StreamExt, channel::mpsc};
use libp2p::{PeerId, StreamProtocol};
use libp2p_stream::OpenStreamError as Libp2pOpenStreamError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

// ─── Producer-side types ─────────────────────────────────────────────────────

/// What the producer hands the SDK on accept. The SDK drains the
/// app-supplied source [`Stream`] of these and writes each as a
/// [`StreamMessage::Frame`] on the wire, stamping `seq` automatically.
///
/// Producer-side timestamping: `timestamp_ns` lives on the producer's
/// session clock — the same clock identified in the
/// [`AcceptInfo::clock_id`] the producer wrote at accept time.
/// Monotonically nondecreasing on a healthy producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducerFrame<T> {
    pub timestamp_ns: i64,
    pub payload: T,
}

/// Source-stream type the app returns inside an `Accept*` variant of
/// [`StreamDispatch`].
///
/// Item is `Result<ProducerFrame<T>, String>` so the producer can end
/// with an error reason — `Some(Err(detail))` is mapped to
/// [`EndReason::ProducerError { detail }`][EndReason::ProducerError]
/// on the wire. `None` (stream returns) is mapped to
/// [`EndReason::SourceEnded`]. The SDK-level shutdown / session-change
/// paths can override either with their own [`EndReason`].
pub type SourceStream<T> =
    Pin<Box<dyn Stream<Item = Result<ProducerFrame<T>, String>> + Send>>;

/// Producer's accept/decline decision for a single inbound request.
/// Closed over the `T`s the SDK supports today: `JpegFrame` (grimsby v1)
/// and `PointCloudFrame` (Dagaz Batch 1, raw CDR per D2). Adding a new
/// `T` is a coordinated SDK + consumer release — bump the runtime, add
/// the variant, every consumer that wants the new sensor type opts in.
///
/// On `Accept*`, the SDK writes [`StreamMessage::Accept(info)`] for the
/// matching `T` and drains the source-Stream onto the substream as
/// [`StreamMessage::Frame`] values until the source ends or the
/// substream closes. On [`StreamDispatch::Decline`], the SDK writes
/// `Decline { reason }` and closes the substream.
pub enum StreamDispatch {
    /// Accept the request with a JPEG source-Stream — grimsby v1's
    /// existing path, byte-for-byte unchanged on the wire from the
    /// pre-Dagaz `StreamProvider<JpegFrame>` shape.
    AcceptJpeg {
        info: AcceptInfo,
        source: SourceStream<JpegFrame>,
    },
    /// Accept the request with a PointCloud source-Stream — Dagaz's new
    /// path. Each [`PointCloudFrame`] carries a single CDR-encoded
    /// `PointCloud2` ROS message; the consumer parses CDR on its side.
    AcceptPointCloud {
        info: AcceptInfo,
        source: SourceStream<PointCloudFrame>,
    },
    /// Accept the request with a PoseStream source-Stream — sawslin
    /// Phase 1 Lane 0 / locked decision #7. Each
    /// [`PoseStreamFrameWire`] wraps a prost-encoded
    /// [`auki_datatypes::pose_stream::PoseStreamFrame`] (`oneof` of
    /// `JointAngles` / `SpatialTransform`); a single dispatch variant
    /// covers boosterapp's joint-angle stream (Phase 1) and
    /// sentinel's per-marker pose stream (Phase 3+) at the protocol
    /// layer. Each substream remains mono-shape in practice — the
    /// `(sensor_id, sensor_hash)` in `info` transitively names which
    /// `oneof` arm the producer will be sending — but the dispatch
    /// variant is the same for both.
    AcceptPoseStream {
        info: AcceptInfo,
        source: SourceStream<PoseStreamFrameWire>,
    },
    /// Decline the request with a typed reason. SDK writes
    /// [`StreamMessage::Decline { reason }`] and closes the substream.
    Decline { reason: DeclineReason },
}

/// The provider callable. Sync return; any async setup the producer
/// needs (subscribing to a fanout channel, opening a hardware handle,
/// allocating buffers) lives *inside* the source-Stream the app
/// constructs and returns.
///
/// The producer dispatches on `request.sensor_id` to pick which
/// [`StreamDispatch`] variant to return — each substream is mono-`T`
/// end-to-end (per grimsby D1: substream lifetime IS the subscription).
///
/// `Send + Sync` because the runtime task holds the callable in an
/// `Arc` shared across spawned per-substream tasks.
pub type StreamProvider = Arc<dyn Fn(StreamRequest) -> StreamDispatch + Send + Sync>;

/// Convenience constructor for consumer-only nodes (Park, analytics
/// tools, future Sentinel-as-consumer) that don't expose any sensors.
/// Declines every inbound request with [`DeclineReason::SensorNotFound`].
pub fn decline_all_streams() -> StreamProvider {
    Arc::new(|_req| StreamDispatch::Decline {
        reason: DeclineReason::SensorNotFound,
    })
}

// ─── Consumer-side types ─────────────────────────────────────────────────────

/// What the consumer reads off the typed iterator. Same shape as
/// [`ProducerFrame<T>`] but with the SDK-stamped `seq` exposed so the
/// consumer can detect drops via gaps in the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumerFrame<T> {
    pub timestamp_ns: i64,
    pub seq: u64,
    pub payload: T,
}

/// Returned by [`ClusterRuntime::open_stream`] on success.
///
/// Iterator semantics: yields `Ok(frame)` for each frame, then a
/// **single final** `Err(StreamError)` describing how the stream ended,
/// then `None`. After the `Err` is yielded the iterator is exhausted.
pub struct StreamSubscription<T> {
    /// Accept-time metadata: `sensor_hash`, `clock_id`, `clock_hash`.
    /// Stable for the lifetime of the subscription — the producer
    /// commits to this at accept time and any change requires opening
    /// a new substream.
    pub info: AcceptInfo,
    /// Typed frame iterator. See struct-level docs for the
    /// terminator-then-`None` pattern.
    pub frames: Pin<Box<dyn Stream<Item = Result<ConsumerFrame<T>, StreamError>> + Send>>,
}

/// Iterator yields one of these as a **single final item** before
/// returning `None`. After the `Err`, the substream is closed and the
/// iterator is exhausted.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Producer ended cleanly; `reason` carries their typed reason.
    /// Surfaces variants like [`EndReason::SourceEnded`],
    /// [`EndReason::ProducerShuttingDown`], [`EndReason::SessionEnded`],
    /// [`EndReason::ProducerError`]. Per grimsby D5c, the consumer's
    /// app-level reconnect policy decides whether to re-request.
    #[error("end of stream: {reason:?}")]
    EndOfStream { reason: EndReason },
    /// Substream closed without an explicit [`StreamMessage::EndOfStream`]
    /// — typically the connection dropped, the producer crashed, or
    /// libp2p tore the substream down. Treat as an implicit
    /// connection-loss equivalent (grimsby D5b — implicit via libp2p
    /// disconnect).
    #[error("connection lost")]
    ConnectionLost,
    /// Peer sent malformed bytes or a wire-incompatible payload.
    /// Substream is closed.
    #[error("protocol error: {0}")]
    Protocol(#[source] StreamProtocolError),
}

/// [`ClusterRuntime::open_stream`] failure modes — i.e., the request
/// never even got to "yielding frames."
#[derive(Debug, thiserror::Error)]
pub enum OpenStreamError {
    /// Producer accepted the substream but declined the request with a
    /// typed reason. Consumer can re-request later if the producer's
    /// state changed (sensor came back online, etc.) — the SDK doesn't
    /// auto-retry per grimsby D5c.
    #[error("declined: {reason:?}")]
    Declined { reason: DeclineReason },
    /// libp2p couldn't open the substream (transport error, peer not
    /// reachable, peer didn't speak [`STREAM_PROTOCOL`], etc.).
    #[error("libp2p open failed: {0}")]
    LibP2p(#[source] Libp2pOpenStreamError),
    /// Underlying I/O / framing failure during the request/reply
    /// exchange (after libp2p opened the substream but before any
    /// frames flowed).
    #[error("protocol error: {0}")]
    Protocol(#[source] StreamProtocolError),
    /// Substream open didn't complete inside [`OPEN_STREAM_TIMEOUT`].
    #[error("open timed out after {0:?}")]
    Timeout(Duration),
}

/// How long [`ClusterRuntime::open_stream`] waits for the responder's
/// `Accept` / `Decline` reply before erroring out with
/// [`OpenStreamError::Timeout`]. A producer's `stream_provider`
/// callable is meant to be sync-fast (per grimsby D3 — async setup
/// lives inside the source-Stream); 30 s is a generous upper bound for
/// the libp2p substream open + wire-handshake round trip.
pub const OPEN_STREAM_TIMEOUT: Duration = Duration::from_secs(30);

// ─── Public methods on `ClusterRuntime` ──────────────────────────────────────

impl ClusterRuntime {
    /// Open an outbound stream subscription on `peer_id` for the named
    /// sensor (or other addressable thing per [`StreamRequest`]).
    ///
    /// Returns once the producer has either Accepted or Declined the
    /// request. On accept, the returned [`StreamSubscription<T>::frames`]
    /// is a typed async iterator the caller drives; on decline, returns
    /// [`OpenStreamError::Declined { reason }`].
    ///
    /// Stream lifetime: the substream stays open as long as the consumer
    /// keeps polling `frames` AND the producer keeps yielding. Either
    /// side can end it — producer by yielding `None` (clean, surfaces as
    /// `Err(StreamError::EndOfStream)`) or by closing the substream
    /// (surfaces as `Err(StreamError::ConnectionLost)`); consumer by
    /// dropping the [`StreamSubscription`] (closes the substream
    /// immediately, which the producer's source-Stream observes via its
    /// own `Drop`).
    ///
    /// Reconnection on stream death is the consumer's responsibility per
    /// grimsby D5c. The SDK never silently retries.
    pub async fn open_stream<T>(
        &self,
        peer_id: PeerId,
        request: StreamRequest,
    ) -> Result<StreamSubscription<T>, OpenStreamError>
    where
        T: Serialize + DeserializeOwned + Send + 'static,
    {
        // Each `open_stream` call gets its own clone of the Control. The
        // libp2p-stream backpressure mechanism rate-limits per-Control
        // (one open at a time per `&mut self`), so cloning per call lets
        // grimsby support concurrent opens to N peers — the typical Park
        // shape (one tile per (peer, sensor) → N concurrent
        // subscriptions). Per the Control docs: "This backpressure
        // mechanism breaks if you clone Controls excessively." For
        // grimsby's open-once-per-tile-mount cadence this is fine.
        let mut control = self.stream_control().clone();

        let proto = StreamProtocol::try_from_owned(STREAM_PROTOCOL.to_string())
            .expect("STREAM_PROTOCOL is a valid libp2p stream protocol id");

        let open_fut = control.open_stream(peer_id, proto);
        let mut substream = match tokio::time::timeout(OPEN_STREAM_TIMEOUT, open_fut).await {
            Err(_) => return Err(OpenStreamError::Timeout(OPEN_STREAM_TIMEOUT)),
            Ok(Err(e)) => return Err(OpenStreamError::LibP2p(e)),
            Ok(Ok(s)) => s,
        };

        // Wire handshake: write Request, read Reply.
        let req_msg: StreamMessage<T> = StreamMessage::Request(request);
        write_message(&mut substream, &req_msg)
            .await
            .map_err(OpenStreamError::Protocol)?;

        let reply: StreamMessage<T> = match tokio::time::timeout(
            OPEN_STREAM_TIMEOUT,
            read_message(&mut substream),
        )
        .await
        {
            Err(_) => return Err(OpenStreamError::Timeout(OPEN_STREAM_TIMEOUT)),
            Ok(Err(e)) => return Err(OpenStreamError::Protocol(e)),
            Ok(Ok(m)) => m,
        };

        let info = match reply {
            StreamMessage::Accept(info) => info,
            StreamMessage::Decline { reason } => {
                return Err(OpenStreamError::Declined { reason });
            }
            _other => {
                // Producer wrote something other than Accept / Decline as
                // its first reply. Wire-protocol violation; treat as a
                // protocol error. (Don't echo the message body in the
                // error — `T` may not be `Debug` and the caller has no
                // recovery action for "unexpected reply variant" anyway.)
                return Err(OpenStreamError::Protocol(
                    StreamProtocolError::Deserialize(serde_json::Error::io(
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "expected Accept or Decline as first reply",
                        ),
                    )),
                ));
            }
        };

        // Spawn the consumer-side reader task. Frames flow through an
        // mpsc channel; the returned `frames` iterator is the receiver
        // side. When the consumer drops the StreamSubscription, the
        // receiver drops, the channel closes, the reader task's `send`
        // fails on the next frame, and the task exits — substream drops,
        // libp2p closes it cleanly on the wire, the producer's source
        // gets dropped on the producer side via the same chain.
        let (tx, rx) = mpsc::channel::<Result<ConsumerFrame<T>, StreamError>>(8);
        tokio::spawn(consumer_reader_task::<T>(substream, tx));

        Ok(StreamSubscription {
            info,
            frames: Box::pin(rx),
        })
    }
}

// ─── Internal helpers used by `cluster_runtime::run_task` ────────────────────

/// Producer-side per-substream task. Spawned by `cluster_runtime`'s
/// driver task for each inbound substream that arrives on
/// [`STREAM_PROTOCOL`]. Non-generic — reads the request type-free,
/// invokes the provider, then dispatches to [`pump_typed`] with the
/// concrete `T` matching the [`StreamDispatch`] variant.
///
/// 1. Read [`StreamMessage::Request`] (the Request variant carries no
///    `T`-dependent data, so we read it as `StreamMessage<JpegFrame>`
///    — any concrete `T` works for the Request shape).
/// 2. Invoke `provider(request) -> StreamDispatch`.
/// 3. Match the dispatch variant; for Accept variants, hand off to
///    [`pump_typed::<T>`] with the matching `T`. Decline → write
///    [`StreamMessage::Decline { reason }`] and close.
///
/// Errors during read/write (peer disconnected, libp2p tore down the
/// substream) terminate the task silently — the substream is dead
/// already.
pub(crate) async fn handle_inbound_substream(
    _peer: PeerId,
    mut substream: libp2p::Stream,
    provider: StreamProvider,
    shutdown_rx: watch::Receiver<bool>,
) {
    // 1. Read the request. The Request variant doesn't carry any
    //    `T`-dependent payload, so deserializing as
    //    `StreamMessage<JpegFrame>` accepts the bytes regardless of
    //    what `T` the producer ultimately picks. If a peer bizarrely
    //    sent a Frame as its first message (wire-protocol violation),
    //    the deserialize either fails or yields a different variant
    //    and we drop the substream.
    let request: StreamRequest = match read_message::<JpegFrame, _>(&mut substream).await {
        Ok(StreamMessage::Request(req)) => req,
        Ok(_other) => {
            // Peer wrote something other than Request first. Drop;
            // their open_stream call surfaces the protocol error.
            return;
        }
        Err(_) => {
            // Read failed — substream's already broken.
            return;
        }
    };

    // 2. Invoke the provider — non-generic; closed enum tells us which
    //    `T` to pump.
    let dispatch = (provider)(request);

    match dispatch {
        StreamDispatch::Decline { reason } => {
            // Decline payload is `T`-free; write with any `T`.
            let msg: StreamMessage<JpegFrame> = StreamMessage::Decline { reason };
            let _ = write_message(&mut substream, &msg).await;
        }
        StreamDispatch::AcceptJpeg { info, source } => {
            pump_typed::<JpegFrame>(substream, info, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptPointCloud { info, source } => {
            pump_typed::<PointCloudFrame>(substream, info, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptPoseStream { info, source } => {
            pump_typed::<PoseStreamFrameWire>(substream, info, source, shutdown_rx).await;
        }
    }
}

/// Per-`T` source-Stream pump. Writes [`StreamMessage::Accept(info)`]
/// then drains `source` onto the substream as `StreamMessage::Frame`
/// values with auto-stamped `seq`. Honors the shutdown signal with an
/// explicit `EndOfStream { reason: ProducerShuttingDown }` (grimsby
/// D5b — best-effort explicit). Source returns `None` →
/// `EndOfStream { reason: SourceEnded }`. Source returns
/// `Some(Err(detail))` → `EndOfStream { reason: ProducerError { detail } }`.
///
/// Generic over `T`: the SDK monomorphizes one copy per variant
/// (`JpegFrame`, `PointCloudFrame`). Adding a new variant means adding
/// a new monomorphization plus extending [`StreamDispatch`].
async fn pump_typed<T>(
    mut substream: libp2p::Stream,
    info: AcceptInfo,
    mut source: SourceStream<T>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    // Write accept.
    let accept_msg: StreamMessage<T> = StreamMessage::Accept(info);
    if write_message(&mut substream, &accept_msg).await.is_err() {
        return;
    }
    // Drain source into Frame messages until end-of-source, shutdown
    // signal, or send error. Shutdown branch writes an explicit
    // `EndOfStream { reason: ProducerShuttingDown }` before exiting
    // (per grimsby D5b — best-effort explicit).
    let mut seq: u64 = 0;
    let end_reason = loop {
        tokio::select! {
            biased;

            // Shutdown signal raced ahead of the next frame — flush a
            // typed EndOfStream and exit. The ClusterRuntime gives us
            // a brief grace period (SHUTDOWN_GRACE) before the swarm
            // tears down, so there's time for this write to complete
            // on a healthy LAN substream.
            res = shutdown_rx.changed() => {
                if res.is_ok() && *shutdown_rx.borrow() {
                    break EndReason::ProducerShuttingDown;
                }
                // Sender dropped without setting true (runtime exiting
                // via Drop, not shutdown(self)) — fall through to the
                // implicit `ConnectionLost` path on the next pump-
                // iteration write failure. Continue polling source.
                continue;
            }

            item = source.next() => match item {
                Some(Ok(frame)) => {
                    let msg: StreamMessage<T> = StreamMessage::Frame {
                        timestamp_ns: frame.timestamp_ns,
                        seq,
                        payload: frame.payload,
                    };
                    if write_message(&mut substream, &msg).await.is_err() {
                        // Consumer disconnected mid-stream. Don't try
                        // to write EndOfStream — the substream is dead.
                        return;
                    }
                    seq = seq.wrapping_add(1);
                }
                Some(Err(detail)) => break EndReason::ProducerError { detail },
                None => break EndReason::SourceEnded,
            },
        }
    };
    // Write the final EndOfStream. Best-effort — if the consumer
    // already disconnected, the write fails silently.
    let end_msg: StreamMessage<T> = StreamMessage::EndOfStream { reason: end_reason };
    let _ = write_message(&mut substream, &end_msg).await;
}

/// Consumer-side per-substream reader task. Spawned by
/// [`ClusterRuntime::open_stream`] after the producer accepts.
///
/// Reads [`StreamMessage::Frame`] values into an mpsc channel until the
/// substream yields a terminal message (EndOfStream / unexpected variant)
/// or an I/O error. Sends a single terminal `Err(StreamError)` to the
/// channel before exiting; consumer's iterator surfaces it as the final
/// `Err` item.
async fn consumer_reader_task<T>(
    mut substream: libp2p::Stream,
    mut tx: mpsc::Sender<Result<ConsumerFrame<T>, StreamError>>,
) where
    T: DeserializeOwned + Send + 'static,
{
    use futures::SinkExt;

    loop {
        let msg: StreamMessage<T> = match read_message(&mut substream).await {
            Ok(m) => m,
            Err(StreamProtocolError::Io(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // Substream closed without an explicit EndOfStream —
                // grimsby D5b's "implicit via libp2p disconnect" path.
                let _ = tx.send(Err(StreamError::ConnectionLost)).await;
                return;
            }
            Err(e) => {
                let _ = tx.send(Err(StreamError::Protocol(e))).await;
                return;
            }
        };

        match msg {
            StreamMessage::Frame { timestamp_ns, seq, payload } => {
                let frame = ConsumerFrame { timestamp_ns, seq, payload };
                if tx.send(Ok(frame)).await.is_err() {
                    // Consumer dropped the StreamSubscription. Stop
                    // reading; substream drops with the task and
                    // libp2p closes it cleanly.
                    return;
                }
            }
            StreamMessage::EndOfStream { reason } => {
                let _ = tx.send(Err(StreamError::EndOfStream { reason })).await;
                return;
            }
            // Producer wrote something out-of-order (Request / Accept /
            // Decline mid-stream). Wire-protocol violation; surface as
            // Protocol error. (Don't echo the message body in the error
            // — `T` may not be `Debug` and the caller has no recovery
            // action for "unexpected mid-stream variant.")
            _other => {
                let err = StreamProtocolError::Deserialize(serde_json::Error::io(
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "unexpected message after Accept",
                    ),
                ));
                let _ = tx.send(Err(StreamError::Protocol(err))).await;
                return;
            }
        }
    }
}

// Type alias re-exports for ergonomics — consumers reading
// `auki_network::stream_runtime::*` get `JpegFrame` here without the
// extra `stream_protocol::` hop.
pub use crate::stream_protocol::JpegFrame as _JpegFrameReExport;
#[allow(unused_imports)]
use _JpegFrameReExport as _;

// Static check that `JpegFrame` satisfies the bounds the runtime expects
// for `T`. If a future refactor breaks this, the test below catches it.
const _: fn() = || {
    fn assert_serialize_send_static<T: Serialize + Send + 'static>() {}
    fn assert_deserialize_send_static<T: DeserializeOwned + Send + 'static>() {}
    assert_serialize_send_static::<JpegFrame>();
    assert_deserialize_send_static::<JpegFrame>();
};

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use crate::cluster_doc::{ClusterDoc, ClusterPeer};
    use crate::cluster_runtime::ClusterRuntime;
    use crate::swarm::{Behaviour, SwarmConfig, build_swarm};
    use futures::stream;
    use libp2p::Swarm;
    use libp2p::swarm::SwarmEvent;
    use std::time::Instant;

    fn test_swarm_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
            enable_mdns: false,
            enable_relay_server: false,
        }
    }

    async fn wait_for_listen_addr(swarm: &mut Swarm<Behaviour>) -> libp2p::Multiaddr {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(SwarmEvent::NewListenAddr { address, .. }) = swarm.next().await {
                    return address;
                }
            }
        })
        .await
        .expect("listen addr did not appear within timeout")
    }

    fn fixture_participant_provider(peer_id: PeerId, name: &str) -> crate::cluster_runtime::ParticipantInfoProvider {
        let name = name.to_string();
        let session_id = format!("session-{name}");
        let session_clock_id = format!("{name}/clock");
        Arc::new(move || {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0);
            Some(crate::ParticipantInfo {
                app: "test".into(),
                name: name.clone(),
                session_id: session_id.clone(),
                session_clock_id: session_clock_id.clone(),
                session_clock_hash: "deadbeef".into(),
                session_now_ns: now,
                cluster_joined_at_ns: None,
                peer_id,
                app_instance: "00163eabcdef".into(),
            })
        })
    }

    async fn poll_until<F: FnMut() -> bool>(mut cond: F, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn jpeg_provider_yielding_three_frames() -> StreamProvider {
        Arc::new(|_req| {
            let frames = vec![
                Ok(ProducerFrame {
                    timestamp_ns: 1_000,
                    payload: JpegFrame { bytes: vec![0xff, 0xd8, 0x01] },
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 2_000,
                    payload: JpegFrame { bytes: vec![0xff, 0xd8, 0x02] },
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 3_000,
                    payload: JpegFrame { bytes: vec![0xff, 0xd8, 0x03] },
                }),
            ];
            StreamDispatch::AcceptJpeg {
                info: AcceptInfo {
                    sensor_hash: "sensor-hash-3".into(),
                    clock_id: "test/session-monotonic".into(),
                    clock_hash: "clock-hash-3".into(),
                },
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    fn jpeg_provider_declines_unknown() -> StreamProvider {
        Arc::new(|req| {
            if req.sensor_id == "exists" {
                let frames = vec![Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: JpegFrame { bytes: vec![0xff] },
                })];
                StreamDispatch::AcceptJpeg {
                    info: AcceptInfo {
                        sensor_hash: "h".into(),
                        clock_id: "c".into(),
                        clock_hash: "ch".into(),
                    },
                    source: Box::pin(stream::iter(frames)),
                }
            } else {
                StreamDispatch::Decline {
                    reason: DeclineReason::SensorNotFound,
                }
            }
        })
    }

    fn jpeg_provider_yields_then_errors() -> StreamProvider {
        Arc::new(|_req| {
            let items = vec![
                Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: JpegFrame { bytes: vec![0xaa] },
                }),
                Err("encoder died".to_string()),
            ];
            StreamDispatch::AcceptJpeg {
                info: AcceptInfo {
                    sensor_hash: "h".into(),
                    clock_id: "c".into(),
                    clock_hash: "ch".into(),
                },
                source: Box::pin(stream::iter(items)),
            }
        })
    }

    /// Dagaz fixture — yields three CDR-shaped (mock) PointCloudFrames.
    /// Each `bytes` payload is a small distinguishable byte sequence so
    /// the e2e test asserts the wire round trip end-to-end. Bandwidth
    /// is irrelevant at three small frames; the production path
    /// compresses by base64 inside JSON regardless.
    fn pointcloud_provider_yielding_three_frames() -> StreamProvider {
        Arc::new(|_req| {
            let frames = vec![
                Ok(ProducerFrame {
                    timestamp_ns: 10_000,
                    payload: PointCloudFrame {
                        bytes: vec![0xCD, 0xAA, 0x01, 0x02, 0x03],
                    },
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 20_000,
                    payload: PointCloudFrame {
                        bytes: vec![0xCD, 0xAA, 0x04, 0x05, 0x06],
                    },
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 30_000,
                    payload: PointCloudFrame {
                        bytes: vec![0xCD, 0xAA, 0x07, 0x08, 0x09],
                    },
                }),
            ];
            StreamDispatch::AcceptPointCloud {
                info: AcceptInfo {
                    sensor_hash: "pc-sensor-hash-3".into(),
                    clock_id: "pc/test/session-monotonic".into(),
                    clock_hash: "pc-clock-hash-3".into(),
                },
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    /// PoseStream test fixture (sawslin Phase 1 Lane 0). Yields three
    /// `PoseStreamFrame`s — two `JointAngles` arms then one
    /// `SpatialTransform` arm — to exercise the oneof envelope on the
    /// wire. Real producers stay mono-shape per substream; this
    /// fixture mixes them for round-trip coverage.
    fn pose_stream_provider_yielding_three_frames() -> StreamProvider {
        use auki_datatypes::joint_state::JointAngles;
        use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
        use auki_datatypes::pose_stream::{PoseStreamFrame, pose_stream_frame::Payload};

        Arc::new(|_req| {
            let f0 = PoseStreamFrame {
                payload: Some(Payload::JointAngles(JointAngles {
                    angles: vec![0.0, 0.5, -0.5],
                })),
            };
            let f1 = PoseStreamFrame {
                payload: Some(Payload::JointAngles(JointAngles {
                    angles: vec![0.1, 0.6, -0.4],
                })),
            };
            let f2 = PoseStreamFrame {
                payload: Some(Payload::SpatialTransform(SpatialTransform {
                    translation: Some(Vec3 {
                        x: 1.0,
                        y: 2.0,
                        z: 3.0,
                    }),
                    orientation: Some(Quat {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    }),
                })),
            };
            let frames = vec![
                Ok(ProducerFrame {
                    timestamp_ns: 100,
                    payload: PoseStreamFrameWire::encode(&f0),
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 200,
                    payload: PoseStreamFrameWire::encode(&f1),
                }),
                Ok(ProducerFrame {
                    timestamp_ns: 300,
                    payload: PoseStreamFrameWire::encode(&f2),
                }),
            ];
            StreamDispatch::AcceptPoseStream {
                info: AcceptInfo {
                    sensor_hash: "pose-sensor-hash-1".into(),
                    clock_id: "pose/test/session-monotonic".into(),
                    clock_hash: "pose-clock-hash-1".into(),
                },
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    /// A `sensor_id`-keyed provider that demonstrates the multi-`T`
    /// dispatch on the producer side: requests for `"camera"` get
    /// `AcceptJpeg`, `"pointcloud"` gets `AcceptPointCloud`. This is
    /// the shape Booster will use once the daemon side wires its
    /// PreviewFanout + PointCloudFanout (Dagaz Batch 3).
    fn multi_t_provider() -> StreamProvider {
        Arc::new(|req| match req.sensor_id.as_str() {
            "camera" => {
                let frames = vec![Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: JpegFrame {
                        bytes: vec![0xff, 0xd8, 0xab],
                    },
                })];
                StreamDispatch::AcceptJpeg {
                    info: AcceptInfo {
                        sensor_hash: "cam-hash".into(),
                        clock_id: "shared/clock".into(),
                        clock_hash: "shared-clock-hash".into(),
                    },
                    source: Box::pin(stream::iter(frames)),
                }
            }
            "pointcloud" => {
                let frames = vec![Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: PointCloudFrame {
                        bytes: vec![0xCD, 0xCD, 0xCD],
                    },
                })];
                StreamDispatch::AcceptPointCloud {
                    info: AcceptInfo {
                        sensor_hash: "pc-hash".into(),
                        clock_id: "shared/clock".into(),
                        clock_hash: "shared-clock-hash".into(),
                    },
                    source: Box::pin(stream::iter(frames)),
                }
            }
            _ => StreamDispatch::Decline {
                reason: DeclineReason::SensorNotFound,
            },
        })
    }

    fn cluster_peer(peer_id: PeerId, addr: libp2p::Multiaddr) -> ClusterPeer {
        ClusterPeer {
            peer_id,
            addresses: vec![addr],
            expected_app_id: None,
            note: None,
        }
    }

    async fn build_listening_swarm(
        identity: &PeerIdentity,
        agent_version: &str,
    ) -> (Swarm<Behaviour>, libp2p::Multiaddr) {
        let mut swarm = build_swarm(identity, test_swarm_config(agent_version)).unwrap();
        let addr = wait_for_listen_addr(&mut swarm).await;
        (swarm, addr)
    }

    /// Two runtimes; producer accepts and yields 3 frames; consumer
    /// reads 3 frames + a final `Err(EndOfStream { reason: SourceEnded })`,
    /// then None.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_jpeg_frames() {
        let id_p = PeerIdentity::from_seed(&[101u8; 32]);
        let id_c = PeerIdentity::from_seed(&[102u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-stream-happy-path".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            jpeg_provider_yielding_three_frames(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        // Wait for cluster connection so open_stream can route through
        // an existing libp2p connection rather than dialing.
        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not see producer in cluster within 15s");

        let sub: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/cam".into(),
                },
            )
            .await
            .expect("open_stream should succeed");

        assert_eq!(sub.info.sensor_hash, "sensor-hash-3");
        assert_eq!(sub.info.clock_id, "test/session-monotonic");
        assert_eq!(sub.info.clock_hash, "clock-hash-3");

        let mut frames = sub.frames;
        let frame_0 = frames.next().await.unwrap().expect("frame 0 ok");
        assert_eq!(frame_0.seq, 0);
        assert_eq!(frame_0.timestamp_ns, 1_000);
        assert_eq!(frame_0.payload.bytes, vec![0xff, 0xd8, 0x01]);

        let frame_1 = frames.next().await.unwrap().expect("frame 1 ok");
        assert_eq!(frame_1.seq, 1);
        assert_eq!(frame_1.timestamp_ns, 2_000);

        let frame_2 = frames.next().await.unwrap().expect("frame 2 ok");
        assert_eq!(frame_2.seq, 2);
        assert_eq!(frame_2.timestamp_ns, 3_000);
        assert_eq!(frame_2.payload.bytes, vec![0xff, 0xd8, 0x03]);

        let end = frames.next().await.unwrap().expect_err("expected end-of-stream marker");
        match end {
            StreamError::EndOfStream { reason: EndReason::SourceEnded } => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }

        // Iterator exhausted after the terminator.
        assert!(frames.next().await.is_none());

        producer.shutdown();
        consumer.shutdown();
    }

    /// Producer's provider returns Decline for unknown sensor_id; consumer
    /// gets `Err(OpenStreamError::Declined { reason: SensorNotFound })`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_declines_unknown_sensor() {
        let id_p = PeerIdentity::from_seed(&[103u8; 32]);
        let id_c = PeerIdentity::from_seed(&[104u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-stream-decline".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            jpeg_provider_declines_unknown(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not see producer within 15s");

        let result: Result<StreamSubscription<JpegFrame>, OpenStreamError> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "does-not-exist".into(),
                },
            )
            .await;

        match result {
            Err(OpenStreamError::Declined {
                reason: DeclineReason::SensorNotFound,
            }) => {}
            Err(other) => panic!("expected Declined(SensorNotFound), got error {other:?}"),
            Ok(_sub) => panic!("expected Declined, got Ok subscription"),
        }

        producer.shutdown();
        consumer.shutdown();
    }

    /// Producer's source-Stream yields Ok(frame) then Err("detail");
    /// consumer reads the frame then sees
    /// Err(EndOfStream { reason: ProducerError { detail: "encoder died" } }).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_error_signals_consumer_with_detail() {
        let id_p = PeerIdentity::from_seed(&[105u8; 32]);
        let id_c = PeerIdentity::from_seed(&[106u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-stream-producer-error".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            jpeg_provider_yields_then_errors(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        let sub: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "any".into(),
                },
            )
            .await
            .expect("open_stream");
        let mut frames = sub.frames;

        let f0 = frames.next().await.unwrap().expect("first frame ok");
        assert_eq!(f0.seq, 0);

        let end = frames.next().await.unwrap().expect_err("expected error end");
        match end {
            StreamError::EndOfStream {
                reason: EndReason::ProducerError { detail },
            } => assert_eq!(detail, "encoder died"),
            other => panic!("expected ProducerError, got {other:?}"),
        }
        assert!(frames.next().await.is_none());

        producer.shutdown();
        consumer.shutdown();
    }

    /// Producer's source-Stream is `pending` (never yields after the first
    /// frame). Producer's runtime is then `shutdown(self)` — consumer should
    /// see a typed `Err(EndOfStream { reason: ProducerShuttingDown })`
    /// rather than the implicit `ConnectionLost` (per grimsby D5b — best-
    /// effort explicit EndOfStream from the producer's shutdown path).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_shutdown_signals_consumer_with_typed_end_of_stream() {
        let id_p = PeerIdentity::from_seed(&[107u8; 32]);
        let id_c = PeerIdentity::from_seed(&[108u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-stream-producer-shutdown".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        // Source-Stream that yields one frame then never yields again.
        // Forces the producer to still be in the pump loop (rather than
        // having returned None and ended naturally) when shutdown fires.
        let provider: StreamProvider = Arc::new(|_req| {
            let first = stream::iter(vec![Ok(ProducerFrame {
                timestamp_ns: 1_000,
                payload: JpegFrame { bytes: vec![0xff, 0xd8, 0x99] },
            })]);
            let then_pending = stream::pending::<Result<ProducerFrame<JpegFrame>, String>>();
            StreamDispatch::AcceptJpeg {
                info: AcceptInfo {
                    sensor_hash: "shutdown-test".into(),
                    clock_id: "test/clock".into(),
                    clock_hash: "h".into(),
                },
                source: Box::pin(first.chain(then_pending)),
            }
        });

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            provider,
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        let sub: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "any".into(),
                },
            )
            .await
            .expect("open_stream");
        let mut frames = sub.frames;

        // Read the first frame to confirm the stream is live.
        let f0 = frames.next().await.unwrap().expect("first frame ok");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.payload.bytes, vec![0xff, 0xd8, 0x99]);

        // Shut down the producer. Per grimsby D5b, the producer's
        // per-substream task should flush a typed EndOfStream{
        // ProducerShuttingDown} before the swarm tears the connection
        // down — within the SHUTDOWN_GRACE window the runtime allows
        // before exiting `run_task`.
        producer.shutdown();

        // Read the next item from the iterator. Allow up to 5s — well
        // beyond SHUTDOWN_GRACE (100ms) plus normal libp2p propagation.
        let next = tokio::time::timeout(Duration::from_secs(5), frames.next())
            .await
            .expect("frame iterator hung after producer shutdown")
            .expect("iterator ended without terminator")
            .expect_err("expected typed end-of-stream");

        match next {
            StreamError::EndOfStream {
                reason: EndReason::ProducerShuttingDown,
            } => {}
            other => panic!(
                "expected EndOfStream(ProducerShuttingDown), got {other:?}"
            ),
        }
        assert!(frames.next().await.is_none());

        consumer.shutdown();
    }

    /// Consumer's `cluster.json` lists a peer that doesn't actually
    /// exist on the network. The consumer's `open_stream` call should
    /// surface the failure as a typed `OpenStreamError` — not hang
    /// forever, not panic, not return a half-constructed
    /// `StreamSubscription`.
    ///
    /// Either `LibP2p(...)` (libp2p couldn't reach the peer — dial
    /// failed, peer didn't speak the protocol, etc.) or `Timeout(...)`
    /// (the open didn't complete inside `OPEN_STREAM_TIMEOUT`) is
    /// acceptable; both are explicit "the request failed" signals the
    /// consumer's app-level reconnect logic can act on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn open_stream_against_unreachable_peer_surfaces_typed_error() {
        let id_consumer = PeerIdentity::from_seed(&[201u8; 32]);
        let id_unreachable = PeerIdentity::from_seed(&[202u8; 32]);

        let (swarm_c, addr_c) = build_listening_swarm(&id_consumer, "test-consumer/0").await;

        // cluster.json lists a "peer" that's just an identity — no
        // listening swarm at the address. Port 1 is reserved and
        // typically refuses connections immediately on Linux/macOS,
        // which gives libp2p a fast dial-failure signal.
        let unreachable_addr: libp2p::Multiaddr =
            "/ip4/127.0.0.1/tcp/1".parse().unwrap();
        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-unreachable".into(),
            peers: vec![
                cluster_peer(id_consumer.peer_id(), addr_c),
                cluster_peer(id_unreachable.peer_id(), unreachable_addr),
            ],
        };

        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_consumer.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        // open_stream — bounded by OPEN_STREAM_TIMEOUT (30s) on the
        // SDK side; we wrap an outer 35s tokio timeout as a safety net
        // to fail the test rather than hang indefinitely if the
        // bound's broken.
        let result = tokio::time::timeout(
            Duration::from_secs(35),
            consumer.open_stream::<JpegFrame>(
                id_unreachable.peer_id(),
                StreamRequest {
                    sensor_id: "any".into(),
                },
            ),
        )
        .await
        .expect("open_stream did not return inside its own OPEN_STREAM_TIMEOUT bound");

        match result {
            Err(OpenStreamError::LibP2p(_)) => {}
            Err(OpenStreamError::Timeout(_)) => {}
            Err(other) => panic!(
                "expected LibP2p(_) or Timeout(_), got typed error: {other:?}"
            ),
            Ok(_) => panic!("expected an error, got an Ok subscription"),
        }

        consumer.shutdown();
    }

    #[test]
    fn decline_all_streams_returns_sensor_not_found() {
        let provider: StreamProvider = decline_all_streams();
        let req = StreamRequest {
            sensor_id: "anything".into(),
        };
        match provider(req) {
            StreamDispatch::Decline {
                reason: DeclineReason::SensorNotFound,
            } => {}
            _ => panic!("decline_all_streams should always decline with SensorNotFound"),
        }
    }

    // ─── Dagaz Batch 1 e2e tests ─────────────────────────────────────────

    /// E2e happy path for `T = PointCloudFrame`: two cluster runtimes
    /// converge over libp2p, consumer opens a stream, producer's
    /// `AcceptPointCloud` flows three CDR-shaped frames through the
    /// base64-of-binary JSON envelope, consumer reads the typed frames
    /// back, then sees the `EndOfStream { SourceEnded }` terminator.
    /// Exercises the full multi-`T` dispatch chain end-to-end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_pointcloud_frames() {
        let id_p = PeerIdentity::from_seed(&[111u8; 32]);
        let id_c = PeerIdentity::from_seed(&[112u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-pc-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-pc-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-pointcloud-happy-path".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            pointcloud_provider_yielding_three_frames(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not see producer in cluster within 15s");

        let sub: StreamSubscription<PointCloudFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/pc".into(),
                },
            )
            .await
            .expect("open_stream<PointCloudFrame> should succeed");

        assert_eq!(sub.info.sensor_hash, "pc-sensor-hash-3");
        assert_eq!(sub.info.clock_id, "pc/test/session-monotonic");
        assert_eq!(sub.info.clock_hash, "pc-clock-hash-3");

        let mut frames = sub.frames;
        let f0 = frames.next().await.unwrap().expect("frame 0 ok");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.timestamp_ns, 10_000);
        assert_eq!(f0.payload.bytes, vec![0xCD, 0xAA, 0x01, 0x02, 0x03]);

        let f1 = frames.next().await.unwrap().expect("frame 1 ok");
        assert_eq!(f1.seq, 1);
        assert_eq!(f1.timestamp_ns, 20_000);
        assert_eq!(f1.payload.bytes, vec![0xCD, 0xAA, 0x04, 0x05, 0x06]);

        let f2 = frames.next().await.unwrap().expect("frame 2 ok");
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.timestamp_ns, 30_000);
        assert_eq!(f2.payload.bytes, vec![0xCD, 0xAA, 0x07, 0x08, 0x09]);

        let end = frames.next().await.unwrap().expect_err("expected EndOfStream");
        match end {
            StreamError::EndOfStream { reason: EndReason::SourceEnded } => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }
        assert!(frames.next().await.is_none());

        producer.shutdown();
        consumer.shutdown();
    }

    /// Sawslin Phase 1 Lane 0 e2e — exercises the
    /// `AcceptPoseStream` dispatch arm + `PoseStreamFrameWire`
    /// (prost-encoded `PoseStreamFrame` envelope) round-trip on the
    /// wire. Producer yields a mix of `JointAngles` and
    /// `SpatialTransform` oneof arms; consumer decodes back and
    /// verifies each arm.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_pose_frames() {
        use auki_datatypes::pose_stream::pose_stream_frame::Payload;

        let id_p = PeerIdentity::from_seed(&[131u8; 32]);
        let id_c = PeerIdentity::from_seed(&[132u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-pose-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-pose-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-posestream-happy-path".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            pose_stream_provider_yielding_three_frames(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not see producer in cluster within 15s");

        let sub: StreamSubscription<PoseStreamFrameWire> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/pose".into(),
                },
            )
            .await
            .expect("open_stream<PoseStreamFrameWire> should succeed");

        assert_eq!(sub.info.sensor_hash, "pose-sensor-hash-1");
        assert_eq!(sub.info.clock_id, "pose/test/session-monotonic");
        assert_eq!(sub.info.clock_hash, "pose-clock-hash-1");

        let mut frames = sub.frames;

        // Frame 0 — JointAngles arm.
        let f0 = frames.next().await.unwrap().expect("frame 0 ok");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.timestamp_ns, 100);
        let decoded = f0.payload.decode().expect("frame 0 decodes");
        match decoded.payload {
            Some(Payload::JointAngles(j)) => assert_eq!(j.angles, vec![0.0, 0.5, -0.5]),
            other => panic!("expected JointAngles arm, got {other:?}"),
        }

        // Frame 1 — JointAngles arm again.
        let f1 = frames.next().await.unwrap().expect("frame 1 ok");
        assert_eq!(f1.seq, 1);
        assert_eq!(f1.timestamp_ns, 200);
        let decoded = f1.payload.decode().expect("frame 1 decodes");
        match decoded.payload {
            Some(Payload::JointAngles(j)) => assert_eq!(j.angles, vec![0.1, 0.6, -0.4]),
            other => panic!("expected JointAngles arm, got {other:?}"),
        }

        // Frame 2 — SpatialTransform arm. Same dispatch variant; oneof
        // arm flips inside the wire envelope.
        let f2 = frames.next().await.unwrap().expect("frame 2 ok");
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.timestamp_ns, 300);
        let decoded = f2.payload.decode().expect("frame 2 decodes");
        match decoded.payload {
            Some(Payload::SpatialTransform(t)) => {
                let translation = t.translation.expect("translation set");
                assert_eq!(translation.x, 1.0);
                assert_eq!(translation.y, 2.0);
                assert_eq!(translation.z, 3.0);
                let orientation = t.orientation.expect("orientation set");
                assert_eq!(orientation.w, 1.0);
            }
            other => panic!("expected SpatialTransform arm, got {other:?}"),
        }

        let end = frames.next().await.unwrap().expect_err("expected EndOfStream");
        match end {
            StreamError::EndOfStream { reason: EndReason::SourceEnded } => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }
        assert!(frames.next().await.is_none());

        producer.shutdown();
        consumer.shutdown();
    }

    /// One producer with a `sensor_id`-keyed multi-`T` provider serves
    /// **both** JPEG and pointcloud over the same `ClusterRuntime`.
    /// Confirms Dagaz D1's per-call `T` dispatch: a daemon doesn't
    /// have to choose one `T` at spawn time. This is the core shape
    /// Booster uses in Batch 3.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_producer_serves_jpeg_and_pointcloud_via_sensor_id_dispatch() {
        let id_p = PeerIdentity::from_seed(&[121u8; 32]);
        let id_c = PeerIdentity::from_seed(&[122u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-multi-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-multi-consumer/0").await;

        let doc = ClusterDoc {
            version: 1,
            cluster_name: "test-multi-t-dispatch".into(),
            peers: vec![
                cluster_peer(id_p.peer_id(), addr_p),
                cluster_peer(id_c.peer_id(), addr_c),
            ],
        };

        let producer = ClusterRuntime::from_swarm(
            swarm_p,
            doc.clone(),
            fixture_participant_provider(id_p.peer_id(), "producer"),
            multi_t_provider(),
        )
        .unwrap();
        let consumer = ClusterRuntime::from_swarm(
            swarm_c,
            doc,
            fixture_participant_provider(id_c.peer_id(), "consumer"),
            decline_all_streams(),
        )
        .unwrap();

        let connected = poll_until(
            || consumer.peers().iter().any(|p| p.peer_id == id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not see producer within 15s");

        // First substream: JPEG.
        let sub_jpeg: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "camera".into(),
                },
            )
            .await
            .expect("open_stream<JpegFrame> on sensor_id=camera");
        assert_eq!(sub_jpeg.info.sensor_hash, "cam-hash");
        let mut jpeg_frames = sub_jpeg.frames;
        let jf = jpeg_frames.next().await.unwrap().expect("jpeg frame");
        assert_eq!(jf.payload.bytes, vec![0xff, 0xd8, 0xab]);

        // Second substream: PointCloud — separate libp2p substream
        // multiplexed over the same yamux/QUIC connection. Mono-`T`
        // per substream (grimsby D1).
        let sub_pc: StreamSubscription<PointCloudFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "pointcloud".into(),
                },
            )
            .await
            .expect("open_stream<PointCloudFrame> on sensor_id=pointcloud");
        assert_eq!(sub_pc.info.sensor_hash, "pc-hash");
        let mut pc_frames = sub_pc.frames;
        let pcf = pc_frames.next().await.unwrap().expect("pc frame");
        assert_eq!(pcf.payload.bytes, vec![0xCD, 0xCD, 0xCD]);

        // Third substream: unknown sensor → producer's provider
        // returns Decline; consumer sees typed Declined error.
        let unknown_result: Result<StreamSubscription<JpegFrame>, OpenStreamError> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "no-such-sensor".into(),
                },
            )
            .await;
        match unknown_result {
            Err(OpenStreamError::Declined {
                reason: DeclineReason::SensorNotFound,
            }) => {}
            Err(other) => panic!("expected Declined(SensorNotFound), got {other:?}"),
            Ok(_) => panic!("expected Declined; producer should have rejected unknown sensor"),
        }

        producer.shutdown();
        consumer.shutdown();
    }
}
