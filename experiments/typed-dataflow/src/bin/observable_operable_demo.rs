use std::sync::Arc;

use auki_typed_dataflow_experiment::{
    CameraBufferRoller, CameraComponent, InvocationContext, ObservationDelivery, ObservationEvent,
    PeerRuntime, SerializedInMemoryTransport, SetResolution, VideoFrame, observation_input,
};

fn rgb8(width: u32, height: u32, value: u8) -> Arc<[u8]> {
    Arc::from(vec![value; width as usize * height as usize * 3])
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
    let buffers = CameraBufferRoller::attach(&camera, 8)?;
    let transport = SerializedInMemoryTransport::default();

    let pinned = observation_input(
        "peer-b.pinned-preview",
        |event: &ObservationEvent<VideoFrame>| match event {
            ObservationEvent::Observation(observation) => println!(
                "pinned observation: output={} sequence={} resolution={}x{}",
                observation.output.output_id,
                observation.sequence,
                observation.payload.width,
                observation.payload.height
            ),
            ObservationEvent::Reconfigured(transition) => println!(
                "pinned transition: {} -> {}",
                transition.previous.output_id, transition.replacement.output_id
            ),
        },
    );
    let pinned_observation = camera
        .current_output()
        .follow_new(&pinned, ObservationDelivery::inline_every_selected())?;

    let following = observation_input(
        "peer-b.following-preview",
        |event: &ObservationEvent<VideoFrame>| match event {
            ObservationEvent::Observation(observation) => println!(
                "following observation: output={} sequence={} resolution={}x{}",
                observation.output.output_id,
                observation.sequence,
                observation.payload.width,
                observation.payload.height
            ),
            ObservationEvent::Reconfigured(transition) => println!(
                "following transition: {} -> {}",
                transition.previous.output_id, transition.replacement.output_id
            ),
        },
    );
    let following_observation = transport.follow_new(
        &camera.follow_current_output(),
        &following,
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

    camera.publish_rgb8(21, 1, 1, rgb8(1, 1, 9))?;
    let latest = transport
        .latest_existing(&buffers.current())?
        .expect("current Buffer has the replacement frame");
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
        "pinned status: {:?}; follow-current status: {:?}",
        pinned_observation.status(),
        following_observation.status()
    );
    println!(
        "latest retained Product observation: output={} sequence={}",
        latest.output.output_id, latest.sequence
    );
    for product in buffers.products() {
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
