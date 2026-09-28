//! Host-orchestrated import over real authenticated loopback peers; no live services.
#![cfg(all(feature = "components", not(target_arch = "wasm32")))]
mod support;
use auki_component_protocol::{
    CatalogResponse, ComponentProtocolClient, ComponentProtocolEndpoint, ObservationStart,
    RemoteObservationEvent,
};
use auki_components::{BufferLimits, ComponentRuntime, InMemoryTransport, InvocationContext};
use auki_datatypes::pose::{Quat, SpatialTransform, Vec3};
use auki_geometry::{compose_spatial_transforms, inverse_spatial_transform};
use auki_scenegraph::{alignment::*, catalog::*, component::*, *};
use auki_sdk::{AukiPeer, Identity};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use uuid::Uuid;

// The host supplies explicitly labelled transforms to the geometry primitives.
fn numeric(t: &RigidTransform) -> SpatialTransform {
    let [x, y, z] = t.translation;
    let [w, qx, qy, qz] = t.rotation_wxyz;
    SpatialTransform {
        translation: Some(Vec3 { x, y, z }),
        orientation: Some(Quat {
            x: qx,
            y: qy,
            z: qz,
            w,
        }),
    }
}
fn labelled(t: SpatialTransform, from: &str, to: &str) -> RigidTransform {
    let p = t.translation.unwrap();
    let q = t.orientation.unwrap();
    RigidTransform {
        from_frame_id: from.into(),
        to_frame_id: to.into(),
        translation: [p.x, p.y, p.z],
        rotation_wxyz: [q.w, q.x, q.y, q.z],
    }
}
fn inverse(t: &RigidTransform) -> RigidTransform {
    labelled(
        inverse_spatial_transform(&numeric(t)).unwrap(),
        &t.to_frame_id,
        &t.from_frame_id,
    )
}
fn compose(
    first: &RigidTransform,
    second: &RigidTransform,
) -> Result<RigidTransform, &'static str> {
    if first.to_frame_id != second.from_frame_id {
        return Err("disconnected frames");
    }
    Ok(labelled(
        compose_spatial_transforms(&numeric(first), &numeric(second)).unwrap(),
        &first.from_frame_id,
        &second.to_frame_id,
    ))
}
fn portal_id(id: &str) -> String {
    Uuid::from_u128(match id {
        "A" => 1,
        "B" => 2,
        "C" => 3,
        _ => panic!("unknown fixture portal"),
    })
    .to_string()
}
fn anchor(id: &str, destination: &str, translation: [f64; 3], rotation_wxyz: [f64; 4]) -> QrAnchor {
    QrAnchor {
        anchor_id: portal_id(id),
        payload: format!("auki:fixture:{id}"),
        side_length_m: 0.1,
        pose_in_map: RigidTransform {
            from_frame_id: format!("portal-{id}-frame"),
            to_frame_id: destination.into(),
            translation,
            rotation_wxyz,
        },
    }
}
fn context(runtime: &ComponentRuntime) -> InvocationContext {
    InvocationContext {
        invocation_id: Uuid::new_v4().to_string(),
        caller_peer_id: runtime_peer(runtime),
        caller_component_id: "mapper".into(),
    }
}
fn runtime_peer(runtime: &ComponentRuntime) -> String {
    runtime.catalog().snapshot().components[0]
        .manifest
        .peer_id
        .clone()
}
fn insert(map: &MapComponent, context: InvocationContext, anchor: QrAnchor) {
    InMemoryTransport
        .invoke(
            map.upsert_qr(),
            context,
            UpsertQr {
                expected_snapshot: map.snapshot_reference(),
                anchor,
            },
        )
        .unwrap();
}
fn map(runtime: &ComponentRuntime, owner: String, frame_id: &str, domain: Uuid) -> MapComponent {
    map_with_id(runtime, owner, frame_id, domain, &format!("map-{frame_id}"))
}
fn map_with_id(
    runtime: &ComponentRuntime,
    owner: String,
    frame_id: &str,
    domain: Uuid,
    map_id: &str,
) -> MapComponent {
    let clock = AtomicU64::new(1);
    let mut frame = MapFrame::z_up_meters(
        frame_id,
        "Explicitly coincident with the root Portal's printed axes",
    );
    frame.up_axis = UpAxis::Y;
    MapComponent::new(
        runtime,
        MapComponentConfig {
            component_id: format!("map-{frame_id}"),
            publication_id: Uuid::new_v4().to_string(),
            clock_id: "fixture-clock".into(),
            map: MapDefinition {
                map_id: map_id.into(),
                name: None,
                domain_reference: Some(domain.to_string()),
                frame,
            },
        },
        move || clock.fetch_add(1, Ordering::SeqCst),
        |_| true,
        move |ctx| ctx.caller_peer_id == owner && ctx.caller_component_id == "mapper",
    )
    .unwrap()
}
async fn fetch(reader: &AukiPeer, owner: &AukiPeer, expected: &[&str]) -> MapSnapshot {
    let client = ComponentProtocolClient::new(reader.protocols());
    let route = owner.listen_addresses()[0].clone();
    let CatalogResponse::Snapshot { snapshot: catalog } = client
        .catalog_exact(owner.peer_id(), route.clone(), None)
        .await
        .unwrap()
    else {
        panic!("expected catalog")
    };
    assert_eq!(catalog.products.len(), 1);
    let product = &catalog.products[0];
    let metadata = product.metadata.as_ref().unwrap();
    assert_eq!(metadata.schema, MAP_CATALOG_SCHEMA);
    let data: MapCatalogData = serde_json::from_value(metadata.value.clone()).unwrap();
    assert_eq!(data.portals.len(), expected.len());
    for id in expected {
        assert!(data.contains_payload(&format!("auki:fixture:{id}")));
    }
    let mut subscription = client
        .subscribe_product_exact::<MapSnapshot>(
            owner.peer_id(),
            route,
            product.manifest.reference(),
            ObservationStart::LatestExisting,
            BufferLimits::entries(2),
            MapSnapshot::encoded_size,
        )
        .await
        .unwrap();
    let RemoteObservationEvent::Observation(observation) =
        subscription.next().await.unwrap().unwrap()
    else {
        panic!("expected map")
    };
    assert_eq!(observation.sequence, metadata.source_sequence);
    observation.payload.validate().unwrap();
    let snapshot = (*observation.payload).clone();
    subscription.close().await.unwrap();
    snapshot
}
// Restricted to this fixture's explicitly equal conventions and meter units.
// This is host orchestration, not an automatic merge service.
fn import(
    local: &MapSnapshot,
    remote: &MapSnapshot,
    shared: &str,
    missing: &str,
) -> Result<QrAnchor, &'static str> {
    local.validate().map_err(|_| "invalid local map")?;
    remote.validate().map_err(|_| "invalid remote map")?;
    let lf = &local.scenegraph.map.frame;
    let rf = &remote.scenegraph.map.frame;
    if lf.up_axis != rf.up_axis
        || lf.handedness != rf.handedness
        || lf.meters_per_unit != rf.meters_per_unit
    {
        return Err("explicit convention conversion required");
    }
    let l = local
        .scenegraph
        .anchors
        .get(&portal_id(shared))
        .ok_or("no local bridge")?;
    let r = remote
        .scenegraph
        .anchors
        .get(&portal_id(shared))
        .ok_or("no remote bridge")?;
    if l.anchor_id != r.anchor_id || l.payload != r.payload || l.side_length_m != r.side_length_m {
        return Err("incompatible bridge");
    }
    let alignment = compose(&inverse(&r.pose_in_map), &l.pose_in_map)?;
    let mut imported = remote
        .scenegraph
        .anchors
        .get(&portal_id(missing))
        .ok_or("missing anchor")?
        .clone();
    imported.pose_in_map = compose(&imported.pose_in_map, &alignment)?;
    imported
        .validate_in_map(&local.scenegraph.map)
        .map_err(|_| "invalid import")?;
    Ok(imported)
}
fn assert_pose(actual: &QrAnchor, expected: &QrAnchor) {
    assert_eq!(actual.anchor_id, expected.anchor_id);
    assert_eq!(actual.payload, expected.payload);
    assert_eq!(actual.side_length_m, expected.side_length_m);
    assert_eq!(
        actual.pose_in_map.from_frame_id,
        expected.pose_in_map.from_frame_id
    );
    assert_eq!(
        actual.pose_in_map.to_frame_id,
        expected.pose_in_map.to_frame_id
    );
    for (a, e) in actual
        .pose_in_map
        .translation
        .iter()
        .zip(expected.pose_in_map.translation)
    {
        assert!((a - e).abs() < 1e-10, "{a} != {e}");
    }
    let dot: f64 = actual
        .pose_in_map
        .rotation_wxyz
        .iter()
        .zip(expected.pose_in_map.rotation_wxyz)
        .map(|(a, e)| a * e)
        .sum();
    assert!(
        (dot.abs() - 1.).abs() < 1e-10,
        "orientation mismatch: {dot}"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_peers_complete_three_portal_maps_by_exchanging_partial_maps() {
    tokio::time::timeout(Duration::from_secs(30),async {
        let domain=Uuid::new_v4(); let i1=Identity::generate(); let i2=Identity::generate();
        let (p1,_a1)=AukiPeer::start_external(i1.clone(),support::authority(&i1,domain),support::direct_config()).await.unwrap();
        let (p2,_a2)=AukiPeer::start_external(i2.clone(),support::authority(&i2,domain),support::direct_config()).await.unwrap();
        let r1=ComponentRuntime::new(p1.peer_id().to_string()); let r2=ComponentRuntime::new(p2.peer_id().to_string());
        let m1=map(&r1,p1.peer_id().to_string(),"portal-A-frame",domain);
        let m2=map(&r2,p2.peer_id().to_string(),"portal-B-frame",domain);
        let c1=context(&r1); let c2=context(&r2);
        let h=std::f64::consts::FRAC_1_SQRT_2;
        // Independent analytic ground truth: B is +90deg Z at (2,3,1) in A;
        // C is +90deg X at (5,4,2) in A. Neither pose is a translation-only case.
        let a1=anchor("A","portal-A-frame",[0.,0.,0.],[1.,0.,0.,0.]);
        let b1=anchor("B","portal-A-frame",[2.,3.,1.],[h,0.,0.,h]);
        let c1_expected=anchor("C","portal-A-frame",[5.,4.,2.],[h,h,0.,0.]);
        let a2_expected=anchor("A","portal-B-frame",[-3.,2.,-1.],[h,0.,0.,-h]);
        let b2=anchor("B","portal-B-frame",[0.,0.,0.],[1.,0.,0.,0.]);
        let c2_initial=anchor("C","portal-B-frame",[1.,-3.,1.],[0.5,0.5,-0.5,-0.5]);
        insert(&m1,c1.clone(),a1.clone()); insert(&m1,c1.clone(),b1.clone());
        insert(&m2,c2.clone(),b2.clone()); insert(&m2,c2.clone(),c2_initial.clone());
        let e1=ComponentProtocolEndpoint::mount(p1.protocols(),r1).unwrap(); e1.export_product(&m1.product()).unwrap();
        let e2=ComponentProtocolEndpoint::mount(p2.protocols(),r2).unwrap(); e2.export_product(&m2.product()).unwrap();
        // Each peer receives the other's original partial map over authenticated P2P.
        let remote2=fetch(&p1,&p2,&["B","C"]).await;
        let remote1=fetch(&p2,&p1,&["A","B"]).await;
        let local1=m1.product().latest_existing().unwrap().unwrap().payload.clone();
        let local2=m2.product().latest_existing().unwrap().unwrap().payload.clone();
        assert!(compose(&c2_initial.pose_in_map,&b2.pose_in_map).is_ok());
        assert!(compose(&b1.pose_in_map,&c2_initial.pose_in_map).is_err());
        assert!(import(&local1,&remote2,"A","C").is_err());
        let mut checker = MapAlignmentChecker::new(AlignmentOptions { automatic: true, ..Default::default() }).unwrap();
        checker.receive_snapshot("peer1", m1.snapshot_reference(), (*local1).clone()).unwrap();
        let events = checker.receive_snapshot("peer2", m2.snapshot_reference(), remote2.clone()).unwrap();
        assert!(events.iter().any(|e| matches!(e.result, AlignmentResult::Available {..})));
        let AlignmentResult::Available {transform: remote_to_local, path} = checker.check("peer2", "peer1") else {panic!("missing production alignment")};
        assert_eq!(path[0].portal_ids, vec![portal_id("B")]);
        let mut production_c = remote2.scenegraph.anchors[&portal_id("C")].clone();
        production_c.pose_in_map = compose(&production_c.pose_in_map, &remote_to_local).unwrap();
        assert_pose(&production_c, &c1_expected);
        let imported_c=import(&local1,&remote2,"B","C").unwrap();
        let imported_a=import(&local2,&remote1,"B","A").unwrap();
        assert_pose(&imported_c,&c1_expected); assert_pose(&imported_a,&a2_expected);
        insert(&m1,c1,imported_c); insert(&m2,c2,imported_a);
        // Fetch the newly advertised complete maps from the opposite peer.
        let complete1=fetch(&p2,&p1,&["A","B","C"]).await;
        let complete2=fetch(&p1,&p2,&["A","B","C"]).await;
        for expected in [a1,b1,c1_expected] { assert_pose(&complete1.scenegraph.anchors[&expected.anchor_id],&expected); }
        for expected in [a2_expected,b2,c2_initial] { assert_pose(&complete2.scenegraph.anchors[&expected.anchor_id],&expected); }
        assert_eq!(complete1.scenegraph.anchors.len(),3); assert_eq!(complete2.scenegraph.anchors.len(),3);
        println!("Peer1 A-frame: A=(0,0,0), B=(2,3,1), C=(5,4,2); Peer2 B-frame: A=(-3,2,-1), B=(0,0,0), C=(1,-3,1). All frame IDs and orientations verified.");
        e1.close().await.unwrap(); e2.close().await.unwrap(); m1.close(); m2.close();
        p1.shutdown().await.unwrap(); p2.shutdown().await.unwrap();
    }).await.expect("portal exchange timed out");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn peer_retrieves_two_remote_maps_aligns_and_advertises_a_new_merged_map() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let domain = Uuid::new_v4();
        let i1 = Identity::generate();
        let i2 = Identity::generate();
        let (p1, _a1) = AukiPeer::start_external(i1.clone(), support::authority(&i1, domain), support::direct_config()).await.unwrap();
        let (p2, _a2) = AukiPeer::start_external(i2.clone(), support::authority(&i2, domain), support::direct_config()).await.unwrap();
        let r1 = ComponentRuntime::new(p1.peer_id().to_string());
        let r2 = ComponentRuntime::new(p2.peer_id().to_string());
        assert!(r1.catalog().snapshot().products.is_empty());
        let ab = map(&r2, p2.peer_id().to_string(), "portal-A-frame", domain);
        let bc = map(&r2, p2.peer_id().to_string(), "portal-B-frame", domain);
        let writer = context(&r2);
        let h = std::f64::consts::FRAC_1_SQRT_2;
        let a = anchor("A", "portal-A-frame", [0.; 3], [1., 0., 0., 0.]);
        let b = anchor("B", "portal-A-frame", [2., 3., 1.], [h, 0., 0., h]);
        let c_expected = anchor("C", "portal-A-frame", [5., 4., 2.], [h, h, 0., 0.]);
        insert(&ab, writer.clone(), a.clone());
        insert(&ab, writer.clone(), b.clone());
        insert(&bc, writer.clone(), anchor("B", "portal-B-frame", [0.; 3], [1., 0., 0., 0.]));
        insert(&bc, writer, anchor("C", "portal-B-frame", [1., -3., 1.], [0.5, 0.5, -0.5, -0.5]));
        let original_ab = ab.snapshot_reference();
        let original_bc = bc.snapshot_reference();
        let e2 = ComponentProtocolEndpoint::mount(p2.protocols(), r2).unwrap();
        e2.export_product(&ab.product()).unwrap();
        e2.export_product(&bc.product()).unwrap();
        let client = ComponentProtocolClient::new(p1.protocols());
        let route = p2.listen_addresses()[0].clone();
        let CatalogResponse::Snapshot { snapshot: catalog } = client.catalog_exact(p2.peer_id(), route.clone(), None).await.unwrap() else { panic!("expected catalog") };
        assert_eq!(catalog.products.len(), 2);
        let mut checker = MapAlignmentChecker::new(AlignmentOptions { automatic: true, ..Default::default() }).unwrap();
        let mut advertised = std::collections::BTreeMap::new();
        for entry in &catalog.products {
            let metadata = entry.metadata.as_ref().unwrap();
            assert_eq!(metadata.schema, MAP_CATALOG_SCHEMA);
            let data: MapCatalogData = serde_json::from_value(metadata.value.clone()).unwrap();
            assert_eq!(data.portals.len(), 2);
            let key = data.map.map_id.clone();
            let reference = SnapshotReference { product: entry.manifest.reference(), sequence: metadata.source_sequence };
            checker.receive_catalog(key.clone(), reference.clone(), data).unwrap();
            advertised.insert(key, reference);
        }
        let ab_key = "map-portal-A-frame";
        let bc_key = "map-portal-B-frame";
        assert!(matches!(checker.check(bc_key, ab_key), AlignmentResult::PotentialConnection { .. }));
        let mut received = std::collections::BTreeMap::new();
        let mut available_event = false;
        for (key, reference) in &advertised {
            let mut subscription = client.subscribe_product_exact::<MapSnapshot>(p2.peer_id(), route.clone(), reference.product.clone(), ObservationStart::LatestExisting, BufferLimits::entries(2), MapSnapshot::encoded_size).await.unwrap();
            let RemoteObservationEvent::Observation(observation) = subscription.next().await.unwrap().unwrap() else { panic!("expected snapshot") };
            assert_eq!(observation.sequence, reference.sequence);
            observation.payload.validate().unwrap();
            let snapshot = (*observation.payload).clone();
            let events = checker.receive_snapshot(key.clone(), reference.clone(), snapshot.clone()).unwrap();
            available_event |= events.iter().any(|e| matches!(e.result, AlignmentResult::Available { .. }));
            received.insert(key.clone(), snapshot);
            subscription.close().await.unwrap();
        }
        assert!(available_event);
        let AlignmentResult::Available { transform, path } = checker.check(bc_key, ab_key) else { panic!("expected alignment") };
        assert_eq!(transform.from_frame_id, "portal-B-frame");
        assert_eq!(transform.to_frame_id, "portal-A-frame");
        assert_eq!(path.len(), 1);
        assert_eq!(path[0].portal_ids, vec![portal_id("B")]);
        assert_eq!(path[0].from_snapshot, advertised[bc_key]);
        assert_eq!(path[0].to_snapshot, advertised[ab_key]);

        // Host-approved merge into a NEW Peer1-owned Product, explicitly using A's frame.
        let merged = map_with_id(&r1, p1.peer_id().to_string(), "portal-A-frame", domain, "merged-ABC");
        let writer = context(&r1);
        for anchor in received[ab_key].scenegraph.anchors.values() { insert(&merged, writer.clone(), anchor.clone()); }
        for remote in received[bc_key].scenegraph.anchors.values() {
            let mut transformed = remote.clone();
            transformed.pose_in_map = compose(&remote.pose_in_map, &transform).unwrap();
            if let Some(existing) = received[ab_key].scenegraph.anchors.get(&remote.anchor_id) {
                // Shared B must agree, and must not be duplicated or overwritten.
                assert_pose(&transformed, existing);
            } else { insert(&merged, writer.clone(), transformed); }
        }
        let e1 = ComponentProtocolEndpoint::mount(p1.protocols(), r1).unwrap();
        e1.export_product(&merged.product()).unwrap();
        assert_eq!(merged.product().reference().peer_id, p1.peer_id().to_string());
        assert_ne!(merged.product().reference(), original_ab.product);
        assert_ne!(merged.product().reference(), original_bc.product);
        // Peer2 independently sees Peer1's new catalog membership and fetches the result.
        let complete = fetch(&p2, &p1, &["A", "B", "C"]).await;
        assert_eq!(complete.scenegraph.anchors.len(), 3);
        assert_eq!(complete.scenegraph.map.map_id, "merged-ABC");
        for expected in [a, b, c_expected] { assert_pose(&complete.scenegraph.anchors[&expected.anchor_id], &expected); }
        // Importing did not change either original publication or its catalog membership.
        assert_eq!(ab.snapshot_reference(), original_ab);
        assert_eq!(bc.snapshot_reference(), original_bc);
        let CatalogResponse::Snapshot { snapshot: unchanged } = client.catalog_exact(p2.peer_id(), route, None).await.unwrap() else { panic!("expected catalog") };
        assert_eq!(unchanged.revision, catalog.revision);
        assert_eq!(unchanged.products.len(), 2);
        for entry in unchanged.products {
            let data: MapCatalogData = serde_json::from_value(entry.metadata.unwrap().value).unwrap();
            assert_eq!(data.portals.len(), 2);
        }
        println!("Peer2 catalog: [A,B], [B,C]. Peer1 discovered alignment through B and published [A,B,C]. Remote catalog, positions, orientations, frame labels, deduplication and unchanged source maps verified.");
        e1.close().await.unwrap(); e2.close().await.unwrap();
        merged.close(); ab.close(); bc.close();
        p1.shutdown().await.unwrap(); p2.shutdown().await.unwrap();
    }).await.expect("remote map merge timed out");
}
