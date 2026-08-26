use std::{path::Path, time::Duration};

use auki_network::{
    blobs_protocol::{
        BlobChunkMeta, BlobRequest, BlobResponse, BlobsProtocolError, MAX_BLOB_BYTES,
        MAX_BLOB_CHUNK_BYTES, MAX_BLOB_META_BYTES, MAX_BLOB_ROUNDS, is_sha256_hex,
        read_blob_request, read_blob_response, write_blob_request, write_blob_response,
    },
    protocol_ids::BLOBS_V0_1_0,
};
use auki_p2p::PeerId;
use futures::{AsyncRead, AsyncWrite};
use tokio::time::timeout;

use super::{
    protocols::{
        DomainProtocolError, DomainProtocolRegistration, DomainProtocolSpec, DomainProtocolStream,
        DomainProtocols,
    },
    storage::{RegistryBlobStorage, StorageError},
};

const BLOBS_V1_MAX_CONCURRENCY: usize = 16;
const BLOBS_V1_OPEN_TIMEOUT: Duration = Duration::from_secs(30);
const BLOBS_V1_ROUND_TIMEOUT: Duration = Duration::from_secs(30);
const BLOBS_V1_FETCH_TIMEOUT: Duration = Duration::from_secs(180);

/// Private authenticated adapter for content-addressed blob transfer 0.1.0.
///
/// Full fetches assemble into a function-local buffer and return it only after
/// the requested SHA-256 has been verified. No error variant carries bytes, so
/// timeout, cancellation, malformed metadata, and integrity failures cannot
/// expose a partial blob.
#[derive(Clone)]
pub(crate) struct BlobsV1 {
    protocols: DomainProtocols,
    storage: RegistryBlobStorage,
}

impl BlobsV1 {
    pub(super) fn new(protocols: DomainProtocols, storage: RegistryBlobStorage) -> Self {
        Self { protocols, storage }
    }

    pub(super) fn register(&self) -> Result<DomainProtocolRegistration, BlobsV1Error> {
        let spec =
            DomainProtocolSpec::new(BLOBS_V0_1_0, BLOBS_V1_MAX_CONCURRENCY, MAX_BLOB_META_BYTES)?;
        let blobs = self.clone();
        self.protocols
            .register(spec, move |stream| {
                let blobs = blobs.clone();
                async move {
                    if let Err(error) = blobs.handle(stream).await {
                        tracing::warn!(%error, "authenticated blob request failed");
                    }
                }
            })
            .map_err(BlobsV1Error::Protocol)
    }

    pub(crate) fn set_app_root(
        &self,
        app_root: impl Into<std::path::PathBuf>,
    ) -> Result<(), BlobsV1Error> {
        self.storage.set_app_root(app_root)?;
        Ok(())
    }

    /// Fetch one complete blob from `expected_peer`, or from local storage
    /// when the expected owner is this Domain's own Peer ID.
    pub(crate) async fn fetch(
        &self,
        expected_peer: PeerId,
        sha256: impl Into<String>,
    ) -> Result<Vec<u8>, BlobsV1Error> {
        self.storage.ensure_running()?;
        let sha256 = sha256.into();
        validate_sha256(&sha256)?;
        if expected_peer == self.storage.local_peer_id() {
            return self.local(&sha256);
        }

        let lifecycle = self.storage.lifecycle().clone();
        let open = timeout(
            BLOBS_V1_OPEN_TIMEOUT,
            self.protocols.open(expected_peer, BLOBS_V0_1_0),
        );
        let mut stream = tokio::select! {
            biased;
            _ = lifecycle.cancelled() => return Err(BlobsV1Error::Stopped),
            result = open => match result {
                Err(_) => return Err(BlobsV1Error::Timeout(BLOBS_V1_OPEN_TIMEOUT)),
                Ok(Err(error)) => return Err(BlobsV1Error::Protocol(error)),
                Ok(Ok(stream)) => stream,
            },
        };

        let assembly = timeout(
            BLOBS_V1_FETCH_TIMEOUT,
            assemble_blob(&mut stream, &sha256, &lifecycle),
        );
        tokio::select! {
            biased;
            _ = lifecycle.cancelled() => Err(BlobsV1Error::Stopped),
            result = assembly => match result {
                Err(_) => Err(BlobsV1Error::Timeout(BLOBS_V1_FETCH_TIMEOUT)),
                Ok(result) => result,
            },
        }
    }

