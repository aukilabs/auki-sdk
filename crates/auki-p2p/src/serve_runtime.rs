//! SDK-owned serving runtime for inbound protocol traffic and published streams.

use crate::api::{
    AukiNode, AukiNodeError, AukiServedInbound, AukiServedSubscription, LifecycleInput,
};
use auki_protocol::v1::{
    error,
    subscribe::{SubscribeEnd, SubscribeEndReason},
};
use futures::StreamExt as _;
use libp2p::PeerId;
use std::collections::BTreeMap;
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{Duration, sleep},
};

const SOURCE_EVENT_BUFFER: usize = 1024;
const CONSUMER_END_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Lightweight counters for the SDK serving loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AukiServeRuntimeStatus {
    /// Lifecycle handshakes served.
    pub lifecycles_served: u64,
    /// Offer-catalog requests served.
    pub offer_catalogs_served: u64,
    /// Successful Get requests served.
    pub gets_served: u64,
    /// Failed Get requests served with a structured protocol response.
    pub gets_rejected: u64,
    /// Subscribe start requests accepted.
    pub subscriptions_accepted: u64,
    /// Subscribe start requests rejected with a structured protocol response.
    pub subscriptions_rejected: u64,
    /// Currently active runtime-managed published subscriptions.
    pub active_subscriptions: u64,
    /// Published source frames received by the runtime.
    pub frames_produced: u64,
    /// Published source frames written to subscribers.
    pub frames_sent: u64,
    /// Published source frames dropped because their subscription was gone.
    pub frames_dropped: u64,
    /// Runtime-managed subscriptions completed by the producer/source.
    pub subscriptions_completed: u64,
    /// Runtime-managed subscriptions cancelled by the consumer.
    pub subscriptions_cancelled: u64,
    /// Runtime-managed subscriptions closed after a local runtime or transport failure.
    pub subscriptions_failed: u64,
    /// Last runtime failure observed while serving active subscriptions.
    pub last_failure: Option<String>,
}

/// Status for one active runtime-managed published subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiActiveSubscriptionStatus {
    /// Runtime-local subscription id.
    pub subscription_id: u64,
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Served domain id.
    pub domain_id: String,
    /// Served offer id.
    pub offer_id: String,
    /// Selected payload type.
    pub payload_type: String,
    /// Frames written to this subscriber.
    pub messages_sent: u64,
}

/// Status for one ended runtime-managed published subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AukiEndedSubscriptionStatus {
    /// Runtime-local subscription id.
    pub subscription_id: u64,
    /// Requesting peer id.
    pub peer_id: PeerId,
    /// Served domain id.
    pub domain_id: String,
    /// Served offer id.
    pub offer_id: String,
    /// End reason.
    pub reason: SubscribeEndReason,
    /// Stable end error code, if any.
    pub error_code: Option<String>,
    /// Retry hint, if supplied by the end frame or runtime.
    pub retryable: Option<bool>,
    /// Frames written before the subscription ended.
    pub messages_sent: u64,
}

/// Event returned by [`AukiServeRuntime::serve_next`].
pub enum AukiServeRuntimeEvent {
    /// A non-runtime-managed inbound protocol stream was served.
    Inbound(AukiServedInbound),
    /// A published Subscribe request was accepted and is now runtime-managed.
    PublishedSubscriptionStarted(AukiActiveSubscriptionStatus),
    /// One published frame was written to an active subscription.
    PublishedSubscriptionMessageSent(AukiActiveSubscriptionStatus),
    /// One runtime-managed published subscription ended.
    PublishedSubscriptionEnded(AukiEndedSubscriptionStatus),
}

struct ActivePublishedSubscription {
    id: u64,
    peer_id: PeerId,
    domain_id: String,
    offer_id: String,
    payload_type: String,
    messages_sent: u64,
    subscription: Option<AukiServedSubscription>,
    source_task: JoinHandle<()>,
}

enum RuntimeSourceEvent {
    Chunk {
        subscription_id: u64,
        chunk: Vec<u8>,
    },
    Complete {
        subscription_id: u64,
    },
}

enum RuntimePoll {
    Inbound(Result<Option<AukiServedInbound>, AukiNodeError>),
    Source(Option<RuntimeSourceEvent>),
    ConsumerEndTick,
}

