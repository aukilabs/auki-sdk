use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use auki_p2p::{RouteCatalog, RouteCatalogLimits};
use parking_lot::Mutex;
use tokio::sync::{Notify, Semaphore};

use crate::relay::{
    CreateRelayBookingResponse, RelayBookingCreateDisposition, RelayLimits, RelaySlotSnapshot,
};

use super::*;

type ApiResult<T> = Result<T, RelayBookingClientError>;

fn principal_http_error(
    status: reqwest::StatusCode,
    code: RelayErrorCode,
) -> RelayBookingClientError {
    RelayBookingClientError::Http {
        operation: RelayOperation::Active,
        status,
        code,
        retry_after: None,
        location: None,
    }
}

#[test]
fn requester_principal_errors_have_expected_startup_and_fencing_semantics() {
    let fallback = Duration::from_secs(7);
    for code in [
        RelayErrorCode::StaleRobotPrincipal,
        RelayErrorCode::StaleRequesterPrincipal,
    ] {
        let error = principal_http_error(reqwest::StatusCode::CONFLICT, code);
        assert!(control_error_ends_authority(&error));
        assert_eq!(
            RelayCoordinatorError::Dms(error).startup_retry_after(fallback),
            Some(fallback)
        );
    }

    for code in [
        RelayErrorCode::InvalidRobotPrincipal,
        RelayErrorCode::InvalidRequesterPrincipal,
    ] {
        let error = principal_http_error(reqwest::StatusCode::FORBIDDEN, code);
        assert!(control_error_ends_authority(&error));
        assert_eq!(
            RelayCoordinatorError::Dms(error).startup_retry_after(fallback),
            None
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ApiCall {
    Active,
    Create {
        key: RelayIdempotencyKey,
        request: CreateRelayBookingRequest,
    },
    Renew(Uuid),
    ReservationFailed {
        booking_id: Uuid,
        request: ReservationFailedRequest,
    },
    Delete(Uuid),
}

#[derive(Default)]
struct ScriptedApi {
    calls: Mutex<Vec<ApiCall>>,
    active: Mutex<VecDeque<ApiResult<Option<RelayBookingSnapshot>>>>,
    create: Mutex<VecDeque<ApiResult<CreateRelayBookingResponse>>>,
    renew: Mutex<VecDeque<ApiResult<RelayBookingSnapshot>>>,
    reservation_failed: Mutex<VecDeque<ApiResult<RelayBookingSnapshot>>>,
}

impl ScriptedApi {
    fn push_active(&self, response: ApiResult<Option<RelayBookingSnapshot>>) {
        self.active.lock().push_back(response);
    }

    fn push_create(&self, response: ApiResult<CreateRelayBookingResponse>) {
        self.create.lock().push_back(response);
    }

    fn push_renew(&self, response: ApiResult<RelayBookingSnapshot>) {
        self.renew.lock().push_back(response);
    }

    fn push_reservation_failed(&self, response: ApiResult<RelayBookingSnapshot>) {
        self.reservation_failed.lock().push_back(response);
    }

    fn calls(&self) -> Vec<ApiCall> {
        self.calls.lock().clone()
    }
}

fn take_scripted<T>(
    responses: &Mutex<VecDeque<ApiResult<T>>>,
    operation: &'static str,
) -> ApiResult<T> {
    responses
        .lock()
        .pop_front()
        .unwrap_or_else(|| panic!("missing scripted {operation} response"))
}

#[async_trait]
impl RelayBookingApi for ScriptedApi {
    async fn active(&self) -> ApiResult<Option<RelayBookingSnapshot>> {
        self.calls.lock().push(ApiCall::Active);
        take_scripted(&self.active, "active")
    }

    async fn create(
        &self,
        idempotency_key: &RelayIdempotencyKey,
        request: &CreateRelayBookingRequest,
    ) -> ApiResult<CreateRelayBookingResponse> {
        self.calls.lock().push(ApiCall::Create {
            key: idempotency_key.clone(),
            request: request.clone(),
        });
        take_scripted(&self.create, "create")
    }

    async fn renew(&self, booking_id: Uuid) -> ApiResult<RelayBookingSnapshot> {
        self.calls.lock().push(ApiCall::Renew(booking_id));
        take_scripted(&self.renew, "renew")
    }

    async fn report_reservation_failed(
        &self,
        booking_id: Uuid,
        request: &ReservationFailedRequest,
    ) -> ApiResult<RelayBookingSnapshot> {
        self.calls.lock().push(ApiCall::ReservationFailed {
            booking_id,
            request: request.clone(),
        });
        take_scripted(&self.reservation_failed, "reservation-failed")
    }

    async fn delete(&self, booking_id: Uuid) -> ApiResult<()> {
        self.calls.lock().push(ApiCall::Delete(booking_id));
        Ok(())
    }
}

struct BlockingActiveApi {
    snapshot: RelayBookingSnapshot,
    active_calls: AtomicUsize,
    active_started: Notify,
    active_release: Semaphore,
    deletes: AtomicUsize,
}

impl BlockingActiveApi {
    fn new(snapshot: RelayBookingSnapshot) -> Self {
        Self {
            snapshot,
            active_calls: AtomicUsize::new(0),
            active_started: Notify::new(),
            active_release: Semaphore::new(0),
            deletes: AtomicUsize::new(0),
        }
    }

    async fn wait_until_blocked(&self) {
        tokio::time::timeout(Duration::from_secs(1), self.active_started.notified())
            .await
            .expect("the actor entered its blocked active lookup");
    }

    fn release(&self) {
        self.active_release.add_permits(1);
    }
}

#[async_trait]
impl RelayBookingApi for BlockingActiveApi {
    async fn active(&self) -> ApiResult<Option<RelayBookingSnapshot>> {
        let call = self.active_calls.fetch_add(1, Ordering::SeqCst);
        if call != 0 {
            self.active_started.notify_one();
            self.active_release
                .acquire()
                .await
                .expect("test active release semaphore remains open")
                .forget();
        }
        Ok(Some(self.snapshot.clone()))
    }

    async fn create(
        &self,
        _idempotency_key: &RelayIdempotencyKey,
        _request: &CreateRelayBookingRequest,
    ) -> ApiResult<CreateRelayBookingResponse> {
        panic!("the blocking API starts from an active booking")
    }

    async fn renew(&self, _booking_id: Uuid) -> ApiResult<RelayBookingSnapshot> {
        panic!("the blocking API test must not reach renewal")
    }

    async fn report_reservation_failed(
        &self,
        _booking_id: Uuid,
        _request: &ReservationFailedRequest,
    ) -> ApiResult<RelayBookingSnapshot> {
        panic!("the blocking API test must not report a reservation failure")
    }

    async fn delete(&self, _booking_id: Uuid) -> ApiResult<()> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct PendingStartDrop<'a>(&'a AtomicUsize);

impl Drop for PendingStartDrop<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

struct PendingStartBackend {
    relay_events: broadcast::Sender<RelayTransportEvent>,
    starts: AtomicUsize,
    dropped_starts: AtomicUsize,
    cancellations: AtomicUsize,
}

impl PendingStartBackend {
    fn new() -> Self {
        let (relay_events, _) = broadcast::channel(8);
        Self {
            relay_events,
            starts: AtomicUsize::new(0),
            dropped_starts: AtomicUsize::new(0),
            cancellations: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl RelayReservationBackend for PendingStartBackend {
    async fn start(
        &self,
        _provider: RelayProvider,
    ) -> Result<RelayReservationHandle, ReservationAttemptFailure> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let _drop = PendingStartDrop(&self.dropped_starts);
        std::future::pending().await
    }

    async fn wait(
        &self,
        _handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, ReservationAttemptFailure> {
        panic!("pending-start backend cannot reach wait")
    }

    async fn cancel(&self, _handle: RelayReservationHandle) -> Result<(), DomainRelayError> {
        self.cancellations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, DomainRelayError> {
        Ok(self.relay_events.subscribe())
    }
}

struct StoppedStartBackend {
    relay_events: broadcast::Sender<RelayTransportEvent>,
}

impl StoppedStartBackend {
    fn new() -> Self {
        let (relay_events, _) = broadcast::channel(8);
        Self { relay_events }
    }
}

#[async_trait]
impl RelayReservationBackend for StoppedStartBackend {
    async fn start(
        &self,
        _provider: RelayProvider,
    ) -> Result<RelayReservationHandle, ReservationAttemptFailure> {
        Err(ReservationAttemptFailure::BackendStopped { handle: None })
    }

    async fn wait(
        &self,
        _handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, ReservationAttemptFailure> {
        panic!("stopped-start backend cannot reach wait")
    }

    async fn cancel(&self, _handle: RelayReservationHandle) -> Result<(), DomainRelayError> {
        Err(DomainRelayError::Stopped)
    }

    fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, DomainRelayError> {
        Ok(self.relay_events.subscribe())
    }
}

struct StoppedSubscriptionBackend;

#[async_trait]
impl RelayReservationBackend for StoppedSubscriptionBackend {
    async fn start(
        &self,
        _provider: RelayProvider,
    ) -> Result<RelayReservationHandle, ReservationAttemptFailure> {
        panic!("stopped subscription must prevent reservation start")
    }

    async fn wait(
        &self,
        _handle: RelayReservationHandle,
    ) -> Result<RelayReservationSnapshot, ReservationAttemptFailure> {
        panic!("stopped subscription must prevent reservation wait")
    }

    async fn cancel(&self, _handle: RelayReservationHandle) -> Result<(), DomainRelayError> {
        panic!("stopped subscription must prevent reservation cancellation")
    }

    fn subscribe(&self) -> Result<broadcast::Receiver<RelayTransportEvent>, DomainRelayError> {
        Err(DomainRelayError::Stopped)
    }
}

#[derive(Default)]
struct RecordingRoutes {
    publications: AtomicUsize,
    tombstones: Mutex<Vec<LocalRelayFence>>,
    fence_all_calls: AtomicUsize,
    fail_tombstones: AtomicBool,
}

#[async_trait]
impl RelayRouteRegistry for RecordingRoutes {
    async fn publish(&self, route: PublishedRelayRoute) -> Result<(), String> {
        self.publications.fetch_add(1, Ordering::SeqCst);
        let _ = route;
        Ok(())
    }

    async fn refresh_authority(
        &self,
        _fence: LocalRelayFence,
        _authorized_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool, String> {
        Ok(true)
    }

    async fn tombstone(&self, fence: LocalRelayFence) -> Result<bool, String> {
        if self.fail_tombstones.load(Ordering::SeqCst) {
            return Err("injected route tombstone failure".to_string());
        }
        self.tombstones.lock().push(fence);
        Ok(true)
    }

    fn fence_all(&self) -> Result<(), String> {
        self.fence_all_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn coordinator_config(idempotency_key: &str) -> RelayCoordinatorConfig {
    RelayCoordinatorConfig {
        idempotency_key: RelayIdempotencyKey::new(idempotency_key).expect("valid test key"),
        mode: RelayBookingMode::Public,
        requested_duration_seconds: 900,
        relay_count: 1,
        status_poll_interval: Duration::from_secs(60),
        reservation_retry_budget: Duration::from_secs(30),
        retry_min: Duration::from_millis(5),
        retry_max: Duration::from_millis(5),
        http_timeout: Duration::from_millis(100),
        authority_safety_margin: Duration::from_secs(15),
    }
}

fn queued_snapshot(booking_id: Uuid) -> RelayBookingSnapshot {
    let now = chrono::Utc::now();
    RelayBookingSnapshot {
        booking_id,
        mode: RelayBookingMode::Public,
        state: RelayBookingState::Active,
        relay_count: 1,
        requested_duration_seconds: 900,
        requested_until: now + chrono::Duration::seconds(900),
        authority_expires_at: now + chrono::Duration::minutes(5),
        assigned_count: 0,
        provider_ready_count: 0,
        unfilled_count: 1,
        created_at: now - chrono::Duration::seconds(1),
        ended_at: None,
        slots: vec![RelaySlotSnapshot {
            slot_id: Uuid::new_v4(),
            slot_index: 0,
            state: RelaySlotState::Queued,
            assignment_id: None,
            reservation_epoch: None,
            provider_peer_id: None,
            provider_base_addresses: None,
            limits: None,
            provider_lease_expires_at: None,
            recovery_expires_at: None,
        }],
    }
}

fn ready_snapshot(
    booking_id: Uuid,
    slot_id: Uuid,
    assignment_id: Uuid,
    reservation_epoch: Uuid,
    relay_peer_id: auki_p2p::PeerId,
) -> RelayBookingSnapshot {
    let now = chrono::Utc::now();
    RelayBookingSnapshot {
        booking_id,
        mode: RelayBookingMode::Public,
        state: RelayBookingState::Active,
        relay_count: 1,
        requested_duration_seconds: 900,
        requested_until: now + chrono::Duration::seconds(900),
        authority_expires_at: now + chrono::Duration::minutes(5),
        assigned_count: 1,
        provider_ready_count: 1,
        unfilled_count: 0,
        created_at: now - chrono::Duration::seconds(1),
        ended_at: None,
        slots: vec![RelaySlotSnapshot {
            slot_id,
            slot_index: 0,
            state: RelaySlotState::Ready,
            assignment_id: Some(assignment_id),
            reservation_epoch: Some(reservation_epoch),
            provider_peer_id: Some(relay_peer_id.to_string()),
            provider_base_addresses: Some(vec![format!(
                "/dns4/relay-a.dev.aukiverse.com/tcp/443/p2p/{relay_peer_id}"
            )]),
            limits: Some(RelayLimits {
                duration_seconds: 900,
                data_bytes_per_direction: 1_048_576,
            }),
            provider_lease_expires_at: Some(now + chrono::Duration::minutes(4)),
            recovery_expires_at: None,
        }],
    }
}

fn two_ready_snapshot(
    booking_id: Uuid,
    first: (Uuid, Uuid, Uuid, PeerId),
    second: (Uuid, Uuid, Uuid, PeerId),
) -> RelayBookingSnapshot {
    let mut snapshot = ready_snapshot(booking_id, first.0, first.1, first.2, first.3);
    let mut second_slot = ready_snapshot(booking_id, second.0, second.1, second.2, second.3)
        .slots
        .pop()
        .expect("ready snapshot has one slot");
    second_slot.slot_index = 1;
    snapshot.relay_count = 2;
    snapshot.assigned_count = 2;
    snapshot.provider_ready_count = 2;
    snapshot.slots.push(second_slot);
    snapshot
}

fn replayed_create(snapshot: RelayBookingSnapshot) -> CreateRelayBookingResponse {
    CreateRelayBookingResponse {
        disposition: RelayBookingCreateDisposition::Replayed,
        location: format!("/relay-bookings/{}", snapshot.booking_id),
        snapshot,
    }
}

fn local_route_catalog(local_peer_id: PeerId) -> RouteCatalog {
    RouteCatalog::new(local_peer_id, Vec::new(), RouteCatalogLimits::new(3, 3))
        .expect("local route catalog")
}

fn confirmed_local_route(
    fence: LocalRelayFence,
    local_peer_id: PeerId,
    relay_peer_id: PeerId,
) -> PublishedRelayRoute {
    let base = format!(
        "/dns4/relay-{}.dev.aukiverse.com/tcp/443/p2p/{relay_peer_id}",
        fence.local_generation
    );
    PublishedRelayRoute {
        fence,
        route: format!("{base}/p2p-circuit/p2p/{local_peer_id}")
            .parse()
            .expect("canonical circuit route"),
        limits: ExpectedRelayLimits::new(Duration::from_secs(900), 1_048_576)
            .expect("finite relay limits"),
        authorized_until: chrono::Utc::now() + chrono::Duration::minutes(4),
        relay_peer_id,
    }
}

#[tokio::test]
async fn fenced_catalog_preserves_direct_routes_and_permanently_rejects_publication() {
    let local_peer_id = PeerId::random();
    let relay_peer_id = PeerId::random();
    let direct_route = format!("/ip4/127.0.0.1/tcp/4001/p2p/{local_peer_id}")
        .parse()
        .expect("valid direct route");
    let catalog = RouteCatalog::new(
        local_peer_id,
        vec![direct_route],
        RouteCatalogLimits::new(3, 2),
    )
    .expect("route catalog");
    let routes = FencedRouteCatalog::new(catalog.clone());
    let fence = LocalRelayFence {
        slot_id: Uuid::new_v4(),
        assignment_id: Uuid::new_v4(),
        reservation_epoch: Uuid::new_v4(),
        local_generation: next_local_relay_generation().expect("available generation"),
    };

    routes
        .publish(confirmed_local_route(fence, local_peer_id, relay_peer_id))
        .await
        .expect("owned relay route publishes");
    routes.fence_all().expect("owned relay route is fenced");

    let snapshot = catalog.snapshot().expect("route snapshot");
    assert_eq!(snapshot.direct_routes.len(), 1);
    assert!(snapshot.relay_routes.is_empty());
    assert!(
        routes
            .publish(confirmed_local_route(fence, local_peer_id, relay_peer_id))
            .await
            .is_err(),
        "a closed coordinator must never publish again"
    );
}

#[tokio::test]
async fn stale_coordinator_fence_cannot_remove_replacement_route() {
    let local_peer_id = PeerId::random();
    let relay_peer_id = PeerId::random();
    let catalog = local_route_catalog(local_peer_id);
    let old_routes = FencedRouteCatalog::new(catalog.clone());
    let replacement_routes = FencedRouteCatalog::new(catalog.clone());
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let old_fence = LocalRelayFence {
        slot_id,
        assignment_id,
        reservation_epoch,
        local_generation: next_local_relay_generation().expect("available generation"),
    };
    let replacement_fence = LocalRelayFence {
        slot_id,
        assignment_id,
        reservation_epoch,
        local_generation: next_local_relay_generation().expect("available generation"),
    };
    assert_ne!(old_fence, replacement_fence);

    old_routes
        .publish(confirmed_local_route(
            old_fence,
            local_peer_id,
            relay_peer_id,
        ))
        .await
        .expect("old route publishes");
    catalog
        .tombstone(route_fence(old_fence))
        .expect("old route is removed outside its stale owner");
    replacement_routes
        .publish(confirmed_local_route(
            replacement_fence,
            local_peer_id,
            relay_peer_id,
        ))
        .await
        .expect("replacement route publishes");

    old_routes
        .fence_all()
        .expect("a stale exact fence is idempotent");

    let snapshot = catalog.snapshot().expect("route snapshot");
    assert_eq!(snapshot.relay_routes.len(), 1);
    assert_eq!(
        snapshot.relay_routes[0].fence,
        route_fence(replacement_fence)
    );
}

fn control_transport_error(operation: RelayOperation) -> RelayBookingClientError {
    RelayBookingClientError::Transport {
        operation,
        timeout: true,
    }
}

fn actor_harness(
    api: Arc<ScriptedApi>,
    backend: Arc<PendingStartBackend>,
    routes: SharedRouteRegistry,
    config: RelayCoordinatorConfig,
    snapshot: RelayBookingSnapshot,
) -> (CoordinatorActor, mpsc::Sender<CoordinatorCommand>) {
    let (commands, command_rx) = mpsc::channel(4);
    let (events, event_rx) = mpsc::channel(16);
    let relay_events = backend.subscribe().expect("backend is running");
    let now = Instant::now();
    (
        CoordinatorActor {
            api,
            backend,
            routes,
            config,
            snapshot,
            slots: HashMap::new(),
            retiring: HashMap::new(),
            pending_relay_failures: HashMap::new(),
            detached_cleanups: JoinSet::new(),
            command_rx,
            events,
            event_rx,
            relay_events,
            next_poll: now + Duration::from_secs(3_600),
            next_renew: now + Duration::from_secs(3_600),
            next_expiry: now + Duration::from_secs(3_600),
            control_fenced: false,
        },
        commands,
    )
}

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("counter reached its expected value");
}

async fn process_next_child_event(actor: &mut CoordinatorActor) {
    let event = tokio::time::timeout(Duration::from_secs(1), actor.event_rx.recv())
        .await
        .expect("child event arrived")
        .expect("child event channel remained open");
    actor
        .handle_child_event(event)
        .await
        .expect("child event was accepted");
}

#[test]
fn provider_construction_uses_the_exact_dms_limits_and_canonical_base() {
    let peer = auki_p2p::PeerId::random();
    let provider = relay_provider(
        &peer.to_string(),
        &[
            format!("/dns4/RELAY.Example.COM./tcp/0443/p2p/{peer}"),
            format!("/dns4/relay.example.com/tcp/04443/wss/p2p/{peer}"),
        ],
        900,
        1_048_576,
    )
    .expect("provider");

    assert_eq!(provider.relay_peer_id(), peer);
    assert_eq!(
        provider.selected_base().to_string(),
        format!("/dns4/relay.example.com/tcp/443/p2p/{peer}")
    );
    assert_eq!(provider.bases().len(), 2);
    assert_eq!(
        provider.expected_limits().duration(),
        Duration::from_secs(900)
    );
    assert_eq!(
        provider.expected_limits().data_bytes_per_direction(),
        1_048_576
    );
}

#[tokio::test]
async fn actor_publishes_exact_fenced_route_to_local_catalog() {
    let booking_id = Uuid::new_v4();
    let local_peer_id = PeerId::random();
    let relay_peer_id = PeerId::random();
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        assignment_id,
        reservation_epoch,
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let catalog = local_route_catalog(local_peer_id);
    let (mut actor, _commands) = actor_harness(
        api,
        backend.clone(),
        Arc::new(catalog.clone()),
        coordinator_config("actor-publish"),
        snapshot,
    );
    actor.apply_current_snapshot().await.unwrap();
    wait_for_count(&backend.starts, 1).await;
    let fence = actor.slots[&slot_id].fence;
    let expected_limits = actor.slots[&slot_id].limits;
    let expected_deadline = actor.slots[&slot_id].authorized_until;
    let route = confirmed_local_route(fence, local_peer_id, relay_peer_id).route;

    actor
        .publish_confirmed_route(fence, route.clone(), relay_peer_id)
        .await
        .expect("actor publishes confirmed route");

    let snapshot = catalog.snapshot().expect("route snapshot");
    assert_eq!(catalog.status().unwrap().confirmed_relay_count, 1);
    assert_eq!(snapshot.relay_routes.len(), 1);
    let published = &snapshot.relay_routes[0];
    assert_eq!(published.fence, route_fence(fence));
    assert_eq!(published.relay_peer_id, relay_peer_id);
    assert_eq!(published.route, route);
    assert_eq!(published.limits, expected_limits);
    assert_eq!(published.authorized_until, expected_deadline);

    actor.remove_all_slots().await.unwrap();
}

#[tokio::test]
async fn actor_retirement_preserves_a_confirmed_sibling_route() {
    let booking_id = Uuid::new_v4();
    let local_peer_id = PeerId::random();
    let first_relay = PeerId::random();
    let second_relay = PeerId::random();
    let first = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), first_relay);
    let second = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), second_relay);
    let snapshot = two_ready_snapshot(booking_id, first, second);
    let mut config = coordinator_config("actor-sibling-retirement");
    config.relay_count = 2;
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let catalog = local_route_catalog(local_peer_id);
    let (mut actor, _commands) = actor_harness(
        api,
        backend.clone(),
        Arc::new(catalog.clone()),
        config,
        snapshot,
    );
    actor.apply_current_snapshot().await.unwrap();
    wait_for_count(&backend.starts, 2).await;
    let first_fence = actor.slots[&first.0].fence;
    let second_fence = actor.slots[&second.0].fence;
    let first_route = confirmed_local_route(first_fence, local_peer_id, first_relay).route;
    let second_route = confirmed_local_route(second_fence, local_peer_id, second_relay).route;
    actor
        .publish_confirmed_route(first_fence, first_route, first_relay)
        .await
        .unwrap();
    actor
        .publish_confirmed_route(second_fence, second_route.clone(), second_relay)
        .await
        .unwrap();

    actor
        .begin_retirement(first_fence, RetirementAction::Remove)
        .await
        .unwrap();
    actor.finish_retirement(first_fence).await.unwrap();

    let snapshot = catalog.snapshot().unwrap();
    assert_eq!(catalog.status().unwrap().confirmed_relay_count, 1);
    assert_eq!(snapshot.relay_routes.len(), 1);
    assert_eq!(snapshot.relay_routes[0].fence, route_fence(second_fence));
    assert_eq!(snapshot.relay_routes[0].route, second_route);
    assert!(!actor.slots.contains_key(&first.0));
    assert!(actor.slots.contains_key(&second.0));

    actor.remove_all_slots().await.unwrap();
}

#[tokio::test]
async fn command_channel_closure_fences_routes_and_reservations_without_deleting_booking() {
    let booking_id = Uuid::new_v4();
    let local_peer_id = PeerId::random();
    let relay_peer_id = PeerId::random();
    let slot_id = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let catalog = local_route_catalog(local_peer_id);
    let (mut actor, commands) = actor_harness(
        api.clone(),
        backend.clone(),
        Arc::new(catalog.clone()),
        coordinator_config("command-channel-closed"),
        snapshot,
    );
    actor.apply_current_snapshot().await.unwrap();
    wait_for_count(&backend.starts, 1).await;
    let fence = actor.slots[&slot_id].fence;
    let route = confirmed_local_route(fence, local_peer_id, relay_peer_id).route;
    actor
        .publish_confirmed_route(fence, route, relay_peer_id)
        .await
        .expect("route is visible before owner loss");
    assert_eq!(catalog.status().unwrap().confirmed_relay_count, 1);

    drop(commands);
    actor
        .run()
        .await
        .expect("owner loss performs bounded local fencing");

    assert!(actor.control_fenced);
    assert!(actor.slots.is_empty());
    assert!(actor.retiring.is_empty());
    assert!(catalog.snapshot().unwrap().relay_routes.is_empty());
    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 0);
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn booking_delete_waits_for_owned_detached_cleanup() {
    let booking_id = Uuid::new_v4();
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend,
        routes,
        coordinator_config("delete-after-detached-cleanup"),
        queued_snapshot(booking_id),
    );
    let (release, released) = oneshot::channel();
    actor.detached_cleanups.spawn(async move {
        released
            .await
            .map_err(|_| "detached cleanup release was dropped".to_string())?;
        Ok(())
    });
    let mut stop = Box::pin(actor.stop(true));

    assert!(
        tokio::time::timeout(Duration::from_millis(10), stop.as_mut())
            .await
            .is_err()
    );
    assert!(api.calls().is_empty());

    release.send(()).expect("release detached cleanup");
    stop.await.expect("cleanup completes before booking delete");
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::Delete(observed_booking_id)] if *observed_booking_id == booking_id
    ));
}

#[tokio::test]
async fn coordinator_graceful_shutdown_drains_reservations_before_deleting_booking() {
    let booking_id = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(Some(snapshot)));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes,
        coordinator_config("graceful-delete"),
    )
    .await
    .expect("coordinator starts");
    wait_for_count(&backend.starts, 1).await;

    let outcome = coordinator
        .shutdown(true, Duration::from_secs(1))
        .await
        .expect("graceful shutdown succeeds");

    assert_eq!(outcome, RelayCoordinatorShutdownOutcome::Graceful);
    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::Active, ApiCall::Delete(observed)] if *observed == booking_id
    ));
}