    /// Fetch and verify one blob without dialing the local Domain itself.
    pub(crate) fn local(&self, sha256: &str) -> Result<Vec<u8>, BlobsV1Error> {
        self.storage.ensure_running()?;
        validate_sha256(sha256)?;
        let root = self
            .storage
            .app_root()?
            .ok_or(BlobsV1Error::NotConfigured)?;
        let result = match auki_registry::get_blob(&root, sha256) {
            Ok(Some(bytes)) => Ok(bytes),
            Ok(None) => Err(BlobsV1Error::NotFound),
            Err(auki_registry::Error::BlobHashMismatch) => Err(BlobsV1Error::HashMismatch),
            Err(error) => Err(BlobsV1Error::InvalidResponse(error.to_string())),
        };
        self.storage.ensure_running()?;
        result
    }

    async fn handle(&self, mut stream: DomainProtocolStream) -> Result<(), BlobsV1Error> {
        let lifecycle = self.storage.lifecycle().clone();
        let mut rounds = 0_u32;
        loop {
            self.storage.ensure_running()?;
            if rounds >= MAX_BLOB_ROUNDS {
                return Err(BlobsV1Error::TooManyRounds);
            }

            let request = tokio::select! {
                biased;
                _ = lifecycle.cancelled() => return Err(BlobsV1Error::Stopped),
                result = timeout(BLOBS_V1_ROUND_TIMEOUT, read_blob_request(&mut stream)) => {
                    match result {
                        Err(_) => return Err(BlobsV1Error::Timeout(BLOBS_V1_ROUND_TIMEOUT)),
                        Ok(Err(BlobsProtocolError::Io(error)))
                            if is_normal_stream_end(error.kind()) => return Ok(()),
                        Ok(Err(error)) => return Err(BlobsV1Error::Codec(error)),
                        Ok(Ok(request)) => request,
                    }
                }
            };

            let root = self.storage.app_root()?;
            let (response, chunk) = serve_blob_request(root.as_deref(), &request);
            // A synchronous, bounded range read may race with leave. Fence the
            // response after that read so no bytes are written after shutdown.
            self.storage.ensure_running()?;
            tokio::select! {
                biased;
                _ = lifecycle.cancelled() => return Err(BlobsV1Error::Stopped),
                result = timeout(
                    BLOBS_V1_ROUND_TIMEOUT,
                    write_blob_response(&mut stream, &response, &chunk),
                ) => match result {
                    Err(_) => return Err(BlobsV1Error::Timeout(BLOBS_V1_ROUND_TIMEOUT)),
                    Ok(Err(error)) => return Err(BlobsV1Error::Codec(error)),
                    Ok(Ok(())) => {}
                }
            }
            rounds += 1;
        }
    }
}

