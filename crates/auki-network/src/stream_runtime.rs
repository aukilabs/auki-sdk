//! Typed `Stream<T>` Rust API on top of [`crate::stream_protocol`]'s wire
//! primitives — [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! deliverables #2 + #3, lifted to multi-`T` dispatch by
//! [Dagaz](https://www.notion.so/3585c8e96592805b8d83c89f849d3577) Batch 1.
//!
//! Producer side: [`StreamProvider`] — a callable [`NetworkRuntime`]
//! invokes per inbound substream. The callable returns a [`StreamDispatch`]
//! variant that pairs the typed source-stream with the [`StreamManifest`],
//! or [`StreamDispatch::Decline`] with a typed reason. The
//! dispatch enum is *closed* over the SDK-supported `T`s (`JpegFrame`,
//! `PointCloudFrame` today; new variants added per coordinated
//! SDK + consumer release). Each substream is mono-`T`; the producer's
//! callback decides which `T` based on `request.sensor_id`.
//!
//! Consumer side: [`NetworkRuntime::open_stream`] returns a typed
//! [`StreamSubscription<T>`] containing the [`StreamManifest`] from the
//! producer plus a [`futures::Stream`] of [`Result<StreamEntry<T>,
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
    AudioFrame, DeclineReason, EndReason, JointEncodersFrame, JpegFrame, PointCloudFrame,
    STREAM_PROTOCOL, StreamEntry as WireStreamEntry, StreamManifest, StreamMessage,
    StreamProtocolError, StreamRequest, read_message, stream_message, write_message,
};
use auki_datatypes::detection::DetectionLogEntry;
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
/// [`StreamMessage::Entry`] on the wire, stamping `seq` automatically.
///
/// Producer-side timestamping: `timestamp_ns` lives on the producer's
/// session clock — the same clock identified in the
/// [`StreamManifest::clock_id`] the producer wrote at accept time.
/// Monotonically nondecreasing on a healthy producer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamItem<T> {
    pub timestamp_ns: i64,
    pub payload: T,
}

/// Source-stream type the app returns inside an `Accept*` variant of
/// [`StreamDispatch`].
///
/// Item is `Result<StreamItem<T>, String>` so the producer can end
/// with an error reason — `Some(Err(detail))` is mapped to
/// [`EndReason::ProducerError { detail }`][EndReason::ProducerError]
/// on the wire. `None` (stream returns) is mapped to
/// [`EndReason::SourceEnded`]. The SDK-level shutdown / session-change
/// paths can override either with their own [`EndReason`].
pub type SourceStream<T> = Pin<Box<dyn Stream<Item = Result<StreamItem<T>, String>> + Send>>;

