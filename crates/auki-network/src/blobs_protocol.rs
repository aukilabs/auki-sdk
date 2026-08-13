//! `/auki/blobs/0.1.0` content-addressed binary blob transfer protocol.

use futures::{AsyncReadExt, AsyncWriteExt};
use libp2p::StreamProtocol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const BLOBS_PROTOCOL: StreamProtocol = StreamProtocol::new("/auki/blobs/0.1.0");
pub const MAX_BLOB_CHUNK_BYTES: u32 = 1024 * 1024;
pub const MAX_BLOB_BYTES: u64 = 64 * 1024 * 1024;
const MAX_META_BYTES: u32 = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobRequest {
    pub sha256: String,
    pub offset: u64,
    pub max_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobResponseMeta {
    pub sha256: String,
    pub offset: u64,
    pub total_size: u64,
    pub chunk_len: u32,
}

#[derive(Debug, Error)]
pub enum BlobsProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("decode: {0}")]
    Decode(#[source] serde_json::Error),
    #[error("validation: {0}")]
    Validation(String),
    #[error("meta frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

impl BlobRequest {
    pub fn validate(&self) -> Result<(), BlobsProtocolError> {
        if !is_sha256_hex(&self.sha256) {
            return Err(BlobsProtocolError::Validation("sha256 must be 64 lowercase hex characters".into()));
        }
        if self.max_len == 0 || self.max_len > MAX_BLOB_CHUNK_BYTES {
            return Err(BlobsProtocolError::Validation("max_len must be between 1 and MAX_BLOB_CHUNK_BYTES".into()));
        }
        if self.offset > MAX_BLOB_BYTES {
            return Err(BlobsProtocolError::Validation("offset exceeds MAX_BLOB_BYTES".into()));
        }
        Ok(())
    }
}

impl BlobResponseMeta {
    pub fn validate(&self) -> Result<(), BlobsProtocolError> {
        if !is_sha256_hex(&self.sha256)
            || self.total_size > MAX_BLOB_BYTES
            || self.offset > self.total_size
            || self.chunk_len > MAX_BLOB_CHUNK_BYTES
            || u64::from(self.chunk_len) > self.total_size.saturating_sub(self.offset)
        {
            return Err(BlobsProtocolError::Validation("invalid blob response metadata".into()));
        }
        Ok(())
    }
}

pub async fn write_blob_request<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    request: &BlobRequest,
) -> Result<(), BlobsProtocolError> {
    request.validate()?;
    write_meta(stream, request).await
}

pub async fn read_blob_request<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<BlobRequest, BlobsProtocolError> {
    let request: BlobRequest = read_meta(stream).await?;
    request.validate()?;
    Ok(request)
}

pub async fn write_blob_response<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    meta: &BlobResponseMeta,
    chunk: &[u8],
) -> Result<(), BlobsProtocolError> {
    meta.validate()?;
    if chunk.len() != meta.chunk_len as usize {
        return Err(BlobsProtocolError::Validation("chunk length does not match metadata".into()));
    }
    write_meta(stream, meta).await?;
    stream.write_all(chunk).await.map_err(BlobsProtocolError::Io)?;
    stream.flush().await.map_err(BlobsProtocolError::Io)
}

pub async fn read_blob_response<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<(BlobResponseMeta, Vec<u8>), BlobsProtocolError> {
    let meta: BlobResponseMeta = read_meta(stream).await?;
    meta.validate()?;
    let mut chunk = vec![0; meta.chunk_len as usize];
    stream.read_exact(&mut chunk).await.map_err(BlobsProtocolError::Io)?;
    Ok((meta, chunk))
}

async fn write_meta<S: AsyncWriteExt + Unpin, T: Serialize>(
    stream: &mut S,
    value: &T,
) -> Result<(), BlobsProtocolError> {
    let bytes = serde_json::to_vec(value).map_err(BlobsProtocolError::Encode)?;
    if bytes.len() > MAX_META_BYTES as usize {
        return Err(BlobsProtocolError::FrameTooLarge { actual: bytes.len() as u64, max: MAX_META_BYTES as u64 });
    }
    stream.write_all(&(bytes.len() as u32).to_be_bytes()).await.map_err(BlobsProtocolError::Io)?;
    stream.write_all(&bytes).await.map_err(BlobsProtocolError::Io)?;
    stream.flush().await.map_err(BlobsProtocolError::Io)
}

async fn read_meta<S: AsyncReadExt + Unpin, T: for<'de> Deserialize<'de>>(
    stream: &mut S,
) -> Result<T, BlobsProtocolError> {
    let mut len = [0; 4];
    stream.read_exact(&mut len).await.map_err(BlobsProtocolError::Io)?;
    let len = u32::from_be_bytes(len);
    if len == 0 || len > MAX_META_BYTES {
        return Err(BlobsProtocolError::FrameTooLarge { actual: len as u64, max: MAX_META_BYTES as u64 });
    }
    let mut bytes = vec![0; len as usize];
    stream.read_exact(&mut bytes).await.map_err(BlobsProtocolError::Io)?;
    serde_json::from_slice(&bytes).map_err(BlobsProtocolError::Decode)
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn response_round_trips_with_raw_chunk() {
        let hash = "a".repeat(64);
        let meta = BlobResponseMeta { sha256: hash, offset: 2, total_size: 5, chunk_len: 3 };
        let mut bytes = Vec::new();
        write_blob_response(&mut bytes, &meta, b"xyz").await.unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        assert_eq!(read_blob_response(&mut cursor).await.unwrap(), (meta, b"xyz".to_vec()));
    }
}
