//! `/auki/auth/1/blobs/0.1.0` content-addressed blob payload codec.
//!
//! ## Shape
//!
//! One substream may carry many request/response rounds. Client opens,
//! then loops [`BlobRequest`] → [`BlobResponse`] until it has assembled
//! `total_size` bytes (or hits NotFound / Error). Raw chunk bytes follow
//! an Ok meta frame; they are never embedded in the protobuf message.
//!
//! ## Wire format
//!
//! Length-prefixed protobuf meta (4-byte big-endian length, then prost
//! body) matching join/stream. For [`BlobResponse::Ok`], exactly
//! `chunk_len` raw payload bytes follow the framed meta.

use futures::{AsyncReadExt, AsyncWriteExt};
use prost::Message;
use thiserror::Error;

#[cfg(feature = "blob-endpoint")]
mod endpoint;
#[cfg(all(feature = "blob-fs-provider", not(target_arch = "wasm32")))]
mod fs;

#[cfg(feature = "blob-endpoint")]
pub use endpoint::{
    BlobClient, BlobEndpoint, BlobEndpointError, BlobFetchReceipt, BlobOperation, BlobProvider,
    BlobProviderError, BlobProviderFuture, CLOSE_TIMEOUT, FETCH_TIMEOUT,
    MAX_CONCURRENCY as ENDPOINT_MAX_CONCURRENCY, OPEN_TIMEOUT, ProvidedBlobChunk, ROUND_TIMEOUT,
    protocol_spec,
};
#[cfg(all(feature = "blob-fs-provider", not(target_arch = "wasm32")))]
pub use fs::FsBlobProvider;

/// Exact authenticated blob 0.1.0 protocol identifier.
pub const ID: &str = crate::ids::BLOBS_V0_1_0;

/// Cap on a single raw chunk payload.
pub const MAX_BLOB_CHUNK_BYTES: u32 = 1024 * 1024;

/// Cap on a single blob's total size (owned by `auki-registry`).
pub use auki_registry::MAX_BLOB_BYTES;

/// Max request/response rounds on one blob substream (full fetch + slack).
pub const MAX_BLOB_ROUNDS: u32 = (MAX_BLOB_BYTES / MAX_BLOB_CHUNK_BYTES as u64) as u32 + 8;

/// Cap on a framed protobuf request/response metadata message.
pub const MAX_BLOB_META_BYTES: u32 = 16 * 1024;

/// Body of an outbound or inbound blob chunk request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRequest {
    /// Content address (64 lowercase hex SHA-256).
    pub sha256: String,
    /// Byte offset into the blob.
    pub offset: u64,
    /// Maximum bytes to return in this round (1..=[`MAX_BLOB_CHUNK_BYTES`]).
    pub max_len: u32,
}

/// Ok chunk metadata (raw payload bytes follow on the wire).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobChunkMeta {
    /// Content address echoed from the request.
    pub sha256: String,
    /// Byte offset echoed from the request.
    pub offset: u64,
    /// Full blob size in bytes.
    pub total_size: u64,
    /// Length of the raw payload that follows this meta frame.
    pub chunk_len: u32,
}

/// Typed status of a blob response. Ok carries chunk bytes separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobResponse {
    /// Chunk available (or empty probe when `offset == total_size`).
    Ok(BlobChunkMeta),
    /// No blob at this address on the serving peer.
    NotFound,
    /// Serving peer fault (e.g. offset past end).
    Error { reason: String },
}

