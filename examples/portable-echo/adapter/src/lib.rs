//! Cross-platform Auki peer adapter for the portable echo protocol.
//!
//! This crate owns the mechanical runtime glue shared by native and Web apps:
//! protocol registration, bounded operation deadlines, stream cleanup, and
//! nonblocking inbound observations. The wire contract remains in
//! `auki-portable-echo-protocol` and has no SDK or platform dependency.

#![forbid(unsafe_code)]

use std::{
    fmt,
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use async_channel::{Receiver, Sender, TrySendError};
use auki_portable_echo_protocol::{
    EchoProtocolError, EchoRequest, ID, MAX_FRAME_BYTES, run_client, run_server,
};
use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::{AsyncWriteExt, FutureExt, pin_mut};
use futures_timer::Delay;

/// Exact portable echo protocol identifier.
pub const PROTOCOL_ID: &str = ID;

/// Maximum number of concurrently served echo streams.
pub const MAX_CONCURRENCY: usize = 32;

/// Fixed deadline for opening, exchanging, or closing one echo stream.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

const SERVED_EVENT_CAPACITY: usize = 32;

/// Build the exact bounded protocol registration shared by every runtime.
pub fn protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(
        PROTOCOL_ID,
        MAX_CONCURRENCY,
        u32::try_from(MAX_FRAME_BYTES).expect("the portable frame bound fits in u32"),
    )
}

/// Cloneable outbound half of the portable echo adapter.
#[derive(Clone)]
pub struct EchoClient {
    protocols: AukiPeerProtocols,
}

impl EchoClient {
    /// Send one echo through the routes configured on the owning native Auki peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn send(
        &self,
        remote_peer_id: PeerId,
        payload: impl Into<Vec<u8>>,
    ) -> Result<EchoSendReceipt, EchoAdapterError> {
        let request = EchoRequest::new(payload)?;
        send_opened(
            remote_peer_id,
            request,
            self.protocols.open(remote_peer_id, PROTOCOL_ID),
        )
        .await
    }

    /// Send one echo through an exact advertised route.
    pub async fn send_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        payload: impl Into<Vec<u8>>,
    ) -> Result<EchoSendReceipt, EchoAdapterError> {
        let request = EchoRequest::new(payload)?;
        send_opened(
            remote_peer_id,
            request,
            self.protocols
                .open_exact(remote_peer_id, route, PROTOCOL_ID),
        )
        .await
    }
}

/// Mounted portable echo service plus its outbound client.
pub struct EchoEndpoint {
    client: EchoClient,
    registration: AukiProtocolRegistration,
    events: EchoEventReceiver,
}

impl EchoEndpoint {
    /// Mount the portable echo protocol on one running Auki peer.
    pub fn mount(protocols: AukiPeerProtocols) -> Result<Self, EchoAdapterError> {
        let (delivery, events) = event_channel(SERVED_EVENT_CAPACITY);
        let registration = protocols.register(protocol_spec()?, move |mut stream| {
            let delivery = delivery.clone();
            async move {
                let remote_peer_id = stream.remote_peer().peer_id;
                let exchange = deadline(EchoOperation::Exchange, run_server(&mut stream))
                    .await
                    .and_then(|result| result.map_err(EchoAdapterError::Protocol));
                let cleanup = deadline(EchoOperation::Close, AsyncWriteExt::close(&mut stream))
                    .await
                    .and_then(|result| {
                        result.map_err(|error| EchoAdapterError::Close(error.to_string()))
                    });

                let event = match prefer_primary(exchange, cleanup) {
                    Ok(request) => EchoServeEvent::Served(EchoServeReceipt {
                        remote_peer_id,
                        payload: request.into_bytes(),
                    }),
                    Err(error) => EchoServeEvent::Failed {
                        remote_peer_id,
                        error: error.to_string(),
                    },
                };
                delivery.publish(event);
            }
        })?;

        Ok(Self {
            client: EchoClient { protocols },
            registration,
            events,
        })
    }

    /// Clone the outbound client without cloning inbound registration ownership.
    pub fn client(&self) -> EchoClient {
        self.client.clone()
    }

    /// Obtain a receiver for inbound completion, failure, and lag events.
    ///
    /// Clones compete for the same bounded queue; applications should normally
    /// retain one receiver.
    pub fn events(&self) -> EchoEventReceiver {
        self.events.clone()
    }

