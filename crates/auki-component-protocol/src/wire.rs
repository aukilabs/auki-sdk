//! Transport-neutral wire contract for the Component protocol family.

use auki_components::{
    CatalogSnapshot, ComponentReference, OutputManifest, OutputReference, ProductManifest,
    ProductReference, TimeRangeRequest,
};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub const CATALOG_PROTOCOL_ID: &str = "/aukilabs/components/catalog/1.0.0";
pub const OBSERVATIONS_PROTOCOL_ID: &str = "/aukilabs/components/observations/1.0.0";
pub const OPERATIONS_PROTOCOL_ID: &str = "/aukilabs/components/operations/1.0.0";

pub const MAX_CONTROL_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_PAYLOAD_FRAME_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_BATCH_OBSERVATIONS: u32 = 4096;
pub const MAX_OPERATION_DEADLINE_MS: u64 = 60_000;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CatalogRequest {
    pub known_revision: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CatalogResponse {
    Snapshot { snapshot: CatalogSnapshot },
    Unchanged { revision: u64 },
    Rejected { code: String, message: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObservationSelection {
    LatestExisting,
    TimeRange {
        request: TimeRangeRequest,
    },
    FromSequence {
        sequence: u64,
        max_observations: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObservationRequest {
    pub product: ProductReference,
    pub selection: ObservationSelection,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SourceGap {
    pub requested_sequence: u64,
    pub available_from: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ObservationBatchHeader {
    Accepted {
        product: Box<ProductManifest>,
        product_manifest_hash: String,
        producer: Box<OutputManifest>,
        observations: u32,
        gap: Option<SourceGap>,
    },
    Rejected {
        code: String,
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObservationRecordHeader {
    pub output: OutputReference,
    pub sequence: u64,
    pub timestamp_ns: u64,
    pub payload_encoding: String,
    pub payload_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OperationRequest {
    pub target_component: ComponentReference,
    pub operable: String,
    pub invocation_id: String,
    pub caller_component_id: String,
    pub deadline_ms: Option<u64>,
    pub instruction_encoding: String,
    pub instruction_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum OperationResponse {
    Completed {
        invocation_id: String,
        result_encoding: String,
        result_bytes: u32,
    },
    Failed {
        invocation_id: String,
        error: RemoteOperationError,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteOperationError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum WireError {
    #[error("wire I/O failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("control frame must not be empty")]
    EmptyControlFrame,
    #[error("{kind} frame is {actual} bytes; maximum is {maximum}")]
    FrameTooLarge {
        kind: &'static str,
        actual: u64,
        maximum: usize,
    },
    #[error("invalid JSON control frame: {0}")]
    Json(#[source] serde_json::Error),
    #[error("payload length {actual} does not match declared length {declared}")]
    PayloadLengthMismatch { declared: u32, actual: usize },
}

pub(crate) async fn write_json<S, T>(stream: &mut S, value: &T) -> Result<(), WireError>
where
    S: AsyncWrite + Unpin,
    T: Serialize,
{
    let payload = serde_json::to_vec(value).map_err(WireError::Json)?;
    if payload.is_empty() {
        return Err(WireError::EmptyControlFrame);
    }
    write_frame(stream, &payload, MAX_CONTROL_FRAME_BYTES, "control").await
}

pub(crate) async fn read_json<S, T>(stream: &mut S) -> Result<T, WireError>
where
    S: AsyncRead + Unpin,
    T: DeserializeOwned,
{
    let payload = read_frame(stream, MAX_CONTROL_FRAME_BYTES, "control").await?;
    if payload.is_empty() {
        return Err(WireError::EmptyControlFrame);
    }
    serde_json::from_slice(&payload).map_err(WireError::Json)
}

pub(crate) async fn write_payload<S>(stream: &mut S, payload: &[u8]) -> Result<(), WireError>
where
    S: AsyncWrite + Unpin,
{
    write_frame(stream, payload, MAX_PAYLOAD_FRAME_BYTES, "payload").await
}

pub(crate) async fn read_payload<S>(stream: &mut S, declared: u32) -> Result<Vec<u8>, WireError>
where
    S: AsyncRead + Unpin,
{
    let payload = read_frame(stream, MAX_PAYLOAD_FRAME_BYTES, "payload").await?;
    if payload.len() != declared as usize {
        return Err(WireError::PayloadLengthMismatch {
            declared,
            actual: payload.len(),
        });
    }
    Ok(payload)
}

async fn write_frame<S>(
    stream: &mut S,
    payload: &[u8],
    maximum: usize,
    kind: &'static str,
) -> Result<(), WireError>
where
    S: AsyncWrite + Unpin,
{
    validate_len(payload.len() as u64, maximum, kind)?;
    let length = u32::try_from(payload.len()).expect("protocol frame bounds fit in u32");
    stream
        .write_all(&length.to_be_bytes())
        .await
        .map_err(WireError::Io)?;
    stream.write_all(payload).await.map_err(WireError::Io)?;
    stream.flush().await.map_err(WireError::Io)
}

async fn read_frame<S>(
    stream: &mut S,
    maximum: usize,
    kind: &'static str,
) -> Result<Vec<u8>, WireError>
where
    S: AsyncRead + Unpin,
{
    let mut length = [0_u8; 4];
    stream
        .read_exact(&mut length)
        .await
        .map_err(WireError::Io)?;
    let length = u32::from_be_bytes(length);
    validate_len(u64::from(length), maximum, kind)?;
    let mut payload = vec![0_u8; length as usize];
    stream
        .read_exact(&mut payload)
        .await
        .map_err(WireError::Io)?;
    Ok(payload)
}

fn validate_len(actual: u64, maximum: usize, kind: &'static str) -> Result<(), WireError> {
    if actual > maximum as u64 {
        return Err(WireError::FrameTooLarge {
            kind,
            actual,
            maximum,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use futures::io::Cursor;

    use super::*;

    #[test]
    fn protocol_ids_are_exact_and_independent_of_manager_protocols() {
        assert_eq!(CATALOG_PROTOCOL_ID, "/aukilabs/components/catalog/1.0.0");
        assert_eq!(
            OBSERVATIONS_PROTOCOL_ID,
            "/aukilabs/components/observations/1.0.0"
        );
        assert_eq!(
            OPERATIONS_PROTOCOL_ID,
            "/aukilabs/components/operations/1.0.0"
        );
    }

    #[test]
    fn json_and_raw_payload_frames_round_trip() {
        block_on(async {
            let request = CatalogRequest {
                known_revision: Some(42),
            };
            let mut bytes = Cursor::new(Vec::new());
            write_json(&mut bytes, &request).await.unwrap();
            write_payload(&mut bytes, b"payload").await.unwrap();
            bytes.set_position(0);
            assert_eq!(
                read_json::<_, CatalogRequest>(&mut bytes).await.unwrap(),
                request
            );
            assert_eq!(read_payload(&mut bytes, 7).await.unwrap(), b"payload");
        });
    }

    #[test]
    fn payload_length_mismatch_is_rejected() {
        block_on(async {
            let mut bytes = Cursor::new(Vec::new());
            write_payload(&mut bytes, b"abc").await.unwrap();
            bytes.set_position(0);
            assert!(matches!(
                read_payload(&mut bytes, 4).await.unwrap_err(),
                WireError::PayloadLengthMismatch {
                    declared: 4,
                    actual: 3
                }
            ));
        });
    }
}
