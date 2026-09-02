use std::sync::{Arc, Mutex};

use auki_typed_dataflow_experiment::{
    ObservationEvent, SerializedInMemoryTransport, observation_input,
};
use auki_typed_dataflow_volume_monitor::{FRAMES_PER_BLOCK, GaugeObservation, VolumePeer};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let peer_a = VolumePeer::new("peer-a")?;
    let peer_b = VolumePeer::new("peer-b")?;
    let received = Arc::new(Mutex::new(Vec::new()));
    let received_input = Arc::clone(&received);
    let input = observation_input(
        "peer-b.input.peer-a-volume",
        move |event: &ObservationEvent<GaugeObservation>| {
            if let ObservationEvent::Observation(observation) = event {
                received_input
                    .lock()
                    .unwrap()
                    .push(observation.payload.value);
            }
        },
    );
    let transport = SerializedInMemoryTransport::default();
    let _remote = peer_a.observe_volume_through(&transport, &input)?;

    peer_a.publish_audio(0, Arc::from(vec![0.5; FRAMES_PER_BLOCK as usize]))?;
    peer_b.publish_audio(0, Arc::from(vec![0.25; FRAMES_PER_BLOCK as usize]))?;
    peer_a.conclude_session(10_000_000)?;
    peer_b.conclude_session(10_000_000)?;

    println!(
        "peer-b received peer-a volume: {:?}",
        received.lock().unwrap()
    );
    println!("serialized transport: {:?}", transport.stats());
    println!(
        "peer-a Catalog Components: {}",
        peer_a.runtime.catalog().components().len()
    );
    println!(
        "peer-a Catalog Products: {}",
        peer_a.runtime.catalog().products().len()
    );
    Ok(())
}
