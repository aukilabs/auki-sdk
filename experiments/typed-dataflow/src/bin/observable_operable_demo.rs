use std::sync::Arc;

use auki_typed_dataflow_experiment::{
    CameraBufferCapture, CameraComponent, InvocationContext, ObservationDelivery,
    ObservationEndReason, ObservationEvent, PeerRuntime, SerializedInMemoryTransport,
    SetResolution, VideoFrame, observation_input,
};

fn rgb8(width: u32, height: u32, value: u8) -> Arc<[u8]> {
    Arc::from(vec![value; width as usize * height as usize * 3])
}

fn print_event(label: &str, event: &ObservationEvent<VideoFrame>) {
    match event {
        ObservationEvent::Observation(observation) => println!(
            "{label} observation: output={} sequence={} resolution={}x{}",
            observation.output.output_id,
            observation.sequence,
            observation.payload.width,
            observation.payload.height
        ),
        ObservationEvent::Ended(end) => match &end.reason {
            ObservationEndReason::Reconfigured { replacement } => println!(
                "{label} subscription ended: output={} reconfigured; replacement={}",
                end.output.output_id,
                replacement
                    .as_ref()
                    .map_or("none", |output| output.output_id.as_str())
            ),
            ObservationEndReason::Failed { reason } => {
                println!("{label} subscription ended: {reason}")
            }
        },
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let peer_a = PeerRuntime::new("peer-a");
    let camera = CameraComponent::new(
        peer_a.peer_id(),
        "front-camera",
        2,
        2,
        peer_a.catalog().clone(),
        ["peer-b".to_owned()],
    )?;
    let first_buffer = CameraBufferCapture::attach(&camera, 8)?;
    let transport = SerializedInMemoryTransport::default();

    let first_input = observation_input(
        "peer-b.first-preview",
        |event: &ObservationEvent<VideoFrame>| print_event("first", event),
    );
    let first_subscription = transport.follow_new(
        &camera.current_output(),
        &first_input,
        ObservationDelivery::inline_every_selected(),
    )?;

    let component_before = camera.component_reference().clone();
    camera.publish_rgb8(10, 2, 2, rgb8(2, 2, 7))?;

    let applied = transport
        .invoke(
            &camera.set_resolution_operable(),
            InvocationContext {
                invocation_id: "resize-1".to_owned(),
                caller_peer_id: "peer-b".to_owned(),
                caller_component_id: "camera-controller".to_owned(),
            },
            SetResolution {
                width: 1,
                height: 1,
                effective_at_timestamp_ns: 20,
            },
        )?
        .result;

    // Reconfiguration ended both subscriptions. The application deliberately
    // attaches a new Buffer and creates a new remote subscription.
    let replacement_buffer = CameraBufferCapture::attach(&camera, 8)?;
    let replacement_input = observation_input(
        "peer-b.replacement-preview",
        |event: &ObservationEvent<VideoFrame>| print_event("replacement", event),
    );
    let replacement_subscription = transport.follow_new(
        &camera.current_output(),
        &replacement_input,
        ObservationDelivery::inline_every_selected(),
    )?;
    camera.publish_rgb8(21, 1, 1, rgb8(1, 1, 9))?;

    let old_product = first_buffer.product();
    let new_product = replacement_buffer.product();
    let latest = transport
        .latest_existing(&new_product)?
        .expect("replacement Buffer has the replacement frame");
    let component_after = camera.component_reference();
    let catalog_component = peer_a.catalog().component("front-camera").unwrap();

    println!(
        "component stable: {} ({})",
        component_before.component_id,
        component_before.manifest_hash == component_after.manifest_hash
    );
    println!(
        "output replaced: {} -> {}",
        applied.previous_output.output_id, applied.replacement_output.output_id
    );
    println!(
        "catalog current output: {}",
        catalog_component.current_outputs["frames"]
            .manifest
            .output_id
    );
    println!(
        "first subscription: {:?}; replacement subscription: {:?}",
        first_subscription.status(),
        replacement_subscription.status()
    );
    println!(
        "latest retained Product observation: output={} sequence={}",
        latest.output.output_id, latest.sequence
    );
    for product in [old_product, new_product] {
        println!(
            "product {} -> output {} ({} observations)",
            product.manifest.product_id,
            product.manifest.producer.output_id,
            product.buffer.range().entries
        );
    }
    let transport_stats = transport.stats();
    println!(
        "serialized transport: {} messages / {} bytes encoded",
        transport_stats.encoded_messages, transport_stats.encoded_bytes
    );

    Ok(())
}
