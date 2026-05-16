//! Minimal r2r diagnostic: subscribes to a few small topics, prints what it
//! sees. Used during K1 bring-up to isolate r2r-vs-publisher mismatches.
//!
//! Build with `cargo build --features ros2 --example r2r_diag` on a ROS2
//! host. Run: `./target/debug/examples/r2r_diag`.

#[cfg(not(feature = "ros2"))]
fn main() {
    eprintln!("rebuild with --features ros2");
    std::process::exit(2);
}

#[cfg(feature = "ros2")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use futures::stream::StreamExt;
    use std::time::Duration;

    let ctx = r2r::Context::create()?;
    let mut node = r2r::Node::create(ctx, "auki_diag", "")?;

    let mut joint_states = node.subscribe::<r2r::sensor_msgs::msg::JointState>(
        "/joint_states",
        r2r::QosProfile::default(),
    )?;
    let mut tf_static = node.subscribe::<r2r::tf2_msgs::msg::TFMessage>(
        "/tf_static",
        r2r::QosProfile::default().transient_local(),
    )?;
    let mut camera_info = node.subscribe::<r2r::sensor_msgs::msg::CameraInfo>(
        "/image_left_raw/camera_info",
        r2r::QosProfile::default(),
    )?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async move {
        let js = async {
            let mut count = 0;
            while let Some(msg) = joint_states.next().await {
                count += 1;
                println!(
                    "joint_states #{count}: {} joints, stamp {}.{:09}",
                    msg.name.len(),
                    msg.header.stamp.sec,
                    msg.header.stamp.nanosec
                );
                if count >= 3 {
                    break;
                }
            }
        };
        let tf = async {
            let mut count = 0;
            while let Some(msg) = tf_static.next().await {
                count += 1;
                println!("tf_static #{count}: {} transforms", msg.transforms.len());
                if count >= 2 {
                    break;
                }
            }
        };
        let ci = async {
            let mut count = 0;
            while let Some(msg) = camera_info.next().await {
                count += 1;
                println!(
                    "camera_info #{count}: {}x{} dmodel='{}' k[0]={} d.len={} stamp {}.{:09}",
                    msg.width,
                    msg.height,
                    msg.distortion_model,
                    msg.k[0],
                    msg.d.len(),
                    msg.header.stamp.sec,
                    msg.header.stamp.nanosec
                );
                if count >= 3 {
                    break;
                }
            }
        };
        let spinner = async {
            for _ in 0..80 {
                // 8s
                node.spin_once(Duration::from_millis(100));
                tokio::task::yield_now().await;
            }
            println!("--- spin loop done ---");
        };
        tokio::join!(js, tf, ci, spinner);
    });

    Ok(())
}
