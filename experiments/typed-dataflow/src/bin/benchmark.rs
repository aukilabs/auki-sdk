use std::hint::black_box;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(feature = "current-sdk-baseline")]
use auki_datatypes::camera::CameraFrame;
#[cfg(feature = "current-sdk-baseline")]
use auki_session::{CameraFrameHub, CameraFrameSample};
use auki_typed_dataflow_experiment::{
    Buffer, ConnectionOptions, CursorStart, Envelope, InputPort, OutputPort, PumpOptions,
    StaticConnection, StreamPump, connect, connect_buffer, connect_direct_latest_pump,
};

#[derive(Clone, Copy)]
struct Small {
    left: u64,
    right: u64,
}

struct LargeHandle {
    bytes: Arc<[u8]>,
    identity: u64,
}

fn consume_small(checksum: &AtomicU64, envelope: &Envelope<Small>) {
    checksum.fetch_add(
        envelope.payload.left ^ envelope.payload.right ^ envelope.sequence,
        Ordering::Relaxed,
    );
}

#[inline(never)]
fn static_codegen_probe(iterations: u64) -> u64 {
    let mut checksum = 0u64;
    {
        let mut connection = StaticConnection::new(|envelope: &Envelope<Small>| {
            checksum = checksum
                .wrapping_add(envelope.payload.left ^ envelope.payload.right ^ envelope.sequence);
        });
        for iteration in 0..iterations {
            connection.publish(
                iteration,
                Small {
                    left: black_box(iteration),
                    right: iteration.rotate_left(7),
                },
            );
        }
    }
    checksum
}