impl AukiServeRuntimeStatus {
    fn record_inbound(&mut self, served: &AukiServedInbound) {
        match served {
            AukiServedInbound::Lifecycle(_) => {
                self.lifecycles_served = self.lifecycles_served.saturating_add(1);
            }
            AukiServedInbound::OfferCatalog(_) => {
                self.offer_catalogs_served = self.offer_catalogs_served.saturating_add(1);
            }
            AukiServedInbound::Get(served) => {
                if served.success {
                    self.gets_served = self.gets_served.saturating_add(1);
                } else {
                    self.gets_rejected = self.gets_rejected.saturating_add(1);
                }
            }
            AukiServedInbound::Subscribe(served) => {
                if served.accepted {
                    self.subscriptions_accepted = self.subscriptions_accepted.saturating_add(1);
                } else {
                    self.subscriptions_rejected = self.subscriptions_rejected.saturating_add(1);
                }
            }
        }
    }
}

impl ActivePublishedSubscription {
    fn status(&self) -> AukiActiveSubscriptionStatus {
        AukiActiveSubscriptionStatus {
            subscription_id: self.id,
            peer_id: self.peer_id,
            domain_id: self.domain_id.clone(),
            offer_id: self.offer_id.clone(),
            payload_type: self.payload_type.clone(),
            messages_sent: self.messages_sent,
        }
    }

    fn ended_status(
        &self,
        reason: SubscribeEndReason,
        error_code: Option<String>,
        retryable: Option<bool>,
    ) -> AukiEndedSubscriptionStatus {
        AukiEndedSubscriptionStatus {
            subscription_id: self.id,
            peer_id: self.peer_id,
            domain_id: self.domain_id.clone(),
            offer_id: self.offer_id.clone(),
            reason,
            error_code,
            retryable,
            messages_sent: self.messages_sent,
        }
    }
}

impl Drop for ActivePublishedSubscription {
    fn drop(&mut self) {
        self.source_task.abort();
    }
}

/// Runtime wrapper that owns one [`AukiNode`] serving inbound SDK protocols.
pub struct AukiServeRuntime {
    node: AukiNode,
    lifecycle_input: LifecycleInput,
    status: AukiServeRuntimeStatus,
    active_subscriptions: BTreeMap<u64, ActivePublishedSubscription>,
    next_subscription_id: u64,
    source_tx: mpsc::Sender<RuntimeSourceEvent>,
    source_rx: mpsc::Receiver<RuntimeSourceEvent>,
}

impl AukiServeRuntime {
    /// Create a serving runtime around an already configured node.
    pub fn new(node: AukiNode) -> Self {
        let (source_tx, source_rx) = mpsc::channel(SOURCE_EVENT_BUFFER);
        Self {
            node,
            lifecycle_input: LifecycleInput::new(),
            status: AukiServeRuntimeStatus::default(),
            active_subscriptions: BTreeMap::new(),
            next_subscription_id: 0,
            source_tx,
            source_rx,
        }
    }

    /// Override the lifecycle policy input used for inbound handshakes.
    pub fn with_lifecycle_input(mut self, lifecycle_input: LifecycleInput) -> Self {
        self.lifecycle_input = lifecycle_input;
        self
    }

    /// Borrow the owned node for diagnostics or local provider registration.
    pub fn node(&self) -> &AukiNode {
        &self.node
    }

    /// Mutably borrow the owned node for configuration before the loop runs.
    pub fn node_mut(&mut self) -> &mut AukiNode {
        &mut self.node
    }

    /// Consume the runtime and return the owned node.
    pub fn into_node(self) -> AukiNode {
        self.node
    }

    /// Return the current serving counters.
    pub fn status(&self) -> &AukiServeRuntimeStatus {
        &self.status
    }

    /// Return active runtime-managed published subscriptions.
    pub fn active_subscriptions(&self) -> Vec<AukiActiveSubscriptionStatus> {
        self.active_subscriptions
            .values()
            .map(ActivePublishedSubscription::status)
            .collect()
    }