#[tokio::test]
async fn failed_graceful_tombstone_still_force_fences_and_drains_local_work() {
    let booking_id = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(Some(snapshot)));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        coordinator_config("failed-graceful-tombstone"),
    )
    .await
    .expect("coordinator starts");
    let mut health = coordinator.health();
    wait_for_count(&backend.starts, 1).await;
    routes.fail_tombstones.store(true, Ordering::SeqCst);

    let error = coordinator
        .shutdown(true, Duration::from_secs(1))
        .await
        .expect_err("the graceful route failure remains visible");

    assert!(matches!(
        error,
        RelayCoordinatorShutdownError::Graceful(RelayCoordinatorError::RouteRegistry(_))
    ));
    assert!(health.is_failed());
    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert!(routes.fence_all_calls.load(Ordering::SeqCst) >= 1);
    assert!(matches!(api.calls().as_slice(), [ApiCall::Active]));
    tokio::time::timeout(Duration::from_secs(1), health.failed())
        .await
        .expect("the failed actor is fully reaped");
}

#[tokio::test]
async fn blocked_actor_forces_bounded_local_fencing_without_late_mutation() {
    let booking_id = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(BlockingActiveApi::new(snapshot));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("blocked-actor-shutdown");
    config.status_poll_interval = Duration::from_millis(10);
    config.http_timeout = Duration::from_secs(5);
    config.reservation_retry_budget = Duration::from_millis(100);
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        config,
    )
    .await
    .expect("coordinator starts");
    let health = coordinator.health();
    wait_for_count(&backend.starts, 1).await;
    api.wait_until_blocked().await;

    let outcome = coordinator
        .shutdown(true, Duration::from_millis(20))
        .await
        .expect("forced local cleanup succeeds");

    assert_eq!(outcome, RelayCoordinatorShutdownOutcome::ForcedAfterTimeout);
    assert!(health.is_failed());
    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 0);
    assert_eq!(api.deletes.load(Ordering::SeqCst), 0);
    assert!(routes.fence_all_calls.load(Ordering::SeqCst) >= 1);
    let calls_after_shutdown = api.active_calls.load(Ordering::SeqCst);
    api.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        api.active_calls.load(Ordering::SeqCst),
        calls_after_shutdown
    );
}

