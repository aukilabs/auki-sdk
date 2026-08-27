use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use auki_typed_dataflow_experiment::{
    Buffer, BufferReader, ChunkBuilder, ChunkBuilderConfig, ConnectionOptions, CursorStart,
    Episode, InputPort, OutputPort, PumpOptions, StreamPump, connect, connect_buffer,
    connect_episode,
};

#[derive(Debug)]
struct CameraFrame {
    width: u16,
    height: u16,
    pixels: Arc<[u8]>,
}

#[derive(Debug)]
struct Brightness(u8);

#[derive(Debug)]
struct Detection {
    bright: bool,
}

struct CameraOutputs {
    frames: OutputPort<CameraFrame>,
}

impl CameraOutputs {
    fn frames(&self) -> &OutputPort<CameraFrame> {
        &self.frames
    }
}

struct FakeCamera {
    outputs: CameraOutputs,
}

impl FakeCamera {
    fn new() -> Self {
        Self {
            outputs: CameraOutputs {
                frames: OutputPort::new("fake-camera.output.frames"),
            },
        }
    }

    fn outputs(&self) -> &CameraOutputs {
        &self.outputs
    }

    fn capture(&self, timestamp_ns: u64, level: u8) {
        self.outputs.frames.publish(
            timestamp_ns,
            CameraFrame {
                width: 64,
                height: 48,
                pixels: Arc::from(vec![level; 64 * 48]),
            },
        );
    }
}

struct BrightnessInputs {
    frames: InputPort<CameraFrame>,
}

impl BrightnessInputs {
    fn frames(&self) -> &InputPort<CameraFrame> {
        &self.frames
    }
}

struct BrightnessOutputs {
    level: OutputPort<Brightness>,
}

impl BrightnessOutputs {
    fn level(&self) -> &OutputPort<Brightness> {
        &self.level
    }
}

struct MeanBrightness {
    inputs: BrightnessInputs,
    outputs: BrightnessOutputs,
}

impl MeanBrightness {
    fn new() -> Self {
        let level = OutputPort::new("mean-brightness.output.level");
        let handler_output = level.clone();
        let frames = InputPort::<CameraFrame>::new("mean-brightness.input.frames", move |frame| {
            let sum: usize = frame
                .payload
                .pixels
                .iter()
                .map(|value| *value as usize)
                .sum();
            let mean = (sum / frame.payload.pixels.len()) as u8;
            handler_output.publish(frame.timestamp_ns, Brightness(mean));
        });
        Self {
            inputs: BrightnessInputs { frames },
            outputs: BrightnessOutputs { level },
        }
    }

    fn inputs(&self) -> &BrightnessInputs {
        &self.inputs
    }

    fn outputs(&self) -> &BrightnessOutputs {
        &self.outputs
    }
}

struct PreviewInputs {
    frames: InputPort<CameraFrame>,
}

impl PreviewInputs {
    fn frames(&self) -> &InputPort<CameraFrame> {
        &self.frames
    }
}

struct SlowPreview {
    inputs: PreviewInputs,
    observed: Arc<Mutex<Vec<u64>>>,
}

impl SlowPreview {
    fn new() -> Self {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let handler_observed = Arc::clone(&observed);
        let frames = InputPort::new("slow-preview.input.frames", move |frame: &_| {
            thread::sleep(Duration::from_millis(3));
            handler_observed.lock().unwrap().push(frame.sequence);
        });
        Self {
            inputs: PreviewInputs { frames },
            observed,
        }
    }

    fn inputs(&self) -> &PreviewInputs {
        &self.inputs
    }
}

struct DetectorInputs {
    frames: InputPort<CameraFrame>,
}

impl DetectorInputs {
    fn frames(&self) -> &InputPort<CameraFrame> {
        &self.frames
    }
}

struct DetectorOutputs {
    detections: OutputPort<Detection>,
}

impl DetectorOutputs {
    fn detections(&self) -> &OutputPort<Detection> {
        &self.detections
    }
}

struct RemoteDetector {
    inputs: DetectorInputs,
    outputs: DetectorOutputs,
}

impl RemoteDetector {
    fn new() -> Self {
        let detections = OutputPort::new("remote-detector.output.detections");
        let handler_output = detections.clone();
        let frames = InputPort::<CameraFrame>::new("remote-detector.input.frames", move |frame| {
            let middle = (frame.payload.width as usize * frame.payload.height as usize) / 2;
            handler_output.publish(
                frame.timestamp_ns,
                Detection {
                    bright: frame.payload.pixels[middle] >= 128,
                },
            );
        });
        Self {
            inputs: DetectorInputs { frames },
            outputs: DetectorOutputs { detections },
        }
    }

