//! SDK-owned serving runtime for inbound protocol traffic and published streams.

use crate::api::{
    AukiNode, AukiNodeError, AukiServedInbound, AukiServedSubscription, LifecycleInput,
};
use crate::{
    AukiSubscriptionBackpressurePolicy, LifecycleStreamDirection, LifecycleStreamGuardError,
    PublishedByteFrame,
};
use auki_protocol::v1::{error, message::SpatialMessage, subscribe::SubscribeEndReason};
use futures::StreamExt as _;
use libp2p::PeerId;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Instant,
};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    task::JoinHandle,
    time::{Duration, sleep},
};

const SOURCE_READY_BUFFER: usize = 1024;
const WRITER_EVENT_BUFFER: usize = 1024;
const SUBSCRIPTION_WRITE_SLOW_THRESHOLD: Duration = Duration::from_millis(50);
const CONSUMER_END_POLL_INTERVAL: Duration = Duration::from_millis(10);
const WRITER_CONSUMER_END_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    /// Raw lifecycle streams accepted below the protocol parser.
    pub raw_inbound_lifecycles: u64,
    /// Raw offer-catalog streams accepted below the protocol parser.
    pub raw_inbound_offer_catalogs: u64,
    /// Raw Get streams accepted below the protocol parser.
    pub raw_inbound_gets: u64,
    /// Raw Subscribe streams accepted below the protocol parser.
    pub raw_inbound_subscribes: u64,
    /// Raw accept handoff queue full count.
    pub inbound_accept_queue_full: u64,
    /// Raw accept handoff queue closed count.
    pub inbound_accept_queue_closed: u64,
    /// Streams queued inside the SDK runtime but not yet served.
    pub pending_inbound_streams: usize,
    /// Streams in the raw accept handoff queue.
    pub inbound_accept_queue_depth: usize,
    /// Subscription writes taking longer than the slow-write threshold.
    pub subscription_slow_writes: u64,
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
    source_task: JoinHandle<()>,
    writer_tx: mpsc::Sender<SubscriptionWriterCommand>,
    closing: bool,
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
    InboundAvailable(Result<bool, AukiNodeError>),
    SourceReady(Option<u64>),
    WriterEvent(Option<SubscriptionWriterEvent>),
    ConsumerEndTick,
}

enum SubscriptionWriterCommand {
    Data(SpatialMessage),
    End {
        reason: SubscribeEndReason,
        error_code: Option<String>,
        retryable: Option<bool>,
    },
}

enum SubscriptionWriterEvent {
    MessageSent {
        subscription_id: u64,
        write_duration: Duration,
    },
    Ended {
        subscription_id: u64,
        reason: SubscribeEndReason,
        error_code: Option<String>,
        retryable: Option<bool>,
        failure: Option<String>,
    },
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
    writer_event_tx: mpsc::Sender<SubscriptionWriterEvent>,
    writer_event_rx: mpsc::Receiver<SubscriptionWriterEvent>,
}