#[tokio::test]
async fn saturated_command_channel_forces_shutdown_instead_of_waiting_forever() {
    let snapshot = queued_snapshot(Uuid::new_v4());
    let api = Arc::new(BlockingActiveApi::new(snapshot));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("blocked-command-shutdown");
    config.status_poll_interval = Duration::from_millis(10);
    config.http_timeout = Duration::from_secs(5);
    let coordinator =
        RelayBookingCoordinator::start_with_backends(api.clone(), backend, routes.clone(), config)
            .await
            .expect("coordinator starts");
    api.wait_until_blocked().await;

    let mut queued_responses = Vec::new();
    for _ in 0..32 {
        let (response, receiver) = oneshot::channel();
        assert!(
            coordinator
                .commands
                .try_send(CoordinatorCommand::Stop {
                    delete_booking: false,
                    response,
                })
                .is_ok(),
            "the test fills the exact bounded command capacity"
        );
        queued_responses.push(receiver);
    }

    let outcome = coordinator
        .shutdown(false, Duration::from_millis(20))
        .await
        .expect("forced shutdown succeeds with a saturated command queue");
    assert_eq!(outcome, RelayCoordinatorShutdownOutcome::ForcedAfterTimeout);
    assert!(routes.fence_all_calls.load(Ordering::SeqCst) >= 1);
    for response in queued_responses {
        assert!(response.await.is_err());
    }
    let calls_after_shutdown = api.active_calls.load(Ordering::SeqCst);
    api.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        api.active_calls.load(Ordering::SeqCst),
        calls_after_shutdown
    );
}