    /// Send one echo through the routes configured on the owning native Auki peer.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn send(
        &self,
        remote_peer_id: PeerId,
        payload: impl Into<Vec<u8>>,
    ) -> Result<EchoSendReceipt, EchoAdapterError> {
        self.client.send(remote_peer_id, payload).await
    }

    /// Send one echo through an exact advertised route.
    pub async fn send_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        payload: impl Into<Vec<u8>>,
    ) -> Result<EchoSendReceipt, EchoAdapterError> {
        self.client.send_exact(remote_peer_id, route, payload).await
    }

    /// Stop accepting inbound echo streams and await admitted handlers.
    pub async fn close(self) -> Result<(), EchoAdapterError> {
        self.registration
            .close()
            .await
            .map_err(EchoAdapterError::Sdk)
    }
}

/// One successful inbound echo exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EchoServeReceipt {
    /// Authenticated remote peer that sent the request.
    pub remote_peer_id: PeerId,
    /// Exact request bytes echoed to the remote peer.
    pub payload: Vec<u8>,
}

/// One successful outbound echo exchange.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EchoSendReceipt {
    /// Authenticated remote peer that returned the response.
    pub remote_peer_id: PeerId,
    /// Exact response bytes validated against the request.
    pub payload: Vec<u8>,
    /// Whether the selected transport route used a relay circuit.
    pub relayed: bool,
}

/// Observable completion from the bounded inbound event queue.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EchoServeEvent {
    /// One request was echoed and its stream closed successfully.
    Served(EchoServeReceipt),
    /// The exchange or its stream cleanup failed.
    Failed {
        /// Authenticated remote peer associated with the stream.
        remote_peer_id: PeerId,
        /// Stable human-readable diagnostic.
        error: String,
    },
    /// Completed events were discarded because the consumer fell behind.
    Lagged {
        /// Number of events discarded since the previous lag observation.
        dropped: u64,
    },
}

/// Single-consumer view of bounded inbound echo observations.
#[derive(Clone)]
pub struct EchoEventReceiver {
    events: Receiver<EchoServeEvent>,
    dropped: Arc<AtomicU64>,
    event_since_lag: Arc<AtomicBool>,
}

impl EchoEventReceiver {
    /// Wait for the next completion, failure, or explicit lag event.
    ///
    /// Returns `None` after the mounted endpoint and all admitted handlers are
    /// gone and buffered events have been drained.
    pub async fn recv(&self) -> Option<EchoServeEvent> {
        if self.event_since_lag.swap(false, Ordering::AcqRel)
            && let Some(dropped) = self.take_dropped()
        {
            return Some(EchoServeEvent::Lagged { dropped });
        }
        match self.events.recv().await {
            Ok(event) => {
                self.event_since_lag.store(true, Ordering::Release);
                Some(event)
            }
            Err(_) => self
                .take_dropped()
                .map(|dropped| EchoServeEvent::Lagged { dropped }),
        }
    }

    fn take_dropped(&self) -> Option<u64> {
        let dropped = self.dropped.swap(0, Ordering::AcqRel);
        if dropped == 0 {
            return None;
        }
        Some(dropped)
    }
}

#[derive(Clone)]
struct EventDelivery {
    events: Sender<EchoServeEvent>,
    dropped: Arc<AtomicU64>,
}

impl EventDelivery {
    fn publish(&self, event: EchoServeEvent) {
        match self.events.try_send(event) {
            Ok(()) | Err(TrySendError::Closed(_)) => {}
            Err(TrySendError::Full(_)) => {
                let _ =
                    self.dropped
                        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |dropped| {
                            Some(dropped.saturating_add(1))
                        });
            }
        }
    }
}

fn event_channel(capacity: usize) -> (EventDelivery, EchoEventReceiver) {
    let (event_sender, event_receiver) = async_channel::bounded(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    let event_since_lag = Arc::new(AtomicBool::new(false));
    (
        EventDelivery {
            events: event_sender,
            dropped: Arc::clone(&dropped),
        },
        EchoEventReceiver {
            events: event_receiver,
            dropped,
            event_since_lag,
        },
    )
}

async fn send_opened<F>(
    remote_peer_id: PeerId,
    request: EchoRequest,
    opening: F,
) -> Result<EchoSendReceipt, EchoAdapterError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(EchoOperation::Open, opening)
        .await?
        .map_err(EchoAdapterError::Sdk)?;
    let relayed = stream.is_relayed();
    let exchange = deadline(EchoOperation::Exchange, run_client(&mut stream, request))
        .await
        .and_then(|result| result.map_err(EchoAdapterError::Protocol));
    let cleanup = deadline(EchoOperation::Close, stream.close())
        .await
        .and_then(|result| result.map_err(|error| EchoAdapterError::Close(error.to_string())));
    let response = prefer_primary(exchange, cleanup)?;

    Ok(EchoSendReceipt {
        remote_peer_id,
        payload: response.into_bytes(),
        relayed,
    })
}