/// Failure modes for blob frame encode/decode and validation.
#[derive(Debug, Error)]
pub enum BlobsProtocolError {
    #[error("io: {0}")]
    Io(#[source] std::io::Error),
    #[error("encode: {0}")]
    Encode(#[source] prost::EncodeError),
    #[error("decode: {0}")]
    Decode(#[source] prost::DecodeError),
    #[error("validation: {0}")]
    Validation(String),
    #[error("blob response has no status set")]
    MissingStatus,
    #[error("meta frame too large: {actual} bytes (max {max})")]
    FrameTooLarge { actual: u64, max: u64 },
}

impl BlobRequest {
    pub fn validate(&self) -> Result<(), BlobsProtocolError> {
        if !is_sha256_hex(&self.sha256) {
            return Err(BlobsProtocolError::Validation(
                "sha256 must be 64 lowercase hex characters".into(),
            ));
        }
        if self.max_len == 0 || self.max_len > MAX_BLOB_CHUNK_BYTES {
            return Err(BlobsProtocolError::Validation(
                "max_len must be between 1 and MAX_BLOB_CHUNK_BYTES".into(),
            ));
        }
        if self.offset > MAX_BLOB_BYTES {
            return Err(BlobsProtocolError::Validation(
                "offset exceeds MAX_BLOB_BYTES".into(),
            ));
        }
        Ok(())
    }
}

impl BlobChunkMeta {
    pub fn validate(&self) -> Result<(), BlobsProtocolError> {
        if !is_sha256_hex(&self.sha256) {
            return Err(BlobsProtocolError::Validation(
                "sha256 must be 64 lowercase hex characters".into(),
            ));
        }
        if self.total_size > MAX_BLOB_BYTES {
            return Err(BlobsProtocolError::Validation(
                "total_size exceeds MAX_BLOB_BYTES".into(),
            ));
        }
        if self.offset > self.total_size {
            return Err(BlobsProtocolError::Validation(
                "offset exceeds total_size".into(),
            ));
        }
        if self.chunk_len > MAX_BLOB_CHUNK_BYTES {
            return Err(BlobsProtocolError::Validation(
                "chunk_len exceeds MAX_BLOB_CHUNK_BYTES".into(),
            ));
        }
        if u64::from(self.chunk_len) > self.total_size.saturating_sub(self.offset) {
            return Err(BlobsProtocolError::Validation(
                "chunk_len exceeds remaining bytes".into(),
            ));
        }
        Ok(())
    }
}

/// Write a [`BlobRequest`] (length-prefixed prost).
pub async fn write_blob_request<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    request: &BlobRequest,
) -> Result<(), BlobsProtocolError> {
    request.validate()?;
    write_frame(
        stream,
        &auki_datatypes::blob::BlobRequest {
            sha256: request.sha256.clone(),
            offset: request.offset,
            max_len: request.max_len,
        },
    )
    .await
}

/// Read a [`BlobRequest`].
pub async fn read_blob_request<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<BlobRequest, BlobsProtocolError> {
    let proto: auki_datatypes::blob::BlobRequest = read_frame(stream).await?;
    let request = BlobRequest {
        sha256: proto.sha256,
        offset: proto.offset,
        max_len: proto.max_len,
    };
    request.validate()?;
    Ok(request)
}

/// Write a [`BlobResponse`]. For [`BlobResponse::Ok`], `chunk` must be
/// exactly `meta.chunk_len` bytes and is written after the meta frame.
pub async fn write_blob_response<S: AsyncWriteExt + Unpin>(
    stream: &mut S,
    response: &BlobResponse,
    chunk: &[u8],
) -> Result<(), BlobsProtocolError> {
    let proto = match response {
        BlobResponse::Ok(meta) => {
            meta.validate()?;
            if chunk.len() != meta.chunk_len as usize {
                return Err(BlobsProtocolError::Validation(
                    "chunk length does not match metadata".into(),
                ));
            }
            auki_datatypes::blob::BlobResponse::ok(auki_datatypes::blob::blob_response::Chunk {
                sha256: meta.sha256.clone(),
                offset: meta.offset,
                total_size: meta.total_size,
                chunk_len: meta.chunk_len,
            })
        }
        BlobResponse::NotFound => auki_datatypes::blob::BlobResponse::not_found(),
        BlobResponse::Error { reason } => auki_datatypes::blob::BlobResponse::error(reason),
    };
    write_frame(stream, &proto).await?;
    if matches!(response, BlobResponse::Ok(_)) && !chunk.is_empty() {
        stream
            .write_all(chunk)
            .await
            .map_err(BlobsProtocolError::Io)?;
        stream.flush().await.map_err(BlobsProtocolError::Io)?;
    }
    Ok(())
}

/// Read a [`BlobResponse`]. Ok variants include the raw chunk bytes.
pub async fn read_blob_response<S: AsyncReadExt + Unpin>(
    stream: &mut S,
) -> Result<(BlobResponse, Vec<u8>), BlobsProtocolError> {
    let proto: auki_datatypes::blob::BlobResponse = read_frame(stream).await?;
    match proto.status.ok_or(BlobsProtocolError::MissingStatus)? {
        auki_datatypes::blob::blob_response::Status::Ok(chunk_meta) => {
            let meta = BlobChunkMeta {
                sha256: chunk_meta.sha256,
                offset: chunk_meta.offset,
                total_size: chunk_meta.total_size,
                chunk_len: chunk_meta.chunk_len,
            };
            meta.validate()?;
            let mut chunk = vec![0; meta.chunk_len as usize];
            if !chunk.is_empty() {
                stream
                    .read_exact(&mut chunk)
                    .await
                    .map_err(BlobsProtocolError::Io)?;
            }
            Ok((BlobResponse::Ok(meta), chunk))
        }
        auki_datatypes::blob::blob_response::Status::NotFound(_) => {
            Ok((BlobResponse::NotFound, Vec::new()))
        }
        auki_datatypes::blob::blob_response::Status::Error(error) => Ok((
            BlobResponse::Error {
                reason: error.reason,
            },
            Vec::new(),
        )),
    }
}

async fn write_frame<S, T>(stream: &mut S, msg: &T) -> Result<(), BlobsProtocolError>
where
    S: AsyncWriteExt + Unpin,
    T: Message,
{
    let mut bytes = Vec::with_capacity(msg.encoded_len());
    msg.encode(&mut bytes).map_err(BlobsProtocolError::Encode)?;
    if bytes.len() as u64 > MAX_BLOB_META_BYTES as u64 {
        return Err(BlobsProtocolError::FrameTooLarge {
            actual: bytes.len() as u64,
            max: MAX_BLOB_META_BYTES as u64,
        });
    }
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .map_err(BlobsProtocolError::Io)?;
    stream
        .write_all(&bytes)
        .await
        .map_err(BlobsProtocolError::Io)?;
    stream.flush().await.map_err(BlobsProtocolError::Io)
}

async fn read_frame<S, T>(stream: &mut S) -> Result<T, BlobsProtocolError>
where
    S: AsyncReadExt + Unpin,
    T: Message + Default,
{
    let mut len = [0; 4];
    stream
        .read_exact(&mut len)
        .await
        .map_err(BlobsProtocolError::Io)?;
    let len = u32::from_be_bytes(len);
    if len == 0 || len > MAX_BLOB_META_BYTES {
        return Err(BlobsProtocolError::FrameTooLarge {
            actual: len as u64,
            max: MAX_BLOB_META_BYTES as u64,
        });
    }
    let mut bytes = vec![0; len as usize];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(BlobsProtocolError::Io)?;
    T::decode(bytes.as_slice()).map_err(BlobsProtocolError::Decode)
}

/// Whether `value` is 64 lowercase hex characters.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ok_response_round_trips_with_raw_chunk() {
        let hash = "a".repeat(64);
        let meta = BlobChunkMeta {
            sha256: hash,
            offset: 2,
            total_size: 5,
            chunk_len: 3,
        };
        let mut bytes = Vec::new();
        write_blob_response(&mut bytes, &BlobResponse::Ok(meta.clone()), b"xyz")
            .await
            .unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        let (response, chunk) = read_blob_response(&mut cursor).await.unwrap();
        assert_eq!(response, BlobResponse::Ok(meta));
        assert_eq!(chunk, b"xyz");
    }