#[tokio::test]
async fn dropping_a_blocked_coordinator_reaps_actor_and_nested_reservation_start() {
    let snapshot = ready_snapshot(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(BlockingActiveApi::new(snapshot));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("drop-blocked-coordinator");
    config.status_poll_interval = Duration::from_millis(10);
    config.http_timeout = Duration::from_secs(5);
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        config,
    )
    .await
    .expect("coordinator starts");
    let mut health = coordinator.health();
    wait_for_count(&backend.starts, 1).await;
    api.wait_until_blocked().await;

    drop(coordinator);
    assert!(routes.fence_all_calls.load(Ordering::SeqCst) >= 1);
    tokio::time::timeout(Duration::from_secs(1), health.failed())
        .await
        .expect("aborted actor publishes terminal health");
    wait_for_count(&backend.dropped_starts, 1).await;
    let calls_after_drop = api.active_calls.load(Ordering::SeqCst);
    api.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(api.active_calls.load(Ordering::SeqCst), calls_after_drop);
}

#[tokio::test]
async fn cancelling_shutdown_future_reaps_owned_actor_without_late_backend_or_route_work() {
    let snapshot = ready_snapshot(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(BlockingActiveApi::new(snapshot));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("cancel-shutdown-future");
    config.status_poll_interval = Duration::from_millis(10);
    config.http_timeout = Duration::from_secs(5);
    let coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        config,
    )
    .await
    .expect("coordinator starts");
    let mut health = coordinator.health();
    wait_for_count(&backend.starts, 1).await;
    api.wait_until_blocked().await;

    let shutdown =
        tokio::spawn(async move { coordinator.shutdown(true, Duration::from_secs(5)).await });
    tokio::task::yield_now().await;
    shutdown.abort();
    assert!(
        shutdown
            .await
            .expect_err("shutdown task is cancelled")
            .is_cancelled()
    );

    assert!(routes.fence_all_calls.load(Ordering::SeqCst) >= 1);
    tokio::time::timeout(Duration::from_secs(1), health.failed())
        .await
        .expect("the owned actor is not detached");
    wait_for_count(&backend.dropped_starts, 1).await;
    assert_eq!(routes.publications.load(Ordering::SeqCst), 0);
    let calls_after_cancel = api.active_calls.load(Ordering::SeqCst);
    api.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(api.active_calls.load(Ordering::SeqCst), calls_after_cancel);
    assert_eq!(backend.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_owned_detached_cleanup_is_terminal_during_run() {
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend,
        routes,
        coordinator_config("failed-detached-cleanup"),
        queued_snapshot(Uuid::new_v4()),
    );
    actor
        .detached_cleanups
        .spawn(async { Err("synthetic detached cleanup failure".to_string()) });

    let result = tokio::time::timeout(Duration::from_secs(1), actor.run())
        .await
        .expect("cleanup failure is surfaced promptly");

    assert!(matches!(
        result,
        Err(RelayCoordinatorError::ReservationCleanup(error))
            if error == "synthetic detached cleanup failure"
    ));
    assert!(api.calls().is_empty());
}

#[tokio::test]
async fn route_catalog_rejection_cleans_local_reservation_without_reporting_provider_failure() {
    let booking_id = Uuid::new_v4();
    let requester_peer_id = PeerId::random();
    let unrelated_catalog_peer_id = PeerId::random();
    let relay_peer_id = PeerId::random();
    let slot_id = Uuid::new_v4();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let catalog = local_route_catalog(unrelated_catalog_peer_id);
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend.clone(),
        Arc::new(catalog.clone()),
        coordinator_config("route-catalog-rejection"),
        snapshot,
    );
    actor.apply_current_snapshot().await.unwrap();
    wait_for_count(&backend.starts, 1).await;
    let fence = actor.slots[&slot_id].fence;
    let route = confirmed_local_route(fence, requester_peer_id, relay_peer_id).route;

    let result = actor
        .publish_confirmed_route(fence, route, relay_peer_id)
        .await;

    assert!(matches!(
        result,
        Err(RelayCoordinatorError::RouteRegistry(_))
    ));
    assert!(actor.slots.is_empty());
    assert!(actor.retiring.is_empty());
    assert!(catalog.snapshot().unwrap().relay_routes.is_empty());
    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 0);
    assert!(
        !api.calls()
            .iter()
            .any(|call| matches!(call, ApiCall::ReservationFailed { .. }))
    );
}

