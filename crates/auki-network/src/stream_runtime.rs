//! Typed `Stream<T>` Rust API on top of [`crate::stream_protocol`]'s wire
//! primitives — [grimsby](https://www.notion.so/3575c8e965928079a955ed9573bbb398)
//! deliverables #2 + #3.
//!
//! Producer side: [`StreamProvider<T>`] — a callable [`ClusterRuntime`]
//! invokes per inbound substream. The callable returns
//! [`StreamDecision::Accept`] (with the [`AcceptInfo`] metadata and a
//! source [`futures::Stream`] of [`ProducerFrame<T>`] values) or
//! [`StreamDecision::Decline`] (with a typed reason). On accept, the
//! runtime spawns a per-substream task that drains the source-Stream,
//! framing each item as a [`StreamMessage::Frame`] on the wire.
//!
//! Consumer side: [`ClusterRuntime::open_stream`] returns a typed
//! [`StreamSubscription<T>`] containing the [`AcceptInfo`] from the
//! producer plus a [`futures::Stream`] of [`Result<ConsumerFrame<T>,
//! StreamError>`] items. The iterator yields one [`Err`] as a final
//! item describing how the stream ended (graceful end-of-stream,
//! connection lost, protocol error) and then returns `None`.
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
//! ## Generic in `T`, locked to `JpegFrame` at runtime construction time
//!
//! Per grimsby D4 — `T = JpegFrame` for grimsby v1; the wire
//! envelope and codec are generic over `T`. The `stream_provider`
//! signature on `ClusterRuntime` is `StreamProvider<JpegFrame>`.
//! `open_stream<T>` is generic on the consumer side because a consumer
//! might subscribe to producers emitting different `T`s; `T` is fixed
//! per call. Generalizing the runtime to multiplex multiple producer-side
//! `T`s on one daemon would be a type-erased follow-up — defer.

use crate::cluster_runtime::ClusterRuntime;
use crate::stream_protocol::{
    AcceptInfo, DeclineReason, EndReason, JpegFrame, STREAM_PROTOCOL, StreamMessage,
    StreamProtocolError, StreamRequest, read_message, write_message,
};
use futures::{Stream, StreamExt, channel::mpsc};
use libp2p::{PeerId, StreamProtocol};
use libp2p_stream::OpenStreamError as Libp2pOpenStreamError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

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

/// Source-stream type the app returns inside [`StreamDecision::Accept`].
///
/// Item is `Result<ProducerFrame<T>, String>` so the producer can end
/// with an error reason — `Some(Err(detail))` is mapped to
/// [`EndReason::ProducerError { detail }`][EndReason::ProducerError]
/// on the wire. `None` (stream returns) is mapped to
/// [`EndReason::SourceEnded`]. The SDK-level shutdown / session-change
/// paths can override either with their own [`EndReason`].
pub type SourceStream<T> =
    Pin<Box<dyn Stream<Item = Result<ProducerFrame<T>, String>> + Send>>;

/// Provider's accept/decline decision for a single inbound request.
pub enum StreamDecision<T> {
    /// Accept the request. SDK writes [`StreamMessage::Accept(info)`] then
    /// drains `source` onto the substream until the stream ends or the
    /// substream closes.
    Accept { info: AcceptInfo, source: SourceStream<T> },
    /// Decline the request with a typed reason. SDK writes
    /// [`StreamMessage::Decline { reason }`] and closes the substream.
    Decline { reason: DeclineReason },
}

/// The provider callable. Sync return; any async setup the producer
/// needs (subscribing to a fanout channel, opening a hardware handle,
/// allocating buffers) lives *inside* the source-Stream the app
/// constructs and returns.
///
/// `Send + Sync` because the runtime task holds the callable in an
/// `Arc` shared across spawned per-substream tasks.
pub type StreamProvider<T> =
    Arc<dyn Fn(StreamRequest) -> StreamDecision<T> + Send + Sync>;

