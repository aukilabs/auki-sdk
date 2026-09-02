use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    ComponentManifest, Exposure, InvocationContext, InvocationOptions, Operable,
    SerializedInMemoryTransport, SharedScheduler,
};

const SAMPLES: usize = 7;

fn context(iteration: u64) -> InvocationContext {
    InvocationContext {
        invocation_id: format!("benchmark-{iteration}"),
        caller_peer_id: "peer-a".to_owned(),
        caller_component_id: "controller".to_owned(),
    }
}

fn measure(iterations: u64, mut operation: impl FnMut(u64)) -> f64 {
    for iteration in 0..iterations.min(5_000) {
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

fn percentile(values: &mut [u64], percentile: usize) -> u64 {
    values.sort_unstable();
    values[(values.len() - 1) * percentile / 100]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = std::env::args()
        .skip_while(|argument| argument != "--iterations")
        .nth(1)
        .map(|argument| argument.parse())
        .transpose()?
        .unwrap_or(100_000_u64);
    let owner = ComponentManifest {
        schema: "benchmark.component/v1".to_owned(),
        peer_id: "peer-a".to_owned(),
        component_id: "counter".to_owned(),
        observables: Vec::new(),
        operables: Vec::new(),
    }
    .reference();
    let checksum = Arc::new(AtomicU64::new(0));
    let operation_checksum = Arc::clone(&checksum);
    let operation = Operable::new(
        "add-one",
        owner,
        Exposure::Cluster,
        |_| true,
        move |_, value: u64| {
            operation_checksum.fetch_add(value, Ordering::Relaxed);
            Ok(value.wrapping_add(1))
        },
    );

    println!("# Operable targeted benchmark");
    println!();
    println!("- compiler: `{}`", rustc_version());
    println!(
        "- target: `{}-{}`",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!("- profile: release");
    println!("- seven samples after warmup; CPU affinity: not pinned");
    println!();
    println!("| case | iterations/sample | median ns/invocation |");
    println!("|---|---:|---:|");

    let direct = measure(iterations, |iteration| {
        black_box(iteration.wrapping_add(1));
    });
    println!("| Handwritten typed operation | {iterations} | {direct:.2} |");

    let local = measure(iterations, |iteration| {
        black_box(operation.invoke(context(iteration), iteration).unwrap());
    });
    println!("| Dynamic local Operable | {iterations} | {local:.2} |");

    let serialized_iterations = iterations.min(20_000);
    let transport = SerializedInMemoryTransport::default();
    let serialized = measure(serialized_iterations, |iteration| {
        black_box(
            transport
                .invoke(&operation, context(iteration), iteration)
                .unwrap(),
        );
    });
    println!("| Serialized in-memory Operable | {serialized_iterations} | {serialized:.2} |");

    let async_iterations = iterations.min(5_000);
    let scheduler = SharedScheduler::new(4)?;
    let dispatcher = scheduler.dispatcher();
    let mut latencies = Vec::with_capacity(async_iterations as usize);
    let started = Instant::now();
    for iteration in 0..async_iterations {
        let invocation_started = Instant::now();
        operation
            .invoke_async(
                context(iteration),
                iteration,
                &dispatcher,
                InvocationOptions::default(),
            )?
            .wait_timeout(Duration::from_secs(1))
            .expect("async invocation timed out")?;
        latencies.push(invocation_started.elapsed().as_nanos() as u64);
    }
    let async_ns = started.elapsed().as_secs_f64() * 1_000_000_000.0 / async_iterations as f64;
    let p50 = percentile(&mut latencies.clone(), 50);
    let p99 = percentile(&mut latencies, 99);
    println!("| Shared-scheduler async Operable | {async_iterations} | {async_ns:.2} |");

    let transport = transport.stats();
    println!();
    println!("Async end-to-end latency: p50 {p50} ns; p99 {p99} ns.");
    println!(
        "Serialized path encoded {} messages / {} bytes and decoded {} messages / {} bytes.",
        transport.encoded_messages,
        transport.encoded_bytes,
        transport.decoded_messages,
        transport.decoded_bytes,
    );
    black_box(checksum.load(Ordering::Relaxed));
    Ok(())
}

fn rustc_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|version| version.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}