    fn inputs(&self) -> &DetectorInputs {
        &self.inputs
    }

    fn outputs(&self) -> &DetectorOutputs {
        &self.outputs
    }
}

fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "demo timed out");
        thread::sleep(Duration::from_millis(1));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let camera = FakeCamera::new();
    let brightness = MeanBrightness::new();
    let preview = SlowPreview::new();
    let detector = RemoteDetector::new();

    let camera_buffer = Buffer::with_limits(
        "local.camera.buffer",
        auki_typed_dataflow_experiment::BufferLimits::entries(8),
        |frame: &CameraFrame| frame.pixels.len(),
    )?;
    let level_buffer = Buffer::new("local.brightness.buffer", 16)?;
    let remote_camera_buffer = Buffer::with_limits(
        "remote.camera.buffer",
        auki_typed_dataflow_experiment::BufferLimits::entries(8),
        |frame: &CameraFrame| frame.pixels.len(),
    )?;
    let detection_buffer = Buffer::new("remote.detections.buffer", 16)?;

    let _camera_to_brightness = connect(
        camera.outputs().frames(),
        brightness.inputs().frames(),
        ConnectionOptions::InlineEvery,
    )?;
    let preview_connection = connect(
        camera.outputs().frames(),
        preview.inputs().frames(),
        ConnectionOptions::Latest,
    )?;
    let _camera_retention = connect_buffer(camera.outputs().frames(), &camera_buffer);
    let _level_retention = connect_buffer(brightness.outputs().level(), &level_buffer);
    let _detection_retention = connect_buffer(detector.outputs().detections(), &detection_buffer);

    let pump = StreamPump::start(
        &camera_buffer,
        CursorStart::Latest,
        &remote_camera_buffer,
        PumpOptions::default(),
    )?;
    let remote_detector_reader = BufferReader::start(
        &remote_camera_buffer,
        CursorStart::Latest,
        detector.inputs().frames(),
    );
    let chunk_builder = ChunkBuilder::start(
        &camera_buffer,
        CursorStart::Latest,
        ChunkBuilderConfig {
            max_entries: 4,
            max_bytes: usize::MAX,
            max_latency: Duration::from_millis(20),
            poll_interval: Duration::from_millis(1),
        },
        |frame: &CameraFrame| frame.pixels.len(),
    )?;

    for level in [10, 20, 30, 40, 140, 150] {
        camera.capture(level as u64, level);
    }
    wait_until(Duration::from_secs(1), || {
        camera_buffer.range().entries == 6
    });

    let episode = Episode::promote("bright-event", &camera_buffer, 2, 5)?;
    let episode_connection = connect_episode(camera.outputs().frames(), &episode);
    for level in [160, 170, 180, 190] {
        camera.capture(level as u64, level);
    }
    wait_until(Duration::from_secs(1), || {
        pump.stats().delivered_sequence == Some(9)
    });
    wait_until(Duration::from_secs(1), || {
        remote_detector_reader.stats().delivered >= 10
    });
    wait_until(Duration::from_secs(1), || {
        preview_connection.stats().delivered >= 2
    });
    episode.conclude(200)?;
    drop(episode_connection);
    chunk_builder.stop();

    let source_latest = camera_buffer.snapshot(9, 9).pop().unwrap();
    let remote_latest = remote_camera_buffer.snapshot(9, 9).pop().unwrap();
    let bright_detections = detection_buffer
        .snapshot(0, u64::MAX)
        .iter()
        .filter(|entry| entry.payload.bright)
        .count();

    println!("camera buffer: {:?}", camera_buffer.range());
    println!("remote camera buffer: {:?}", remote_camera_buffer.range());
    println!("brightness buffer: {:?}", level_buffer.range());
    println!(
        "latest brightness: {:?}",
        level_buffer
            .snapshot(0, u64::MAX)
            .last()
            .map(|entry| entry.payload.0)
    );
    println!("preview connection: {:?}", preview_connection.stats());
    println!("preview observed: {:?}", preview.observed.lock().unwrap());
    println!("pump: {:?}", pump.stats());
    println!("remote reader: {:?}", remote_detector_reader.stats());
    println!("bright detections: {bright_detections}");
    println!("episode: {:?}", episode);
    println!("chunks: {:?}", chunk_builder.stats());
    println!(
        "source and remote share storage: {}",
        Arc::ptr_eq(&source_latest, &remote_latest)
    );

    Ok(())
}