    #[tokio::test]
    async fn not_found_round_trips() {
        let mut bytes = Vec::new();
        write_blob_response(&mut bytes, &BlobResponse::NotFound, &[])
            .await
            .unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        let (response, chunk) = read_blob_response(&mut cursor).await.unwrap();
        assert_eq!(response, BlobResponse::NotFound);
        assert!(chunk.is_empty());
    }

    #[tokio::test]
    async fn request_round_trips() {
        let request = BlobRequest {
            sha256: "b".repeat(64),
            offset: 10,
            max_len: 1024,
        };
        let mut bytes = Vec::new();
        write_blob_request(&mut bytes, &request).await.unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        assert_eq!(read_blob_request(&mut cursor).await.unwrap(), request);
    }

    #[tokio::test]
    async fn request_and_ok_response_framed_bytes_are_locked() {
        let hash = "a".repeat(64);
        let request = BlobRequest {
            sha256: hash.clone(),
            offset: 2,
            max_len: 3,
        };
        let mut request_bytes = Vec::new();
        write_blob_request(&mut request_bytes, &request)
            .await
            .unwrap();

        let mut expected_request = vec![0x00, 0x00, 0x00, 0x46, 0x0a, 0x40];
        expected_request.extend_from_slice(&[b'a'; 64]);
        expected_request.extend_from_slice(&[0x10, 0x02, 0x18, 0x03]);
        assert_eq!(request_bytes, expected_request);

        let response = BlobResponse::Ok(BlobChunkMeta {
            sha256: hash,
            offset: 2,
            total_size: 5,
            chunk_len: 3,
        });
        let mut response_bytes = Vec::new();
        write_blob_response(&mut response_bytes, &response, b"xyz")
            .await
            .unwrap();

        let mut expected_response = vec![0x00, 0x00, 0x00, 0x4a, 0x0a, 0x48, 0x0a, 0x40];
        expected_response.extend_from_slice(&[b'a'; 64]);
        expected_response
            .extend_from_slice(&[0x10, 0x02, 0x18, 0x05, 0x20, 0x03, b'x', b'y', b'z']);
        assert_eq!(response_bytes, expected_response);
    }

    #[tokio::test]
    async fn empty_ok_chunk_at_eof_round_trips() {
        let hash = "c".repeat(64);
        let meta = BlobChunkMeta {
            sha256: hash,
            offset: 5,
            total_size: 5,
            chunk_len: 0,
        };
        let mut bytes = Vec::new();
        write_blob_response(&mut bytes, &BlobResponse::Ok(meta.clone()), &[])
            .await
            .unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        let (response, chunk) = read_blob_response(&mut cursor).await.unwrap();
        assert_eq!(response, BlobResponse::Ok(meta));
        assert!(chunk.is_empty());
    }

