//! SDK-owned serving runtime for inbound protocol traffic and published streams.

use crate::api::{
    AukiNode, AukiNodeError, AukiServedInbound, AukiServedSubscription, LifecycleInput,
};
use crate::{
    AukiSubscriptionBackpressurePolicy, LifecycleStreamDirection, LifecycleStreamGuardError,
    PublishedByteFrame,
};
use auki_protocol::v1::{
    error,
    subscribe::{SubscribeEnd, SubscribeEndReason},
};
use futures::StreamExt as _;
use libp2p::PeerId;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    task::JoinHandle,
    time::{Duration, sleep},
};

const SOURCE_READY_BUFFER: usize = 1024;
const CONSUMER_END_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Lightweight counters for the SDK serving loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AukiServeRuntimeStatus {
    /// Lifecycle handshakes served.
    pub lifecycles_served: u64,
    /// Duplicate inbound lifecycle streams ignored by the runtime.
    pub duplicate_lifecycle_attempts: u64,
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
    /// Runtime-managed subscriptions closed because their queue filled.
    pub subscriptions_closed_for_backpressure: u64,
    /// Runtime-managed subscriptions closed by local producer/runtime intent.
    pub subscriptions_closed_by_producer: u64,
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
    /// Queue/backpressure policy selected for this subscriber.
    pub backpressure_policy: AukiSubscriptionBackpressurePolicy,
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
    /// Queue/backpressure policy selected for this subscriber.
    pub backpressure_policy: AukiSubscriptionBackpressurePolicy,
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
    backpressure_policy: AukiSubscriptionBackpressurePolicy,
    source_queue: SharedSourceQueue,
    subscription: Option<AukiServedSubscription>,
    source_task: JoinHandle<()>,
}

#[derive(Clone)]
struct SharedSourceQueue {
    subscription_id: u64,
    state: Arc<Mutex<SourceQueueState>>,
    space_available: Arc<Notify>,
    ready_tx: mpsc::Sender<u64>,
}

#[derive(Default)]
struct SourceQueueState {
    queue: VecDeque<QueuedSourceEvent>,
    produced_frames: u64,
    dropped_frames: u64,
    closed: bool,
}

enum QueuedSourceEvent {
    Chunk(PublishedByteFrame),
    Complete,
    CloseForBackpressure,
}

enum RuntimePoll {
    Inbound(Result<Option<AukiServedInbound>, AukiNodeError>),
    SourceReady(Option<u64>),
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
            backpressure_policy: self.backpressure_policy,
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
            backpressure_policy: self.backpressure_policy,
        }
    }
}

impl SharedSourceQueue {
    fn new(subscription_id: u64, ready_tx: mpsc::Sender<u64>) -> Self {
        Self {
            subscription_id,
            state: Arc::new(Mutex::new(SourceQueueState::default())),
            space_available: Arc::new(Notify::new()),
            ready_tx,
        }
    }

