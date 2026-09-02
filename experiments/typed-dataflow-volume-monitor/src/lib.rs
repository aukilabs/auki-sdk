//! Clean-room two-Peer volume-monitor application built only from the public
//! typed-dataflow experiment API.
//!
//! Incompatible payloads cannot be connected:
//!
//! ```compile_fail
//! use auki_typed_dataflow_experiment::{
//!     InputPort, Observable, ObservationDelivery, ObservationEvent,
//! };
//! use auki_typed_dataflow_volume_monitor::{AudioBlock, GaugeObservation};
//!
//! fn incompatible(
//!     audio: &Observable<AudioBlock>,
//!     gauge_input: &InputPort<ObservationEvent<GaugeObservation>>,
//! ) {
//!     audio
//!         .follow_new(gauge_input, ObservationDelivery::inline_every_selected())
//!         .unwrap();
//! }
//! ```

use std::fmt;
use std::sync::{Arc, Mutex};

use auki_typed_dataflow_experiment::{
    AudioLayout, AudioPayloadContract, AudioSampleFormat, BufferLimits, BufferProductCapture,
    ComponentBuildError, ComponentSpec, ConfiguredObservable, ConfiguredObservableSpec,
    ContractType, EpisodeProductCapture, Exposure, GaugePayloadContract, ObservableContract,
    ObservationAccess, ObservationDelivery, ObservationError, ObservationEvent, ObservationHandle,
    PayloadContract, PeerRuntime, ProductCaptureError, PublishError, SerializedInMemoryTransport,
    observation_input,
};
use serde::{Deserialize, Serialize};

pub const SAMPLE_RATE_HZ: u32 = 48_000;
pub const CHANNELS: u16 = 1;
pub const FRAMES_PER_BLOCK: u32 = 480;
pub const AUDIO_BUFFER_BLOCKS: usize = 6_000;
pub const SILENCE_FLOOR_DBFS: f64 = -120.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioBlock {
    pub interleaved_samples: Arc<[f32]>,
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub frames_per_channel: u32,
}