async fn assemble_blob<S>(
    stream: &mut S,
    sha256: &str,
    lifecycle: &tokio_util::sync::CancellationToken,
) -> Result<Vec<u8>, BlobsV1Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    validate_sha256(sha256)?;
    let mut bytes = Vec::new();
    let mut offset = 0_u64;
    let mut expected_total = None;
    let mut rounds = 0_u32;

    loop {
        if rounds >= MAX_BLOB_ROUNDS {
            return Err(BlobsV1Error::TooManyRounds);
        }
        let request = BlobRequest {
            sha256: sha256.to_owned(),
            offset,
            max_len: MAX_BLOB_CHUNK_BYTES,
        };
        let (meta, chunk) = blob_round_trip(stream, &request, lifecycle).await?;
        rounds += 1;

        if meta.total_size > MAX_BLOB_BYTES {
            return Err(BlobsV1Error::InvalidResponse(
                "total_size exceeds MAX_BLOB_BYTES".into(),
            ));
        }
        match expected_total {
            None => {
                expected_total = Some(meta.total_size);
                bytes
                    .try_reserve_exact(meta.total_size as usize)
                    .map_err(|_| {
                        BlobsV1Error::InvalidResponse(
                            "cannot reserve the advertised blob size".into(),
                        )
                    })?;
            }
            Some(expected) if expected != meta.total_size => {
                return Err(BlobsV1Error::SizeMismatch {
                    expected,
                    actual: meta.total_size,
                });
            }
            Some(_) => {}
        }

        if meta.total_size == 0 {
            break;
        }
        if chunk.is_empty() {
            if offset == meta.total_size {
                break;
            }
            return Err(BlobsV1Error::InvalidResponse(
                "empty chunk before end of blob".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
        offset = offset
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| BlobsV1Error::InvalidResponse("blob offset overflow".into()))?;
        if offset == meta.total_size {
            break;
        }
        if offset > meta.total_size {
            return Err(BlobsV1Error::InvalidResponse(
                "chunk exceeds advertised size".into(),
            ));
        }
    }

    if bytes.len() as u64 != expected_total.unwrap_or_default() {
        return Err(BlobsV1Error::InvalidResponse(
            "assembled size differs from advertised size".into(),
        ));
    }
    if auki_registry::sha256_hex(&bytes) != sha256 {
        return Err(BlobsV1Error::HashMismatch);
    }
    Ok(bytes)
}

async fn blob_round_trip<S>(
    stream: &mut S,
    request: &BlobRequest,
    lifecycle: &tokio_util::sync::CancellationToken,
) -> Result<(BlobChunkMeta, Vec<u8>), BlobsV1Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(BlobsV1Error::Stopped),
        result = timeout(BLOBS_V1_ROUND_TIMEOUT, write_blob_request(stream, request)) => {
            match result {
                Err(_) => return Err(BlobsV1Error::Timeout(BLOBS_V1_ROUND_TIMEOUT)),
                Ok(Err(error)) => return Err(BlobsV1Error::Codec(error)),
                Ok(Ok(())) => {}
            }
        }
    }

    let (response, chunk) = tokio::select! {
        biased;
        _ = lifecycle.cancelled() => return Err(BlobsV1Error::Stopped),
        result = timeout(BLOBS_V1_ROUND_TIMEOUT, read_blob_response(stream)) => {
            match result {
                Err(_) => return Err(BlobsV1Error::Timeout(BLOBS_V1_ROUND_TIMEOUT)),
                Ok(Err(error)) => return Err(BlobsV1Error::Codec(error)),
                Ok(Ok(response)) => response,
            }
        }
    };

    match response {
        BlobResponse::NotFound => Err(BlobsV1Error::NotFound),
        BlobResponse::Error { reason } => Err(BlobsV1Error::RemoteError(reason)),
        BlobResponse::Ok(meta) => {
            if meta.sha256 != request.sha256 || meta.offset != request.offset {
                return Err(BlobsV1Error::InvalidResponse(
                    "response address or offset differs from request".into(),
                ));
            }
            if meta.chunk_len > request.max_len || chunk.len() > request.max_len as usize {
                return Err(BlobsV1Error::InvalidResponse(
                    "response chunk exceeds the requested maximum".into(),
                ));
            }
            Ok((meta, chunk))
        }
    }
}