#[tokio::test]
async fn stopped_reservation_backend_is_terminal_without_reporting_provider_failure() {
    let snapshot = ready_snapshot(
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        PeerId::random(),
    );
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(Some(snapshot)));
    let backend = Arc::new(StoppedStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut coordinator = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend,
        routes,
        coordinator_config("stopped-reservation-backend"),
    )
    .await
    .expect("coordinator starts before the reservation worker reports stop");

    let result = tokio::time::timeout(
        Duration::from_secs(1),
        coordinator.task.take().expect("coordinator task"),
    )
    .await
    .expect("terminal backend result is bounded")
    .expect("coordinator task did not panic");

    assert!(matches!(
        result,
        Err(RelayCoordinatorError::DomainRelay(
            DomainRelayError::Stopped
        ))
    ));
    assert!(matches!(api.calls().as_slice(), [ApiCall::Active]));
}

#[tokio::test]
async fn stopped_subscription_is_rejected_before_any_dms_booking_call() {
    let api = Arc::new(ScriptedApi::default());
    let result = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        Arc::new(StoppedSubscriptionBackend),
        Arc::new(RecordingRoutes::default()),
        coordinator_config("stopped-subscription"),
    )
    .await;

    assert!(matches!(
        result,
        Err(RelayCoordinatorError::DomainRelay(
            DomainRelayError::Stopped
        ))
    ));
    assert!(api.calls().is_empty());
}