fn measure(name: &str, iterations: u64, mut operation: impl FnMut(u64)) {
    let started = Instant::now();
    for iteration in 0..iterations {
        operation(black_box(iteration));
    }
    let elapsed = started.elapsed();
    let ns_per_operation = elapsed.as_secs_f64() * 1_000_000_000.0 / iterations as f64;
    let operations_per_second = iterations as f64 / elapsed.as_secs_f64();
    println!("| {name} | {iterations} | {ns_per_operation:.2} | {operations_per_second:.0} |");
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "benchmark drain timed out");
        thread::yield_now();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = std::env::args()
        .skip_while(|argument| argument != "--iterations")
        .nth(1)
        .map(|argument| argument.parse())
        .transpose()?
        .unwrap_or(500_000u64);
    let rustc = Command::new("rustc")
        .arg("--version")
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|_| "unknown rustc".into());
    black_box(static_codegen_probe(black_box(16)));

    println!("# Typed dataflow raw benchmark");
    println!();
    println!("- compiler: `{rustc}`");
    println!(
        "- target: `{}-{}`",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!("- profile: release");
    println!("- iterations: {iterations}");
    println!("- CPU affinity: not pinned");
    println!();
    println!("| case | iterations | ns/publish | publishes/s |");
    println!("|---|---:|---:|---:|");

    let direct_checksum = AtomicU64::new(0);
    let mut direct_sequence = 0u64;
    measure("direct concrete call", iterations, |iteration| {
        let sequence = direct_sequence;
        direct_sequence += 1;
        let envelope = Envelope::new(
            sequence,
            iteration,
            Small {
                left: iteration,
                right: iteration.rotate_left(7),
            },
        );
        consume_small(&direct_checksum, black_box(&envelope));
    });

    let static_checksum = AtomicU64::new(0);
    let mut static_connection = StaticConnection::new(|envelope: &Envelope<Small>| {
        consume_small(&static_checksum, envelope)
    });
    measure("static InlineEvery", iterations, |iteration| {
        static_connection.publish(
            iteration,
            Small {
                left: iteration,
                right: iteration.rotate_left(7),
            },
        );
    });

    let dynamic_checksum = Arc::new(AtomicU64::new(0));
    let input_checksum = Arc::clone(&dynamic_checksum);
    let dynamic_input = InputPort::new("consumer.input.values", move |envelope: &_| {
        consume_small(&input_checksum, envelope);
    });
    let dynamic_output = OutputPort::new("producer.output.values");
    let _dynamic_connection = connect(
        &dynamic_output,
        &dynamic_input,
        ConnectionOptions::InlineEvery,
    )?;
    measure("dynamic InlineEvery", iterations, |iteration| {
        dynamic_output.publish(
            iteration,
            Small {
                left: iteration,
                right: iteration.rotate_left(7),
            },
        );
    });

    let retained_output = OutputPort::new("producer.output.values");
    let retained_buffer = Buffer::new("values.buffer", 1)?;
    let _retention = connect_buffer(&retained_output, &retained_buffer);
    measure("one-entry Buffer append", iterations, |iteration| {
        retained_output.publish(
            iteration,
            Small {
                left: iteration,
                right: iteration.rotate_left(7),
            },
        );
    });

    let large_iterations = iterations.min(50_000);
    let fixture: Arc<[u8]> = Arc::from(vec![11u8; 6 * 1024 * 1024]);
    let fanout_output = OutputPort::new("camera.output.frames");
    let fanout_buffers = (0..8)
        .map(|index| Buffer::new(format!("consumer-{index}.buffer"), 1))
        .collect::<Result<Vec<_>, _>>()?;
    let _fanout_connections = fanout_buffers
        .iter()
        .map(|buffer| connect_buffer(&fanout_output, buffer))
        .collect::<Vec<_>>();
    measure(
        "8-way Buffer fan-out, 6 MiB handle",
        large_iterations,
        |iteration| {
            fanout_output.publish(
                iteration,
                LargeHandle {
                    bytes: Arc::clone(&fixture),
                    identity: iteration,
                },
            );
        },
    );
    let retained_identities = fanout_buffers
        .iter()
        .map(|buffer| {
            let entry = buffer.snapshot(0, u64::MAX).pop().unwrap();
            (
                entry.payload.bytes.as_ptr() as usize,
                entry.payload.identity,
            )
        })
        .collect::<Vec<_>>();
    assert!(
        retained_identities
            .iter()
            .all(|identity| *identity == retained_identities[0])
    );

    #[cfg(feature = "current-sdk-baseline")]
    {
        let current_frame = Arc::new(CameraFrame {
            dynamic_intrinsics: None,
            frame: vec![11u8; 6 * 1024 * 1024],
        });
        let current_hub = CameraFrameHub::new(1);
        let _subscribers = (0..8).map(|_| current_hub.subscribe()).collect::<Vec<_>>();
        measure(
            "current CameraFrameHub, 8 stalled subscribers",
            large_iterations,
            |iteration| {
                current_hub.publish(CameraFrameSample {
                    timestamp_ns: iteration as i64,
                    frame: Arc::clone(&current_frame),
                });
            },
        );
    }

    #[cfg(not(feature = "current-sdk-baseline"))]
    println!("| current CameraFrameHub | 0 | not run | enable `current-sdk-baseline` |");

    let pump_iterations = iterations.min(100_000);
    let direct_output = OutputPort::new("direct.output.values");
    let direct_recipient = Buffer::new("direct.remote.buffer", 1)?;
    let direct_pump = connect_direct_latest_pump(&direct_output, &direct_recipient, Duration::ZERO);
    measure("direct Latest pump publish", pump_iterations, |iteration| {
        direct_output.publish(
            iteration,
            Small {
                left: iteration,
                right: iteration,
            },
        );
    });
    wait_until(Duration::from_secs(2), || {
        direct_pump.stats().delivered + direct_pump.stats().replaced >= pump_iterations
    });

    let buffered_output = OutputPort::new("buffered.output.values");
    let source = Buffer::new("one-entry.source.buffer", 1)?;
    let buffered_recipient = Buffer::new("buffered.remote.buffer", 1)?;
    let _source_connection = connect_buffer(&buffered_output, &source);
    let buffered_pump = StreamPump::start(
        &source,
        CursorStart::Latest,
        &buffered_recipient,
        PumpOptions::default(),
    )?;
    measure(
        "one-entry Buffer then pump publish",
        pump_iterations,
        |iteration| {
            buffered_output.publish(
                iteration,
                Small {
                    left: iteration,
                    right: iteration,
                },
            );
        },
    );
    wait_until(Duration::from_secs(2), || {
        buffered_pump.stats().delivered_sequence == Some(pump_iterations - 1)
    });

    black_box(direct_checksum.load(Ordering::Relaxed));
    black_box(static_checksum.load(Ordering::Relaxed));
    black_box(dynamic_checksum.load(Ordering::Relaxed));
    println!();
    println!(
        "Direct pump replacements: {}; Buffer cursor skipped entries: {} in {} gap events",
        direct_pump.stats().replaced,
        buffered_pump.stats().source_gap_entries,
        buffered_pump.stats().source_gap_events,
    );
    println!(
        "Large fan-out shared one byte allocation and one envelope allocation across all eight Buffers."
    );

    Ok(())
}
