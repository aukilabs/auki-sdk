use rodio::{
    OutputStream, OutputStreamHandle, Sink,
    source::{SineWave, Source},
};
use std::time::Duration;

const BEEP_HZ: f32 = 880.0;
const BEEP_MS: u64 = 140;
const BEEP_VOLUME: f32 = 0.20;

pub struct SoundEngine {
    _stream: Option<OutputStream>,
    handle: Option<OutputStreamHandle>,
    unavailable_reason: Option<String>,
}

impl SoundEngine {
    pub fn new() -> Self {
        match OutputStream::try_default() {
            Ok((stream, handle)) => Self {
                _stream: Some(stream),
                handle: Some(handle),
                unavailable_reason: None,
            },
            Err(error) => Self {
                _stream: None,
                handle: None,
                unavailable_reason: Some(error.to_string()),
            },
        }
    }

    pub fn is_available(&self) -> bool {
        self.handle.is_some()
    }

    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    pub fn beep(&self) {
        let Some(handle) = &self.handle else {
            return;
        };
        let Ok(sink) = Sink::try_new(handle) else {
            return;
        };

        let source = SineWave::new(BEEP_HZ)
            .take_duration(Duration::from_millis(BEEP_MS))
            .amplify(BEEP_VOLUME);
        sink.append(source);
        sink.detach();
    }
}