    async fn push(
        &self,
        policy: AukiSubscriptionBackpressurePolicy,
        event: QueuedSourceEvent,
    ) -> bool {
        match policy {
            AukiSubscriptionBackpressurePolicy::LatestOnly => self.push_latest_only(event).await,
            AukiSubscriptionBackpressurePolicy::Bounded { capacity } => {
                self.push_bounded(capacity.max(1), event).await
            }
            AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity } => {
                self.push_close_on_full(capacity.max(1), event).await
            }
        }
    }

    async fn pop(&self) -> Option<QueuedSourceEvent> {
        let mut state = self.state.lock().await;
        let event = state.queue.pop_front();
        drop(state);
        if event.is_some() {
            self.space_available.notify_one();
        }
        event
    }

    async fn take_counters(&self) -> (u64, u64) {
        let mut state = self.state.lock().await;
        let counters = (state.produced_frames, state.dropped_frames);
        state.produced_frames = 0;
        state.dropped_frames = 0;
        counters
    }

    async fn push_latest_only(&self, event: QueuedSourceEvent) -> bool {
        let mut state = self.state.lock().await;
        if state.closed {
            return false;
        }

        match event {
            QueuedSourceEvent::Chunk(chunk) => {
                state.produced_frames = state.produced_frames.saturating_add(1);
                let old_len = state.queue.len() as u64;
                state.queue.clear();
                state.dropped_frames = state.dropped_frames.saturating_add(old_len);
                state.queue.push_back(QueuedSourceEvent::Chunk(chunk));
            }
            QueuedSourceEvent::Complete => {
                state.queue.push_back(QueuedSourceEvent::Complete);
                state.closed = true;
            }
            QueuedSourceEvent::CloseForBackpressure => {
                state.queue.clear();
                state
                    .queue
                    .push_back(QueuedSourceEvent::CloseForBackpressure);
                state.closed = true;
            }
        }
        drop(state);
        self.notify_ready();
        true
    }

    async fn push_bounded(&self, capacity: usize, event: QueuedSourceEvent) -> bool {
        let mut event = Some(event);
        loop {
            let mut state = self.state.lock().await;
            if state.closed {
                return false;
            }
            if state.queue.len() < capacity {
                let event = event
                    .take()
                    .expect("bounded source event should be present");
                if matches!(event, QueuedSourceEvent::Chunk(_)) {
                    state.produced_frames = state.produced_frames.saturating_add(1);
                }
                if matches!(
                    event,
                    QueuedSourceEvent::Complete | QueuedSourceEvent::CloseForBackpressure
                ) {
                    state.closed = true;
                }
                state.queue.push_back(event);
                drop(state);
                self.notify_ready();
                return true;
            }

            let notified = self.space_available.notified();
            drop(state);
            notified.await;
        }
    }

    async fn push_close_on_full(&self, capacity: usize, event: QueuedSourceEvent) -> bool {
        let mut state = self.state.lock().await;
        if state.closed {
            return false;
        }

        match event {
            QueuedSourceEvent::Chunk(chunk) => {
                state.produced_frames = state.produced_frames.saturating_add(1);
                if state.queue.len() >= capacity {
                    state.dropped_frames = state
                        .dropped_frames
                        .saturating_add(1_u64.saturating_add(state.queue.len() as u64));
                    state.queue.clear();
                    state
                        .queue
                        .push_back(QueuedSourceEvent::CloseForBackpressure);
                    state.closed = true;
                    drop(state);
                    self.notify_ready();
                    return false;
                }
                state.queue.push_back(QueuedSourceEvent::Chunk(chunk));
            }
            QueuedSourceEvent::Complete => {
                state.queue.push_back(QueuedSourceEvent::Complete);
                state.closed = true;
            }
            QueuedSourceEvent::CloseForBackpressure => {
                state.queue.clear();
                state
                    .queue
                    .push_back(QueuedSourceEvent::CloseForBackpressure);
                state.closed = true;
            }
        }
        drop(state);
        self.notify_ready();
        true
    }

    fn notify_ready(&self) {
        let _ = self.ready_tx.try_send(self.subscription_id);
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
    ready_tx: mpsc::Sender<u64>,
    ready_rx: mpsc::Receiver<u64>,
}

