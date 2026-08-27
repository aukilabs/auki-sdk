use std::sync::{Arc, Mutex};

use auki_typed_dataflow_experiment::{
    CameraBufferRoller, CameraComponent, CatalogError, InMemoryTransport, InvocationContext,
    InvocationError, ObservationDelivery, ObservationEvent, PeerRuntime, ReseedDriver,
    SetResolution, observation_input,
};

fn frame_bytes(width: u32, height: u32, value: u8) -> Arc<[u8]> {
    Arc::from(vec![value; width as usize * height as usize * 3])
}

fn invocation(peer: &str, component: &str, id: &str) -> InvocationContext {
    InvocationContext {
        invocation_id: id.to_owned(),
        caller_peer_id: peer.to_owned(),
        caller_component_id: component.to_owned(),
    }
}

#[test]
fn resolution_change_replaces_output_not_component() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();

    let component_before = camera.component_reference().clone();
    let output_before = camera.current_output_reference();
    let manifest_before = camera.current_output_manifest();

    let result = InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            invocation("peer-b", "camera-controller", "resize-1"),
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: 20,
            },
        )
        .unwrap()
        .result;

    let component_after = camera.component_reference().clone();
    let output_after = camera.current_output_reference();
    let manifest_after = camera.current_output_manifest();

    assert!(result.changed);
    assert_eq!(component_before, component_after);
    assert_eq!(result.component, component_before);
    assert_eq!(result.previous_output, output_before);
    assert_eq!(result.replacement_output, output_after);
    assert_eq!(
        output_before.component_manifest_hash,
        component_before.manifest_hash
    );
    assert_eq!(
        output_after.component_manifest_hash,
        component_after.manifest_hash
    );
    assert_ne!(output_before.output_id, output_after.output_id);
    assert_ne!(output_before.manifest_hash, output_after.manifest_hash);
    assert_eq!(manifest_before.payload.width, Some(2));
    assert_eq!(manifest_before.payload.height, Some(2));
    assert_eq!(manifest_after.payload.width, Some(1));
    assert_eq!(manifest_after.payload.height, Some(1));

    let catalog_component = peer_a.catalog().component("front-camera").unwrap();
    assert_eq!(
        catalog_component.manifest_hash,
        component_before.manifest_hash
    );
    let catalog_output = catalog_component.current_outputs.get("frames").unwrap();
    assert_eq!(catalog_output.manifest.reference(), output_after);
}

#[test]
fn pinned_observation_stops_at_transition_while_follow_current_crosses_it() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    let output_before = camera.current_output_reference();

    let pinned_events = Arc::new(Mutex::new(Vec::new()));
    let pinned_sink = Arc::clone(&pinned_events);
    let pinned_input = observation_input("pinned-preview", move |event| {
        pinned_sink.lock().unwrap().push(event.clone());
    });
    let _pinned = camera
        .current_output()
        .follow_new(&pinned_input, ObservationDelivery::inline_every_selected())
        .unwrap();

    let following_events = Arc::new(Mutex::new(Vec::new()));
    let following_sink = Arc::clone(&following_events);
    let following_input = observation_input("following-preview", move |event| {
        following_sink.lock().unwrap().push(event.clone());
    });
    let _following = InMemoryTransport
        .follow_new(
            &camera.follow_current_output(),
            &following_input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();

    let first = camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 7)).unwrap();
    let first_storage = Arc::as_ptr(&first.payload.bytes);

    let applied = InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            invocation("peer-b", "camera-controller", "resize-2"),
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: 20,
            },
        )
        .unwrap()
        .result;
    let output_after = applied.replacement_output.clone();
    camera.publish_rgb8(21, 1, 1, frame_bytes(1, 1, 9)).unwrap();

    let pinned = pinned_events.lock().unwrap();
    assert_eq!(pinned.len(), 2);
    match &pinned[0] {
        ObservationEvent::Observation(observation) => {
            assert_eq!(observation.output, output_before);
            assert_eq!(Arc::as_ptr(&observation.payload.bytes), first_storage);
        }
        other => panic!("expected initial observation, got {other:?}"),
    }
    match &pinned[1] {
        ObservationEvent::Reconfigured(transition) => {
            assert_eq!(transition.previous, output_before);
            assert_eq!(transition.replacement, output_after);
            assert_eq!(transition.previous_last_sequence, Some(0));
        }
        other => panic!("expected explicit reconfiguration, got {other:?}"),
    }

    let following = following_events.lock().unwrap();
    assert_eq!(following.len(), 3);
    assert!(matches!(following[0], ObservationEvent::Observation(_)));
    assert!(matches!(following[1], ObservationEvent::Reconfigured(_)));
    match &following[2] {
        ObservationEvent::Observation(observation) => {
            assert_eq!(observation.output, output_after);
            assert_eq!(observation.sequence, 0);
            assert_eq!(observation.payload.width, 1);
            assert_eq!(observation.payload.height, 1);
        }
        other => panic!("expected replacement observation, got {other:?}"),
    }
}

