//! Typed `Stream<T>` Rust API on top of [`crate::stream_protocol`]'s wire
//! primitives — [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! deliverables #2 + #3, lifted to multi-`T` dispatch by
//! [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) Batch 1.
//!
//! Producer side: [`StreamProvider`] — a callable [`NetworkRuntime`]
//! invokes per inbound substream. The callable returns a [`StreamDispatch`]
//! variant that pairs the typed source-stream with the [`AcceptInfo`]
//! metadata, or [`StreamDispatch::Decline`] with a typed reason. The
//! dispatch enum is *closed* over the SDK-supported `T`s (`JpegFrame`,
//! `PointCloudFrame` today; new variants added per coordinated
//! SDK + consumer release). Each substream is mono-`T`; the producer's
//! callback decides which `T` based on `request.sensor_id`.
//!
//! Consumer side: [`NetworkRuntime::open_stream`] returns a typed
//! [`StreamSubscription<T>`] containing the [`AcceptInfo`] from the
//! producer plus a [`futures::Stream`] of [`Result<ConsumerFrame<T>,
//! StreamError>`] items. The iterator yields one [`Err`] as a final
//! item describing how the stream ended (graceful end-of-stream,
//! connection lost, protocol error) and then returns `None`. The
//! consumer side stays generic over `T` — the consumer statically
//! knows which `T` it expects per call.
//!
//! ## Layered on top of `NetworkRuntime`
//!
//! Per grimsby's "share the swarm" decision — the same [`NetworkRuntime`]
//! that runs ansuz's cluster orchestration drives this protocol too. One
//! libp2p stack, one swarm, one driver task. `NetworkRuntime::spawn`
//! takes a `stream_provider` argument; `NetworkRuntime::open_stream`
//! opens outbound subscriptions through the same swarm. No second
//! Noise handshake, no second connection per peer pair.
//!
//! ## Per-call `T` on the producer side (Dagaz D1)
//!
//! grimsby v1 pinned `T = JpegFrame` at `NetworkRuntime::spawn` time —
//! the producer's callback was `StreamProvider<JpegFrame>`, returning
//! `StreamDecision<JpegFrame>`. Dagaz lifts that pinning so a single
//! daemon can serve multiple `T`s (camera + pointcloud, today). The
//! producer dispatches on `request.sensor_id` and returns a
//! [`StreamDispatch`] variant matching whichever `T` that sensor emits.
//! New `T` = new `StreamDispatch` variant + a coordinated SDK-consumer
//! release; on the wire each substream stays purely typed end-to-end.