    /// Serve one runtime event without fixed per-protocol timeout sequencing.
    pub async fn serve_next(
        &mut self,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        loop {
            if let Some(event) = self.poll_consumer_end()? {
                return Ok(Some(event));
            }

            let lifecycle_input = self.lifecycle_input.clone();
            let has_active_subscriptions = !self.active_subscriptions.is_empty();
            let poll = {
                let node = &mut self.node;
                let source_rx = &mut self.source_rx;
                tokio::select! {
                    biased;
                    inbound = node.serve_next_inbound(lifecycle_input, now) => {
                        RuntimePoll::Inbound(inbound)
                    }
                    source = source_rx.recv(), if has_active_subscriptions => {
                        RuntimePoll::Source(source)
                    }
                    _ = sleep(CONSUMER_END_POLL_INTERVAL), if has_active_subscriptions => {
                        RuntimePoll::ConsumerEndTick
                    }
                }
            };

            match poll {
                RuntimePoll::Inbound(result) => {
                    let Some(served) = result? else {
                        return Ok(None);
                    };
                    return self.handle_inbound(served);
                }
                RuntimePoll::Source(Some(event)) => {
                    if let Some(event) = self.handle_source_event(event, now).await? {
                        return Ok(Some(event));
                    }
                }
                RuntimePoll::Source(None) => return Ok(None),
                RuntimePoll::ConsumerEndTick => {}
            }
        }
    }

