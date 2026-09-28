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
    let old_frame = b.view().unwrap().display_frame;
    place(&mut a, "bridge", 0., 0.);
    place(&mut b, "bridge", 1., 1.);
    exchange(&mut a, &mut b);
    assert!(b.place("oops", 0., 0., &old_frame).is_err());
    b.close();
    b.close();
    let frame = b.view().unwrap().display_frame;
    assert!(b.place("oops", 0., 0., &frame).is_err());
    assert!(b.receive(a.publication()).is_err());
}