#[test]
fn buffer_products_roll_at_output_boundary_without_mixing_contracts() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    let roller = CameraBufferRoller::attach(&camera, 8).unwrap();
    let output_before = camera.current_output_reference();

    let first = camera.publish_rgb8(10, 2, 2, frame_bytes(2, 2, 1)).unwrap();
    let first_storage = Arc::as_ptr(&first.payload.bytes);
    let applied = InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            invocation("peer-b", "camera-controller", "resize-3"),
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: 20,
            },
        )
        .unwrap()
        .result;
    let output_after = applied.replacement_output;
    camera.publish_rgb8(21, 1, 1, frame_bytes(1, 1, 2)).unwrap();

    assert!(roller.errors().is_empty());
    let products = roller.products();
    assert_eq!(products.len(), 2);
    let old = products
        .iter()
        .find(|product| product.manifest.producer == output_before)
        .unwrap();
    let new = products
        .iter()
        .find(|product| product.manifest.producer == output_after)
        .unwrap();
    assert_ne!(old.manifest_hash, new.manifest_hash);
    assert_eq!(old.buffer.range().entries, 1);
    assert_eq!(new.buffer.range().entries, 1);

    let old_observation = old.buffer.snapshot(0, 0).pop().unwrap();
    assert_eq!(old_observation.payload.output, output_before);
    assert_eq!(
        Arc::as_ptr(&old_observation.payload.payload.bytes),
        first_storage
    );
    let new_observation = new.buffer.snapshot(0, 0).pop().unwrap();
    assert_eq!(new_observation.payload.output, output_after);
    assert_eq!(new_observation.payload.payload.width, 1);

    assert_eq!(peer_a.catalog().products().len(), 2);
    assert!(peer_a.catalog().product(&old.manifest.product_id).is_some());
    assert!(peer_a.catalog().product(&new.manifest.product_id).is_some());
}

#[test]
fn local_operable_is_not_discoverable_or_remotely_invocable() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();

    let local = camera.local_reseed_operable();
    let local_result = local
        .invoke(
            invocation("peer-a", "driver-supervisor", "reseed-local"),
            ReseedDriver,
        )
        .unwrap();
    assert_eq!(local_result.result.reset_count, 1);

    let remote = InMemoryTransport.invoke(
        &local,
        invocation("peer-b", "camera-controller", "reseed-remote"),
        ReseedDriver,
    );
    assert_eq!(remote.unwrap_err(), InvocationError::NotExposed);

    let catalog_component = peer_a.catalog().component("front-camera").unwrap();
    assert!(
        catalog_component
            .manifest
            .operables
            .iter()
            .all(|operable| operable.name != "reseed_driver")
    );
}

#[test]
fn unauthorized_remote_caller_cannot_reconfigure_camera() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    let before = camera.current_output_reference();

    let result = InMemoryTransport.invoke(
        &camera.set_resolution_operable(),
        invocation("peer-c", "untrusted-controller", "resize-denied"),
        SetResolution {
            width: 1,
            height: 1,
            effective_at_timestamp_ns: 20,
        },
    );

    assert_eq!(result.unwrap_err(), InvocationError::Unauthorized);
    assert_eq!(camera.current_output_reference(), before);
}

#[test]
fn frame_payload_must_match_current_output_manifest() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();

    assert!(camera.publish_rgb8(1, 2, 2, frame_bytes(2, 2, 0)).is_ok());
    assert!(camera.publish_rgb8(2, 1, 1, frame_bytes(1, 1, 0)).is_err());

    InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            invocation("peer-b", "camera-controller", "resize-4"),
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: 3,
            },
        )
        .unwrap();

    assert!(camera.publish_rgb8(4, 2, 2, frame_bytes(2, 2, 0)).is_err());
    assert!(camera.publish_rgb8(5, 1, 1, frame_bytes(1, 1, 0)).is_ok());
}

#[test]
fn setting_the_existing_resolution_does_not_create_a_new_output() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    let before = camera.current_output_reference();

    let result = InMemoryTransport
        .invoke(
            &camera.set_resolution_operable(),
            invocation("peer-b", "camera-controller", "resize-noop"),
            SetResolution {
                width: 2,
                height: 2,
                effective_at_timestamp_ns: 10,
            },
        )
        .unwrap()
        .result;

    assert!(!result.changed);
    assert_eq!(result.previous_output, before);
    assert_eq!(result.replacement_output, before);
    assert_eq!(camera.current_output_reference(), before);
}

#[test]
fn catalog_rejects_output_that_does_not_pin_its_component_manifest() {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )
    .unwrap();
    let mut invalid = camera.current_output_manifest();
    invalid.output_id = "forged-output".to_owned();
    invalid.component_manifest_hash = "sha256:not-the-component".to_owned();

    let result = peer_a.catalog().set_current_output(invalid);
    assert!(matches!(
        result,
        Err(CatalogError::ComponentManifestMismatch { .. })
    ));
    assert_eq!(
        peer_a
            .catalog()
            .component("front-camera")
            .unwrap()
            .current_outputs["frames"]
            .manifest
            .output_id,
        "frames-1"
    );
}
