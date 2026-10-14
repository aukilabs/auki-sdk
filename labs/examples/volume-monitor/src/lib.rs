//! Native example: typed audio retention, derived dBFS, and real peer subscriptions.
//! No shared services, disk recording, or sound playback are involved.

pub mod demo;
pub mod local;
#[cfg(feature = "microphone")]
pub mod microphone;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail, ensure};
use auki_components::{
    AudioLayout, AudioPayloadContract, AudioSampleFormat, BufferLimits, BufferProductCapture,
    Component, ComponentRuntime, ComponentSpec, ConfiguredBufferInput, ConfiguredObservable,
    ConfiguredObservableSpec, ContractType, CursorStart, EpisodeProductCapture, Exposure,
    GaugePayloadContract, InputPort, ObservableContract, Observation, ObservationAccess,
    PayloadContract, ProductForm, ProductInputContract,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

pub const BLOCK_NS: u64 = 10_000_000;
pub const AUDIO_BUFFER_BLOCKS: usize = 6_000;
pub const LEVEL_BUFFER_BLOCKS: usize = 100;
pub const AUDIO_PRODUCT: &str = "audio.buffer-60s";
pub const LEVEL_PRODUCT: &str = "level.buffer-1s";
pub const EPISODE_PRODUCT: &str = "level.session-episode";
pub const AUDIO_SCHEMA: &str = "demo.audio-block-f32/v1";
pub const LEVEL_SCHEMA: &str = "demo.rms-dbfs/v1";
pub const SILENCE_FLOOR_DBFS: f64 = -120.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub frames_per_block: u32,
}

impl AudioFormat {
    pub fn new(sample_rate_hz: u32, channels: u16) -> Result<Self> {
        ensure!(
            (8_000..=192_000).contains(&sample_rate_hz),
            "unsupported sample rate"
        );
        ensure!(
            sample_rate_hz.is_multiple_of(100),
            "sample rate must allow exact 10 ms blocks"
        );
        ensure!((1..=8).contains(&channels), "expected 1 to 8 channels");
        Ok(Self {
            sample_rate_hz,
            channels,
            frames_per_block: sample_rate_hz / 100,
        })
    }

    pub fn samples_per_block(self) -> usize {
        self.frames_per_block as usize * usize::from(self.channels)
    }