#[test]
fn failure_mapping_distinguishes_configuration_dial_and_loss() {
    let mismatch = DomainRelayError::P2p(auki_p2p::Error::RelayConfirmationRejected(
        RelayConfirmationRejection::MissingLimits,
    ));
    assert_eq!(
        reservation_failure_reason(&mismatch, false),
        ReservationFailureReason::LimitMismatch
    );
    assert!(!reservation_failure_is_retryable(&mismatch));

    let missing_transport = DomainRelayError::P2p(auki_p2p::Error::RelayReservation(
        auki_p2p::RelayReservationError::MissingTransportBase(auki_p2p::RelayBaseTransport::Tcp),
    ));
    assert_eq!(
        reservation_failure_reason(&missing_transport, false),
        ReservationFailureReason::AddressMismatch
    );
    assert!(!reservation_failure_is_retryable(&missing_transport));

    let dns = DomainRelayError::P2p(auki_p2p::Error::Dns("NXDOMAIN".to_string()));
    assert_eq!(
        reservation_failure_reason(&dns, false),
        ReservationFailureReason::DialFailed
    );
    assert!(reservation_failure_is_retryable(&dns));

    let closed = DomainRelayError::P2p(auki_p2p::Error::RelayReservationClosed(
        "closed".to_string(),
    ));
    assert_eq!(
        reservation_failure_reason(&closed, true),
        ReservationFailureReason::ReservationLost
    );
    assert_eq!(
        preferred_failure_reason(
            ReservationFailureReason::ReservationDenied,
            ReservationFailureReason::ReservationLost,
        ),
        ReservationFailureReason::ReservationLost
    );
    assert_eq!(
        preferred_failure_reason(
            ReservationFailureReason::ReservationLost,
            ReservationFailureReason::LimitMismatch,
        ),
        ReservationFailureReason::LimitMismatch
    );
    assert!(matches!(
        reservation_attempt_failure(DomainRelayError::Stopped, None),
        ReservationAttemptFailure::BackendStopped { handle: None }
    ));
}

#[tokio::test]
async fn lost_create_response_is_recovered_by_active_lookup_without_duplicate_create() {
    let booking_id = Uuid::new_v4();
    let snapshot = queued_snapshot(booking_id);
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(None));
    api.push_create(Err(control_transport_error(RelayOperation::Create)));
    api.push_active(Ok(Some(snapshot)));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let config = coordinator_config("lost-create-active-first");

    let first = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        config.clone(),
    )
    .await;
    assert!(matches!(
        first,
        Err(RelayCoordinatorError::Dms(
            RelayBookingClientError::Transport {
                operation: RelayOperation::Create,
                timeout: true
            }
        ))
    ));

    let recovered =
        RelayBookingCoordinator::start_with_backends(api.clone(), backend, routes, config)
            .await
            .expect("active booking recovers the lost create response");
    recovered
        .shutdown(false, Duration::from_secs(1))
        .await
        .expect("clean shutdown");

    let calls = api.calls();
    assert_eq!(calls.len(), 3);
    assert!(matches!(calls[0], ApiCall::Active));
    assert!(matches!(calls[1], ApiCall::Create { .. }));
    assert!(matches!(calls[2], ApiCall::Active));
}

#[tokio::test]
async fn lost_create_retry_reuses_one_stable_idempotency_key() {
    let snapshot = queued_snapshot(Uuid::new_v4());
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(None));
    api.push_create(Err(control_transport_error(RelayOperation::Create)));
    api.push_active(Ok(None));
    api.push_create(Ok(replayed_create(snapshot)));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let config = coordinator_config("stable-create-key");
    let expected_key = config.idempotency_key.clone();

    let first = RelayBookingCoordinator::start_with_backends(
        api.clone(),
        backend.clone(),
        routes.clone(),
        config.clone(),
    )
    .await;
    assert!(first.is_err());
    let recovered =
        RelayBookingCoordinator::start_with_backends(api.clone(), backend, routes, config)
            .await
            .expect("idempotent create replay");
    recovered
        .shutdown(false, Duration::from_secs(1))
        .await
        .expect("clean shutdown");

    let calls = api.calls();
    assert_eq!(calls.len(), 4);
    assert!(matches!(calls[0], ApiCall::Active));
    assert!(matches!(calls[2], ApiCall::Active));
    let create_keys: Vec<_> = calls
        .iter()
        .filter_map(|call| match call {
            ApiCall::Create { key, .. } => Some(key),
            _ => None,
        })
        .collect();
    assert_eq!(create_keys, vec![&expected_key, &expected_key]);
}

