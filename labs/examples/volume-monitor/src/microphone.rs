//! Optional native capture. Never opened by the synthetic runner or tests.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tokio::sync::{mpsc, watch};

use crate::{AudioBlock, AudioFormat};

pub struct Microphone {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    format: AudioFormat,
    stream: Option<cpal::Stream>,
    receiver: Option<mpsc::Receiver<AudioBlock>>,
    error: Option<watch::Receiver<Option<String>>>,
}

impl Microphone {
    /// Inspect configuration without starting capture.
    pub fn prepare() -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .context("no default microphone")?;
        let config = device.default_input_config()?;
        let format = AudioFormat::new(config.sample_rate().0, config.channels())?;
        ensure!(
            matches!(
                config.sample_format(),
                cpal::SampleFormat::F32
                    | cpal::SampleFormat::I16
                    | cpal::SampleFormat::U16
                    | cpal::SampleFormat::I32
            ),
            "unsupported native microphone sample format: {:?}",
            config.sample_format()
        );
        eprintln!(
            "Microphone A: {} Hz, {} channels, {:?} -> interleaved f32, 10 ms blocks",
            format.sample_rate_hz,
            format.channels,
            config.sample_format()
        );
        Ok(Self {
            device,
            config,
            format,
            stream: None,
            receiver: None,
            error: None,
        })
    }

    pub fn format(&self) -> AudioFormat {
        self.format
    }

    pub fn start(&mut self) -> Result<()> {
        ensure!(self.stream.is_none(), "microphone already started");
        let (sender, receiver) = mpsc::channel(32);
        let (error, error_receiver) = watch::channel(None);
        let config = self.config.config();
        let stream = match self.config.sample_format() {
            cpal::SampleFormat::F32 => {
                build::<f32>(&self.device, &config, self.format, sender, error)?
            }
            cpal::SampleFormat::I16 => {
                build::<i16>(&self.device, &config, self.format, sender, error)?
            }
            cpal::SampleFormat::U16 => {
                build::<u16>(&self.device, &config, self.format, sender, error)?
            }
            cpal::SampleFormat::I32 => {
                build::<i32>(&self.device, &config, self.format, sender, error)?
            }
            _ => bail!("unsupported microphone sample format"),
        };
        stream.play()?;
        self.stream = Some(stream);
        self.receiver = Some(receiver);
        self.error = Some(error_receiver);
        Ok(())
    }

    pub async fn next(&mut self) -> Result<AudioBlock> {
        let receiver = self.receiver.as_mut().context("microphone not started")?;
        let errors = self.error.as_mut().context("microphone not started")?;
        if let Some(error) = errors.borrow().as_ref() {
            bail!("microphone capture failed: {error}");
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            tokio::select! {
                biased;
                _ = errors.changed() => bail!("microphone capture failed: {:?}", *errors.borrow()),
                block = receiver.recv() => block.context("microphone capture stopped"),
            }
        })
        .await
        .context("microphone did not produce audio within three seconds")?
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: AudioFormat,
    sender: mpsc::Sender<AudioBlock>,
    error: watch::Sender<Option<String>>,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let failed = Arc::new(AtomicBool::new(false));
    let callback_failed = failed.clone();
    let callback_error = error.clone();
    let mut assembler = BlockAssembler::new(format);
    Ok(device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if callback_failed.load(Ordering::Acquire) {
                return;
            }
            for sample in data {
                let sample = <f32 as cpal::FromSample<T>>::from_sample_(*sample);
                let result = assembler.push(sample, &sender);
                if let Err(reason) = result {
                    callback_failed.store(true, Ordering::Release);
                    callback_error.send_replace(Some(reason));
                    break;
                }
            }
        },
        move |failure| {
            failed.store(true, Ordering::Release);
            error.send_replace(Some(failure.to_string()));
        },
        None,
    )?)
}

struct BlockAssembler {
    format: AudioFormat,
    samples: Vec<f32>,
}

impl BlockAssembler {
    fn new(format: AudioFormat) -> Self {
        Self {
            format,
            samples: Vec::with_capacity(format.samples_per_block()),
        }
    }

    fn push(&mut self, sample: f32, sender: &mpsc::Sender<AudioBlock>) -> Result<(), String> {
        if !sample.is_finite() || sample.abs() > 1.0 {
            return Err("invalid microphone amplitude".into());
        }
        self.samples.push(sample);
        if self.samples.len() == self.format.samples_per_block() {
            let samples = std::mem::replace(
                &mut self.samples,
                Vec::with_capacity(self.format.samples_per_block()),
            );
            sender
                .try_send(AudioBlock {
                    format: self.format,
                    interleaved_samples: samples,
                })
                .map_err(|_| {
                    "capture queue full or closed; stopping instead of silently dropping audio"
                        .to_string()
                })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_boundaries_do_not_change_block_shape_and_overflow_is_visible() {
        let format = AudioFormat::new(8_000, 2).unwrap();
        let (sender, mut receiver) = mpsc::channel(1);
        let mut assembler = BlockAssembler::new(format);
        for _ in 0..format.samples_per_block() - 1 {
            assembler.push(0.5, &sender).unwrap();
        }
        assert!(receiver.try_recv().is_err());
        assembler.push(0.25, &sender).unwrap();
        let first = receiver.try_recv().unwrap();
        first.validate(format).unwrap();
        assert_eq!(first.interleaved_samples.last(), Some(&0.25));
        for _ in 0..format.samples_per_block() {
            assembler.push(0.5, &sender).unwrap();
        }
        for _ in 0..format.samples_per_block() - 1 {
            assembler.push(0.5, &sender).unwrap();
        }
        assert!(assembler.push(0.5, &sender).is_err());
        assert!(assembler.push(f32::NAN, &sender).is_err());
    }
}
