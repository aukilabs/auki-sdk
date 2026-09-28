//! Authenticated loopback only. No DDS, DMS, discovery, or shared relay booking.
#![cfg(not(target_arch = "wasm32"))]
#[path = "../../../auki-scenegraph/tests/support/mod.rs"]
mod support;
use auki_collaborative_mapping::{DemoMap, PublishedMap, discover_map};
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart,
    RemoteObservationEvent, RemoteProductSubscription,
};
use auki_components::BufferLimits;
use auki_scenegraph::{MapSnapshot, component::SnapshotReference};
use auki_sdk::{AukiPeer, Identity};
use std::time::Duration;
use uuid::Uuid;

fn place(model: &mut DemoMap, name: &str, x: f64, y: f64) {
    let frame = model.view().unwrap().display_frame;
    model.place(name, x, y, &frame).unwrap();
}
async fn subscribe(
    reader: &AukiPeer,
    owner: &AukiPeer,
    model: &mut DemoMap,
) -> RemoteProductSubscription<MapSnapshot> {
    let client = ComponentProtocolClient::new(reader.protocols());
    let route = owner.listen_addresses()[0].clone();
    let CatalogResponse::Snapshot { snapshot } = client
        .catalog_exact(owner.peer_id(), route.clone(), None)
        .await
        .unwrap()
    else {
        panic!("Catalog missing")
    };
    assert_eq!(snapshot.products.len(), 1);
    let domain = model
        .publication()
        .snapshot
        .scenegraph
        .map
        .domain_reference
        .unwrap();
    let product = discover_map(&snapshot, &owner.peer_id().to_string(), &domain, "demo")
        .unwrap()
        .unwrap();
    model.select_partner(product.clone()).unwrap();
    client
        .subscribe_product_exact(
            owner.peer_id(),
            route,
            product,
            ObservationStart::LatestExisting,
            BufferLimits::entries(1),
            MapSnapshot::encoded_size,
        )
        .await
        .unwrap()
}
async fn receive(subscription: &mut RemoteProductSubscription<MapSnapshot>, model: &mut DemoMap) {
    loop {
        match subscription.next().await.unwrap().unwrap() {
            RemoteObservationEvent::Observation(observation) => {
                model
                    .receive(PublishedMap {
                        reference: SnapshotReference {
                            product: subscription.product().reference(),
                            sequence: observation.sequence,
                        },
                        snapshot: (*observation.payload).clone(),
                    })
                    .unwrap();
                return;
            }
            RemoteObservationEvent::Gap(_) => continue,
            RemoteObservationEvent::Closed(_) => panic!("Unexpected closed Product"),
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authenticated_exchange_resubscription_and_idle_shutdown() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let domain = Uuid::new_v4();
        let ia = Identity::generate();
        let ib = Identity::generate();
        let (pa, _aa) = AukiPeer::start_external(
            ia.clone(),
            support::authority(&ia, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let (pb, _ab) = AukiPeer::start_external(
            ib.clone(),
            support::authority(&ib, domain),
            support::direct_config(),
        )
        .await
        .unwrap();
        let mut a =
            DemoMap::new(pa.peer_id().to_string(), domain.to_string(), "demo".into()).unwrap();
        let mut b =
            DemoMap::new(pb.peer_id().to_string(), domain.to_string(), "demo".into()).unwrap();
        let ea = ComponentProtocolEndpoint::mount(pa.protocols(), a.runtime.clone()).unwrap();
        let eb = ComponentProtocolEndpoint::mount(pb.protocols(), b.runtime.clone()).unwrap();
        ea.export_product(&a.map.product()).unwrap();
        eb.export_product(&b.map.product()).unwrap();
        place(&mut a, "apple", 1., 2.);
        place(&mut b, "banana", 4., 5.);
        let mut sa = subscribe(&pa, &pb, &mut a).await;
        let mut sb = subscribe(&pb, &pa, &mut b).await;
        receive(&mut sa, &mut a).await;
        receive(&mut sb, &mut b).await;
        assert_eq!(a.view().unwrap().state, "separate");
        place(&mut a, "bridge", 0., 0.);
        place(&mut b, "bridge", 10., 20.);
        receive(&mut sa, &mut a).await;
        receive(&mut sb, &mut b).await;
        assert_eq!(a.view().unwrap().state, "aligned");
        assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
        // Conflicting evidence and removal propagate through the same snapshot stream.
        let af = a.view().unwrap().local_frame;
        let bf = b.view().unwrap().local_frame;
        a.place("bad", 2., 2., &af).unwrap();
        b.place("bad", 99., 99., &bf).unwrap();
        receive(&mut sa, &mut a).await;
        receive(&mut sb, &mut b).await;
        assert_eq!(a.view().unwrap().state, "conflict");
        assert_eq!(b.view().unwrap().state, "conflict");
        a.remove("bad").unwrap();
        receive(&mut sb, &mut b).await;
        assert_eq!(a.view().unwrap().state, "aligned");
        assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
        assert!(
            b.view()
                .unwrap()
                .local_portals
                .iter()
                .any(|p| p.name == "bad")
        );
        // Closing a pending idle read must not lose the cursor or prevent orderly shutdown.
        assert!(
            tokio::time::timeout(Duration::from_millis(25), sa.next())
                .await
                .is_err()
        );
        sa.close().await.unwrap();
        place(&mut b, "after-disconnect", 3., 3.);
        sa = subscribe(&pa, &pb, &mut a).await;
        receive(&mut sa, &mut a).await;
        assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
        sa.close().await.unwrap();
        sb.close().await.unwrap();
        ea.close().await.unwrap();
        eb.close().await.unwrap();
        a.close();
        b.close();
        pa.shutdown().await.unwrap();
        pb.shutdown().await.unwrap();
    })
    .await
    .expect("Exchange and cleanup exceeded deadline");
}
