use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    Connection, ConnectionOptions, EveryFullPolicy, InputPort, OutputPort, SharedDelivery,
    SharedScheduler, connect, connect_shared,
};

#[derive(Debug)]
struct TimedValue {
    sent_ns: u64,
}

type LatencySet = Arc<Mutex<Vec<u64>>>;

#[derive(Clone, Copy, Debug)]
struct Sample {
    publish_ns: f64,
    completion_ns: f64,
    deliveries_per_second: f64,
    p50_latency_ns: u64,
    p99_latency_ns: u64,
    shutdown_ns: f64,
}

fn main() {
    let samples = argument("--samples").unwrap_or(5);
    let iteration_override = argument("--iterations");
    let worker_count = argument("--workers").unwrap_or(4);
    let relationships = [1usize, 8, 64, 256];

    println!("# Typed dataflow scheduler stress benchmark");
    println!("rustc: {}", rustc_version());
    println!(
        "target: {}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    println!("profile: release recommended");
    println!("samples: {samples}; shared workers: {worker_count}");
    println!();
    println!(
        "| relationships | topology | OS workers by construction | iterations | median publish ns | median completion ns | deliveries/s | p50 latency ns | p99 latency ns | shutdown ns |"
    );
    println!("|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|");

    for count in relationships {
        let iterations = iteration_override.unwrap_or_else(|| default_iterations(count));
        let threaded = repeated(samples, || run_threaded(count, iterations));
        let shared = repeated(samples, || run_shared(count, iterations, worker_count));
        print_row(
            count,
            "thread per relationship",
            count,
            iterations,
            threaded,
        );
        print_row(count, "shared scheduler", worker_count, iterations, shared);
    }

    println!();
    println!("Blocked-observer isolation probe:");
    for count in relationships {
        let threaded = blocked_threaded(count);
        let shared = blocked_shared(count, worker_count);
        println!(
            "- {count} relationships: threaded fast observers completed in {:.0} ns; shared completed in {:.0} ns; both isolated={}",
            threaded.0,
            shared.0,
            threaded.1 && shared.1
        );
    }
}

fn argument(name: &str) -> Option<usize> {
    let mut arguments = std::env::args();
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments.next().and_then(|value| value.parse().ok());
        }
    }
    None
}

fn default_iterations(relationships: usize) -> usize {
    match relationships {
        1 => 20_000,
        8 => 5_000,
        64 => 1_000,
        _ => 250,
    }
}

fn repeated(samples: usize, mut sample: impl FnMut() -> Sample) -> Sample {
    let mut values = Vec::with_capacity(samples);
    for _ in 0..samples {
        values.push(sample());
    }
    values.sort_by(|left, right| left.publish_ns.total_cmp(&right.publish_ns));
    values[values.len() / 2]
}

fn print_row(
    relationships: usize,
    topology: &str,
    workers: usize,
    iterations: usize,
    sample: Sample,
) {
    println!(
        "| {relationships} | {topology} | {workers} | {iterations} | {:.1} | {:.1} | {:.0} | {} | {} | {:.1} |",
        sample.publish_ns,
        sample.completion_ns,
        sample.deliveries_per_second,
        sample.p50_latency_ns,
        sample.p99_latency_ns,
        sample.shutdown_ns
    );
}