    pub fn buffer_limits(self) -> BufferLimits {
        BufferLimits {
            max_entries: Some(AUDIO_BUFFER_BLOCKS),
            max_bytes: Some(
                AUDIO_BUFFER_BLOCKS
                    * (size_of::<AudioBlock>() + self.samples_per_block() * size_of::<f32>()),
            ),
            target_duration: Some(Duration::from_secs(60)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBlock {
    pub format: AudioFormat,
    pub interleaved_samples: Vec<f32>,
}

impl ContractType for AudioBlock {
    const DATATYPE: &'static str = "audio_block_f32";
}

impl AudioBlock {
    pub fn validate(&self, expected: AudioFormat) -> Result<()> {
        ensure!(
            self.format == expected,
            "audio configuration differs from its manifest"
        );
        ensure!(
            self.interleaved_samples.len() == expected.samples_per_block(),
            "incorrect audio block length"
        );
        ensure!(
            self.interleaved_samples
                .iter()
                .all(|sample| sample.is_finite() && sample.abs() <= 1.0),
            "audio samples must be finite full-scale amplitudes in [-1, 1]"
        );
        Ok(())
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.interleaved_samples.len() * size_of::<f32>()
    }
}

/// Finite digital full-scale RMS, not calibrated sound-pressure level (dB SPL).
pub fn rms_dbfs(samples: &[f32]) -> Result<f64> {
    ensure!(!samples.is_empty(), "empty audio block");
    ensure!(
        samples.iter().all(|x| x.is_finite() && x.abs() <= 1.0),
        "invalid full-scale samples"
    );
    let power = samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    Ok(if power == 0.0 {
        SILENCE_FLOOR_DBFS
    } else {
        (10.0 * power.log10()).max(SILENCE_FLOOR_DBFS)
    })
}

pub fn synthetic_block(format: AudioFormat, amplitude: f32) -> AudioBlock {
    // Ten complete periods per block, identical channels: repeatable RMS.
    let mut samples = Vec::with_capacity(format.samples_per_block());
    for frame in 0..format.frames_per_block {
        let value = amplitude
            * (std::f32::consts::TAU * 10.0 * frame as f32 / format.frames_per_block as f32).sin();
        samples.extend(std::iter::repeat_n(value, usize::from(format.channels)));
    }
    AudioBlock {
        format,
        interleaved_samples: samples,
    }
}

fn observable(name: &str, datatype: &str, schema: &str) -> ObservableContract {
    ObservableContract {
        name: name.into(),
        datatype: datatype.into(),
        schema: schema.into(),
        access: vec![ObservationAccess::FollowNew],
        exposure: Exposure::Cluster,
    }
}

#[derive(Default)]
struct MeterProgress {
    accepted: u64,
    last_input: Option<Arc<AudioBlock>>,
    error: Option<String>,
}

/// The audio Buffer feeds the meter through an ordinary typed Product input.
/// One level publication is shared by the rolling level Buffer and Episode.
pub struct VolumePeer {
    // Drop the reader before the capture handles and producers it feeds.
    meter_input: ConfiguredBufferInput<AudioBlock>,
    _microphone: Component,
    _meter: Component,
    pub runtime: ComponentRuntime,
    audio: ConfiguredObservable<AudioBlock>,
    pub audio_buffer: BufferProductCapture<AudioBlock>,
    pub level_buffer: BufferProductCapture<f64>,
    pub level_episode: EpisodeProductCapture<f64>,
    format: AudioFormat,
    published: u64,
    finished: bool,
    progress: Arc<Mutex<MeterProgress>>,
    changed: Arc<Notify>,
}

impl VolumePeer {
    pub fn new(peer_id: &str, format: AudioFormat, synthetic: bool) -> Result<Self> {
        ensure!(
            format == AudioFormat::new(format.sample_rate_hz, format.channels)?,
            "invalid block size"
        );
        let runtime = ComponentRuntime::new(peer_id);
        let source_name = if synthetic {
            "synthetic-audio"
        } else {
            "microphone"
        };
        // A source-relative sample timeline, not UTC or another peer's clock.
        let clock = format!("{peer_id}.audio-sample-clock");
        let microphone =
            runtime.component(ComponentSpec::new(source_name).observable(observable(
                "audio",
                AudioBlock::DATATYPE,
                AUDIO_SCHEMA,
            )))?;
        let audio = microphone.configured_observable(ConfiguredObservableSpec::new(
            "audio",
            "audio-1",
            &clock,
            PayloadContract::Audio(AudioPayloadContract {
                datatype: AudioBlock::DATATYPE.into(),
                schema: AUDIO_SCHEMA.into(),
                sample_format: AudioSampleFormat::F32,
                layout: AudioLayout::Interleaved,
                sample_rate_hz: format.sample_rate_hz,
                channels: format.channels,
                frames_per_block: format.frames_per_block,
                observes: if synthetic {
                    "synthetic_waveform"
                } else {
                    "microphone_signal"
                }
                .into(),
                unit: Some("full_scale_amplitude".into()),
            }),
        ))?;
        microphone.expose()?;
        let audio_buffer = runtime.capture_buffer(
            AUDIO_PRODUCT,
            &audio,
            format.buffer_limits(),
            AudioBlock::retained_bytes,
        )?;

        let meter = runtime.component(
            ComponentSpec::new("volume-meter")
                .product_input(ProductInputContract {
                    name: "audio".into(),
                    form: ProductForm::Buffer,
                    datatype: AudioBlock::DATATYPE.into(),
                    schema: AUDIO_SCHEMA.into(),
                    exposure: Exposure::Cluster,
                })
                .observable(observable("level", "float64", LEVEL_SCHEMA)),
        )?;
        let level = meter.configured_observable::<f64>(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            &clock,
            PayloadContract::Gauge(GaugePayloadContract {
                datatype: "float64".into(),
                schema: LEVEL_SCHEMA.into(),
                observes: "audio_rms_level".into(),
                unit: "dBFS".into(),
            }),
        ))?;
        let progress = Arc::new(Mutex::new(MeterProgress::default()));
        let changed = Arc::new(Notify::new());
        let callback_progress = progress.clone();
        let callback_changed = changed.clone();
        let meter_output = level.clone();
        let input = InputPort::try_new(
            "volume-meter.input.audio",
            move |envelope: &auki_components::Envelope<Observation<AudioBlock>>| {
                let observation = &envelope.payload;
                let result = (|| -> Result<()> {
                    observation.payload.validate(format)?;
                    let dbfs = rms_dbfs(&observation.payload.interleaved_samples)?;
                    meter_output.publish(observation.timestamp_ns, Arc::new(dbfs))?;
                    Ok(())
                })();
                let mut progress = callback_progress.lock().unwrap();
                if let Err(error) = &result {
                    progress.error = Some(error.to_string());
                } else {
                    progress.accepted += 1;
                    progress.last_input = Some(observation.payload.clone());
                }
                drop(progress);
                callback_changed.notify_one();
                result.map_err(|error| error.to_string())
            },
        );
        let meter_input = meter.configured_buffer_input(
            "audio",
            &audio_buffer.product(),
            CursorStart::FromSequence(0),
            &input,
        )?;
        meter.expose()?;
        let level_buffer = runtime.capture_buffer(
            LEVEL_PRODUCT,
            &level,
            BufferLimits::entries(LEVEL_BUFFER_BLOCKS),
            |_| size_of::<f64>(),
        )?;
        let level_episode = runtime.capture_episode(EPISODE_PRODUCT, &level)?;
        Ok(Self {
            meter_input,
            _microphone: microphone,
            _meter: meter,
            runtime,
            audio,
            audio_buffer,
            level_buffer,
            level_episode,
            format,
            published: 0,
            finished: false,
            progress,
            changed,
        })
    }

    pub fn publish_audio(&mut self, block: AudioBlock) -> Result<Arc<AudioBlock>> {
        ensure!(!self.finished, "session already concluded");
        block.validate(self.format)?;
        let payload = Arc::new(block);
        self.audio
            .publish(self.published * BLOCK_NS, payload.clone())?;
        self.published += 1;
        ensure!(
            self.audio_buffer.errors().is_empty(),
            "audio retention failed"
        );
        Ok(payload)
    }

    pub async fn drain_meter(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let notified = self.changed.notified();
                {
                    let progress = self.progress.lock().unwrap();
                    if let Some(error) = &progress.error {
                        bail!("volume meter failed: {error}");
                    }
                    if progress.accepted == self.published {
                        break;
                    }
                }
                notified.await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("volume meter did not drain; {:?}", self.meter_input.stats())
        })??;
        let stats = self.meter_input.stats();
        ensure!(
            stats.gap_entries == 0 && !stats.failed,
            "volume meter lost input: {stats:?}"
        );
        ensure!(
            self.level_buffer.errors().is_empty() && self.level_episode.errors().is_empty(),
            "volume retention failed"
        );
        Ok(())
    }

    pub fn last_meter_input(&self) -> Option<Arc<AudioBlock>> {
        self.progress.lock().unwrap().last_input.clone()
    }

    pub async fn finish(&mut self) -> Result<()> {
        ensure!(!self.finished, "session already concluded");
        self.drain_meter().await?;
        self.level_episode.conclude(self.published * BLOCK_NS)?;
        self.audio_buffer.cancel();
        self.level_buffer.cancel();
        self.finished = true;
        Ok(())
    }
}