impl AukiServeRuntime {
    /// Create a serving runtime around an already configured node.
    pub fn new(node: AukiNode) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel(SOURCE_READY_BUFFER);
        Self {
            node,
            lifecycle_input: LifecycleInput::new(),
            status: AukiServeRuntimeStatus::default(),
            active_subscriptions: BTreeMap::new(),
            next_subscription_id: 0,
            ready_tx,
            ready_rx,
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

    /// End every active runtime-managed published subscription.
    ///
    /// This is intended for app/runtime shutdown and offer withdrawal paths. The
    /// runtime removes each subscription from the active set before attempting
    /// to send the terminal SubscribeEnd, so source tasks are stopped even if a
    /// transport is already closed.
    pub async fn shutdown_active_subscriptions(
        &mut self,
        reason: SubscribeEndReason,
    ) -> Result<Vec<AukiEndedSubscriptionStatus>, AukiNodeError> {
        let ids = self
            .active_subscriptions
            .keys()
            .copied()
            .collect::<Vec<_>>();
        let mut ended = Vec::new();
        let mut first_error = None;

        for id in ids {
            let Some(mut active) = self.active_subscriptions.remove(&id) else {
                continue;
            };
            let ended_status = active.ended_status(reason, None, None);
            if let Some(subscription) = active.subscription.take() {
                if let Err(error) = self
                    .node
                    .end_served_subscription(subscription, reason, None, None)
                    .await
                {
                    self.status.subscriptions_failed =
                        self.status.subscriptions_failed.saturating_add(1);
                    self.status.last_failure = Some(error.to_string());
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                } else {
                    self.record_subscription_end_reason(reason);
                }
            }
            ended.push(ended_status);
        }

        self.refresh_active_subscription_count();
        if let Some(error) = first_error {
            Err(error)
        } else {
            Ok(ended)
        }
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
            if let Some(event) = self.pop_queued_source_event(now).await? {
                return Ok(Some(event));
            }

            let lifecycle_input = self.lifecycle_input.clone();
            let has_active_subscriptions = !self.active_subscriptions.is_empty();
            let poll = {
                let node = &mut self.node;
                let ready_rx = &mut self.ready_rx;
                tokio::select! {
                    biased;
                    inbound = node.serve_next_inbound(lifecycle_input, now) => {
                        RuntimePoll::Inbound(inbound)
                    }
                    ready = ready_rx.recv(), if has_active_subscriptions => {
                        RuntimePoll::SourceReady(ready)
                    }
                    _ = sleep(CONSUMER_END_POLL_INTERVAL), if has_active_subscriptions => {
                        RuntimePoll::ConsumerEndTick
                    }
                }
            };

            match poll {
                RuntimePoll::Inbound(Ok(Some(served))) => return self.handle_inbound(served),
                RuntimePoll::Inbound(Ok(None)) => return Ok(None),
                RuntimePoll::Inbound(Err(error)) if is_duplicate_inbound_lifecycle(&error) => {
                    self.status.duplicate_lifecycle_attempts =
                        self.status.duplicate_lifecycle_attempts.saturating_add(1);
                }
                RuntimePoll::Inbound(Err(error)) => return Err(error),
                RuntimePoll::SourceReady(Some(_subscription_id)) => {
                    if let Some(event) = self.pop_queued_source_event(now).await? {
                        return Ok(Some(event));
                    }
                }
                RuntimePoll::SourceReady(None) => return Ok(None),
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
        let backpressure_policy = self
            .node
            .local_publication_backpressure_policy(&domain_id, &offer_id)
            .unwrap_or_default();

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
        let source_queue = SharedSourceQueue::new(id, self.ready_tx.clone());
        let source_task = spawn_source_task(source, source_queue.clone(), backpressure_policy);
        let active = ActivePublishedSubscription {
            id,
            peer_id,
            domain_id,
            offer_id,
            payload_type,
            messages_sent: 0,
            backpressure_policy,
            source_queue,
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

    async fn pop_queued_source_event(
        &mut self,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let ids = self
            .active_subscriptions
            .keys()
            .copied()
            .collect::<Vec<_>>();
        for subscription_id in ids {
            let Some(queue) = self
                .active_subscriptions
                .get(&subscription_id)
                .map(|active| active.source_queue.clone())
            else {
                continue;
            };
            let (produced, dropped) = queue.take_counters().await;
            self.status.frames_produced = self.status.frames_produced.saturating_add(produced);
            self.status.frames_dropped = self.status.frames_dropped.saturating_add(dropped);

            let Some(event) = queue.pop().await else {
                continue;
            };
            return self.handle_source_event(subscription_id, event, now).await;
        }
        Ok(None)
    }

    async fn handle_source_event(
        &mut self,
        subscription_id: u64,
        event: QueuedSourceEvent,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        match event {
            QueuedSourceEvent::Chunk(chunk) => {
                self.handle_source_chunk(subscription_id, chunk, now).await
            }
            QueuedSourceEvent::Complete => self.handle_source_complete(subscription_id).await,
            QueuedSourceEvent::CloseForBackpressure => {
                self.handle_source_backpressure_close(subscription_id).await
            }
        }
    }

    async fn handle_source_chunk(
        &mut self,
        subscription_id: u64,
        frame: PublishedByteFrame,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let Some(mut active) = self.active_subscriptions.remove(&subscription_id) else {
            return Ok(None);
        };

        match consumer_end_event(&mut active) {
            Ok(Some(event)) => {
                let ended = active.ended_status(
                    event.reason,
                    event.error.as_ref().map(|error| error.code.clone()),
                    event.retryable,
                );
                self.record_subscription_end_reason(event.reason);
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
                .next_publication_message(&active.domain_id, &active.offer_id, frame, now)?;
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
        self.record_subscription_end_reason(SubscribeEndReason::Complete);
        self.refresh_active_subscription_count();
        Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
            ended,
        )))
    }

    async fn handle_source_backpressure_close(
        &mut self,
        subscription_id: u64,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let Some(mut active) = self.active_subscriptions.remove(&subscription_id) else {
            return Ok(None);
        };
        let ended = active.ended_status(
            SubscribeEndReason::Error,
            Some(error::SUBSCRIBE_BACKPRESSURE.to_owned()),
            Some(true),
        );
        let subscription = active
            .subscription
            .take()
            .expect("active subscription should hold a stream");
        self.node
            .end_served_subscription(
                subscription,
                SubscribeEndReason::Error,
                Some(error::SUBSCRIBE_BACKPRESSURE.to_owned()),
                Some(true),
            )
            .await?;
        self.status.subscriptions_closed_for_backpressure = self
            .status
            .subscriptions_closed_for_backpressure
            .saturating_add(1);
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
                    self.record_subscription_end_reason(end.reason);
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

    fn record_subscription_end_reason(&mut self, reason: SubscribeEndReason) {
        match reason {
            SubscribeEndReason::Complete => {
                self.status.subscriptions_completed =
                    self.status.subscriptions_completed.saturating_add(1);
            }
            SubscribeEndReason::Cancelled => {
                self.status.subscriptions_cancelled =
                    self.status.subscriptions_cancelled.saturating_add(1);
            }
            SubscribeEndReason::Error => {
                self.status.subscriptions_failed =
                    self.status.subscriptions_failed.saturating_add(1);
            }
            SubscribeEndReason::OfferWithdrawn
            | SubscribeEndReason::NotAuthorized
            | SubscribeEndReason::ProducerShutdown => {
                self.status.subscriptions_closed_by_producer = self
                    .status
                    .subscriptions_closed_by_producer
                    .saturating_add(1);
            }
        }
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

fn is_duplicate_inbound_lifecycle(error: &AukiNodeError) -> bool {
    matches!(
        error,
        AukiNodeError::LifecycleGuard(LifecycleStreamGuardError {
            direction: LifecycleStreamDirection::Inbound,
            ..
        })
    )
}

fn spawn_source_task(
    mut source: crate::PublishedByteSource,
    source_queue: SharedSourceQueue,
    backpressure_policy: AukiSubscriptionBackpressurePolicy,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(chunk) = source.next().await {
            if !source_queue
                .push(backpressure_policy, QueuedSourceEvent::Chunk(chunk))
                .await
            {
                return;
            }
        }
        let _ = source_queue
            .push(backpressure_policy, QueuedSourceEvent::Complete)
            .await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_inbound_lifecycle_errors_are_runtime_nonfatal() {
        let peer_id = "12D3KooWAvnEo4RaYZtqt2w83qzmQ7WVW2HhN2cay95EXAiVKcar"
            .parse()
            .expect("peer id");
        let inbound = AukiNodeError::LifecycleGuard(LifecycleStreamGuardError {
            peer_id,
            direction: LifecycleStreamDirection::Inbound,
        });
        let outbound = AukiNodeError::LifecycleGuard(LifecycleStreamGuardError {
            peer_id,
            direction: LifecycleStreamDirection::Outbound,
        });

        assert!(is_duplicate_inbound_lifecycle(&inbound));
        assert!(!is_duplicate_inbound_lifecycle(&outbound));
    }

    #[tokio::test]
    async fn latest_only_keeps_newest_queued_chunk() {
        let (ready_tx, mut ready_rx) = mpsc::channel(4);
        let queue = SharedSourceQueue::new(7, ready_tx);

        assert!(
            queue
                .push(
                    AukiSubscriptionBackpressurePolicy::LatestOnly,
                    QueuedSourceEvent::Chunk(PublishedByteFrame::new(vec![1])),
                )
                .await
        );
        assert!(
            queue
                .push(
                    AukiSubscriptionBackpressurePolicy::LatestOnly,
                    QueuedSourceEvent::Chunk(PublishedByteFrame::new(vec![2])),
                )
                .await
        );

        assert_eq!(ready_rx.recv().await, Some(7));
        let (produced, dropped) = queue.take_counters().await;
        assert_eq!(produced, 2);
        assert_eq!(dropped, 1);
        match queue.pop().await {
            Some(QueuedSourceEvent::Chunk(chunk)) => assert_eq!(chunk.bytes, vec![2]),
            _ => panic!("expected newest queued chunk"),
        }
        assert!(queue.pop().await.is_none());
    }

    #[tokio::test]
    async fn close_on_full_reports_backpressure_close() {
        let (ready_tx, _ready_rx) = mpsc::channel(4);
        let queue = SharedSourceQueue::new(8, ready_tx);
        let policy = AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity: 1 };

        assert!(
            queue
                .push(
                    policy,
                    QueuedSourceEvent::Chunk(PublishedByteFrame::new(vec![1]))
                )
                .await
        );
        assert!(
            !queue
                .push(
                    policy,
                    QueuedSourceEvent::Chunk(PublishedByteFrame::new(vec![2]))
                )
                .await
        );

        let (produced, dropped) = queue.take_counters().await;
        assert_eq!(produced, 2);
        assert_eq!(dropped, 2);
        assert!(matches!(
            queue.pop().await,
            Some(QueuedSourceEvent::CloseForBackpressure)
        ));
        assert!(queue.pop().await.is_none());
    }
}