#[tokio::test]
async fn cancel_during_blocked_start_is_bounded_and_emits_no_handle() {
    let backend = Arc::new(PendingStartBackend::new());
    let relay_peer_id = auki_p2p::PeerId::random();
    let provider = relay_provider(
        &relay_peer_id.to_string(),
        &[format!(
            "/dns4/relay-a.dev.aukiverse.com/tcp/443/p2p/{relay_peer_id}"
        )],
        900,
        1_048_576,
    )
    .expect("provider");
    let fence = LocalRelayFence {
        slot_id: Uuid::new_v4(),
        assignment_id: Uuid::new_v4(),
        reservation_epoch: Uuid::new_v4(),
        local_generation: 1,
    };
    let (events, mut event_rx) = mpsc::channel(4);
    let cancellation = CancellationToken::new();
    let worker = spawn_reservation_worker(
        backend.clone(),
        events,
        fence,
        provider,
        cancellation.clone(),
        coordinator_config("blocked-start"),
    );
    wait_for_count(&backend.starts, 1).await;

    cancellation.cancel();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .expect("worker cancellation is bounded")
        .expect("worker did not panic")
        .expect("worker cleanup succeeded");

    assert_eq!(backend.dropped_starts.load(Ordering::SeqCst), 1);
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 0);
    assert!(matches!(
        event_rx.try_recv(),
        Err(mpsc::error::TryRecvError::Disconnected)
    ));
}

#[tokio::test]
async fn preconfirmation_failure_tombstones_without_publishing() {
    let booking_id = Uuid::new_v4();
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let ready = ready_snapshot(
        booking_id,
        slot_id,
        assignment_id,
        reservation_epoch,
        auki_p2p::PeerId::random(),
    );
    let api = Arc::new(ScriptedApi::default());
    api.push_reservation_failed(Ok(queued_snapshot(booking_id)));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend.clone(),
        routes.clone(),
        coordinator_config("preconfirmation-loss"),
        ready,
    );
    actor
        .apply_current_snapshot()
        .await
        .expect("ready slot starts reservation");
    wait_for_count(&backend.starts, 1).await;
    let fence = actor.slots[&slot_id].fence;

    actor
        .handle_local_failure(fence, ReservationFailureReason::ReservationLost)
        .await
        .expect("pre-confirmation loss is fenced");
    process_next_child_event(&mut actor).await;

    assert_eq!(routes.publications.load(Ordering::SeqCst), 0);
    assert!(routes.tombstones.lock().contains(&fence));
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 0);
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::ReservationFailed {
            booking_id: observed_booking_id,
            request
        }] if *observed_booking_id == booking_id
            && request.slot_id == slot_id
            && request.assignment_id == assignment_id
            && request.reservation_epoch == reservation_epoch
            && request.reason == ReservationFailureReason::ReservationLost
    ));
}

#[tokio::test]
async fn reservation_loss_tombstones_confirmed_route_before_failure_report() {
    let booking_id = Uuid::new_v4();
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let relay_peer_id = PeerId::random();
    let local_peer_id = PeerId::random();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        assignment_id,
        reservation_epoch,
        relay_peer_id,
    );
    let fence = LocalRelayFence {
        slot_id,
        assignment_id,
        reservation_epoch,
        local_generation: 1,
    };
    let catalog = local_route_catalog(local_peer_id);
    let published = confirmed_local_route(fence, local_peer_id, relay_peer_id);
    catalog
        .publish(published.clone())
        .await
        .expect("confirmed route is initially visible");

    let api = Arc::new(ScriptedApi::default());
    api.push_reservation_failed(Ok(queued_snapshot(booking_id)));
    let backend = Arc::new(PendingStartBackend::new());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend,
        Arc::new(catalog.clone()),
        coordinator_config("confirmed-loss-ordering"),
        snapshot.clone(),
    );
    actor.slots.insert(
        slot_id,
        LocalSlot {
            fence,
            relay_peer_id: relay_peer_id.to_string(),
            provider_base_addresses: snapshot.slots[0].provider_base_addresses.clone().unwrap(),
            limits: published.limits,
            authorized_until: published.authorized_until,
            // The exact reservation handle is transport-owned and opaque
            // to this test. Retirement ordering is identical before the
            // optional handle cleanup begins.
            state: LocalSlotState::Reserving(None),
            cancel_retry: CancellationToken::new(),
            worker: None,
        },
    );

    actor
        .handle_local_failure(fence, ReservationFailureReason::ReservationLost)
        .await
        .expect("reservation loss begins fenced retirement");

    assert!(catalog.snapshot().unwrap().relay_routes.is_empty());
    assert!(api.calls().is_empty());

    process_next_child_event(&mut actor).await;
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::ReservationFailed {
            booking_id: observed_booking_id,
            request
        }] if *observed_booking_id == booking_id
            && request.slot_id == slot_id
            && request.assignment_id == assignment_id
            && request.reservation_epoch == reservation_epoch
            && request.reason == ReservationFailureReason::ReservationLost
    ));
}

#[tokio::test]
async fn local_authority_cutoff_tombstones_without_reporting_provider_failure() {
    let booking_id = Uuid::new_v4();
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let relay_peer_id = auki_p2p::PeerId::random();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        assignment_id,
        reservation_epoch,
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend,
        routes.clone(),
        coordinator_config("local-authority-cutoff"),
        snapshot.clone(),
    );
    let fence = LocalRelayFence {
        slot_id,
        assignment_id,
        reservation_epoch,
        local_generation: 1,
    };
    actor.slots.insert(
        slot_id,
        LocalSlot {
            fence,
            relay_peer_id: relay_peer_id.to_string(),
            provider_base_addresses: snapshot.slots[0].provider_base_addresses.clone().unwrap(),
            limits: ExpectedRelayLimits::new(Duration::from_secs(900), 1_048_576).unwrap(),
            authorized_until: chrono::Utc::now() - chrono::Duration::milliseconds(1),
            state: LocalSlotState::Reserving(None),
            cancel_retry: CancellationToken::new(),
            worker: None,
        },
    );

    actor
        .expire_local_authority()
        .await
        .expect("local cutoff starts fenced retirement");
    process_next_child_event(&mut actor).await;

    assert!(!actor.slots.contains_key(&slot_id));
    assert!(routes.tombstones.lock().contains(&fence));
    assert!(
        !api.calls()
            .iter()
            .any(|call| matches!(call, ApiCall::ReservationFailed { .. }))
    );
}

#[tokio::test]
async fn lagged_relay_events_fail_closed() {
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api,
        backend,
        routes,
        coordinator_config("lagged-relay-events"),
        queued_snapshot(Uuid::new_v4()),
    );

    let result = actor
        .handle_relay_receive(Err(broadcast::error::RecvError::Lagged(129)))
        .await;

    assert!(matches!(
        result,
        Err(RelayCoordinatorError::RelayEventLagged(129))
    ));
}

