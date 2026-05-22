#![allow(dead_code)]

pub const AUDIO_FRAME_MS: u32 = 20;
pub const AUDIO_SAMPLES_PER_FRAME: usize = 320;
pub const AUDIO_BYTES_PER_FRAME: usize = 640;

pub fn f32_to_s16le(input: &[f32], output: &mut Vec<u8>) {
    output.clear();
    output.reserve(input.len() * 2);
    for sample in input {
        let clamped = sample.clamp(-1.0, 1.0);
        let value = if clamped < 0.0 {
            (clamped * 32768.0).round() as i16
        } else {
            (clamped * 32767.0).round() as i16
        };
        output.extend_from_slice(&value.to_le_bytes());
    }
}

#[allow(dead_code)]
pub fn s16le_to_f32(input: &[u8], output: &mut Vec<f32>) -> Result<(), &'static str> {
    if input.len() % 2 != 0 {
        return Err("pcm_s16le input length must be even");
    }
    output.clear();
    output.reserve(input.len() / 2);
    for chunk in input.chunks_exact(2) {
        let value = i16::from_le_bytes([chunk[0], chunk[1]]);
        output.push((value as f32 / 32768.0).clamp(-1.0, 1.0));
    }
    Ok(())
}

pub fn generated_audio_frame(frame_index: u32) -> Vec<u8> {
    let mut samples = Vec::with_capacity(AUDIO_SAMPLES_PER_FRAME);
    for i in 0..AUDIO_SAMPLES_PER_FRAME {
        let phase = ((frame_index as usize * AUDIO_SAMPLES_PER_FRAME + i) % 80) as f32 / 80.0;
        samples.push((phase * std::f32::consts::TAU).sin() * 0.25);
    }
    let mut bytes = Vec::with_capacity(AUDIO_BYTES_PER_FRAME);
    f32_to_s16le(&samples, &mut bytes);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_float_samples_to_s16le() {
        let mut out = Vec::new();
        f32_to_s16le(&[-1.0, 0.0, 1.0], &mut out);
        assert_eq!(out, vec![0x00, 0x80, 0x00, 0x00, 0xff, 0x7f]);
    }

    #[test]
    fn decodes_s16le_to_float_samples() {
        let mut out = Vec::new();
        s16le_to_f32(&[0x00, 0x80, 0x00, 0x00, 0xff, 0x7f], &mut out).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out[0] <= -0.99);
        assert_eq!(out[1], 0.0);
        assert!(out[2] >= 0.99);
    }

    #[test]
    fn generated_audio_frame_is_non_silent_twenty_ms() {
        let frame = generated_audio_frame(7);
        assert_eq!(frame.len(), AUDIO_BYTES_PER_FRAME);
        assert!(frame.iter().any(|byte| *byte != 0));
    }
}