    #[tokio::test]
    async fn multi_round_on_one_stream() {
        let hash = "d".repeat(64);
        let mut bytes = Vec::new();
        write_blob_request(
            &mut bytes,
            &BlobRequest {
                sha256: hash.clone(),
                offset: 0,
                max_len: 2,
            },
        )
        .await
        .unwrap();
        write_blob_response(
            &mut bytes,
            &BlobResponse::Ok(BlobChunkMeta {
                sha256: hash.clone(),
                offset: 0,
                total_size: 4,
                chunk_len: 2,
            }),
            b"ab",
        )
        .await
        .unwrap();
        write_blob_request(
            &mut bytes,
            &BlobRequest {
                sha256: hash.clone(),
                offset: 2,
                max_len: 2,
            },
        )
        .await
        .unwrap();
        write_blob_response(
            &mut bytes,
            &BlobResponse::Ok(BlobChunkMeta {
                sha256: hash.clone(),
                offset: 2,
                total_size: 4,
                chunk_len: 2,
            }),
            b"cd",
        )
        .await
        .unwrap();

        let mut cursor = futures::io::Cursor::new(bytes);
        let req1 = read_blob_request(&mut cursor).await.unwrap();
        assert_eq!(req1.offset, 0);
        let (resp1, chunk1) = read_blob_response(&mut cursor).await.unwrap();
        assert!(matches!(resp1, BlobResponse::Ok(_)));
        assert_eq!(chunk1, b"ab");
        let req2 = read_blob_request(&mut cursor).await.unwrap();
        assert_eq!(req2.offset, 2);
        let (resp2, chunk2) = read_blob_response(&mut cursor).await.unwrap();
        assert!(matches!(resp2, BlobResponse::Ok(_)));
        assert_eq!(chunk2, b"cd");
    }

    #[tokio::test]
    async fn error_response_round_trips() {
        let mut bytes = Vec::new();
        write_blob_response(
            &mut bytes,
            &BlobResponse::Error {
                reason: "offset past end of blob".into(),
            },
            &[],
        )
        .await
        .unwrap();
        let mut cursor = futures::io::Cursor::new(bytes);
        let (response, chunk) = read_blob_response(&mut cursor).await.unwrap();
        assert_eq!(
            response,
            BlobResponse::Error {
                reason: "offset past end of blob".into(),
            }
        );
        assert!(chunk.is_empty());
    }

    #[test]
    fn max_blob_rounds_covers_full_blob_plus_slack() {
        assert_eq!(
            MAX_BLOB_ROUNDS,
            (MAX_BLOB_BYTES / MAX_BLOB_CHUNK_BYTES as u64) as u32 + 8
        );
        // A lying peer that returns 1-byte chunks for a max-sized blob still
        // exhausts within the capped round budget (plus the +8 slack).
        assert!(MAX_BLOB_ROUNDS as u64 > MAX_BLOB_BYTES / MAX_BLOB_CHUNK_BYTES as u64);
    }

    #[tokio::test]
    async fn inbound_style_loop_stops_after_max_blob_rounds() {
        // Script MAX_BLOB_ROUNDS+1 framed requests; the inbound gate serves
        // only MAX_BLOB_ROUNDS and leaves the next request unread on the wire.
        let hash = "a".repeat(64);
        let mut bytes = Vec::new();
        for offset in 0..=MAX_BLOB_ROUNDS {
            write_blob_request(
                &mut bytes,
                &BlobRequest {
                    sha256: hash.clone(),
                    offset: offset as u64,
                    max_len: 1,
                },
            )
            .await
            .unwrap();
            write_blob_response(
                &mut bytes,
                &BlobResponse::Ok(BlobChunkMeta {
                    sha256: hash.clone(),
                    offset: offset as u64,
                    total_size: (MAX_BLOB_ROUNDS as u64) + 1,
                    chunk_len: 1,
                }),
                b"x",
            )
            .await
            .unwrap();
        }
        let mut cursor = futures::io::Cursor::new(bytes);
        let mut served = 0u32;
        loop {
            if served >= MAX_BLOB_ROUNDS {
                break;
            }
            let _ = read_blob_request(&mut cursor).await.unwrap();
            let _ = read_blob_response(&mut cursor).await.unwrap();
            served += 1;
        }
        assert_eq!(served, MAX_BLOB_ROUNDS);
        // Next framed request is still on the wire — the loop gate refused it.
        assert!(read_blob_request(&mut cursor).await.is_ok());
    }
}
