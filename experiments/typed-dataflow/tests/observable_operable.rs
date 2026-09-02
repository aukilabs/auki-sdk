use std::sync::{Arc, Mutex};

use auki_typed_dataflow_experiment::{
    CameraBufferCapture, CameraComponent, CatalogError, InMemoryTransport, InvocationContext,
    InvocationError, ObservationDelivery, ObservationEndReason, ObservationEvent,
    ObservationStatus, PeerRuntime, ReseedDriver, SetResolution, observation_input,
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
fn subscription_ends_at_reconfiguration_and_replacement_requires_resubscription() {
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

    let first_events = Arc::new(Mutex::new(Vec::new()));
    let first_sink = Arc::clone(&first_events);
    let first_input = observation_input("first-preview", move |event| {
        first_sink.lock().unwrap().push(event.clone());
    });
    let subscription = InMemoryTransport
        .follow_new(
            &camera.current_output(),
            &first_input,
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

    let received = first_events.lock().unwrap();
    assert_eq!(received.len(), 2);
    match &received[0] {
        ObservationEvent::Observation(observation) => {
            assert_eq!(observation.output, output_before);
            assert_eq!(Arc::as_ptr(&observation.payload.bytes), first_storage);
        }
        other => panic!("expected initial observation, got {other:?}"),
    }
    match &received[1] {
        ObservationEvent::Ended(end) => {
            assert_eq!(end.output, output_before);
            assert_eq!(end.last_sequence, Some(0));
            assert!(matches!(
                end.reason,
                ObservationEndReason::Reconfigured {
                    replacement: Some(ref replacement)
                } if replacement == &output_after
            ));
        }
        other => panic!("expected terminal reconfiguration notice, got {other:?}"),
    }
    drop(received);
    assert!(matches!(
        subscription.status(),
        ObservationStatus::Ended(ref end)
            if matches!(end.reason, ObservationEndReason::Reconfigured { .. })
    ));

    let replacement_events = Arc::new(Mutex::new(Vec::new()));
    let replacement_sink = Arc::clone(&replacement_events);
    let replacement_input = observation_input("replacement-preview", move |event| {
        replacement_sink.lock().unwrap().push(event.clone());
    });
    let _replacement = InMemoryTransport
        .follow_new(
            &camera.current_output(),
            &replacement_input,
            ObservationDelivery::inline_every_selected(),
        )
        .unwrap();
    camera
        .publish_rgb8(22, 1, 1, frame_bytes(1, 1, 10))
        .unwrap();

    let replacement = replacement_events.lock().unwrap();
    assert_eq!(replacement.len(), 1);
    match &replacement[0] {
        ObservationEvent::Observation(observation) => {
            assert_eq!(observation.output, output_after);
            assert_eq!(observation.sequence, 1);
            assert_eq!(observation.payload.width, 1);
            assert_eq!(observation.payload.height, 1);
        }
        other => panic!("expected replacement observation, got {other:?}"),
    }
}

#[test]
fn buffer_subscription_ends_and_replacement_buffer_requires_explicit_attachment() {
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
    let old_capture = CameraBufferCapture::attach(&camera, 8).unwrap();
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
    assert!(matches!(
        old_capture.end_notice(),
        Some(ref end)
            if matches!(end.reason, ObservationEndReason::Reconfigured { .. })
    ));
    let new_capture = CameraBufferCapture::attach(&camera, 8).unwrap();
    camera.publish_rgb8(21, 1, 1, frame_bytes(1, 1, 2)).unwrap();

    assert!(old_capture.errors().is_empty());
    assert!(new_capture.errors().is_empty());
    let old = old_capture.product();
    let new = new_capture.product();
    assert_eq!(old.manifest.producer, output_before);
    assert_eq!(new.manifest.producer, output_after);
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
