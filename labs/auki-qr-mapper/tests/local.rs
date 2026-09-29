use auki_components::*;
use auki_domain_client::{Portal, PortalId};
use auki_qr_mapper::{local::*, *};
use auki_scenegraph::{component::*, resolution::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
const PAYLOAD: &str = "HTTPS://R8.HR/ABC12345678";
#[derive(Clone)]
struct Database {
    calls: Arc<AtomicUsize>,
    fail: bool,
}
impl PortalMetadataSource for Database {
    async fn resolve(
        &self,
        _: Uuid,
        id: &PortalId,
        _: &CancellationToken,
    ) -> Result<ResolvedPortal, PortalSizeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id.as_str(), "ABC12345678");
        if self.fail {
            return Err(PortalSizeError::InvalidSize);
        }
        let portal:Portal=serde_json::from_value(serde_json::json!({"id":"00000000-0000-0000-0000-000000000001","short_id":"ABC12345678","name":"qr","size":20.,"organization_id":null,"default_domain_id":null,"redirect_url":null,"created_at":"2026-09-01T00:00:00Z","updated_at":"2026-09-01T00:00:00Z"})).unwrap();
        ResolvedPortal::try_from(portal)
    }
}
fn fixture(write: bool) -> (PortalMaps, Arc<MapComponent>, Database) {
    let runtime = ComponentRuntime::new("local");
    let clock = AtomicU64::new(1);
    let map = Arc::new(
        MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: "map".into(),
                publication_id: "run".into(),
                clock: fixture_clock("clock"),
                map: MapDefinition {
                    map_id: "map".into(),
                    name: None,
                    domain_reference: None,
                    frame: MapFrame::z_up_meters("frame", "explicit bootstrap"),
                },
            },
            move || clock.fetch_add(1, Ordering::SeqCst),
            |_| true,
            move |_| write,
        )
        .unwrap(),
    );
    let maps = PortalMaps::new(InvocationContext {
        invocation_id: "mapping".into(),
        caller_peer_id: "local".into(),
        caller_component_id: "mapper".into(),
    });
    maps.register(map.clone()).unwrap();
    (
        maps,
        map,
        Database {
            calls: Arc::new(AtomicUsize::new(0)),
            fail: false,
        },
    )
}
fn placement(map: &Arc<MapComponent>) -> AnchorPlacement {
    AnchorPlacement {
        map: map.clone(),
        expected_snapshot: map.snapshot_reference(),
        pose_in_map: RigidTransform::identity("portal-frame", "frame"),
    }
}
#[tokio::test]
async fn unseen_qr_is_resolved_and_committed_then_known_qr_works_without_database() {
    let (maps, map, db) = fixture(true);
    let calls = db.calls.clone();
    let mapper = PortalMapper::new(maps.clone(), db);
    let cancel = CancellationToken::new();
    let added = mapper
        .lookup_helper(PAYLOAD, Uuid::nil(), Some(placement(&map)), &cancel)
        .await
        .unwrap();
    let EnsureQrResult::Added(qr) = added else {
        panic!("not added")
    };
    assert_eq!(qr.anchor.side_length_m, 0.2);
    assert_eq!(qr.snapshot, map.snapshot_reference());
    assert_eq!(maps.resolve(PAYLOAD).unwrap().len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let offline = PortalMapper::new(
        maps,
        Database {
            calls: calls.clone(),
            fail: true,
        },
    );
    assert!(matches!(
        offline
            .lookup_helper(PAYLOAD, Uuid::nil(), None, &cancel)
            .await
            .unwrap(),
        EnsureQrResult::Known(_)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn metadata_without_placement_does_not_invent_an_anchor() {
    let (maps, map, db) = fixture(true);
    let before = map.snapshot_reference();
    let mapper = PortalMapper::new(maps.clone(), db);
    assert!(matches!(
        mapper
            .lookup_helper(PAYLOAD, Uuid::nil(), None, &CancellationToken::new())
            .await
            .unwrap(),
        EnsureQrResult::NeedsPlacement(_)
    ));
    assert_eq!(before, map.snapshot_reference());
    assert!(maps.resolve(PAYLOAD).unwrap().is_empty());
}
#[tokio::test]
async fn failed_lookup_and_denied_write_leave_map_unchanged() {
    for denied in [false, true] {
        let (maps, map, mut db) = fixture(!denied);
        db.fail = !denied;
        let before = map.snapshot_reference();
        let mapper = PortalMapper::new(maps.clone(), db);
        assert!(
            mapper
                .lookup_helper(
                    PAYLOAD,
                    Uuid::nil(),
                    Some(placement(&map)),
                    &CancellationToken::new()
                )
                .await
                .is_err()
        );
        assert_eq!(before, map.snapshot_reference());
        assert!(maps.resolve(PAYLOAD).unwrap().is_empty());
    }
}
#[tokio::test]
async fn cancellation_and_arbitrary_urls_do_not_reach_database() {
    let (maps, map, db) = fixture(true);
    let calls = db.calls.clone();
    let mapper = PortalMapper::new(maps, db);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        mapper
            .lookup_helper(PAYLOAD, Uuid::nil(), Some(placement(&map)), &cancel)
            .await
            .is_err()
    );
    for payload in [
        "https://evil.invalid/ABC12345678",
        "https://r8.hr/ABC12345678?other=1",
        "https://r8.hr/ABC12345678/extra",
    ] {
        assert!(
            mapper
                .lookup_helper(payload, Uuid::nil(), None, &CancellationToken::new())
                .await
                .is_err()
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn stale_placement_does_not_overwrite_newer_state() {
    let (maps, map, db) = fixture(true);
    let stale = placement(&map);
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            maps.context().clone(),
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor: QrAnchor {
                    anchor_id: "other".into(),
                    payload: "other".into(),
                    side_length_m: 0.1,
                    pose_in_map: RigidTransform::identity("portal-frame", "frame"),
                },
            },
        )
        .unwrap();
    let current = map.snapshot_reference();
    let mapper = PortalMapper::new(maps.clone(), db);
    assert!(
        mapper
            .lookup_helper(PAYLOAD, Uuid::nil(), Some(stale), &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(current, map.snapshot_reference());
    assert!(maps.resolve(PAYLOAD).unwrap().is_empty());
}

#[path = "../../auki-components/tests/support/clock.rs"]
mod clock_fixture;
use clock_fixture::fixture_clock;