/// Convenience constructor for consumer-only nodes (Park, analytics
/// tools, future Sentinel-as-consumer) that don't expose any sensors.
/// Declines every inbound request with [`DeclineReason::SensorNotFound`].
pub fn decline_all_streams<T>() -> StreamProvider<T>
where
    T: 'static,
{
    Arc::new(|_req| StreamDecision::Decline {
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
/// [`STREAM_PROTOCOL`].
///
/// 1. Read [`StreamMessage::Request`].
/// 2. Invoke `provider(request) -> StreamDecision<T>`.
/// 3. If `Decline`: write [`StreamMessage::Decline { reason }`] and close.
/// 4. If `Accept(info, source)`: write [`StreamMessage::Accept(info)`]
///    then drain `source` onto the substream as
///    [`StreamMessage::Frame`] values, stamping `seq` 0, 1, 2, …
/// 5. Source returns `None` → write
///    [`StreamMessage::EndOfStream { reason: SourceEnded }`] and close.
/// 6. Source returns `Some(Err(detail))` → write
///    [`StreamMessage::EndOfStream { reason: ProducerError { detail } }`]
///    and close.
///
/// Errors during write (consumer disconnected, libp2p tore down the
/// substream) terminate the task silently — the substream is dead
/// already and the producer's source-Stream gets dropped along with the
/// task.
pub(crate) async fn handle_inbound_substream<T>(
    _peer: PeerId,
    mut substream: libp2p::Stream,
    provider: StreamProvider<T>,
) where
    T: Serialize + DeserializeOwned + Send + 'static,
{
    // 1. Read the request.
    let request: StreamRequest = match read_message::<T, _>(&mut substream).await {
        Ok(StreamMessage::Request(req)) => req,
        Ok(_other) => {
            // Wire-protocol violation: peer wrote something other than
            // Request as its first message. Drop the substream silently;
            // the peer's open_stream call will surface its own protocol
            // error from the read side.
            return;
        }
        Err(_) => {
            // Read failed — substream's already broken. Nothing to do.
            return;
        }
    };

    // 2. Invoke the provider.
    let decision = (provider)(request);

    match decision {
        StreamDecision::Decline { reason } => {
            // 3. Write decline and close.
            let msg: StreamMessage<T> = StreamMessage::Decline { reason };
            let _ = write_message(&mut substream, &msg).await;
        }
        StreamDecision::Accept { info, mut source } => {
            // 4. Write accept.
            let accept_msg: StreamMessage<T> = StreamMessage::Accept(info);
            if write_message(&mut substream, &accept_msg).await.is_err() {
                return;
            }
            // 5–6. Drain source into Frame messages until end-of-source
            // or send error.
            let mut seq: u64 = 0;
            let end_reason = loop {
                match source.next().await {
                    Some(Ok(frame)) => {
                        let msg: StreamMessage<T> = StreamMessage::Frame {
                            timestamp_ns: frame.timestamp_ns,
                            seq,
                            payload: frame.payload,
                        };
                        if write_message(&mut substream, &msg).await.is_err() {
                            // Consumer disconnected mid-stream. Don't
                            // try to write EndOfStream — the substream
                            // is dead. Just exit; source drops with the
                            // task.
                            return;
                        }
                        seq = seq.wrapping_add(1);
                    }
                    Some(Err(detail)) => break EndReason::ProducerError { detail },
                    None => break EndReason::SourceEnded,
                }
            };
            // Write the final EndOfStream. Best-effort — if the consumer
            // already disconnected, the write fails silently.
            let end_msg: StreamMessage<T> = StreamMessage::EndOfStream { reason: end_reason };
            let _ = write_message(&mut substream, &end_msg).await;
        }
    }
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

    fn jpeg_provider_yielding_three_frames() -> StreamProvider<JpegFrame> {
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
            StreamDecision::Accept {
                info: AcceptInfo {
                    sensor_hash: "sensor-hash-3".into(),
                    clock_id: "test/session-monotonic".into(),
                    clock_hash: "clock-hash-3".into(),
                },
                source: Box::pin(stream::iter(frames)),
            }
        })
    }

    fn jpeg_provider_declines_unknown() -> StreamProvider<JpegFrame> {
        Arc::new(|req| {
            if req.sensor_id == "exists" {
                let frames = vec![Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: JpegFrame { bytes: vec![0xff] },
                })];
                StreamDecision::Accept {
                    info: AcceptInfo {
                        sensor_hash: "h".into(),
                        clock_id: "c".into(),
                        clock_hash: "ch".into(),
                    },
                    source: Box::pin(stream::iter(frames)),
                }
            } else {
                StreamDecision::Decline {
                    reason: DeclineReason::SensorNotFound,
                }
            }
        })
    }

    fn jpeg_provider_yields_then_errors() -> StreamProvider<JpegFrame> {
        Arc::new(|_req| {
            let items = vec![
                Ok(ProducerFrame {
                    timestamp_ns: 1,
                    payload: JpegFrame { bytes: vec![0xaa] },
                }),
                Err("encoder died".to_string()),
            ];
            StreamDecision::Accept {
                info: AcceptInfo {
                    sensor_hash: "h".into(),
                    clock_id: "c".into(),
                    clock_hash: "ch".into(),
                },
                source: Box::pin(stream::iter(items)),
            }
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

    #[test]
    fn decline_all_streams_returns_sensor_not_found() {
        let provider: StreamProvider<JpegFrame> = decline_all_streams();
        let req = StreamRequest {
            sensor_id: "anything".into(),
        };
        match provider(req) {
            StreamDecision::Decline {
                reason: DeclineReason::SensorNotFound,
            } => {}
            _ => panic!("decline_all_streams should always decline with SensorNotFound"),
        }
    }
}