/// Producer's accept/decline decision for a single inbound request.
/// Closed over the `T`s the SDK supports today: `JpegFrame` (grimsby v1),
/// `PointCloudFrame` (native Auki pointcloud samples),
/// `JointEncodersFrame` (sawslin Phase B — `repeated float angles_rad`,
/// byte-identical to the on-disk `JointEncodersLogEntry`), and
/// `AudioFrame` (Dialogue Batch 1 — opaque interleaved PCM bytes,
/// byte-identical to the on-disk `AudioLogEntry`). Adding a new `T`
/// is a coordinated SDK + consumer release — bump the runtime, add
/// the variant, every consumer that wants the new sensor type opts in.
///
/// On `Accept*`, the SDK writes [`StreamMessage::Accept(manifest)`]
/// for the matching `T` and drains the source-Stream onto the substream as
/// [`StreamMessage::Entry`] values until the source ends or the
/// substream closes. On [`StreamDispatch::Decline`], the SDK writes
/// `Decline { reason }` and closes the substream.
///
/// SDK-supported `T`s: `JpegFrame`, `PointCloudFrame`,
/// `JointEncodersFrame`, `AudioFrame`, and (Cuba T8) `DetectionLogEntry`.
pub enum StreamDispatch {
    /// Accept the request with a JPEG source-Stream — grimsby v1's
    /// original stream path, now carrying the same manifest metadata
    /// as every other `T`.
    AcceptJpeg {
        manifest: StreamManifest,
        source: SourceStream<JpegFrame>,
    },
    /// Accept the request with a PointCloud source-Stream. Each
    /// [`PointCloudFrame`] carries a `point_count` plus packed point
    /// records whose fixed field layout is declared by the Sensor Registry.
    AcceptPointCloud {
        manifest: StreamManifest,
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
        manifest: StreamManifest,
        source: SourceStream<JointEncodersFrame>,
    },
    /// Accept the request with an Audio source-Stream — Dialogue Batch
    /// 1. Each [`AudioFrame`] carries interleaved PCM bytes; the
    /// consumer pushes them into a player session (e.g. the K1's
    /// `AudioPlayer` in `PCM_STREAM` mode). Sample format, channels,
    /// sample rate, and channel layout are resolved out-of-band via
    /// `(sensor_id, sensor_hash) → SensorBody::Audio` — the wire
    /// payload is opaque-bytes-only. Wire bytes are byte-identical to
    /// the on-disk `AudioLogEntry` payload by design (locked in
    /// `auki-datatypes` by `audio_disk_wire_byte_identical`).
    AcceptAudio {
        manifest: StreamManifest,
        source: SourceStream<AudioFrame>,
    },
    /// Accept the request with a Detection source-Stream — Cuba T8.
    /// Each [`DetectionLogEntry`] is the same on-disk Detection Log
    /// payload reused on the wire — `bytes data` (opaque per-detector
    /// schema, decoded via the `type=<vocab>` discriminator), plus
    /// `sensor_hash` (the bound input frame's sensor, Cuba T5) and
    /// `type` (open-string discriminator, Cuba T12). The wire bytes
    /// match the on-disk payload by construction, since both sides
    /// `prost::Message::encode_to_vec` the same struct.
    ///
    /// On `StreamManifest.sensor_hash`: this is the *bound input
    /// sensor* the detector was bound to — the frame the detection
    /// was computed against. The detector's own identity
    /// (`detector_id` / `detector_hash`) is resolved out-of-band via
    /// the daemon's `/auki/registries/0.0.1` Detector Registry
    /// exchange (Cuba T4).
    AcceptDetection {
        manifest: StreamManifest,
        source: SourceStream<DetectionLogEntry>,
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
/// The first argument is the libp2p [`PeerId`] of the requester. The
/// SDK has known the requester since the inbound substream landed on
/// the swarm; passing it through lets the producer enforce
/// per-requester policy (Park's Dialogue audio is the load-bearing
/// example: with N robots in one cluster, the operator's mic must
/// only stream to the one robot they are currently inspecting, so
/// the provider declines opens from any other peer).
///
/// `Send + Sync` because the runtime task holds the callable in an
/// `Arc` shared across spawned per-substream tasks.
pub type StreamProvider = Arc<dyn Fn(PeerId, StreamRequest) -> StreamDispatch + Send + Sync>;

/// Convenience constructor for consumer-only nodes (Park, analytics
/// tools, future Sentinel-as-consumer) that don't expose any sensors.
/// Declines every inbound request with [`DeclineReason::SensorNotFound`].
pub fn decline_all_streams() -> StreamProvider {
    Arc::new(|_peer, _req| StreamDispatch::Decline {
        reason: DeclineReason::sensor_not_found(),
    })
}

// ─── Consumer-side types ─────────────────────────────────────────────────────

/// What the consumer reads off the typed iterator. Same shape as
/// [`StreamItem<T>`] but with the SDK-stamped `seq` exposed so the
/// consumer can detect drops via gaps in the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEntry<T> {
    pub timestamp_ns: i64,
    pub seq: u64,
    pub payload: T,
}

/// Returned by [`NetworkRuntime::open_stream`] on success.
///
/// Iterator semantics: yields `Ok(entry)` for each entry, then a
/// **single final** `Err(StreamError)` describing how the stream ended,
/// then `None`. After the `Err` is yielded the iterator is exhausted.
pub struct StreamSubscription<T> {
    /// Accept-time stream manifest. Stable for the lifetime of the
    /// subscription — the producer commits to this at accept time and
    /// any change requires opening a new substream.
    pub manifest: StreamManifest,
    /// Typed entry iterator. See struct-level docs for the
    /// terminator-then-`None` pattern.
    pub entries: Pin<Box<dyn Stream<Item = Result<StreamEntry<T>, StreamError>> + Send>>,
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
/// never even got to "yielding entries."
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
    /// entries flowed).
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
    /// request. On accept, the returned [`StreamSubscription<T>::entries`]
    /// is a typed async iterator the caller drives; on decline, returns
    /// [`OpenStreamError::Declined { reason }`].
    ///
    /// Stream lifetime: the substream stays open as long as the consumer
    /// keeps polling `entries` AND the producer keeps yielding. Either
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

        let reply =
            match tokio::time::timeout(OPEN_STREAM_TIMEOUT, read_message(&mut substream)).await {
                Err(_) => return Err(OpenStreamError::Timeout(OPEN_STREAM_TIMEOUT)),
                Ok(Err(e)) => return Err(OpenStreamError::Protocol(e)),
                Ok(Ok(m)) => m,
            };

