//! Mirrors auki-k1's R2rCameraSubscriber threading model with just one
//! topic, so we can isolate "spawned worker thread + tokio + queue" from
//! "subscriber breaks specifically with image stream / multiple topics."

#[cfg(not(feature = "ros2"))]
fn main() {
    eprintln!("rebuild with --features ros2");
    std::process::exit(2);
}

#[cfg(feature = "ros2")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use futures::stream::StreamExt;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    let ctx = r2r::Context::create()?;
    let mut node = r2r::Node::create(ctx, "auki_thread_diag", "")?;
    let mut info_sub = node
        .subscribe::<r2r::sensor_msgs::msg::CameraInfo>(
            "/image_left_raw/camera_info",
            r2r::QosProfile::default(),
        )?;
    let mut image_sub = node
        .subscribe::<r2r::sensor_msgs::msg::Image>(
            "/image_left_raw",
            r2r::QosProfile::default(),
        )?;

    let queue: Arc<Mutex<VecDeque<r2r::sensor_msgs::msg::CameraInfo>>> =
        Arc::new(Mutex::new(VecDeque::new()));
    let stop = Arc::new(AtomicBool::new(false));

    let q_clone = Arc::clone(&queue);
    let stop_clone = Arc::clone(&stop);

    println!("[main] spawning worker thread...");
    let worker = thread::spawn(move || {
        println!("[worker] tokio runtime starting");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let q_inner = Arc::clone(&q_clone);
            let handler = async move {
                println!("[worker.handler] awaiting first message...");
                while let Some(msg) = info_sub.next().await {
                    println!(
                        "[worker.handler] got camera_info: {}x{}",
                        msg.width, msg.height
                    );
                    q_inner.lock().unwrap().push_back(msg);
                }
                println!("[worker.handler] stream ended");
            };
            let image_handler = async move {
                let mut count = 0u32;
                while let Some(msg) = image_sub.next().await {
                    count += 1;
                    if count <= 5 || count % 20 == 0 {
                        println!(
                            "[worker.image] got image #{}: {}x{} encoding={} step={} datalen={}",
                            count, msg.width, msg.height, msg.encoding, msg.step, msg.data.len()
                        );
                    }
                }
                println!("[worker.image] stream ended after {count}");
            };
            let spinner = async move {
                let mut spins = 0u32;
                while !stop_clone.load(Ordering::Relaxed) {
                    node.spin_once(Duration::from_millis(100));
                    spins += 1;
                    if spins % 10 == 0 {
                        println!("[worker.spinner] {} spins, stop={}", spins, stop_clone.load(Ordering::Relaxed));
                    }
                    tokio::task::yield_now().await;
                }
                println!("[worker.spinner] stopping");
            };
            tokio::join!(handler, image_handler, spinner);
        });
        println!("[worker] runtime done");
    });

    println!("[main] polling queue for up to 5s...");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut got = 0;
    while Instant::now() < deadline {
        if let Some(_msg) = queue.lock().unwrap().pop_front() {
            got += 1;
            println!("[main] popped #{got}");
            if got >= 3 { break; }
        }
        thread::sleep(Duration::from_millis(50));
    }
    println!("[main] received {got} messages from queue");

    stop.store(true, Ordering::Relaxed);
    let _ = worker.join();
    Ok(())
}