fn run_threaded(relationships: usize, iterations: usize) -> Sample {
    let output = OutputPort::new("benchmark.source");
    let anchor = Instant::now();
    let (inputs, latencies) = inputs(relationships, anchor);
    let connections = inputs
        .iter()
        .map(|input| {
            connect(
                &output,
                input,
                ConnectionOptions::queued_every(iterations.max(1), EveryFullPolicy::Backpressure),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    exercise(output, connections, None, latencies, anchor, iterations)
}

fn run_shared(relationships: usize, iterations: usize, worker_count: usize) -> Sample {
    let scheduler = SharedScheduler::new(worker_count).unwrap();
    let dispatcher = scheduler.dispatcher();
    let output = OutputPort::new("benchmark.source");
    let anchor = Instant::now();
    let (inputs, latencies) = inputs(relationships, anchor);
    let connections = inputs
        .iter()
        .map(|input| {
            connect_shared(
                &output,
                input,
                &dispatcher,
                SharedDelivery::every_selected(iterations.max(1), EveryFullPolicy::Backpressure),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    exercise(
        output,
        connections,
        Some(scheduler),
        latencies,
        anchor,
        iterations,
    )
}

fn inputs(relationships: usize, anchor: Instant) -> (Vec<InputPort<TimedValue>>, Vec<LatencySet>) {
    let mut inputs = Vec::with_capacity(relationships);
    let mut all_latencies = Vec::with_capacity(relationships);
    for index in 0..relationships {
        let latencies = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&latencies);
        inputs.push(InputPort::<TimedValue>::new(
            format!("benchmark.observer-{index}"),
            move |envelope: &_| {
                let received_ns = anchor.elapsed().as_nanos() as u64;
                observed
                    .lock()
                    .unwrap()
                    .push(received_ns.saturating_sub(envelope.payload.sent_ns));
            },
        ));
        all_latencies.push(latencies);
    }
    (inputs, all_latencies)
}

fn exercise(
    output: OutputPort<TimedValue>,
    connections: Vec<Connection<TimedValue>>,
    scheduler: Option<SharedScheduler>,
    latency_sets: Vec<LatencySet>,
    anchor: Instant,
    iterations: usize,
) -> Sample {
    let exercise_start = Instant::now();
    let publish_start = Instant::now();
    for _ in 0..iterations {
        output.publish(
            anchor.elapsed().as_nanos() as u64,
            TimedValue {
                sent_ns: anchor.elapsed().as_nanos() as u64,
            },
        );
    }
    let publish_ns = publish_start.elapsed().as_nanos() as f64 / iterations as f64;
    let completion_start = Instant::now();
    let expected = iterations as u64;
    wait_until(Duration::from_secs(10), || {
        connections
            .iter()
            .all(|connection| connection.stats().delivered == expected)
    });
    let completion_ns = completion_start.elapsed().as_nanos() as f64;
    let exercise_seconds = exercise_start.elapsed().as_secs_f64();
    let deliveries_per_second =
        relationships(&latency_sets) as f64 * iterations as f64 / exercise_seconds;

    let mut latencies = latency_sets
        .iter()
        .flat_map(|values| values.lock().unwrap().clone())
        .collect::<Vec<_>>();
    latencies.sort_unstable();
    let p50_latency_ns = percentile(&latencies, 50);
    let p99_latency_ns = percentile(&latencies, 99);

    let shutdown_start = Instant::now();
    drop(connections);
    drop(scheduler);
    let shutdown_ns = shutdown_start.elapsed().as_nanos() as f64;
    Sample {
        publish_ns,
        completion_ns,
        deliveries_per_second,
        p50_latency_ns,
        p99_latency_ns,
        shutdown_ns,
    }
}

fn relationships(latency_sets: &[LatencySet]) -> usize {
    latency_sets.len()
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    let index = (values.len() - 1) * percentile / 100;
    values[index]
}

fn blocked_threaded(relationships: usize) -> (f64, bool) {
    blocked_probe(relationships, None)
}

fn blocked_shared(relationships: usize, worker_count: usize) -> (f64, bool) {
    blocked_probe(
        relationships,
        Some(SharedScheduler::new(worker_count).unwrap()),
    )
}

fn blocked_probe(relationships: usize, scheduler: Option<SharedScheduler>) -> (f64, bool) {
    let output = OutputPort::new("blocked.source");
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let entered = Arc::new(AtomicBool::new(false));
    let delivered = Arc::new(AtomicUsize::new(0));
    let dispatcher = scheduler.as_ref().map(SharedScheduler::dispatcher);
    let mut connections = Vec::with_capacity(relationships);
    for index in 0..relationships {
        let gate = Arc::clone(&gate);
        let entered = Arc::clone(&entered);
        let delivered = Arc::clone(&delivered);
        let input = InputPort::new(format!("blocked.observer-{index}"), move |_: &_| {
            if index == 0 {
                entered.store(true, Ordering::Release);
                let (lock, changed) = &*gate;
                let mut open = lock.lock().unwrap();
                while !*open {
                    open = changed.wait(open).unwrap();
                }
            } else {
                delivered.fetch_add(1, Ordering::Relaxed);
            }
        });
        let connection = if let Some(dispatcher) = &dispatcher {
            connect_shared(
                &output,
                &input,
                dispatcher,
                SharedDelivery::coalesce_latest(),
            )
            .unwrap()
        } else {
            connect(&output, &input, ConnectionOptions::latest()).unwrap()
        };
        connections.push(connection);
    }

    let start = Instant::now();
    output.publish(0, 1u64);
    wait_until(Duration::from_secs(2), || entered.load(Ordering::Acquire));
    let target = relationships.saturating_sub(1);
    let isolated = wait_until_result(Duration::from_secs(2), || {
        delivered.load(Ordering::Relaxed) == target
    });
    let elapsed = start.elapsed().as_nanos() as f64;

    let (lock, changed) = &*gate;
    *lock.lock().unwrap() = true;
    changed.notify_all();
    wait_until(Duration::from_secs(2), || {
        connections
            .iter()
            .all(|connection| connection.stats().delivered == 1 || connection.stats().failed)
    });
    drop(connections);
    drop(scheduler);
    (elapsed, isolated)
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    assert!(
        wait_until_result(timeout, &mut predicate),
        "condition timed out"
    );
}

fn wait_until_result(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::yield_now();
        thread::sleep(Duration::from_micros(50));
    }
    true
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