async fn deadline<T>(
    operation: EchoOperation,
    future: impl Future<Output = T>,
) -> Result<T, EchoAdapterError> {
    deadline_after(operation, OPERATION_TIMEOUT, future).await
}

async fn deadline_after<T>(
    operation: EchoOperation,
    duration: Duration,
    future: impl Future<Output = T>,
) -> Result<T, EchoAdapterError> {
    let work = future.fuse();
    let timer = Delay::new(duration).fuse();
    pin_mut!(work, timer);
    futures::select_biased! {
        result = work => Ok(result),
        () = timer => Err(EchoAdapterError::Timeout(operation)),
    }
}

fn prefer_primary<T, E>(primary: Result<T, E>, cleanup: Result<(), E>) -> Result<T, E> {
    match primary {
        Err(error) => Err(error),
        Ok(value) => cleanup.map(|()| value),
    }
}

/// One bounded echo operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EchoOperation {
    /// Opening and mutually authenticating the application stream.
    Open,
    /// Running the exact portable request/response conversation.
    Exchange,
    /// Closing one authenticated stream.
    Close,
}

impl fmt::Display for EchoOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Exchange => "exchange",
            Self::Close => "close",
        })
    }
}

/// Failure from the shared portable echo runtime adapter.
#[derive(Debug, thiserror::Error)]
pub enum EchoAdapterError {
    /// The SDK protocol surface rejected registration or stream opening.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// The portable wire contract or conversation failed.
    #[error("portable echo protocol failed: {0}")]
    Protocol(#[from] EchoProtocolError),
    /// One fixed-deadline operation did not complete.
    #[error("echo {0} timed out after 5 seconds")]
    Timeout(EchoOperation),
    /// Authenticated stream cleanup failed after the exchange.
    #[error("close authenticated echo stream: {0}")]
    Close(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use auki_sdk::Identity;

    #[test]
    fn spec_mounts_the_exact_portable_contract() {
        let spec = protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), PROTOCOL_ID);
        assert_eq!(spec.max_concurrency(), MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_FRAME_BYTES as u32);
    }

    #[test]
    fn overloaded_event_queue_reports_lag_without_starving_buffered_events() {
        let peer_id = Identity::generate().peer_id();
        let (delivery, events) = event_channel(1);
        delivery.publish(EchoServeEvent::Served(EchoServeReceipt {
            remote_peer_id: peer_id,
            payload: b"first".to_vec(),
        }));
        delivery.publish(EchoServeEvent::Served(EchoServeReceipt {
            remote_peer_id: peer_id,
            payload: b"dropped".to_vec(),
        }));

        assert_eq!(
            futures::executor::block_on(events.recv()),
            Some(EchoServeEvent::Served(EchoServeReceipt {
                remote_peer_id: peer_id,
                payload: b"first".to_vec(),
            }))
        );

        delivery.publish(EchoServeEvent::Served(EchoServeReceipt {
            remote_peer_id: peer_id,
            payload: b"second".to_vec(),
        }));
        delivery.publish(EchoServeEvent::Served(EchoServeReceipt {
            remote_peer_id: peer_id,
            payload: b"also dropped".to_vec(),
        }));

        assert_eq!(
            futures::executor::block_on(events.recv()),
            Some(EchoServeEvent::Lagged { dropped: 2 })
        );

        delivery.publish(EchoServeEvent::Served(EchoServeReceipt {
            remote_peer_id: peer_id,
            payload: b"still dropped".to_vec(),
        }));

        assert_eq!(
            futures::executor::block_on(events.recv()),
            Some(EchoServeEvent::Served(EchoServeReceipt {
                remote_peer_id: peer_id,
                payload: b"second".to_vec(),
            }))
        );
        assert_eq!(
            futures::executor::block_on(events.recv()),
            Some(EchoServeEvent::Lagged { dropped: 1 })
        );
    }

    #[test]
    fn exchange_failure_wins_over_cleanup_failure() {
        assert_eq!(
            prefer_primary::<(), _>(Err("exchange"), Err("cleanup")),
            Err("exchange")
        );
        assert_eq!(prefer_primary(Ok(7), Err("cleanup")), Err("cleanup"));
        assert_eq!(prefer_primary::<_, &str>(Ok(7), Ok(())), Ok(7));
    }

    #[test]
    fn expired_deadline_reports_the_interrupted_adapter_operation() {
        let result = futures::executor::block_on(deadline_after(
            EchoOperation::Open,
            Duration::ZERO,
            futures::future::pending::<()>(),
        ));
        assert!(matches!(
            result,
            Err(EchoAdapterError::Timeout(EchoOperation::Open))
        ));
    }
}
