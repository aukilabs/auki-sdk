//! Transport-neutral typed-stream business types.
//!
//! The authenticated Domain runtime owns transport, authentication, handler
//! tasks, and shutdown. This module retains only the application-facing
//! dispatch and subscription shapes shared by producers and consumers.

use std::{pin::Pin, sync::Arc};

use auki_datatypes::{detection::DetectionFrame, map::MapUpdate};
use futures::Stream;
use libp2p_identity::PeerId;

use crate::stream_protocol::{
    CameraFrame, DeclineReason, EndReason, StreamManifest, StreamProtocolError, StreamRequest,
    audio, joint_encoders, point_cloud, pose,
};

/// One producer item before the SDK stamps its wire sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamItem<T> {
    /// Timestamp expressed in the clock pinned by the stream manifest.
    pub timestamp_ns: i64,
    /// Typed payload.
    pub payload: T,
}

/// Application-owned producer stream.
pub type SourceStream<T> = Pin<Box<dyn Stream<Item = Result<StreamItem<T>, String>> + Send>>;

/// Closed set of typed payloads supported by the retained stream protocol.
pub enum StreamDispatch {
    /// Accept with camera frames.
    AcceptCamera {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<CameraFrame>,
    },
    /// Accept with point-cloud data.
    AcceptPointCloud {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<point_cloud::Data>,
    },
    /// Accept with joint-encoder data.
    AcceptJointEncoders {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<joint_encoders::Data>,
    },
    /// Accept with audio data.
    AcceptAudio {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<audio::Data>,
    },
    /// Accept with scalar data.
    AcceptScalar {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<auki_datatypes::scalar::Data>,
    },
    /// Accept with spatial transforms.
    AcceptPose {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<pose::SpatialTransform>,
    },
    /// Accept with detection frames.
    AcceptDetection {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<DetectionFrame>,
    },
    /// Accept with map updates.
    AcceptMap {
        /// Manifest fixed for the stream lifetime.
        manifest: StreamManifest,
        /// Producer items.
        source: SourceStream<MapUpdate>,
    },
    /// Decline without exposing a typed source.
    Decline {
        /// Stable wire reason.
        reason: DeclineReason,
    },
}

/// Synchronous, cheap application dispatch callback for an authenticated open.
pub type StreamProvider = Arc<dyn Fn(PeerId, StreamRequest) -> StreamDispatch + Send + Sync>;

/// Provider for consumer-only nodes.
pub fn decline_all_streams() -> StreamProvider {
    Arc::new(|_peer, _request| StreamDispatch::Decline {
        reason: DeclineReason::sensor_not_found(),
    })
}

/// One typed consumer item with its SDK wire sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEntry<T> {
    /// Producer timestamp.
    pub timestamp_ns: i64,
    /// Monotonic per-stream sequence number.
    pub seq: u64,
    /// Typed payload.
    pub payload: T,
}

/// Accepted typed subscription.
pub struct StreamSubscription<T> {
    /// Manifest fixed by the producer during the open handshake.
    pub manifest: StreamManifest,
    /// Entries followed by exactly one terminal error, then end-of-stream.
    pub entries: Pin<Box<dyn Stream<Item = Result<StreamEntry<T>, StreamError>> + Send>>,
}

/// One terminal typed-stream outcome.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    /// Producer sent an explicit end reason.
    #[error("end of stream: {reason:?}")]
    EndOfStream {
        /// Typed producer reason.
        reason: EndReason,
    },
    /// The transport closed without an explicit end frame.
    #[error("connection lost")]
    ConnectionLost,
    /// The peer sent a malformed envelope or typed payload.
    #[error("protocol error: {0}")]
    Protocol(#[source] StreamProtocolError),
}
