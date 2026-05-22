#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code, unused_imports))]

use futures::{AsyncReadExt, AsyncWriteExt};
use prost::Message;

pub use auki_datatypes::audio;
pub use auki_datatypes::stream::{
    DeclineReason, StreamEntry, StreamManifest, StreamMessage, StreamRequest, stream_message,
};

pub const STREAM_PROTOCOL: &str = "/auki/stream/0.1.0";
const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

pub async fn write_message<S>(stream: &mut S, msg: &StreamMessage) -> Result<(), String>
where
    S: AsyncWriteExt + Unpin,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes)
        .map_err(|err| format!("stream message encode failed: {err}"))?;
    if bytes.len() as u64 > MAX_FRAME_BYTES as u64 {
        return Err(format!("stream message too large: {} bytes", bytes.len()));
    }
    let len = bytes.len() as u32;
    stream
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|err| format!("stream length write failed: {err}"))?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|err| format!("stream payload write failed: {err}"))?;
    stream
        .flush()
        .await
        .map_err(|err| format!("stream flush failed: {err}"))?;
    Ok(())
}

pub async fn read_message<S>(stream: &mut S) -> Result<StreamMessage, String>
where
    S: AsyncReadExt + Unpin,
{
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|err| format!("stream length read failed: {err}"))?;
    let len = u32::from_be_bytes(len_buf);
    if len == 0 {
        return Err("stream frame is empty".to_string());
    }
    if len > MAX_FRAME_BYTES {
        return Err(format!("stream frame too large: {len} bytes"));
    }
    let mut payload = vec![0u8; len as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(|err| format!("stream payload read failed: {err}"))?;
    StreamMessage::decode(&*payload).map_err(|err| format!("stream message decode failed: {err}"))
}