impl ContractType for AudioBlock {
    const DATATYPE: &'static str = "audio_block_f32";
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GaugeObservation {
    pub value: f64,
}

impl ContractType for GaugeObservation {
    const DATATYPE: &'static str = "float64";
}

#[derive(Debug)]
pub enum VolumeAppError {
    Component(Box<ComponentBuildError>),
    Product(Box<ProductCaptureError>),
    Observation(ObservationError),
    Publish(PublishError),
    InvalidBlockLength { expected: usize, actual: usize },
}

impl fmt::Display for VolumeAppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Component(error) => error.fmt(formatter),
            Self::Product(error) => error.fmt(formatter),
            Self::Observation(error) => error.fmt(formatter),
            Self::Publish(error) => error.fmt(formatter),
            Self::InvalidBlockLength { expected, actual } => {
                write!(
                    formatter,
                    "audio block has {actual} samples; expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for VolumeAppError {}

impl From<ComponentBuildError> for VolumeAppError {
    fn from(error: ComponentBuildError) -> Self {
        Self::Component(Box::new(error))
    }
}

impl From<ProductCaptureError> for VolumeAppError {
    fn from(error: ProductCaptureError) -> Self {
        Self::Product(Box::new(error))
    }
}

impl From<ObservationError> for VolumeAppError {
    fn from(error: ObservationError) -> Self {
        Self::Observation(error)
    }
}

impl From<PublishError> for VolumeAppError {
    fn from(error: PublishError) -> Self {
        Self::Publish(error)
    }
}

pub struct VolumePeer {
    pub runtime: PeerRuntime,
    pub audio: ConfiguredObservable<AudioBlock>,
    pub level: ConfiguredObservable<GaugeObservation>,
    pub audio_buffer: BufferProductCapture<AudioBlock>,
    pub level_episode: EpisodeProductCapture<GaugeObservation>,
    last_meter_input: Arc<Mutex<Option<Arc<AudioBlock>>>>,
    _meter_observation: ObservationHandle<AudioBlock>,
}

impl VolumePeer {
    pub fn new(peer_id: &str) -> Result<Self, VolumeAppError> {
        let runtime = PeerRuntime::new(peer_id);
        let microphone = runtime.component(ComponentSpec::new("microphone").observable(
            ObservableContract {
                name: "audio".to_owned(),
                datatype: "audio_block_f32".to_owned(),
                schema: "demo.audio-block/v1".to_owned(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            },
        ))?;
        let audio = microphone.configured_observable(ConfiguredObservableSpec::new(
            "audio",
            "audio-1",
            format!("{peer_id}.session-clock"),
            PayloadContract::Audio(AudioPayloadContract {
                datatype: "audio_block_f32".to_owned(),
                schema: "demo.audio-block/v1".to_owned(),
                sample_format: AudioSampleFormat::F32,
                layout: AudioLayout::Interleaved,
                sample_rate_hz: SAMPLE_RATE_HZ,
                channels: CHANNELS,
                frames_per_block: FRAMES_PER_BLOCK,
                observes: "acoustic_pressure_waveform".to_owned(),
                unit: Some("full_scale_amplitude".to_owned()),
            }),
        ))?;
        microphone.expose()?;

        let volume_meter = runtime.component(ComponentSpec::new("volume-meter").observable(
            ObservableContract {
                name: "level".to_owned(),
                datatype: "float64".to_owned(),
                schema: "demo.gauge-observation/v1".to_owned(),
                access: vec![ObservationAccess::FollowNew],
                exposure: Exposure::Cluster,
            },
        ))?;
        let level = volume_meter.configured_observable(ConfiguredObservableSpec::new(
            "level",
            "level-1",
            format!("{peer_id}.session-clock"),
            PayloadContract::Gauge(GaugePayloadContract {
                datatype: "float64".to_owned(),
                schema: "demo.gauge-observation/v1".to_owned(),
                observes: "audio_level".to_owned(),
                unit: "dBFS".to_owned(),
            }),
        ))?;
        volume_meter.expose()?;

        let audio_buffer = runtime.capture_buffer(
            "microphone.audio.buffer-60s",
            &audio,
            BufferLimits::entries(AUDIO_BUFFER_BLOCKS),
            |block: &AudioBlock| block.interleaved_samples.len() * size_of::<f32>(),
        )?;
        let level_episode =
            runtime.capture_episode("volume-meter.level.session-episode", &level)?;

        let last_meter_input = Arc::new(Mutex::new(None));
        let meter_input_storage = Arc::clone(&last_meter_input);
        let meter_output = level.clone();
        let meter_input = observation_input(
            "volume-meter.input.audio",
            move |event: &ObservationEvent<AudioBlock>| {
                if let ObservationEvent::Observation(observation) = event {
                    *meter_input_storage.lock().unwrap() = Some(Arc::clone(&observation.payload));
                    meter_output
                        .publish(
                            observation.timestamp_ns,
                            Arc::new(GaugeObservation {
                                value: rms_dbfs(&observation.payload.interleaved_samples),
                            }),
                        )
                        .expect("meter Output remains active for the session");
                }
            },
        );
        let meter_observation = audio
            .observable()
            .follow_new(&meter_input, ObservationDelivery::inline_every_selected())?;

        Ok(Self {
            runtime,
            audio,
            level,
            audio_buffer,
            level_episode,
            last_meter_input,
            _meter_observation: meter_observation,
        })
    }

    pub fn publish_audio(
        &self,
        timestamp_ns: u64,
        interleaved_samples: Arc<[f32]>,
    ) -> Result<Arc<AudioBlock>, VolumeAppError> {
        let expected = FRAMES_PER_BLOCK as usize * CHANNELS as usize;
        if interleaved_samples.len() != expected {
            return Err(VolumeAppError::InvalidBlockLength {
                expected,
                actual: interleaved_samples.len(),
            });
        }
        let block = Arc::new(AudioBlock {
            interleaved_samples,
            sample_rate_hz: SAMPLE_RATE_HZ,
            channels: CHANNELS,
            frames_per_channel: FRAMES_PER_BLOCK,
        });
        self.audio.publish(timestamp_ns, Arc::clone(&block))?;
        Ok(block)
    }

    pub fn last_meter_input(&self) -> Option<Arc<AudioBlock>> {
        self.last_meter_input.lock().unwrap().clone()
    }

    pub fn observe_volume_through(
        &self,
        transport: &SerializedInMemoryTransport,
        input: &auki_typed_dataflow_experiment::InputPort<ObservationEvent<GaugeObservation>>,
    ) -> Result<ObservationHandle<GaugeObservation>, VolumeAppError> {
        Ok(transport.follow_new(
            &self.level.observable(),
            input,
            ObservationDelivery::inline_every_selected(),
        )?)
    }

    pub fn conclude_session(&self, timestamp_ns: u64) -> Result<(), VolumeAppError> {
        self.level_episode.conclude(timestamp_ns)?;
        Ok(())
    }
}

pub fn rms_dbfs(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return SILENCE_FLOOR_DBFS;
    }
    let mean_square = samples
        .iter()
        .map(|sample| f64::from(*sample).powi(2))
        .sum::<f64>()
        / samples.len() as f64;
    if mean_square == 0.0 {
        SILENCE_FLOOR_DBFS
    } else {
        (20.0 * mean_square.sqrt().log10()).max(SILENCE_FLOOR_DBFS)
    }
}