impl AukiServeRuntime {
    /// Create a serving runtime around an already configured node.
    pub fn new(node: AukiNode) -> Self {
        let (ready_tx, ready_rx) = mpsc::channel(SOURCE_READY_BUFFER);
        let (writer_event_tx, writer_event_rx) = mpsc::channel(WRITER_EVENT_BUFFER);
        Self {
            node,
            lifecycle_input: LifecycleInput::new(),
            status: AukiServeRuntimeStatus::default(),
            active_subscriptions: BTreeMap::new(),
            next_subscription_id: 0,
            ready_tx,
            ready_rx,
            writer_event_tx,
            writer_event_rx,
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

        for id in ids {
            let Some(active) = self.active_subscriptions.remove(&id) else {
                continue;
            };
            let ended_status = active.ended_status(reason, None, None);
            if active
                .writer_tx
                .send(SubscriptionWriterCommand::End {
                    reason,
                    error_code: None,
                    retryable: None,
                })
                .await
                .is_err()
            {
                self.status.subscriptions_failed =
                    self.status.subscriptions_failed.saturating_add(1);
                self.status.last_failure = Some("subscription writer is closed".to_owned());
            } else {
                self.record_subscription_end_reason(reason);
            }
            ended.push(ended_status);
        }

        self.refresh_active_subscription_count();
        Ok(ended)
    }

    /// Serve one runtime event without fixed per-protocol timeout sequencing.
    pub async fn serve_next(
        &mut self,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        loop {
            self.refresh_node_diagnostics();
            if self.node.has_pending_inbound_streams() {
                if let Some(event) = self.serve_available_inbound(now).await? {
                    return Ok(Some(event));
                }
            }
            if let Some(event) = self.poll_writer_event()? {
                return Ok(Some(event));
            }
            if let Some(event) = self.pop_queued_source_event(now).await? {
                return Ok(Some(event));
            }

            let has_active_subscriptions = !self.active_subscriptions.is_empty();
            let poll = {
                let node = &mut self.node;
                let ready_rx = &mut self.ready_rx;
                let writer_event_rx = &mut self.writer_event_rx;
                tokio::select! {
                    biased;
                    inbound = node.wait_for_inbound_stream() => {
                        RuntimePoll::InboundAvailable(inbound)
                    }
                    writer_event = writer_event_rx.recv() => {
                        RuntimePoll::WriterEvent(writer_event)
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
                RuntimePoll::InboundAvailable(Ok(true)) => {
                    if let Some(event) = self.serve_available_inbound(now).await? {
                        return Ok(Some(event));
                    }
                }
                RuntimePoll::InboundAvailable(Ok(false)) => return Ok(None),
                RuntimePoll::InboundAvailable(Err(error)) => return Err(error),
                RuntimePoll::WriterEvent(Some(event)) => {
                    if let Some(event) = self.handle_writer_event(event)? {
                        return Ok(Some(event));
                    }
                }
                RuntimePoll::WriterEvent(None) => return Ok(None),
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

    async fn serve_available_inbound(
        &mut self,
        now: &str,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let lifecycle_input = self.lifecycle_input.clone();
        match self.node.serve_next_inbound(lifecycle_input, now).await {
            Ok(Some(served)) => self.handle_inbound(served),
            Ok(None) => Ok(None),
            Err(error) if is_duplicate_inbound_lifecycle(&error) => {
                self.status.duplicate_lifecycle_attempts =
                    self.status.duplicate_lifecycle_attempts.saturating_add(1);
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    fn handle_inbound(
        &mut self,
        served: AukiServedInbound,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.refresh_node_diagnostics();
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
        let (writer_tx, writer_rx) =
            mpsc::channel(subscription_writer_command_capacity(backpressure_policy));
        std::mem::drop(spawn_subscription_writer_task(
            id,
            subscription,
            writer_rx,
            self.writer_event_tx.clone(),
            self.node.subscribe_message_frame_body_bytes(),
        ));
        let active = ActivePublishedSubscription {
            id,
            peer_id,
            domain_id,
            offer_id,
            payload_type,
            messages_sent: 0,
            backpressure_policy,
            source_queue,
            source_task,
            writer_tx,
            closing: false,
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
        let Some((domain_id, offer_id, writer_tx, backpressure_policy, closing)) = self
            .active_subscriptions
            .get(&subscription_id)
            .map(|active| {
                (
                    active.domain_id.clone(),
                    active.offer_id.clone(),
                    active.writer_tx.clone(),
                    active.backpressure_policy,
                    active.closing,
                )
            })
        else {
            return Ok(None);
        };
        if closing {
            self.status.frames_dropped = self.status.frames_dropped.saturating_add(1);
            return Ok(None);
        }

        let message = self
            .node
            .next_publication_message(&domain_id, &offer_id, frame, now)?;
        match writer_tx.try_send(SubscriptionWriterCommand::Data(message)) {
            Ok(()) => Ok(None),
            Err(mpsc::error::TrySendError::Full(_command)) => match backpressure_policy {
                AukiSubscriptionBackpressurePolicy::LatestOnly => {
                    self.status.frames_dropped = self.status.frames_dropped.saturating_add(1);
                    Ok(None)
                }
                AukiSubscriptionBackpressurePolicy::Bounded { .. }
                | AukiSubscriptionBackpressurePolicy::CloseOnFull { .. } => {
                    self.close_subscription_for_backpressure(subscription_id)
                }
            },
            Err(mpsc::error::TrySendError::Closed(_command)) => self
                .end_subscription_for_writer_failure(
                    subscription_id,
                    "subscription writer is closed".to_owned(),
                ),
        }
    }

    async fn handle_source_complete(
        &mut self,
        subscription_id: u64,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.send_subscription_end_command(
            subscription_id,
            SubscribeEndReason::Complete,
            None,
            None,
        )
    }

    async fn handle_source_backpressure_close(
        &mut self,
        subscription_id: u64,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.close_subscription_for_backpressure(subscription_id)
    }

    fn poll_writer_event(&mut self) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        match self.writer_event_rx.try_recv() {
            Ok(event) => self.handle_writer_event(event),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Ok(None),
        }
    }

    fn handle_writer_event(
        &mut self,
        event: SubscriptionWriterEvent,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        match event {
            SubscriptionWriterEvent::MessageSent {
                subscription_id,
                write_duration,
            } => {
                let Some(active) = self.active_subscriptions.get_mut(&subscription_id) else {
                    return Ok(None);
                };
                active.messages_sent = active.messages_sent.saturating_add(1);
                self.status.frames_sent = self.status.frames_sent.saturating_add(1);
                if write_duration >= SUBSCRIPTION_WRITE_SLOW_THRESHOLD {
                    self.status.subscription_slow_writes =
                        self.status.subscription_slow_writes.saturating_add(1);
                }
                Ok(Some(
                    AukiServeRuntimeEvent::PublishedSubscriptionMessageSent(active.status()),
                ))
            }
            SubscriptionWriterEvent::Ended {
                subscription_id,
                reason,
                error_code,
                retryable,
                failure,
            } => {
                let Some(active) = self.active_subscriptions.remove(&subscription_id) else {
                    return Ok(None);
                };
                if let Some(failure) = failure {
                    self.status.subscriptions_failed =
                        self.status.subscriptions_failed.saturating_add(1);
                    self.status.last_failure = Some(failure);
                } else {
                    self.record_subscription_end_reason(reason);
                }
                let ended = active.ended_status(reason, error_code, retryable);
                self.refresh_active_subscription_count();
                Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
                    ended,
                )))
            }
        }
    }

    fn send_subscription_end_command(
        &mut self,
        subscription_id: u64,
        reason: SubscribeEndReason,
        error_code: Option<String>,
        retryable: Option<bool>,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let Some(writer_tx) = self
            .active_subscriptions
            .get_mut(&subscription_id)
            .and_then(|active| {
                if active.closing {
                    return None;
                }
                active.closing = true;
                Some(active.writer_tx.clone())
            })
        else {
            return Ok(None);
        };
        match writer_tx.try_send(SubscriptionWriterCommand::End {
            reason,
            error_code: error_code.clone(),
            retryable,
        }) {
            Ok(()) => Ok(None),
            Err(mpsc::error::TrySendError::Full(_command)) => self
                .end_subscription_for_writer_failure(
                    subscription_id,
                    "subscription writer command queue is full".to_owned(),
                ),
            Err(mpsc::error::TrySendError::Closed(_command)) => self
                .end_subscription_for_writer_failure(
                    subscription_id,
                    "subscription writer is closed".to_owned(),
                ),
        }
    }

    fn close_subscription_for_backpressure(
        &mut self,
        subscription_id: u64,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        self.status.subscriptions_closed_for_backpressure = self
            .status
            .subscriptions_closed_for_backpressure
            .saturating_add(1);
        self.send_subscription_end_command(
            subscription_id,
            SubscribeEndReason::Error,
            Some(error::SUBSCRIBE_BACKPRESSURE.to_owned()),
            Some(true),
        )
    }

    fn end_subscription_for_writer_failure(
        &mut self,
        subscription_id: u64,
        failure: String,
    ) -> Result<Option<AukiServeRuntimeEvent>, AukiNodeError> {
        let Some(active) = self.active_subscriptions.remove(&subscription_id) else {
            return Ok(None);
        };
        self.status.subscriptions_failed = self.status.subscriptions_failed.saturating_add(1);
        self.status.last_failure = Some(failure);
        let ended = active.ended_status(
            SubscribeEndReason::Error,
            Some(error::TRANSPORT_FAILED.to_owned()),
            Some(true),
        );
        self.refresh_active_subscription_count();
        Ok(Some(AukiServeRuntimeEvent::PublishedSubscriptionEnded(
            ended,
        )))
    }

    fn refresh_node_diagnostics(&mut self) {
        let diagnostics = self.node.inbound_accept_diagnostics();
        self.status.raw_inbound_lifecycles = diagnostics.raw_lifecycles;
        self.status.raw_inbound_offer_catalogs = diagnostics.raw_offer_catalogs;
        self.status.raw_inbound_gets = diagnostics.raw_gets;
        self.status.raw_inbound_subscribes = diagnostics.raw_subscribes;
        self.status.inbound_accept_queue_full = diagnostics.accept_queue_full;
        self.status.inbound_accept_queue_closed = diagnostics.accept_queue_closed;
        self.status.pending_inbound_streams = diagnostics.pending_inbound_streams;
        self.status.inbound_accept_queue_depth = diagnostics.inbound_accept_queue_depth;
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

fn subscription_writer_command_capacity(policy: AukiSubscriptionBackpressurePolicy) -> usize {
    match policy {
        AukiSubscriptionBackpressurePolicy::LatestOnly => 1,
        AukiSubscriptionBackpressurePolicy::Bounded { capacity }
        | AukiSubscriptionBackpressurePolicy::CloseOnFull { capacity } => capacity.max(1),
    }
}

fn spawn_subscription_writer_task(
    subscription_id: u64,
    mut subscription: AukiServedSubscription,
    mut command_rx: mpsc::Receiver<SubscriptionWriterCommand>,
    event_tx: mpsc::Sender<SubscriptionWriterEvent>,
    max_body_len: u64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut consumer_tick = tokio::time::interval(WRITER_CONSUMER_END_POLL_INTERVAL);
        consumer_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                biased;
                _ = consumer_tick.tick() => {
                    if emit_consumer_end_if_ready(subscription_id, &mut subscription, &event_tx).await {
                        return;
                    }
                }
                command = command_rx.recv() => {
                    let Some(command) = command else {
                        return;
                    };
                    if emit_consumer_end_if_ready(subscription_id, &mut subscription, &event_tx).await {
                        return;
                    }
                    match command {
                        SubscriptionWriterCommand::Data(message) => {
                            let frame = match subscription.encode_data_frame(&message, max_body_len) {
                                Ok(frame) => frame,
                                Err(error) => {
                                    emit_writer_failure(subscription_id, error.to_string(), &event_tx).await;
                                    return;
                                }
                            };
                            let started = Instant::now();
                            match subscription.write_encoded_data_frame(&frame).await {
                                Ok(()) => {
                                    let _ = event_tx
                                        .send(SubscriptionWriterEvent::MessageSent {
                                            subscription_id,
                                            write_duration: started.elapsed(),
                                        })
                                        .await;
                                }
                                Err(error) => {
                                    emit_writer_failure(subscription_id, error.to_string(), &event_tx).await;
                                    return;
                                }
                            }
                        }
                        SubscriptionWriterCommand::End {
                            reason,
                            error_code,
                            retryable,
                        } => {
                            let failure = subscription
                                .write_end_frame(reason, error_code.clone(), retryable, max_body_len)
                                .await
                                .err()
                                .map(|error| error.to_string());
                            let (reason, error_code, retryable) = if failure.is_some() {
                                (
                                    SubscribeEndReason::Error,
                                    Some(error::TRANSPORT_FAILED.to_owned()),
                                    Some(true),
                                )
                            } else {
                                (reason, error_code, retryable)
                            };
                            let _ = event_tx
                                .send(SubscriptionWriterEvent::Ended {
                                    subscription_id,
                                    reason,
                                    error_code,
                                    retryable,
                                    failure,
                                })
                                .await;
                            return;
                        }
                    }
                }
            }
        }
    })
}

async fn emit_consumer_end_if_ready(
    subscription_id: u64,
    subscription: &mut AukiServedSubscription,
    event_tx: &mpsc::Sender<SubscriptionWriterEvent>,
) -> bool {
    match subscription.try_consumer_end() {
        Ok(Some(end)) => {
            let _ = event_tx
                .send(SubscriptionWriterEvent::Ended {
                    subscription_id,
                    reason: end.reason,
                    error_code: end.error.map(|error| error.code),
                    retryable: end.retryable,
                    failure: None,
                })
                .await;
            true
        }
        Ok(None) => false,
        Err(error) => {
            emit_writer_failure(subscription_id, error.to_string(), event_tx).await;
            true
        }
    }
}

async fn emit_writer_failure(
    subscription_id: u64,
    failure: String,
    event_tx: &mpsc::Sender<SubscriptionWriterEvent>,
) {
    let _ = event_tx
        .send(SubscriptionWriterEvent::Ended {
            subscription_id,
            reason: SubscribeEndReason::Error,
            error_code: Some(error::TRANSPORT_FAILED.to_owned()),
            retryable: Some(true),
            failure: Some(failure),
        })
        .await;
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
