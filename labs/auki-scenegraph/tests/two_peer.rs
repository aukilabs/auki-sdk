#![cfg(all(feature = "components", not(target_arch = "wasm32")))]
mod support;
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart,
    RemoteObservationEvent,
};
use auki_components::{
    BufferLimits, ComponentRuntime, InMemoryTransport, Invocation, InvocationContext,
};
use auki_scenegraph::{catalog::*, component::*, *};
use auki_sdk::{AukiPeer, Identity};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use uuid::Uuid;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discover_fetch_lookup_move_and_follow_one_qr_across_authenticated_peers() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let domain = Uuid::new_v4();
        let si = Identity::generate();
        let ci = Identity::generate();
        let (server, _sa) = AukiPeer::start_external(
            si.clone(),
            support::authority(&si, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let (consumer, _ca) = AukiPeer::start_external(
            ci.clone(),
            support::authority(&ci, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let route = server.listen_addresses()[0].clone();
        let runtime = ComponentRuntime::new(server.peer_id().to_string());
        let clock = Arc::new(AtomicU64::new(1));
        let publication_clock = clock.clone();
        let expected_writer = consumer.peer_id().to_string();
        let map = MapComponent::new(
            &runtime,
            MapComponentConfig {
                component_id: "qr-map".into(),
                publication_id: Uuid::new_v4().to_string(),
                clock_id: "test-clock".into(),
                map: MapDefinition {
                    map_id: "supermarket-qr-map".into(),
                    name: Some("QR map".into()),
                    domain_reference: Some(domain.to_string()),
                    frame: MapFrame::z_up_meters("map-root", "manually established test frame"),
                },
            },
            move || publication_clock.load(Ordering::SeqCst),
            |_| true,
            move |ctx| ctx.caller_peer_id == expected_writer && ctx.caller_component_id == "mapper",
        )
        .unwrap();
        let anchor = QrAnchor {
            anchor_id: "qr-123".into(),
            payload: "auki:fixture-qr".into(),
            side_length_m: 0.2,
            pose_in_map: RigidTransform::identity("qr-123-frame", "map-root"),
        };
        clock.store(2, Ordering::SeqCst);
        InMemoryTransport
            .invoke(
                map.upsert_qr(),
                InvocationContext {
                    invocation_id: "seed".into(),
                    caller_peer_id: consumer.peer_id().to_string(),
                    caller_component_id: "mapper".into(),
                },
                UpsertQr {
                    expected_snapshot: map.snapshot_reference(),
                    anchor: anchor.clone(),
                },
            )
            .unwrap();
        let endpoint = ComponentProtocolEndpoint::mount(server.protocols(), runtime).unwrap();
        endpoint.export_product(&map.product()).unwrap();
        endpoint.export_operable(map.lookup_qr()).unwrap();
        endpoint.export_operable(map.find_qr()).unwrap();
        endpoint.export_operable(map.upsert_qr()).unwrap();
        let client = ComponentProtocolClient::new(consumer.protocols());
        let CatalogResponse::Snapshot { snapshot: catalog } = client
            .catalog_exact(server.peer_id(), route.clone(), None)
            .await
            .unwrap()
        else {
            panic!("expected catalog");
        };
        let product = &catalog.products[0];
        assert_eq!(catalog.products.len(), 1);
        // Relevance is available from the Catalog before fetching any map snapshot.
        let advertised = product.metadata.as_ref().unwrap();
        assert_eq!(advertised.schema, MAP_CATALOG_SCHEMA);
        let relevance: MapCatalogData = serde_json::from_value(advertised.value.clone()).unwrap();
        assert_eq!(relevance.portals.len(), 1);
        assert!(relevance.contains_payload(&anchor.payload));
        assert_eq!(
            advertised.source_sequence,
            map.snapshot_reference().sequence
        );

        let target = catalog
            .components
            .iter()
            .find(|c| c.manifest.component_id == "qr-map")
            .unwrap()
            .manifest
            .reference();
        let mut subscription = client
            .subscribe_product_exact::<MapSnapshot>(
                server.peer_id(),
                route.clone(),
                product.manifest.reference(),
                ObservationStart::LatestExisting,
                BufferLimits::entries(2),
                MapSnapshot::encoded_size,
            )
            .await
            .unwrap();
        let RemoteObservationEvent::Observation(first) =
            subscription.next().await.unwrap().unwrap()
        else {
            panic!("expected snapshot");
        };
        first.payload.validate().unwrap();
        assert_eq!(first.payload.scenegraph.anchors["qr-123"], anchor);
        let lookup: Invocation<LookupQrResult> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                LOOKUP_QR,
                "localizer",
                "lookup",
                Some(Duration::from_secs(2)),
                &LookupQr {
                    anchor_id: "qr-123".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(lookup.result.snapshot.sequence, first.sequence);
        assert_eq!(lookup.result.snapshot.product, product.manifest.reference());
        assert_eq!(lookup.result.anchor, Some(anchor.clone()));
        let found_by_payload: Invocation<FindQrResult> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                FIND_QR,
                "localizer",
                "find-payload",
                Some(Duration::from_secs(2)),
                &FindQr {
                    payload: anchor.payload.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(found_by_payload.result.anchors, vec![anchor.clone()]);
        assert_eq!(found_by_payload.result.snapshot, lookup.result.snapshot);
        let mut moved = anchor;
        moved.pose_in_map.translation = [2.0, 0.0, 0.0];
        let update = UpsertQr {
            expected_snapshot: lookup.result.snapshot,
            anchor: moved.clone(),
        };
        assert!(
            client
                .invoke_exact::<_, SnapshotReference>(
                    server.peer_id(),
                    route.clone(),
                    target.clone(),
                    UPSERT_QR,
                    "localizer",
                    "denied",
                    Some(Duration::from_secs(2)),
                    &update
                )
                .await
                .is_err()
        );
        clock.store(3, Ordering::SeqCst);
        let applied: Invocation<SnapshotReference> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                UPSERT_QR,
                "mapper",
                "move",
                Some(Duration::from_secs(2)),
                &update,
            )
            .await
            .unwrap();
        let RemoteObservationEvent::Observation(second) =
            subscription.next().await.unwrap().unwrap()
        else {
            panic!("expected updated snapshot");
        };
        assert_eq!(second.sequence, applied.result.sequence);
        let CatalogResponse::Snapshot {
            snapshot: updated_catalog,
        } = client
            .catalog_exact(server.peer_id(), route.clone(), Some(catalog.revision))
            .await
            .unwrap()
        else {
            panic!("metadata update must invalidate cached Catalog");
        };
        assert!(updated_catalog.revision > catalog.revision);
        let updated_product = &updated_catalog.products[0];
        assert_eq!(
            updated_product.manifest.reference(),
            product.manifest.reference()
        );
        assert_eq!(
            updated_product.metadata.as_ref().unwrap().source_sequence,
            applied.result.sequence
        );

        assert_eq!(second.payload.scenegraph.anchors["qr-123"], moved);
        assert!(
            second
                .payload
                .usda
                .contains("xformOp:translate = (2, 0, 0)")
        );
        let found: Invocation<LookupQrResult> = client
            .invoke_exact(
                server.peer_id(),
                route.clone(),
                target.clone(),
                LOOKUP_QR,
                "localizer",
                "lookup-moved",
                Some(Duration::from_secs(2)),
                &LookupQr {
                    anchor_id: "qr-123".into(),
                },
            )
            .await
            .unwrap();
        assert_eq!(found.result.anchor, Some(moved));
        assert_eq!(found.result.snapshot, applied.result);
        let missing: Invocation<LookupQrResult> = client
            .invoke_exact(
                server.peer_id(),
                route,
                target,
                LOOKUP_QR,
                "localizer",
                "missing",
                Some(Duration::from_secs(2)),
                &LookupQr {
                    anchor_id: "unknown".into(),
                },
            )
            .await
            .unwrap();
        assert!(missing.result.anchor.is_none());
        assert_eq!(missing.result.snapshot, applied.result);
        subscription.close().await.unwrap();
        endpoint.close().await.unwrap();
        map.close();
        consumer.shutdown().await.unwrap();
        server.shutdown().await.unwrap();
    })
    .await
    .expect("two-peer map acceptance timed out");
}