fn serve_blob_request(app_root: Option<&Path>, request: &BlobRequest) -> (BlobResponse, Vec<u8>) {
    let Some(app_root) = app_root else {
        return (
            BlobResponse::Error {
                reason: "blobs not configured".into(),
            },
            Vec::new(),
        );
    };
    match auki_registry::read_blob_range(app_root, &request.sha256, request.offset, request.max_len)
    {
        Ok(None) => (BlobResponse::NotFound, Vec::new()),
        Ok(Some(range)) => (
            BlobResponse::Ok(BlobChunkMeta {
                sha256: request.sha256.clone(),
                offset: request.offset,
                total_size: range.total_size,
                chunk_len: range.chunk.len() as u32,
            }),
            range.chunk,
        ),
        Err(auki_registry::Error::BlobOffsetPastEnd) => (
            BlobResponse::Error {
                reason: "offset past end of blob".into(),
            },
            Vec::new(),
        ),
        Err(auki_registry::Error::Io(_)) => (
            BlobResponse::Error {
                reason: "io".into(),
            },
            Vec::new(),
        ),
        Err(auki_registry::Error::InvalidBlob(_)) | Err(auki_registry::Error::BlobHashMismatch) => {
            (
                BlobResponse::Error {
                    reason: "invalid blob".into(),
                },
                Vec::new(),
            )
        }
        Err(_) => (
            BlobResponse::Error {
                reason: "invalid blob".into(),
            },
            Vec::new(),
        ),
    }
}

fn validate_sha256(sha256: &str) -> Result<(), BlobsV1Error> {
    if is_sha256_hex(sha256) {
        Ok(())
    } else {
        Err(BlobsV1Error::InvalidRequest(
            "sha256 must be 64 lowercase hex characters".into(),
        ))
    }
}