        let manifest = match reply.variant {
            Some(stream_message::Variant::Accept(manifest)) => manifest,
            Some(stream_message::Variant::Decline(reason)) => {
                return Err(OpenStreamError::Declined { reason });
            }
            _ => {
                // Producer wrote something other than Accept / Decline as
                // its first reply, or the envelope was empty. Wire-
                // protocol violation; treat as a protocol error.
                return Err(OpenStreamError::Protocol(StreamProtocolError::Decode(
                    prost::DecodeError::new("expected Accept or Decline as first reply"),
                )));
            }
        };

        // Spawn the consumer-side reader task. Entries flow through an
        // mpsc channel; the returned `entries` iterator is the receiver
        // side. When the consumer drops the StreamSubscription, the
        // receiver drops, the channel closes, the reader task's `send`
        // fails on the next entry, and the task exits — substream drops,
        // libp2p closes it cleanly on the wire, the producer's source
        // gets dropped on the producer side via the same chain.
        let (tx, rx) = mpsc::channel::<Result<StreamEntry<T>, StreamError>>(8);
        tokio::spawn(consumer_reader_task::<T>(substream, tx));

        Ok(StreamSubscription {
            manifest,
            entries: Box::pin(rx),
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
    peer: PeerId,
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
    //    `T` to pump. Pass the requester's PeerId through so producers
    //    can apply per-requester policy.
    let dispatch = (provider)(peer, request);

    match dispatch {
        StreamDispatch::Decline { reason } => {
            let msg = StreamMessage::decline(reason);
            let _ = write_message(&mut substream, &msg).await;
        }
        StreamDispatch::AcceptJpeg { manifest, source } => {
            pump_typed::<JpegFrame>(substream, manifest, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptPointCloud { manifest, source } => {
            pump_typed::<PointCloudFrame>(substream, manifest, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptJointEncoders { manifest, source } => {
            pump_typed::<JointEncodersFrame>(substream, manifest, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptAudio { manifest, source } => {
            pump_typed::<AudioFrame>(substream, manifest, source, shutdown_rx).await;
        }
        StreamDispatch::AcceptDetection { manifest, source } => {
            pump_typed::<DetectionLogEntry>(substream, manifest, source, shutdown_rx).await;
        }
    }
}

/// Per-`T` source-Stream pump. Writes
/// [`StreamMessage::Accept(manifest)`]
/// then drains `source` onto the substream as `StreamMessage::Entry`
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
    manifest: StreamManifest,
    mut source: SourceStream<T>,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    T: Message + Default + Send + 'static,
{
    // Write accept.
    let accept_msg = StreamMessage::accept(manifest);
    if write_message(&mut substream, &accept_msg).await.is_err() {
        return;
    }
    // Drain source into StreamEntry messages until end-of-source, shutdown
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
                    let msg = StreamMessage::entry(WireStreamEntry {
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
/// Reads [`StreamMessage::Entry`] values into an mpsc channel until the
/// substream yields a terminal message (EndOfStream / unexpected variant)
/// or an I/O error. Sends a single terminal `Err(StreamError)` to the
/// channel before exiting; consumer's iterator surfaces it as the final
/// `Err` item.
async fn consumer_reader_task<T>(
    mut substream: libp2p::Stream,
    mut tx: mpsc::Sender<Result<StreamEntry<T>, StreamError>>,
) where
    T: Message + Default + Send + 'static,
{
    use futures::SinkExt;

    loop {
        let msg = match read_message(&mut substream).await {
            Ok(m) => m,
            Err(StreamProtocolError::Io(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
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
            Some(stream_message::Variant::Entry(f)) => {
                let payload = match T::decode(&*f.payload) {
                    Ok(p) => p,
                    Err(e) => {
                        let _ = tx
                            .send(Err(StreamError::Protocol(StreamProtocolError::Decode(e))))
                            .await;
                        return;
                    }
                };
                let frame = StreamEntry {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PeerIdentity;
    use crate::network_runtime::AllowedPeer;
    use crate::stream_protocol::{decline_reason, end_reason};
    use crate::swarm::{Behaviour, SwarmConfig, build_swarm};
    use futures::stream;
    use libp2p::Swarm;
    use libp2p::swarm::SwarmEvent;
    use std::time::Instant;

    // ─── Common test helpers ─────────────────────────────────────────────

    fn test_swarm_config(agent_version: &str) -> SwarmConfig {
        SwarmConfig {
            listen_addresses: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
            agent_version: agent_version.into(),
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

    async fn build_listening_swarm(
        identity: &PeerIdentity,
        agent_version: &str,
    ) -> (Swarm<Behaviour>, libp2p::Multiaddr) {
        let mut swarm = build_swarm(identity, test_swarm_config(agent_version)).unwrap();
        let addr = wait_for_listen_addr(&mut swarm).await;
        (swarm, addr)
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

    fn manifest(
        sensor_id: &str,
        sensor_hash: &str,
        clock_id: &str,
        clock_hash: &str,
        frame_id: &str,
        frame_hash: &str,
    ) -> StreamManifest {
        StreamManifest {
            sensor_id: sensor_id.into(),
            sensor_hash: sensor_hash.into(),
            clock_id: clock_id.into(),
            clock_hash: clock_hash.into(),
            frame_id: frame_id.into(),
            frame_hash: frame_hash.into(),
        }
    }

    fn native_pointcloud(points: &[[f32; 3]]) -> PointCloudFrame {
        let mut data = Vec::with_capacity(points.len() * 12);
        for point in points {
            for value in point {
                data.extend_from_slice(&value.to_le_bytes());
            }
        }
        PointCloudFrame {
            point_count: points.len() as u32,
            data,
        }
    }

    // ─── Provider fixtures ───────────────────────────────────────────────

    fn jpeg_provider_yielding_three_frames() -> StreamProvider {
        Arc::new(|_peer, _req| {
            let frames = vec![
                Ok(StreamItem {
                    timestamp_ns: 1_000,
                    payload: JpegFrame {
                        bytes: vec![0xff, 0xd8, 0x01],
                    },
                }),
                Ok(StreamItem {
                    timestamp_ns: 2_000,
                    payload: JpegFrame {
                        bytes: vec![0xff, 0xd8, 0x02],
                    },
                }),
                Ok(StreamItem {
                    timestamp_ns: 3_000,
                    payload: JpegFrame {
                        bytes: vec![0xff, 0xd8, 0x03],
                    },
                }),
            ];
            StreamDispatch::AcceptJpeg {
                manifest: manifest(
                    "test/cam",
                    "sensor-hash-3",
                    "test/session-monotonic",
                    "clock-hash-3",
                    "test/cam/frame",
                    "frame-hash-3",
                ),
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    fn jpeg_provider_declines_unknown() -> StreamProvider {
        Arc::new(|_peer, req| {
            if req.sensor_id == "exists" {
                StreamDispatch::AcceptJpeg {
                    manifest: manifest("exists", "h", "c", "ch", "exists/frame", "fh"),
                    source: Box::pin(stream::iter(vec![Ok(StreamItem {
                        timestamp_ns: 1,
                        payload: JpegFrame { bytes: vec![0xff] },
                    })])),
                }
            } else {
                StreamDispatch::Decline {
                    reason: DeclineReason::sensor_not_found(),
                }
            }
        })
    }

    fn jpeg_provider_yields_then_errors() -> StreamProvider {
        Arc::new(|_peer, _req| {
            let items = vec![
                Ok(StreamItem {
                    timestamp_ns: 1,
                    payload: JpegFrame { bytes: vec![0xaa] },
                }),
                Err("encoder died".to_string()),
            ];
            StreamDispatch::AcceptJpeg {
                manifest: manifest("test/cam", "h", "c", "ch", "test/cam/frame", "fh"),
                source: Box::pin(stream::iter(items)),
            }
        })
    }

    fn pointcloud_provider_yielding_three_frames() -> StreamProvider {
        Arc::new(|_peer, _req| {
            let frames = vec![
                Ok(StreamItem {
                    timestamp_ns: 10_000,
                    payload: native_pointcloud(&[[1.0, 2.0, 3.0]]),
                }),
                Ok(StreamItem {
                    timestamp_ns: 20_000,
                    payload: native_pointcloud(&[[4.0, 5.0, 6.0]]),
                }),
                Ok(StreamItem {
                    timestamp_ns: 30_000,
                    payload: native_pointcloud(&[[7.0, 8.0, 9.0]]),
                }),
            ];
            StreamDispatch::AcceptPointCloud {
                manifest: manifest(
                    "test/pc",
                    "pc-sensor-hash-3",
                    "pc/test/session-monotonic",
                    "pc-clock-hash-3",
                    "test/pc/frame",
                    "pc-frame-hash-3",
                ),
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    fn audio_provider_yielding_three_frames() -> StreamProvider {
        Arc::new(|_peer, _req| {
            // 20 ms × 3 of 48 kHz mono int16-LE — the canonical Dialogue
            // fixture shape. Contents are arbitrary deterministic bytes;
            // the SDK treats them as opaque per the `bytes data` proto.
            let frames = vec![
                Ok(StreamItem {
                    timestamp_ns: 10_000,
                    payload: AudioFrame {
                        data: vec![0x00, 0x10, 0x20, 0x30, 0x40, 0x50],
                    },
                }),
                Ok(StreamItem {
                    timestamp_ns: 30_000,
                    payload: AudioFrame {
                        data: vec![0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0],
                    },
                }),
                Ok(StreamItem {
                    timestamp_ns: 50_000,
                    payload: AudioFrame {
                        data: vec![0xc0, 0xd0, 0xe0, 0xf0, 0x01, 0x02],
                    },
                }),
            ];
            StreamDispatch::AcceptAudio {
                manifest: manifest(
                    "test/audio",
                    "audio-sensor-hash-3",
                    "audio/test/session-monotonic",
                    "audio-clock-hash-3",
                    "",
                    "",
                ),
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    fn multi_t_provider() -> StreamProvider {
        Arc::new(|_peer, req| match req.sensor_id.as_str() {
            "camera" => StreamDispatch::AcceptJpeg {
                manifest: manifest(
                    "camera",
                    "cam-hash",
                    "shared/clock",
                    "shared-clock-hash",
                    "camera/frame",
                    "cam-frame-hash",
                ),
                source: Box::pin(stream::iter(vec![Ok(StreamItem {
                    timestamp_ns: 1,
                    payload: JpegFrame {
                        bytes: vec![0xff, 0xd8, 0xab],
                    },
                })])),
            },
            "pointcloud" => StreamDispatch::AcceptPointCloud {
                manifest: manifest(
                    "pointcloud",
                    "pc-hash",
                    "shared/clock",
                    "shared-clock-hash",
                    "pointcloud/frame",
                    "pc-frame-hash",
                ),
                source: Box::pin(stream::iter(vec![Ok(StreamItem {
                    timestamp_ns: 1,
                    payload: native_pointcloud(&[[1.0, 1.0, 1.0]]),
                })])),
            },
            _ => StreamDispatch::Decline {
                reason: DeclineReason::sensor_not_found(),
            },
        })
    }

    // ─── Cluster-trust-boundary gate (resolved 2026-05-13) ───────────────

    /// Non-cluster peer's `/auki/stream/0.1.0` substream is silently
    /// dropped. Producer's `StreamProvider` is **never invoked**.
    /// Pins the option-A trust-boundary resolution (server-side gate,
    /// silent-drop, no typed `Decline { NotInCluster }` variant).
    ///
    /// Setup: P has an empty allow-list (C is unknown). C has P in its
    /// allow-list (so C auto-dials P; libp2p connection-layer is open
    /// by default per PR #106, so the handshake completes regardless).
    /// C opens a `/auki/stream/0.1.0` substream; P's runtime sees the
    /// inbound substream from a non-allow-listed peer at
    /// `network_runtime.rs:604` and drops it before
    /// `handle_inbound_substream` (and thus the provider) ever runs.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn non_cluster_peer_stream_substream_is_silently_dropped() {
        let id_p = PeerIdentity::from_seed(&[131u8; 32]);
        let id_c = PeerIdentity::from_seed(&[132u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-gate-p/0").await;
        let (swarm_c, _addr_c) = build_listening_swarm(&id_c, "test-gate-c/0").await;

        // Provider that PANICS if invoked — proves the substream never
        // reached the per-substream handler.
        let panicking_provider: StreamProvider = Arc::new(|_peer, _req| {
            panic!("StreamProvider must not be invoked for non-cluster peer — gate failed")
        });

        // Producer has empty allow-list — C is unknown to P.
        let (producer, ..) =
            crate::network_runtime::NetworkRuntime::spawn(swarm_p, vec![], panicking_provider)
                .expect("producer spawn");

        // Consumer has P in its allow-list, so it auto-dials.
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        // Wait for libp2p connection to complete (connection-layer is
        // open by default; this is just confirming the auto-dial worked).
        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(10),
        )
        .await;
        assert!(
            connected,
            "consumer's libp2p connection to producer did not establish within 10s"
        );

        // Attempt to open a stream. The substream open at the libp2p
        // layer succeeds (handshake completed), but P drops the
        // substream silently when it surfaces on `incoming_streams`
        // because C isn't in P's allow-list. C's reply read on the
        // closed substream surfaces as a protocol error (Io
        // UnexpectedEof) or, if libp2p detects the close first, as
        // LibP2p; we accept either.
        let result = tokio::time::timeout(
            Duration::from_secs(35),
            consumer.open_stream::<JpegFrame>(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "anything".into(),
                },
            ),
        )
        .await
        .expect("open_stream did not return within its own OPEN_STREAM_TIMEOUT bound");

        match result {
            Err(OpenStreamError::Protocol(_)) => {}
            Err(OpenStreamError::LibP2p(_)) => {}
            Err(OpenStreamError::Timeout(_)) => {}
            Err(OpenStreamError::Declined { reason }) => panic!(
                "expected silent drop, not a typed Decline — the gate must NOT \
                 advertise a `NotInCluster` reason to outsiders (probe signal). \
                 Got Declined(reason={reason:?})"
            ),
            Ok(_) => {
                panic!("expected silent drop, but open_stream returned Ok — the gate is broken")
            }
        }

        producer.shutdown();
        consumer.shutdown();
    }

    // ─── Happy-path tests (ported from pre-NetworkRuntime fixture) ───────

    /// Two runtimes; producer accepts and yields 3 frames; consumer
    /// reads 3 frames + a final `Err(EndOfStream { reason: SourceEnded })`,
    /// then None.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_jpeg_frames() {
        let id_p = PeerIdentity::from_seed(&[101u8; 32]);
        let id_c = PeerIdentity::from_seed(&[102u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            jpeg_provider_yielding_three_frames(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected, "consumer did not connect to producer within 15s");

        let sub: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/cam".into(),
                },
            )
            .await
            .expect("open_stream should succeed");

        assert_eq!(sub.manifest.sensor_id, "test/cam");
        assert_eq!(sub.manifest.sensor_hash, "sensor-hash-3");
        assert_eq!(sub.manifest.clock_id, "test/session-monotonic");
        assert_eq!(sub.manifest.clock_hash, "clock-hash-3");
        assert_eq!(sub.manifest.frame_id, "test/cam/frame");
        assert_eq!(sub.manifest.frame_hash, "frame-hash-3");

        let mut entries = sub.entries;
        let f0 = entries.next().await.unwrap().expect("frame 0 ok");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.timestamp_ns, 1_000);
        assert_eq!(f0.payload.bytes, vec![0xff, 0xd8, 0x01]);

        let f1 = entries.next().await.unwrap().expect("frame 1 ok");
        assert_eq!(f1.seq, 1);
        assert_eq!(f1.timestamp_ns, 2_000);

        let f2 = entries.next().await.unwrap().expect("frame 2 ok");
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.timestamp_ns, 3_000);
        assert_eq!(f2.payload.bytes, vec![0xff, 0xd8, 0x03]);

        let end = entries
            .next()
            .await
            .unwrap()
            .expect_err("expected terminator");
        match end {
            StreamError::EndOfStream { reason }
                if matches!(reason.kind, Some(end_reason::Kind::SourceEnded(_))) => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }
        assert!(entries.next().await.is_none());

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

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            jpeg_provider_declines_unknown(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        let result: Result<StreamSubscription<JpegFrame>, OpenStreamError> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "does-not-exist".into(),
                },
            )
            .await;

        match result {
            Err(OpenStreamError::Declined { reason })
                if matches!(reason.kind, Some(decline_reason::Kind::SensorNotFound(_))) => {}
            Err(other) => panic!("expected Declined(SensorNotFound), got {other:?}"),
            Ok(_) => panic!("expected Declined, got Ok subscription"),
        }

        producer.shutdown();
        consumer.shutdown();
    }

    /// Producer's source-Stream yields Ok(frame) then Err("detail");
    /// consumer reads the frame then sees `EndOfStream { ProducerError }`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_error_signals_consumer_with_detail() {
        let id_p = PeerIdentity::from_seed(&[105u8; 32]);
        let id_c = PeerIdentity::from_seed(&[106u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            jpeg_provider_yields_then_errors(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
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
        let mut entries = sub.entries;

        let f0 = entries.next().await.unwrap().expect("first frame ok");
        assert_eq!(f0.seq, 0);

        let end = entries
            .next()
            .await
            .unwrap()
            .expect_err("expected error end");
        match end {
            StreamError::EndOfStream { reason } => match reason.kind {
                Some(end_reason::Kind::ProducerError(end_reason::ProducerError { detail })) => {
                    assert_eq!(detail, "encoder died");
                }
                other => panic!("expected ProducerError, got {other:?}"),
            },
            other => panic!("expected EndOfStream, got {other:?}"),
        }
        assert!(entries.next().await.is_none());

        producer.shutdown();
        consumer.shutdown();
    }

    /// Source-Stream that yields one frame then `pending` (never yields).
    /// Producer is then `shutdown(self)` — consumer should see
    /// `EndOfStream { ProducerShuttingDown }`, not `ConnectionLost`
    /// (per grimsby D5b — best-effort explicit EndOfStream from the
    /// shutdown grace window).
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_shutdown_signals_consumer_with_typed_end_of_stream() {
        let id_p = PeerIdentity::from_seed(&[107u8; 32]);
        let id_c = PeerIdentity::from_seed(&[108u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let provider: StreamProvider = Arc::new(|_peer, _req| {
            let first = stream::iter(vec![Ok(StreamItem {
                timestamp_ns: 1_000,
                payload: JpegFrame {
                    bytes: vec![0xff, 0xd8, 0x99],
                },
            })]);
            let then_pending = stream::pending::<Result<StreamItem<JpegFrame>, String>>();
            StreamDispatch::AcceptJpeg {
                manifest: manifest(
                    "shutdown/cam",
                    "shutdown-test",
                    "test/clock",
                    "h",
                    "shutdown/frame",
                    "fh",
                ),
                source: Box::pin(first.chain(then_pending)),
            }
        });

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            provider,
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
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
        let mut entries = sub.entries;

        let f0 = entries.next().await.unwrap().expect("first frame ok");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.payload.bytes, vec![0xff, 0xd8, 0x99]);

        producer.shutdown();

        let next = tokio::time::timeout(Duration::from_secs(5), entries.next())
            .await
            .expect("entry iterator hung after producer shutdown")
            .expect("iterator ended without terminator")
            .expect_err("expected typed end-of-stream");

        match next {
            StreamError::EndOfStream { reason }
                if matches!(reason.kind, Some(end_reason::Kind::ProducerShuttingDown(_))) => {}
            other => panic!("expected EndOfStream(ProducerShuttingDown), got {other:?}"),
        }
        assert!(entries.next().await.is_none());

        consumer.shutdown();
    }

    /// Allow-list lists a peer-id whose address points at nothing
    /// listening. `open_stream` surfaces a typed `OpenStreamError`
    /// (LibP2p or Timeout) rather than hanging.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn open_stream_against_unreachable_peer_surfaces_typed_error() {
        let id_c = PeerIdentity::from_seed(&[201u8; 32]);
        let id_unreachable = PeerIdentity::from_seed(&[202u8; 32]);

        let (swarm_c, _addr_c) = build_listening_swarm(&id_c, "test-consumer/0").await;

        let unreachable_addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/1".parse().unwrap();

        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_unreachable.peer_id(),
                multiaddrs: vec![unreachable_addr],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

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
            Err(other) => panic!("expected LibP2p or Timeout, got {other:?}"),
            Ok(_) => panic!("expected an error, got Ok subscription"),
        }

        consumer.shutdown();
    }

    /// Same happy-path as `producer_accepts_and_streams_jpeg_frames`
    /// but with `T = PointCloudFrame`. Exercises the multi-`T`
    /// dispatch chain (Dagaz D1) end-to-end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_pointcloud_frames() {
        let id_p = PeerIdentity::from_seed(&[111u8; 32]);
        let id_c = PeerIdentity::from_seed(&[112u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-pc-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-pc-consumer/0").await;

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            pointcloud_provider_yielding_three_frames(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        let sub: StreamSubscription<PointCloudFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/pc".into(),
                },
            )
            .await
            .expect("open_stream<PointCloudFrame>");

        assert_eq!(sub.manifest.sensor_id, "test/pc");
        assert_eq!(sub.manifest.sensor_hash, "pc-sensor-hash-3");
        assert_eq!(sub.manifest.clock_id, "pc/test/session-monotonic");
        assert_eq!(sub.manifest.frame_id, "test/pc/frame");
        assert_eq!(sub.manifest.frame_hash, "pc-frame-hash-3");

        let mut entries = sub.entries;
        let f0 = entries.next().await.unwrap().expect("frame 0");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.payload, native_pointcloud(&[[1.0, 2.0, 3.0]]));
        let f1 = entries.next().await.unwrap().expect("frame 1");
        assert_eq!(f1.seq, 1);
        assert_eq!(f1.payload.point_count, 1);
        let f2 = entries.next().await.unwrap().expect("frame 2");
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.payload, native_pointcloud(&[[7.0, 8.0, 9.0]]));

        let end = entries
            .next()
            .await
            .unwrap()
            .expect_err("expected EndOfStream");
        match end {
            StreamError::EndOfStream { reason }
                if matches!(reason.kind, Some(end_reason::Kind::SourceEnded(_))) => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }

        producer.shutdown();
        consumer.shutdown();
    }

    /// Same happy-path as `producer_accepts_and_streams_pointcloud_frames`
    /// but with `T = AudioFrame` — Dialogue Batch 1 end-to-end.
    /// Exercises the `AcceptAudio` arm + `pump_typed::<AudioFrame>`
    /// dispatch through a real libp2p substream.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn producer_accepts_and_streams_audio_frames() {
        let id_p = PeerIdentity::from_seed(&[141u8; 32]);
        let id_c = PeerIdentity::from_seed(&[142u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-audio-producer/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-audio-consumer/0").await;

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            audio_provider_yielding_three_frames(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        let sub: StreamSubscription<AudioFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "test/audio".into(),
                },
            )
            .await
            .expect("open_stream<AudioFrame>");

        assert_eq!(sub.manifest.sensor_id, "test/audio");
        assert_eq!(sub.manifest.sensor_hash, "audio-sensor-hash-3");
        assert_eq!(sub.manifest.clock_id, "audio/test/session-monotonic");
        assert_eq!(sub.manifest.frame_id, "");
        assert_eq!(sub.manifest.frame_hash, "");

        let mut entries = sub.entries;
        let f0 = entries.next().await.unwrap().expect("frame 0");
        assert_eq!(f0.seq, 0);
        assert_eq!(f0.payload.data, vec![0x00, 0x10, 0x20, 0x30, 0x40, 0x50]);
        let f1 = entries.next().await.unwrap().expect("frame 1");
        assert_eq!(f1.seq, 1);
        let f2 = entries.next().await.unwrap().expect("frame 2");
        assert_eq!(f2.seq, 2);
        assert_eq!(f2.payload.data, vec![0xc0, 0xd0, 0xe0, 0xf0, 0x01, 0x02]);

        let end = entries
            .next()
            .await
            .unwrap()
            .expect_err("expected EndOfStream");
        match end {
            StreamError::EndOfStream { reason }
                if matches!(reason.kind, Some(end_reason::Kind::SourceEnded(_))) => {}
            other => panic!("expected SourceEnded, got {other:?}"),
        }

        producer.shutdown();
        consumer.shutdown();
    }

    /// One producer serves both JPEG and PointCloud over the same
    /// runtime via `sensor_id` dispatch (Dagaz D1). Three substreams
    /// over one libp2p connection: camera → AcceptJpeg, pointcloud →
    /// AcceptPointCloud, unknown sensor → Decline.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn one_producer_serves_jpeg_and_pointcloud_via_sensor_id_dispatch() {
        let id_p = PeerIdentity::from_seed(&[121u8; 32]);
        let id_c = PeerIdentity::from_seed(&[122u8; 32]);

        let (swarm_p, addr_p) = build_listening_swarm(&id_p, "test-multi-p/0").await;
        let (swarm_c, addr_c) = build_listening_swarm(&id_c, "test-multi-c/0").await;

        let (producer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_p,
            vec![AllowedPeer {
                peer_id: id_c.peer_id(),
                multiaddrs: vec![addr_c],
            }],
            multi_t_provider(),
        )
        .expect("producer spawn");
        let (consumer, ..) = crate::network_runtime::NetworkRuntime::spawn(
            swarm_c,
            vec![AllowedPeer {
                peer_id: id_p.peer_id(),
                multiaddrs: vec![addr_p],
            }],
            decline_all_streams(),
        )
        .expect("consumer spawn");

        let connected = poll_until(
            || consumer.connected_peers().contains(&id_p.peer_id()),
            Duration::from_secs(15),
        )
        .await;
        assert!(connected);

        // Substream 1: JPEG.
        let sub_jpeg: StreamSubscription<JpegFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "camera".into(),
                },
            )
            .await
            .expect("open_stream<JpegFrame> camera");
        assert_eq!(sub_jpeg.manifest.sensor_id, "camera");
        assert_eq!(sub_jpeg.manifest.sensor_hash, "cam-hash");
        let mut jpeg_entries = sub_jpeg.entries;
        let jf = jpeg_entries.next().await.unwrap().expect("jpeg frame");
        assert_eq!(jf.payload.bytes, vec![0xff, 0xd8, 0xab]);

        // Substream 2: PointCloud.
        let sub_pc: StreamSubscription<PointCloudFrame> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "pointcloud".into(),
                },
            )
            .await
            .expect("open_stream<PointCloudFrame> pointcloud");
        assert_eq!(sub_pc.manifest.sensor_id, "pointcloud");
        assert_eq!(sub_pc.manifest.sensor_hash, "pc-hash");
        let mut pc_entries = sub_pc.entries;
        let pcf = pc_entries.next().await.unwrap().expect("pc frame");
        assert_eq!(pcf.payload, native_pointcloud(&[[1.0, 1.0, 1.0]]));

        // Substream 3: unknown sensor → Decline.
        let unknown: Result<StreamSubscription<JpegFrame>, OpenStreamError> = consumer
            .open_stream(
                id_p.peer_id(),
                StreamRequest {
                    sensor_id: "no-such-sensor".into(),
                },
            )
            .await;
        match unknown {
            Err(OpenStreamError::Declined { reason })
                if matches!(reason.kind, Some(decline_reason::Kind::SensorNotFound(_))) => {}
            Err(other) => panic!("expected Declined(SensorNotFound), got {other:?}"),
            Ok(_) => panic!("expected Declined for unknown sensor"),
        }

        producer.shutdown();
        consumer.shutdown();
    }

    #[test]
    fn decline_all_streams_returns_sensor_not_found() {
        let provider: StreamProvider = decline_all_streams();
        let req = StreamRequest {
            sensor_id: "anything".into(),
        };
        let any_peer = libp2p_identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        match provider(any_peer, req) {
            StreamDispatch::Decline { reason }
                if matches!(reason.kind, Some(decline_reason::Kind::SensorNotFound(_))) => {}
            _ => panic!("decline_all_streams should always decline with SensorNotFound"),
        }
    }
}