#[tokio::test]
async fn retiring_one_child_does_not_block_parent_renewal() {
    let booking_id = Uuid::new_v4();
    let slot_id = Uuid::new_v4();
    let assignment_id = Uuid::new_v4();
    let reservation_epoch = Uuid::new_v4();
    let relay_peer_id = auki_p2p::PeerId::random();
    let snapshot = ready_snapshot(
        booking_id,
        slot_id,
        assignment_id,
        reservation_epoch,
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    api.push_renew(Ok(snapshot.clone()));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("retiring-sibling");
    config.reservation_retry_budget = Duration::from_millis(25);
    let (mut actor, _commands) =
        actor_harness(api.clone(), backend, routes, config, snapshot.clone());
    let fence = LocalRelayFence {
        slot_id,
        assignment_id,
        reservation_epoch,
        local_generation: 1,
    };
    actor.slots.insert(
        slot_id,
        LocalSlot {
            fence,
            relay_peer_id: relay_peer_id.to_string(),
            provider_base_addresses: snapshot.slots[0].provider_base_addresses.clone().unwrap(),
            limits: ExpectedRelayLimits::new(Duration::from_secs(900), 1_048_576).unwrap(),
            authorized_until: chrono::Utc::now() + chrono::Duration::minutes(2),
            state: LocalSlotState::Reserving(None),
            cancel_retry: CancellationToken::new(),
            worker: Some(AbortOnDropHandle::new(tokio::spawn(std::future::pending()))),
        },
    );

    actor
        .handle_local_failure(fence, ReservationFailureReason::ReservationLost)
        .await
        .expect("retirement begins without joining the blocked child");
    tokio::time::timeout(Duration::from_millis(10), actor.renew())
        .await
        .expect("parent renewal is not blocked by child cleanup")
        .expect("parent renewal succeeds");

    assert!(
        api.calls()
            .iter()
            .any(|call| matches!(call, ApiCall::Renew(id) if *id == booking_id))
    );
    assert!(
        !api.calls()
            .iter()
            .any(|call| matches!(call, ApiCall::ReservationFailed { .. }))
    );
    assert!(matches!(
        actor.remove_all_slots().await,
        Err(RelayCoordinatorError::ReservationCleanup(_))
    ));
}

#[tokio::test]
async fn stale_assignment_event_cannot_retire_replacement_generation() {
    let booking_id = Uuid::new_v4();
    let slot_id = Uuid::new_v4();
    let relay_peer_id = auki_p2p::PeerId::random();
    let snapshot_a = ready_snapshot(
        booking_id,
        slot_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        relay_peer_id,
    );
    let snapshot_b = ready_snapshot(
        booking_id,
        slot_id,
        Uuid::new_v4(),
        Uuid::new_v4(),
        relay_peer_id,
    );
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend.clone(),
        routes.clone(),
        coordinator_config("stale-a-b"),
        snapshot_a,
    );
    actor
        .apply_current_snapshot()
        .await
        .expect("generation A starts");
    wait_for_count(&backend.starts, 1).await;
    let fence_a = actor.slots[&slot_id].fence;

    actor
        .apply_snapshot(snapshot_b)
        .await
        .expect("generation B replaces A");
    process_next_child_event(&mut actor).await;
    wait_for_count(&backend.starts, 2).await;
    let fence_b = actor.slots[&slot_id].fence;
    assert_ne!(fence_a, fence_b);
    let tombstones_before_stale_event = routes.tombstones.lock().len();

    actor
        .handle_child_event(ChildEvent::Failed {
            fence: fence_a,
            handle: None,
            reason: ReservationFailureReason::ReservationLost,
        })
        .await
        .expect("stale failure is harmless");

    assert_eq!(actor.slots[&slot_id].fence, fence_b);
    assert!(matches!(
        actor.slots[&slot_id].state,
        LocalSlotState::Reserving(None)
    ));
    assert_eq!(
        routes.tombstones.lock().len(),
        tombstones_before_stale_event
    );
    assert!(api.calls().is_empty());

    actor
        .remove_all_slots()
        .await
        .expect("test cleanup is bounded");
}

#[tokio::test]
async fn foreign_booking_snapshot_does_not_replace_pinned_authority() {
    let booking_id = Uuid::new_v4();
    let original = queued_snapshot(booking_id);
    let foreign = queued_snapshot(Uuid::new_v4());
    let api = Arc::new(ScriptedApi::default());
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let (mut actor, _commands) = actor_harness(
        api.clone(),
        backend,
        routes.clone(),
        coordinator_config("booking-id-pin"),
        original,
    );

    let result = actor.apply_snapshot(foreign).await;
    assert!(matches!(
        result,
        Err(RelayCoordinatorError::ActiveBookingMismatch)
    ));
    assert_eq!(actor.snapshot.booking_id, booking_id);
    assert!(actor.slots.is_empty());
    assert!(api.calls().is_empty());
    assert_eq!(routes.publications.load(Ordering::SeqCst), 0);
    assert!(routes.tombstones.lock().is_empty());
}

#[tokio::test]
async fn disappeared_booking_is_terminal() {
    let snapshot = queued_snapshot(Uuid::new_v4());
    let api = Arc::new(ScriptedApi::default());
    api.push_active(Ok(Some(snapshot)));
    api.push_active(Ok(None));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("terminal-health");
    config.status_poll_interval = Duration::from_millis(10);

    let mut coordinator =
        RelayBookingCoordinator::start_with_backends(api.clone(), backend, routes, config)
            .await
            .expect("coordinator starts");
    let mut health = coordinator.health();
    tokio::time::timeout(Duration::from_secs(1), health.failed())
        .await
        .expect("terminal health signal is bounded");
    let result = coordinator
        .task
        .take()
        .expect("coordinator task")
        .await
        .expect("coordinator task did not panic");
    assert!(matches!(result, Err(RelayCoordinatorError::AuthorityEnded)));
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::Active, ApiCall::Active]
    ));
}

#[tokio::test(start_paused = true)]
async fn protected_renewal_runs_before_adjacent_status_poll() {
    let booking_id = Uuid::new_v4();
    let snapshot = queued_snapshot(booking_id);
    let api = Arc::new(ScriptedApi::default());
    api.push_renew(Ok(snapshot.clone()));
    api.push_active(Ok(Some(snapshot.clone())));
    let backend = Arc::new(PendingStartBackend::new());
    let routes = Arc::new(RecordingRoutes::default());
    let mut config = coordinator_config("renew-before-poll");
    config.http_timeout = Duration::from_secs(1);
    let (mut actor, commands) = actor_harness(api.clone(), backend, routes, config, snapshot);
    let now = Instant::now();
    actor.next_poll = now + Duration::from_millis(9_500);
    actor.next_renew = now + Duration::from_secs(10);
    actor.next_expiry = now + Duration::from_secs(3_600);
    let task = tokio::spawn(async move { actor.run().await });
    tokio::task::yield_now().await;

    tokio::time::advance(Duration::from_millis(9_500)).await;
    tokio::task::yield_now().await;
    assert!(api.calls().is_empty());

    tokio::time::advance(Duration::from_millis(500)).await;
    for _ in 0..16 {
        if api.calls().len() >= 2 {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(matches!(
        api.calls().as_slice(),
        [ApiCall::Renew(observed_booking_id), ApiCall::Active]
            if *observed_booking_id == booking_id
    ));

    let (response, receiver) = oneshot::channel();
    commands
        .send(CoordinatorCommand::Stop {
            delete_booking: false,
            response,
        })
        .await
        .expect("actor receives stop");
    receiver
        .await
        .expect("actor responds to stop")
        .expect("actor stops cleanly");
    task.await
        .expect("actor did not panic")
        .expect("actor run completed");
}