fn is_normal_stream_end(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::UnexpectedEof
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::BrokenPipe
    )
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum BlobsV1Error {
    #[error("the Domain runtime is stopped")]
    Stopped,
    #[error("blob request is invalid: {0}")]
    InvalidRequest(String),
    #[error("blob storage is not configured")]
    NotConfigured,
    #[error("blob was not found")]
    NotFound,
    #[error("remote blob error: {0}")]
    RemoteError(String),
    #[error("blob protocol failed: {0}")]
    Protocol(#[from] DomainProtocolError),
    #[error("blob codec failed: {0}")]
    Codec(#[from] BlobsProtocolError),
    #[error("blob operation exceeded {0:?}")]
    Timeout(Duration),
    #[error("assembled bytes fail SHA-256 verification")]
    HashMismatch,
    #[error("remote total_size changed mid-fetch: was {expected}, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("invalid remote blob response: {0}")]
    InvalidResponse(String),
    #[error("blob transfer exceeded MAX_BLOB_ROUNDS")]
    TooManyRounds,
}

impl From<StorageError> for BlobsV1Error {
    fn from(_: StorageError) -> Self {
        Self::Stopped
    }
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::io::Cursor;
    use tokio_util::sync::CancellationToken;

    use super::*;

    struct ScriptedStream {
        inbound: Cursor<Vec<u8>>,
        outbound: Vec<u8>,
    }

    impl ScriptedStream {
        fn new(inbound: Vec<u8>) -> Self {
            Self {
                inbound: Cursor::new(inbound),
                outbound: Vec::new(),
            }
        }
    }

    impl AsyncRead for ScriptedStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
            buffer: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.inbound).poll_read(context, buffer)
        }
    }

    impl AsyncWrite for ScriptedStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            self.outbound.extend_from_slice(buffer);
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    struct PendingReadStream;

    impl AsyncRead for PendingReadStream {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &mut [u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Pending
        }
    }

    impl AsyncWrite for PendingReadStream {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(buffer.len()))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    async fn encoded_ok(
        bytes: &mut Vec<u8>,
        sha256: &str,
        offset: u64,
        total_size: u64,
        chunk: &[u8],
    ) {
        write_blob_response(
            bytes,
            &BlobResponse::Ok(BlobChunkMeta {
                sha256: sha256.into(),
                offset,
                total_size,
                chunk_len: chunk.len() as u32,
            }),
            chunk,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn multi_round_assembly_returns_only_verified_complete_bytes() {
        let payload = b"abcdef";
        let sha256 = auki_registry::sha256_hex(payload);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &sha256, 0, payload.len() as u64, b"abc").await;
        encoded_ok(&mut inbound, &sha256, 3, payload.len() as u64, b"def").await;
        let mut stream = ScriptedStream::new(inbound);

        let bytes = assemble_blob(&mut stream, &sha256, &CancellationToken::new())
            .await
            .unwrap();

        assert_eq!(bytes, payload);
        assert!(!stream.outbound.is_empty(), "requests must be written");
    }

    #[tokio::test]
    async fn final_hash_mismatch_returns_no_partial_value() {
        let requested_sha = "a".repeat(64);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &requested_sha, 0, 3, b"bad").await;
        let mut stream = ScriptedStream::new(inbound);

        let result = assemble_blob(&mut stream, &requested_sha, &CancellationToken::new()).await;

        assert!(matches!(result, Err(BlobsV1Error::HashMismatch)));
    }

    #[tokio::test]
    async fn changed_total_size_rejects_the_whole_assembly() {
        let requested_sha = "b".repeat(64);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &requested_sha, 0, 4, b"ab").await;
        encoded_ok(&mut inbound, &requested_sha, 2, 5, b"cd").await;
        let mut stream = ScriptedStream::new(inbound);

        let result = assemble_blob(&mut stream, &requested_sha, &CancellationToken::new()).await;

        assert!(matches!(
            result,
            Err(BlobsV1Error::SizeMismatch {
                expected: 4,
                actual: 5
            })
        ));
    }

    #[tokio::test]
    async fn round_budget_rejects_a_drip_feed_without_returning_bytes() {
        let requested_sha = "c".repeat(64);
        let total_size = u64::from(MAX_BLOB_ROUNDS) + 1;
        let mut inbound = Vec::new();
        for offset in 0..u64::from(MAX_BLOB_ROUNDS) {
            encoded_ok(&mut inbound, &requested_sha, offset, total_size, b"x").await;
        }
        let mut stream = ScriptedStream::new(inbound);

        let result = assemble_blob(&mut stream, &requested_sha, &CancellationToken::new()).await;

        assert!(matches!(result, Err(BlobsV1Error::TooManyRounds)));
    }

    #[tokio::test]
    async fn lifecycle_cancellation_interrupts_a_stalled_round() {
        let lifecycle = CancellationToken::new();
        let task_lifecycle = lifecycle.clone();
        let task = tokio::spawn(async move {
            let mut stream = PendingReadStream;
            assemble_blob(&mut stream, &"d".repeat(64), &task_lifecycle).await
        });
        tokio::task::yield_now().await;
        lifecycle.cancel();

        let result = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("cancellation must beat the 30-second round timeout")
            .unwrap();

        assert!(matches!(result, Err(BlobsV1Error::Stopped)));
    }

    #[test]
    fn local_serving_is_bounded_and_uses_stable_statuses() {
        let temp = tempfile::tempdir().unwrap();
        let payload = vec![7_u8; MAX_BLOB_CHUNK_BYTES as usize + 3];
        let sha256 = auki_registry::put_blob(temp.path(), &payload).unwrap();

        let (unconfigured, bytes) = serve_blob_request(
            None,
            &BlobRequest {
                sha256: sha256.clone(),
                offset: 0,
                max_len: MAX_BLOB_CHUNK_BYTES,
            },
        );
        assert!(matches!(
            unconfigured,
            BlobResponse::Error { ref reason } if reason == "blobs not configured"
        ));
        assert!(bytes.is_empty());

        let (first, bytes) = serve_blob_request(
            Some(temp.path()),
            &BlobRequest {
                sha256,
                offset: 0,
                max_len: MAX_BLOB_CHUNK_BYTES,
            },
        );
        assert!(matches!(
            first,
            BlobResponse::Ok(BlobChunkMeta {
                chunk_len: MAX_BLOB_CHUNK_BYTES,
                ..
            })
        ));
        assert_eq!(bytes.len(), MAX_BLOB_CHUNK_BYTES as usize);
    }
}
