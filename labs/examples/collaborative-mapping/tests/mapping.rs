use auki_collaborative_mapping::{DemoMap, PublishedMap, portal_id};
use auki_scenegraph::MapSnapshot;

fn map(peer: &str) -> DemoMap {
    DemoMap::new(peer.into(), "domain".into(), "demo".into()).unwrap()
}
fn place(map: &mut DemoMap, name: &str, x: f64, y: f64) {
    let frame = map.view().unwrap().display_frame;
    map.place(name, x, y, &frame).unwrap();
}
fn pair(a: &mut DemoMap, b: &mut DemoMap) {
    a.select_partner(b.publication().reference.product).unwrap();
    b.select_partner(a.publication().reference.product).unwrap();
}
fn exchange(a: &mut DemoMap, b: &mut DemoMap) {
    let aa = a.publication();
    let bb = b.publication();
    a.receive(bb).unwrap();
    b.receive(aa).unwrap();
}
fn rebuild(
    mut published: PublishedMap,
    change: impl FnOnce(&mut auki_scenegraph::Scenegraph),
) -> PublishedMap {
    change(&mut published.snapshot.scenegraph);
    published.snapshot = MapSnapshot::new(published.snapshot.scenegraph).unwrap();
    published
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn independent_maps_converge_and_preserve_original_placements() {
    let (mut a, mut b) = (map("peer-a"), map("peer-b"));
    pair(&mut a, &mut b);
    place(&mut a, "apple", 1., 2.);
    place(&mut b, "banana", 4., 5.);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().state, "separate");
    assert_eq!(b.view().unwrap().portals.len(), 1);
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 10., 20.);
    let original_a = a.publication();
    let original_b = b.publication();
    exchange(&mut a, &mut b);
    let va = a.view().unwrap();
    let vb = b.view().unwrap();
    assert_eq!(va.state, "aligned");
    assert_eq!(va.portals, vb.portals);
    assert_eq!(va.display_frame, vb.display_frame);
    assert_eq!(va.portals.len(), 3);
    let banana = va.portals.iter().find(|p| p.name == "banana").unwrap();
    assert_eq!((banana.x, banana.y), (-6., -15.));
    assert_eq!(
        va.portals
            .iter()
            .find(|p| p.name == "bridge")
            .unwrap()
            .contributors
            .len(),
        2
    );
    assert_eq!(a.publication().snapshot, original_a.snapshot);
    assert_eq!(b.publication().snapshot, original_b.snapshot);
    // Place in the shared view on non-canonical peer B; its original frame gets inverse translation.
    place(&mut b, "cafe", 2., 3.);
    let anchor = &b.publication().snapshot.scenegraph.anchors[&portal_id("demo", "cafe")];
    assert_eq!(anchor.pose_in_map.translation, [12., 23., 0.]);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn simultaneous_updates_and_replays_converge_without_echoing_imports() {
    let (mut a, mut b) = (map("a"), map("b"));
    pair(&mut a, &mut b);
    place(&mut a, "bridge", 2., 3.);
    place(&mut b, "bridge", -1., -2.);
    exchange(&mut a, &mut b);
    place(&mut a, "alpha", 5., 5.);
    place(&mut b, "beta", 8., 8.);
    let aa = a.publication();
    let bb = b.publication();
    b.receive(aa.clone()).unwrap();
    a.receive(bb.clone()).unwrap();
    b.receive(aa).unwrap();
    a.receive(bb).unwrap();
    assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
    assert_eq!(a.publication().snapshot.scenegraph.anchors.len(), 2);
    assert_eq!(b.publication().snapshot.scenegraph.anchors.len(), 2);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn conflicting_shared_portals_withdraw_alignment_and_recover() {
    let (mut a, mut b) = (map("a"), map("b"));
    pair(&mut a, &mut b);
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 10., 10.);
    exchange(&mut a, &mut b);
    place(&mut a, "second", 2., 2.);
    place(&mut b, "second", 3., 3.);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().state, "conflict");
    assert_eq!(b.view().unwrap().state, "conflict");
    // Conflict UI is back in B's original coordinates.
    place(&mut b, "second", 12., 12.);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().state, "aligned");
    assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn names_are_exact_session_scoped_and_local_duplicates_are_updates() {
    assert_ne!(portal_id("demo", "Bridge"), portal_id("demo", "bridge"));
    assert_ne!(portal_id("other", "bridge"), portal_id("demo", "bridge"));
    assert_ne!(portal_id("ab", "c"), portal_id("a", "bc"));
    let mut a = map("a");
    place(&mut a, "bridge", 0., 0.);
    let reference = a.publication().reference;
    place(&mut a, "bridge", 0., 0.);
    assert_eq!(a.publication().reference, reference);
    place(&mut a, "bridge", 3., 2.);
    assert_eq!(a.view().unwrap().portals.len(), 1);
    let frame = a.view().unwrap().display_frame;
    for name in ["", " bridge", "bridge ", "<script>", "line\nbreak"] {
        assert!(a.place(name, 0., 0., &frame).is_err());
    }
    assert!(a.place("ok", f64::NAN, 0., &frame).is_err());
    assert!(a.place("ok", 0.5, 0., &frame).is_err());
    assert!(a.place("ok", 10001., 0., &frame).is_err());
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn invalid_remote_contracts_cannot_replace_accepted_evidence() {
    let (mut a, mut b) = (map("a"), map("b"));
    pair(&mut a, &mut b);
    place(&mut b, "bridge", 1., 1.);
    a.receive(b.publication()).unwrap();
    let original = a.view().unwrap().remote_reference;
    let changed = rebuild(b.publication(), |s| {
        s.map.domain_reference = Some("wrong-domain".into())
    });
    assert!(a.receive(changed).is_err());
    let changed = rebuild(b.publication(), |s| {
        s.anchors
            .values_mut()
            .next()
            .unwrap()
            .pose_in_map
            .from_frame_id = "wrong-marker-frame".into()
    });
    assert!(a.receive(changed).is_err());
    let changed = rebuild(b.publication(), |s| {
        s.anchors
            .values_mut()
            .next()
            .unwrap()
            .pose_in_map
            .rotation_wxyz = [0., 0., 0., 1.]
    });
    assert!(a.receive(changed).is_err());
    let changed = rebuild(b.publication(), |s| {
        s.anchors.values_mut().next().unwrap().payload = "different-name".into()
    });
    assert!(a.receive(changed).is_err());
    let mut changed = b.publication();
    changed.reference.product.peer_id = "intruder".into();
    assert!(a.receive(changed).is_err());
    let mut changed = b.publication();
    changed.snapshot.usda.push_str("corrupt");
    assert!(a.receive(changed).is_err());
    assert_eq!(a.view().unwrap().remote_reference, original);
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn stale_snapshots_and_unselected_publications_are_rejected() {
    let (mut a, mut b) = (map("a"), map("b"));
    pair(&mut a, &mut b);
    let old = b.publication();
    place(&mut b, "b", 0., 0.);
    a.receive(b.publication()).unwrap();
    assert!(a.receive(old).is_err());
    let mut restarted = map("b");
    place(&mut restarted, "new", 0., 0.);
    assert!(a.receive(restarted.publication()).is_err());
    a.select_partner(restarted.publication().reference.product)
        .unwrap();
    assert!(a.view().unwrap().remote_reference.is_none());
    a.receive(restarted.publication()).unwrap();
    assert_eq!(a.view().unwrap().remote_portals[0].name, "new");
}
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn stale_display_frames_and_edits_after_close_are_rejected() {
    let (mut a, mut b) = (map("a"), map("b"));
    pair(&mut a, &mut b);
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 1., 1.);
    exchange(&mut a, &mut b);
    let old_frame = b.view().unwrap().display_frame;
    // Losing shared evidence invalidates the former combined frame on peer B.
    a.remove("bridge").unwrap();
    exchange(&mut a, &mut b);
    assert!(b.place("oops", 0., 0., &old_frame).is_err());
    b.close();
    b.close();
    let frame = b.view().unwrap().display_frame;
    assert!(b.place("oops", 0., 0., &frame).is_err());
    assert!(b.receive(a.publication()).is_err());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn catalog_discovery_matches_domain_session_and_authenticated_peer() {
    use auki_collaborative_mapping::discover_map;
    let b = map("b");
    let catalog = b.runtime.catalog().snapshot();
    assert_eq!(
        discover_map(&catalog, "b", "domain", "demo").unwrap(),
        Some(b.publication().reference.product)
    );
    assert_eq!(
        discover_map(&catalog, "b", "domain", "other-session").unwrap(),
        None
    );
    assert_eq!(
        discover_map(&catalog, "b", "other-domain", "demo").unwrap(),
        None
    );
    assert!(discover_map(&catalog, "intruder", "domain", "demo").is_err());
    let mut corrupt = catalog.clone();
    corrupt.products[0].manifest_hash = "tampered".into();
    assert!(discover_map(&corrupt, "b", "domain", "demo").is_err());
    let mut no_metadata = catalog.clone();
    no_metadata.products[0].metadata = None;
    assert_eq!(
        discover_map(&no_metadata, "b", "domain", "demo").unwrap(),
        None
    );
    let mut multiple = catalog.clone();
    multiple.products.push(catalog.products[0].clone());
    assert!(discover_map(&multiple, "b", "domain", "demo").is_err());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn discovered_peers_subscribe_both_ways_without_a_shared_portal_first() {
    use auki_collaborative_mapping::discover_map;
    let (mut a, mut b) = (map("a"), map("b"));
    // Both discover the other's empty map: sharing a Portal is not a discovery prerequisite.
    let a_product = discover_map(&a.runtime.catalog().snapshot(), "a", "domain", "demo")
        .unwrap()
        .unwrap();
    let b_product = discover_map(&b.runtime.catalog().snapshot(), "b", "domain", "demo")
        .unwrap()
        .unwrap();
    a.select_partner(b_product).unwrap();
    b.select_partner(a_product).unwrap();
    place(&mut a, "apple", 1., 2.);
    place(&mut b, "banana", 4., 5.);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().remote_portals[0].name, "banana");
    assert_eq!(b.view().unwrap().remote_portals[0].name, "apple");
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 10., 20.);
    exchange(&mut a, &mut b);
    assert_eq!(a.view().unwrap().state, "aligned");
    assert_eq!(a.view().unwrap().portals, b.view().unwrap().portals);
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn conflicting_portals_are_identified_and_local_removal_restores_alignment() {
    let (mut a, mut b) = (map("peer-a"), map("peer-b"));
    pair(&mut a, &mut b);
    for (name, ax, bx) in [("bridge", 0., 10.), ("cafe", 2., 12.), ("bad", 4., 19.)] {
        place(&mut a, name, ax, 0.);
        place(&mut b, name, bx, 20.);
    }
    exchange(&mut a, &mut b);
    let view = a.view().unwrap();
    assert_eq!(view.state, "conflict");
    assert_eq!(view.shared_names.len(), 3);
    let bad = view.conflicts.iter().find(|c| c.name == "bad").unwrap();
    assert_eq!(bad.disagrees_with.len(), 2);
    let bridge = view.conflicts.iter().find(|c| c.name == "bridge").unwrap();
    assert_eq!(bridge.disagrees_with, vec!["bad"]);
    let original_b = b.publication();
    a.remove("bad").unwrap();
    exchange(&mut a, &mut b);
    let va = a.view().unwrap();
    let vb = b.view().unwrap();
    assert_eq!(va.state, "aligned");
    assert!(va.conflicts.is_empty());
    assert_eq!(va.portals, vb.portals);
    assert_eq!(va.local_portals.len(), 2);
    assert_eq!(va.remote_portals.len(), 3);
    assert_eq!(b.publication().snapshot, original_b.snapshot);
    // Local coordinates stay editable even on the noncanonical peer after alignment.
    b.place("local", 25., 26., &vb.local_frame).unwrap();
    let local = b
        .view()
        .unwrap()
        .local_portals
        .into_iter()
        .find(|p| p.name == "local")
        .unwrap();
    assert_eq!((local.x, local.y), (25., 26.));
    a.remove("bridge").unwrap();
    a.remove("cafe").unwrap();
    exchange(&mut a, &mut b);
    assert_eq!(b.view().unwrap().state, "separate");
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn removal_requires_owner_and_current_snapshot_and_updates_catalog() {
    use auki_components::{InMemoryTransport, InvocationContext};
    use auki_scenegraph::component::RemoveQr;
    let mut a = map("peer-a");
    place(&mut a, "bridge", 0., 0.);
    let reference = a.publication().reference;
    let request = RemoveQr {
        expected_snapshot: reference.clone(),
        anchor_id: portal_id("demo", "bridge"),
    };
    let context = |peer: &str| InvocationContext {
        invocation_id: "remove-test".into(),
        caller_peer_id: peer.into(),
        caller_component_id: "grid-editor".into(),
    };
    assert!(
        InMemoryTransport
            .invoke(a.map.remove_qr(), context("peer-b"), request.clone())
            .is_err()
    );
    assert_eq!(a.publication().reference, reference);
    place(&mut a, "cafe", 1., 1.);
    assert!(
        InMemoryTransport
            .invoke(a.map.remove_qr(), context("peer-a"), request)
            .is_err()
    );
    let before = a.publication().reference.sequence;
    a.remove("bridge").unwrap();
    assert_eq!(a.publication().reference.sequence, before + 1);
    let catalog = a.runtime.catalog().snapshot();
    let serialized = serde_json::to_string(&catalog).unwrap();
    assert!(!serialized.contains("\"payload\":\"bridge\""));
    assert!(serialized.contains("\"payload\":\"cafe\""));
    a.remove("bridge").unwrap(); // no-op does not publish a new snapshot
    assert_eq!(a.publication().reference.sequence, before + 1);
    a.close();
    assert!(a.remove("cafe").is_err());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn multi_peer_layers_align_transitively_and_withdraw_on_departure() {
    let (mut a, mut b, mut c) = (map("a"), map("b"), map("c"));
    place(&mut a, "ab", 0., 0.);
    place(&mut b, "ab", 10., 20.);
    place(&mut b, "bc", 12., 23.);
    place(&mut c, "bc", 102., 203.);
    place(&mut c, "c-only", 105., 206.);
    for (receiver, others) in [(&mut a, vec![b.publication(), c.publication()])] {
        for other in others {
            receiver
                .select_partner(other.reference.product.clone())
                .unwrap();
            receiver.receive(other).unwrap();
        }
    }
    for other in [a.publication(), c.publication()] {
        b.select_partner(other.reference.product.clone()).unwrap();
        b.receive(other).unwrap();
    }
    for other in [a.publication(), b.publication()] {
        c.select_partner(other.reference.product.clone()).unwrap();
        c.receive(other).unwrap();
    }
    let av = a.session_view().unwrap();
    assert_eq!(av.layers.len(), 3);
    assert!(av.layers.iter().all(|l| l.to_display.is_some()));
    assert_eq!(av.portals, b.session_view().unwrap().portals);
    assert_eq!(av.portals, c.session_view().unwrap().portals);
    let p = av.portals.iter().find(|p| p.name == "c-only").unwrap();
    assert_eq!((p.x, p.y), (5., 6.));
    // Drop in canonical coordinates, preserving the source frame on C.
    c.place("new", 7., 8., &av.display_frame).unwrap();
    let cp = c
        .session_view()
        .unwrap()
        .layers
        .into_iter()
        .find(|l| l.peer == "c")
        .unwrap();
    let p = cp.portals.iter().find(|p| p.name == "new").unwrap();
    assert_eq!((p.x, p.y), (107., 208.));
    // The bridge disappears: C is still inspectable but cannot be overlaid in A.
    a.forget_peer("b");
    let disconnected = a.session_view().unwrap();
    assert_eq!(disconnected.layers.len(), 2);
    assert!(
        disconnected
            .layers
            .iter()
            .find(|l| l.peer == "c")
            .unwrap()
            .aligned_portals
            .is_none()
    );
    assert_eq!(disconnected.portals.len(), 1);
    // A new publication of B can join without restarting A.
    let mut restarted = map("b");
    place(&mut restarted, "ab", 10., 20.);
    place(&mut restarted, "bc", 12., 23.);
    a.select_partner(restarted.publication().reference.product)
        .unwrap();
    a.receive(restarted.publication()).unwrap();
    assert!(
        a.session_view()
            .unwrap()
            .layers
            .iter()
            .all(|l| l.to_display.is_some())
    );
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn remote_conflicts_and_stale_evidence_do_not_create_false_overlays() {
    let (mut a, mut b, mut c) = (map("a"), map("b"), map("c"));
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 1., 1.);
    place(&mut b, "bad", 2., 2.);
    place(&mut c, "bridge", 10., 10.);
    place(&mut c, "bad", 50., 50.);
    for other in [b.publication(), c.publication()] {
        a.select_partner(other.reference.product.clone()).unwrap();
        a.receive(other).unwrap();
    }
    let view = a.session_view().unwrap();
    assert!(view.conflicts.iter().any(|c| c.peers == ["b", "c"]));
    assert!(
        view.layers
            .iter()
            .filter(|l| l.peer != "a")
            .all(|l| l.to_display.is_none())
    );
    a.clear_evidence("c");
    assert_eq!(a.session_view().unwrap().layers.len(), 2);
    assert!(
        a.session_view()
            .unwrap()
            .layers
            .iter()
            .all(|l| l.to_display.is_some())
    );
    // Reconnected stream remains authorized for its selected Product.
    c.remove("bad").unwrap();
    a.receive(c.publication()).unwrap();
    assert_eq!(a.session_view().unwrap().layers.len(), 3);
    assert!(a.session_view().unwrap().conflicts.is_empty());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn all_axis_presets_align_without_relabelling_source_coordinates() {
    use auki_collaborative_mapping::{Convention, discover_map};
    // Coordinates of screen-plane (10,20) and (13,24) in each preset, independently specified.
    let bridge = [[10., 20.], [20., -10.], [-10., -20.], [-20., 10.]];
    let extra = [[13., 24.], [24., -13.], [-13., -24.], [-24., 13.]];
    let expected = [[3., 4.], [4., -3.], [-3., -4.], [-4., 3.]];
    for (ai, ac) in Convention::ALL.into_iter().enumerate() {
        for (bi, bc) in Convention::ALL.into_iter().enumerate() {
            let mut a =
                DemoMap::with_convention("a".into(), "domain".into(), "demo".into(), ac).unwrap();
            let mut b =
                DemoMap::with_convention("b".into(), "domain".into(), "demo".into(), bc).unwrap();
            let af = a.publication().snapshot.scenegraph.map.frame.id;
            let bf = b.publication().snapshot.scenegraph.map.frame.id;
            a.place("bridge", 0., 0., &af).unwrap();
            b.place("bridge", bridge[bi][0], bridge[bi][1], &bf)
                .unwrap();
            b.place("extra", extra[bi][0], extra[bi][1], &bf).unwrap();
            assert_eq!(
                discover_map(&b.runtime.catalog().snapshot(), "b", "domain", "demo").unwrap(),
                Some(b.publication().reference.product)
            );
            pair(&mut a, &mut b);
            exchange(&mut a, &mut b);
            let av = a.session_view().unwrap();
            let bv = b.session_view().unwrap();
            assert_eq!(av.portals, bv.portals);
            let pin = av.portals.iter().find(|p| p.name == "extra").unwrap();
            assert_eq!([pin.x, pin.y], expected[ai]);
            assert_eq!(
                bv.layers.iter().find(|l| l.peer == "b").unwrap().convention,
                bc
            );
            // Editing through A's displayed frame must apply inverse rotation as well as translation.
            b.place("placed", expected[ai][0], expected[ai][1], &af)
                .unwrap();
            let local = b
                .session_view()
                .unwrap()
                .layers
                .into_iter()
                .find(|l| l.peer == "b")
                .unwrap();
            let pin = local.portals.iter().find(|p| p.name == "placed").unwrap();
            assert_eq!([pin.x, pin.y], extra[bi]);
            a.place("extra", expected[ai][0], expected[ai][1], &af)
                .unwrap();
            exchange(&mut a, &mut b);
            assert!(a.session_view().unwrap().conflicts.is_empty());
            a.place("extra", expected[ai][0] + 1., expected[ai][1], &af)
                .unwrap();
            exchange(&mut a, &mut b);
            assert!(!a.session_view().unwrap().conflicts.is_empty());
            a.remove("extra").unwrap();
            exchange(&mut a, &mut b);
            assert!(
                a.session_view()
                    .unwrap()
                    .layers
                    .iter()
                    .all(|l| l.to_display.is_some())
            );
        }
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn convention_contract_rejects_unknown_axes_and_mismatched_portal_orientation() {
    use auki_collaborative_mapping::Convention;
    let mut a = map("a");
    let mut b =
        DemoMap::with_convention("b".into(), "domain".into(), "demo".into(), Convention::XUp)
            .unwrap();
    place(&mut b, "bridge", 1., 2.);
    pair(&mut a, &mut b);
    assert!(Convention::parse("anything").is_err());
    let invalid = rebuild(b.publication(), |s| {
        s.map.frame.origin_description = "unlabelled axes".into()
    });
    assert!(a.receive(invalid).is_err());
    let invalid = rebuild(b.publication(), |s| {
        s.anchors
            .values_mut()
            .next()
            .unwrap()
            .pose_in_map
            .rotation_wxyz = [1., 0., 0., 0.]
    });
    assert!(a.receive(invalid).is_err());
    a.receive(b.publication()).unwrap();
    assert!(
        a.session_view()
            .unwrap()
            .layers
            .iter()
            .find(|l| l.peer == "b")
            .unwrap()
            .to_display
            .is_none()
    );
}