use crate::network_runtime::NetworkRuntime;
use crate::stream_protocol::{
    AcceptInfo, DeclineReason, EndReason, Frame, JointEncodersFrame, JpegFrame, PointCloudFrame,
    STREAM_PROTOCOL,
    StreamMessage, StreamProtocolError, StreamRequest, read_message, stream_message,
    write_message,
};
use futures::{Stream, StreamExt, channel::mpsc};
use libp2p::{PeerId, StreamProtocol};
use libp2p_stream::OpenStreamError as Libp2pOpenStreamError;
use prost::Message;
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
/// Closed over the `T`s the SDK supports today: `JpegFrame` (grimsby v1),
/// `PointCloudFrame` (Dagaz Batch 1, raw CDR per D2), and
/// `JointEncodersFrame` (sawslin Phase B — `repeated float angles_rad`,
/// byte-identical to the on-disk `JointEncodersLogEntry`). Adding a new
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
    /// Accept the request with a JointEncoders source-Stream — sawslin
    /// Phase B. Each [`JointEncodersFrame`] carries one
    /// `repeated float angles_rad` sample (joint angles in radians,
    /// indexed in the producer's emit order, length pinned by the
    /// registry entry's `joint_count`). Wire bytes are identical to the
    /// on-disk `JointEncodersLogEntry` payload by design (locked in
    /// `auki-datatypes` by `joint_encoders_disk_wire_byte_identical`).
    AcceptJointEncoders {
        info: AcceptInfo,
        source: SourceStream<JointEncodersFrame>,
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
        reason: DeclineReason::sensor_not_found(),
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

/// Returned by [`NetworkRuntime::open_stream`] on success.
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

/// [`NetworkRuntime::open_stream`] failure modes — i.e., the request
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

/// How long [`NetworkRuntime::open_stream`] waits for the responder's
/// `Accept` / `Decline` reply before erroring out with
/// [`OpenStreamError::Timeout`]. A producer's `stream_provider`
/// callable is meant to be sync-fast (per grimsby D3 — async setup
/// lives inside the source-Stream); 30 s is a generous upper bound for
/// the libp2p substream open + wire-handshake round trip.
pub const OPEN_STREAM_TIMEOUT: Duration = Duration::from_secs(30);

// ─── Public methods on `NetworkRuntime` ──────────────────────────────────────

impl NetworkRuntime {
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
        T: Message + Default + Send + 'static,
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
        let req_msg = StreamMessage::request(request);
        write_message(&mut substream, &req_msg)
            .await
            .map_err(OpenStreamError::Protocol)?;

        let reply = match tokio::time::timeout(
            OPEN_STREAM_TIMEOUT,
            read_message(&mut substream),
        )
        .await
        {
            Err(_) => return Err(OpenStreamError::Timeout(OPEN_STREAM_TIMEOUT)),
            Ok(Err(e)) => return Err(OpenStreamError::Protocol(e)),
            Ok(Ok(m)) => m,
        };

        let info = match reply.variant {
            Some(stream_message::Variant::Accept(info)) => info,
            Some(stream_message::Variant::Decline(reason)) => {
                return Err(OpenStreamError::Declined { reason });
            }
            _ => {
                // Producer wrote something other than Accept / Decline as
                // its first reply, or the envelope was empty. Wire-
                // protocol violation; treat as a protocol error.
                return Err(OpenStreamError::Protocol(
                    StreamProtocolError::Decode(prost::DecodeError::new(
                        "expected Accept or Decline as first reply",
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

// ─── Internal helpers used by `network_runtime::run_task` ────────────────────

/// Producer-side per-substream task. Spawned by `network_runtime`'s
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
    // 1. Read the first envelope; expect Request.
    let request = match read_message(&mut substream).await {
        Ok(msg) => match msg.variant {
            Some(stream_message::Variant::Request(req)) => req,
            _ => {
                // Peer wrote something other than Request first, or the
                // envelope was empty. Drop; their open_stream surfaces
                // the protocol error on its end.
                return;
            }
        },
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
            let msg = StreamMessage::decline(reason);
            let _ = write_message(&mut substream, &msg).await;
        }
        StreamDispatch::AcceptJpeg { info, source } => {
            pump_typed::<JpegFrame>(substream, info, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptPointCloud { info, source } => {
            pump_typed::<PointCloudFrame>(substream, info, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptJointEncoders { info, source } => {
            pump_typed::<JointEncodersFrame>(substream, info, source, shutdown_rx).await;
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
/// (`JpegFrame`, `PointCloudFrame`, `JointEncodersFrame`). Adding a new
/// variant means adding a new monomorphization plus extending
/// [`StreamDispatch`].
async fn pump_typed<T>(
    mut substream: libp2p::Stream,
    info: AcceptInfo,
    mut source: SourceStream<T>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    T: Message + Default + Send + 'static,
{
    // Write accept.
    let accept_msg = StreamMessage::accept(info);
    if write_message(&mut substream, &accept_msg).await.is_err() {
        return;
    }
    // Drain source into Frame messages until end-of-source, shutdown
    // signal, or send error.
    let mut seq: u64 = 0;
    let end_reason = loop {
        tokio::select! {
            biased;

            res = shutdown_rx.changed() => {
                if res.is_ok() && *shutdown_rx.borrow() {
                    break EndReason::producer_shutting_down();
                }
                continue;
            }

            item = source.next() => match item {
                Some(Ok(frame)) => {
                    let payload = frame.payload.encode_to_vec();
                    let msg = StreamMessage::frame(Frame {
                        timestamp_ns: frame.timestamp_ns,
                        seq,
                        payload,
                    });
                    if write_message(&mut substream, &msg).await.is_err() {
                        return;
                    }
                    seq = seq.wrapping_add(1);
                }
                Some(Err(detail)) => break EndReason::producer_error(detail),
                None => break EndReason::source_ended(),
            },
        }
    };
    let end_msg = StreamMessage::end_of_stream(end_reason);
    let _ = write_message(&mut substream, &end_msg).await;
}

/// Consumer-side per-substream reader task. Spawned by
/// [`NetworkRuntime::open_stream`] after the producer accepts.
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
    T: Message + Default + Send + 'static,
{
    use futures::SinkExt;

    loop {
        let msg = match read_message(&mut substream).await {
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

        match msg.variant {
            Some(stream_message::Variant::Frame(f)) => {
                let payload = match T::decode(&*f.payload) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx
                            .send(Err(StreamError::Protocol(StreamProtocolError::Decode(e))))
                            .await;
                        return;
                    }
                };
                let frame = ConsumerFrame {
                    timestamp_ns: f.timestamp_ns,
                    seq: f.seq,
                    payload,
                };
                if tx.send(Ok(frame)).await.is_err() {
                    return;
                }
            }
            Some(stream_message::Variant::EndOfStream(reason)) => {
                let _ = tx.send(Err(StreamError::EndOfStream { reason })).await;
                return;
            }
            // Producer wrote something out-of-order, or empty envelope.
            _ => {
                let err = StreamProtocolError::Decode(prost::DecodeError::new(
                    "unexpected message after Accept",
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

// Static check that `JpegFrame` and `PointCloudFrame` satisfy the
// bounds the runtime expects for `T`.
const _: fn() = || {
    fn assert_message_send_static<T: Message + Default + Send + 'static>() {}
    assert_message_send_static::<JpegFrame>();
    assert_message_send_static::<PointCloudFrame>();
};

// ─── Tests ───────────────────────────────────────────────────────────────────

