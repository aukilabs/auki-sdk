use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use auki_typed_dataflow_experiment::{
    CameraComponent, ObservationDelivery, ObservationEvent, PeerRuntime,
    SerializedInMemoryTransport, VideoFrame, observation_input,
};

const SAMPLES: usize = 7;

fn new_camera() -> CameraComponent {
    let peer = PeerRuntime::new("benchmark-peer");
    CameraComponent::new(
        peer.peer_id(),
        "benchmark-camera",
        1,
        1,
        peer.catalog().clone(),
        std::iter::empty(),
    )
    .unwrap()
}

fn measure(iterations: u64, mut operation: impl FnMut(u64)) -> f64 {
    for iteration in 0..iterations.min(10_000) {
        operation(black_box(iteration));
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for sample in 0..SAMPLES {
        let started = Instant::now();
        for iteration in 0..iterations {
            operation(black_box(iteration + sample as u64 * iterations));
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000_000.0 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[SAMPLES / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = std::env::args()
        .skip_while(|argument| argument != "--iterations")
        .nth(1)
        .map(|argument| argument.parse())
        .transpose()?
        .unwrap_or(100_000_u64);
    let bytes: Arc<[u8]> = Arc::from([1_u8, 2, 3]);

    println!("# Observation request Phase 1 targeted benchmark");
    println!();
    println!("Seven samples after warmup; table reports median nanoseconds per publication.");
    println!();
    println!("| case | iterations/sample | median ns/publication |");
    println!("|---|---:|---:|");

    let camera = new_camera();
    let no_observer = measure(iterations, |timestamp_ns| {
        black_box(
            camera
                .publish_rgb8(timestamp_ns, 1, 1, Arc::clone(&bytes))
                .unwrap(),
        );
    });
    println!("| Camera publication, no observer | {iterations} | {no_observer:.2} |");

    let camera = new_camera();
    let first_checksum = Arc::new(AtomicU64::new(0));
    let sink = Arc::clone(&first_checksum);
    let first_input = observation_input("first-subscription", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            sink.fetch_add(observation.sequence, Ordering::Relaxed);
        }
    });
    let _first = camera
        .current_output()
        .follow_new(&first_input, ObservationDelivery::inline_every_selected())?;
    let one_subscription = measure(iterations, |timestamp_ns| {
        black_box(
            camera
                .publish_rgb8(timestamp_ns, 1, 1, Arc::clone(&bytes))
                .unwrap(),
        );
    });
    println!("| One local subscription | {iterations} | {one_subscription:.2} |");

    let second_checksum = Arc::new(AtomicU64::new(0));
    let sink = Arc::clone(&second_checksum);
    let second_input = observation_input("second-subscription", move |event| {
        if let ObservationEvent::Observation(observation) = event {
            sink.fetch_add(observation.sequence, Ordering::Relaxed);
        }
    });
    let _second = camera
        .current_output()
        .follow_new(&second_input, ObservationDelivery::inline_every_selected())?;
    let two_subscriptions = measure(iterations, |timestamp_ns| {
        black_box(
            camera
                .publish_rgb8(timestamp_ns, 1, 1, Arc::clone(&bytes))
                .unwrap(),
        );
    });
    println!("| Two local subscriptions | {iterations} | {two_subscriptions:.2} |");

    let serialized_iterations = iterations.min(20_000);
    let camera = new_camera();
    let transport = SerializedInMemoryTransport::default();
    let remote_checksum = Arc::new(AtomicU64::new(0));
    let sink = Arc::clone(&remote_checksum);
    let remote_input = observation_input(
        "serialized-remote",
        move |event: &ObservationEvent<VideoFrame>| {
            if let ObservationEvent::Observation(observation) = event {
                sink.fetch_add(observation.sequence, Ordering::Relaxed);
            }
        },
    );
    let _remote = transport.follow_new(
        &camera.current_output(),
        &remote_input,
        ObservationDelivery::inline_every_selected(),
    )?;
    let serialized = measure(serialized_iterations, |timestamp_ns| {
        black_box(
            camera
                .publish_rgb8(timestamp_ns, 1, 1, Arc::clone(&bytes))
                .unwrap(),
        );
    });
    println!("| Serialized in-memory observer | {serialized_iterations} | {serialized:.2} |");

    black_box(first_checksum.load(Ordering::Relaxed));
    black_box(second_checksum.load(Ordering::Relaxed));
    black_box(remote_checksum.load(Ordering::Relaxed));

    let transport = transport.stats();
    println!();
    println!(
        "Serialized path encoded {} messages / {} bytes and decoded {} messages / {} bytes.",
        transport.encoded_messages,
        transport.encoded_bytes,
        transport.decoded_messages,
        transport.decoded_bytes,
    );
    println!(
        "Marginal second local observer in this run: {:.2} ns/publication.",
        two_subscriptions - one_subscription
    );

    Ok(())
}