    fn handle_inbound(
        &mut self,
        served: AukiServedInbound,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.status.record_inbound(&served);

        let AukiServedInbound::Subscribe(served_subscribe) = served else {
            return Ok(Some(AukiServeRuntimeEvent::Inbound(served)));
        };

        if !served_subscribe.accepted {
            return Ok(Some(AukiServeRuntimeEvent::Inbound(
                AukiServedInbound::Subscribe(served_subscribe),
            )));
        }

        let Some(domain_id) = served_subscribe.domain_id.clone() else {
            return Ok(Some(AukiServeRuntimeEvent::Inbound(
                AukiServedInbound::Subscribe(served_subscribe),
            )));
        };
        let Some(offer_id) = served_subscribe.offer_id.clone() else {
            return Ok(Some(AukiServeRuntimeEvent::Inbound(
                AukiServedInbound::Subscribe(served_subscribe),
            )));
        };
        if !self.node.has_local_publication(&domain_id, &offer_id) {
            return Ok(Some(AukiServeRuntimeEvent::Inbound(
                AukiServedInbound::Subscribe(served_subscribe),
            )));
        }

        let subscription =
            served_subscribe
                .into_subscription()
                .ok_or(AukiNodeError::SubscribeServe(
                    crate::SubscribeServeError::AlreadyEnded,
                ))?;
        let source = self.node.open_publication_source(&domain_id, &offer_id)?;
        let id = self.allocate_subscription_id();
        let peer_id = subscription.peer_id();
        let payload_type = subscription.payload_type().to_owned();
        let source_task = spawn_source_task(id, source, self.source_tx.clone());
        let active = ActivePublishedSubscription {
            id,
            peer_id,
            domain_id,
            offer_id,
            payload_type,
            messages_sent: 0,
            subscription: Some(subscription),
            source_task,
        };
        let status = active.status();
        self.active_subscriptions.insert(id, active);
        self.refresh_active_subscription_count();
        Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionStarted(
            status,
        )))
    }

    async fn handle_source_event(
        &mut self,
        event: RuntimeSourceEvent,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        match event {
            RuntimeSourceEvent::Chunk {
                subscription_id,
                chunk,
            } => self.handle_source_chunk(subscription_id, chunk, now).await,
            RuntimeSourceEvent::Complete { subscription_id } => {
                self.handle_source_complete(subscription_id).await
            }
        }
    }

    async fn handle_source_chunk(
        &mut self,
        subscription_id: u64,
        chunk: Vec<u8>,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.status.frames_produced = self.status.frames_produced.saturating_add(1);
        let Some(mut active) = self.active_subscriptions.remove(&subscription_id) else {
            self.status.frames_dropped = self.status.frames_dropped.saturating_add(1);
            return Ok(None);
        };

        match consumer_end_event(&mut active) {
            Ok(Some(event)) => {
                let ended = active.ended_status(
                    event.reason,
                    event.error.as_ref().map(|error| error.code.clone()),
                    event.retryable,
                );
                self.status.subscriptions_cancelled =
                    self.status.subscriptions_cancelled.saturating_add(1);
                self.refresh_active_subscription_count();
                return Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                    ended,
                )));
            }
            Ok(None) => {}
            Err(error) => {
                self.status.subscriptions_failed =
                    self.status.subscriptions_failed.saturating_add(1);
                self.status.last_failure = Some(format!("{error:?}"));
                let ended = active.ended_status(
                    SubscribeEndReason::Error,
                    Some(error::TRANSPORT_FAILED.to_owned()),
                    Some(true),
                );
                self.refresh_active_subscription_count();
                return Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                    ended,
                )));
            }
        }

        let message =
            self.node
                .next_publication_message(&active.domain_id, &active.offer_id, chunk, now)?;
        let send_result = self
            .node
            .send_served_subscription_message(
                active
                    .subscription
                    .as_mut()
                    .expect("active subscription should hold a stream"),
                &message,
            )
            .await;
        match send_result {
            Ok(()) => {
                active.messages_sent = active.messages_sent.saturating_add(1);
                self.status.frames_sent = self.status.frames_sent.saturating_add(1);
                let status = active.status();
                self.active_subscriptions.insert(subscription_id, active);
                self.refresh_active_subscription_count();
                Ok(Some(
                    AukiServeRuntimeEvent::PublishedSubscriptionMessageSent(status),
                ))
            }
            Err(error) => {
                self.status.subscriptions_failed =
                    self.status.subscriptions_failed.saturating_add(1);
                self.status.last_failure = Some(format!("{error:?}"));
                self.refresh_active_subscription_count();
                let ended = active.ended_status(
                    SubscribeEndReason::Error,
                    Some(error::TRANSPORT_FAILED.to_owned()),
                    Some(true),
                );
                Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                    ended,
                )))
            }
        }
    }

    async fn handle_source_complete(
        &mut self,
        subscription_id: u64,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let Some(mut active) = self.active_subscriptions.remove(&subscription_id) else {
            return Ok(None);
        };
        let ended = active.ended_status(SubscribeEndReason::Complete, None, None);
        let subscription = active
            .subscription
            .take()
            .expect("active subscription should hold a stream");
        self.node
            .end_served_subscription(subscription, SubscribeEndReason::Complete, None, None)
            .await?;
        self.status.subscriptions_completed = self.status.subscriptions_completed.saturating_add(1);
        self.refresh_active_subscription_count();
        Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
            ended,
        )))
    }

    fn poll_consumer_end(&mut self) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let ids = self
            .active_subscriptions
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for id in ids {
            let Some(mut active) = self.active_subscriptions.remove(&id) else {
                continue;
            };
            match consumer_end_event(&mut active) {
                Ok(Some(end)) => {
                    let ended = active.ended_status(
                        end.reason,
                        end.error.as_ref().map(|error| error.code.clone()),
                        end.retryable,
                    );
                    self.status.subscriptions_cancelled =
                        self.status.subscriptions_cancelled.saturating_add(1);
                    self.refresh_active_subscription_count();
                    return Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                        ended,
                    )));
                }
                Ok(None) => {
                    self.active_subscriptions.insert(id, active);
                }
                Err(error) => {
                    self.status.subscriptions_failed =
                        self.status.subscriptions_failed.saturating_add(1);
                    self.status.last_failure = Some(format!("{error:?}"));
                    let ended = active.ended_status(
                        SubscribeEndReason::Error,
                        Some(error::TRANSPORT_FAILED.to_owned()),
                        Some(true),
                    );
                    self.refresh_active_subscription_count();
                    return Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                        ended,
                    )));
                }
            }
        }
        Ok(None)
    }

    fn allocate_subscription_id(&mut self) -> u64 {
        let id = self.next_subscription_id;
        self.next_subscription_id = self.next_subscription_id.saturating_add(1);
        id
    }

    fn refresh_active_subscription_count(&mut self) {
        self.status.active_subscriptions = self.active_subscriptions.len() as u64;
    }
}

fn consumer_end_event(
    active: &mut ActivePublishedSubscription,
) -> Result<Option<SubscribeEnd>, crate::SubscribeServeError> {
    active
        .subscription
        .as_mut()
        .expect("active subscription should hold a stream")
        .try_consumer_end()
}

fn spawn_source_task(
    subscription_id: u64,
    mut source: crate::PublishedByteSource,
    source_tx: mpsc::Sender<RuntimeSourceEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(chunk) = source.next().await {
            if source_tx
                .send(RuntimeSourceEvent::Chunk {
                    subscription_id,
                    chunk,
                })
                .await
                .is_err()
            {
                return;
            }
        }
        let _ = source_tx
            .send(RuntimeSourceEvent::Complete { subscription_id })
            .await;
    })
}
