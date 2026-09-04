//! Cross-platform Auki peer endpoint for Blob v1.
//!
//! The endpoint owns authenticated protocol registration, bounded multi-round
//! serving, and stream cleanup. Storage remains an application concern behind
//! [`BlobProvider`], while [`BlobClient`] assembles and verifies complete blobs
//! before exposing any bytes to its caller.

#![forbid(unsafe_code)]

use std::{fmt, future::Future, pin::Pin, time::Duration};

use auki_sdk::{
    AukiPeerProtocols, AukiProtocolError, AukiProtocolRegistration, AukiProtocolSpec,
    AukiProtocolStream, AuthenticatedPeer, AuthenticatedRouteStream, Multiaddr, PeerId,
};
use futures::{AsyncRead, AsyncWrite, AsyncWriteExt};

use crate::endpoint_support::{
    clone_shared, deadline_after as support_deadline_after, prefer_primary, share,
};

use super::{
    BlobChunkMeta, BlobRequest, BlobResponse, BlobsProtocolError, ID, MAX_BLOB_BYTES,
    MAX_BLOB_CHUNK_BYTES, MAX_BLOB_META_BYTES, MAX_BLOB_ROUNDS, is_sha256_hex, read_blob_request,
    read_blob_response, write_blob_request, write_blob_response,
};

/// Maximum number of concurrently served Blob v1 streams.
pub const BLOB_MAX_CONCURRENCY: usize = 16;

/// Fixed deadline for opening one authenticated Blob v1 stream.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

/// Fixed deadline for one wire read, provider lookup, or wire write.
pub const ROUND_TIMEOUT: Duration = Duration::from_secs(30);

/// Fixed deadline for assembling and validating one complete remote blob.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(180);

/// Fixed deadline for closing one stream or mounted registration.
pub const CLOSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Build the exact bounded Blob v1 registration.
pub fn blob_protocol_spec() -> Result<AukiProtocolSpec, AukiProtocolError> {
    AukiProtocolSpec::new(ID, BLOB_MAX_CONCURRENCY, MAX_BLOB_META_BYTES)
}

/// One provider-owned range returned to the Blob v1 endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvidedBlobChunk {
    /// Complete size of the addressed blob, which must remain stable across rounds.
    pub total_size: u64,
    /// Bytes beginning at the request's exact offset.
    pub bytes: Vec<u8>,
}

impl ProvidedBlobChunk {
    /// Construct one provider result. Request-relative bounds are checked by the endpoint.
    pub fn new(total_size: u64, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            total_size,
            bytes: bytes.into(),
        }
    }
}

/// A storage-independent provider failure safe to report to the authenticated caller.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{reason}")]
pub struct BlobProviderError {
    reason: String,
}

impl BlobProviderError {
    /// Construct a provider failure with a stable remote-facing diagnostic.
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    /// Borrow the remote-facing diagnostic.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

/// Future returned by a cross-platform [`BlobProvider`].
#[cfg(not(target_arch = "wasm32"))]
pub type BlobProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ProvidedBlobChunk>, BlobProviderError>> + Send + 'a>>;

/// Future returned by a cross-platform [`BlobProvider`].
#[cfg(target_arch = "wasm32")]
pub type BlobProviderFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<ProvidedBlobChunk>, BlobProviderError>> + 'a>>;

/// Application-owned Blob v1 range provider.
///
/// `None` means the requested content address is absent. Implementations may
/// consult the authenticated peer's subject, type, scopes, or application
/// metadata before returning bytes. The endpoint validates every returned
/// range against the request and never assumes a filesystem.
#[cfg(not(target_arch = "wasm32"))]
pub trait BlobProvider: Send + Sync + 'static {
    /// Resolve one exact range request for one authenticated remote peer.
    fn provide<'a>(
        &'a self,
        remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a>;
}

/// Application-owned Blob v1 range provider.
///
/// `None` means the requested content address is absent. Implementations may
/// consult the authenticated peer's subject, type, scopes, or application
/// metadata before returning bytes. The endpoint validates every returned
/// range against the request and never assumes a filesystem.
#[cfg(target_arch = "wasm32")]
pub trait BlobProvider: 'static {
    /// Resolve one exact range request for one authenticated remote peer.
    fn provide<'a>(
        &'a self,
        remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a>;
}

/// Cloneable outbound half of the Blob v1 endpoint.
#[derive(Clone)]
pub struct BlobClient {
    protocols: AukiPeerProtocols,
}

impl BlobClient {
    /// Construct an outbound client without mounting an inbound provider.
    pub fn new(protocols: AukiPeerProtocols) -> Self {
        Self { protocols }
    }

    /// Fetch and verify one complete blob through the owning native peer's routes.
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn fetch(
        &self,
        remote_peer_id: PeerId,
        sha256: impl Into<String>,
    ) -> Result<BlobFetchReceipt, BlobEndpointError> {
        let sha256 = validate_sha256(sha256.into())?;
        fetch_opened(
            remote_peer_id,
            sha256,
            self.protocols.open(remote_peer_id, ID),
        )
        .await
    }

    /// Fetch and verify one complete blob through one exact advertised route.
    pub async fn fetch_exact(
        &self,
        remote_peer_id: PeerId,
        route: Multiaddr,
        sha256: impl Into<String>,
    ) -> Result<BlobFetchReceipt, BlobEndpointError> {
        let sha256 = validate_sha256(sha256.into())?;
        fetch_opened(
            remote_peer_id,
            sha256,
            self.protocols.open_exact(remote_peer_id, route, ID),
        )
        .await
    }
}

/// Mounted Blob v1 service plus its outbound client.
pub struct BlobEndpoint {
    client: BlobClient,
    registration: AukiProtocolRegistration,
}

impl BlobEndpoint {
    /// Mount Blob v1 on one running peer with application-owned storage.
    pub fn mount<P>(protocols: AukiPeerProtocols, provider: P) -> Result<Self, BlobEndpointError>
    where
        P: BlobProvider,
    {
        let provider = share(provider);
        let registration = protocols.register(blob_protocol_spec()?, move |mut stream| {
            let provider = clone_shared(&provider);
            async move {
                let _ = serve_and_close(&mut stream, provider.as_ref()).await;
            }
        })?;

        Ok(Self {
            client: BlobClient::new(protocols),
            registration,
        })
    }

    /// Clone the outbound client without cloning registration ownership.
    pub fn client(&self) -> BlobClient {
        self.client.clone()
    }

    /// Stop accepting Blob v1 streams and await already-admitted handlers.
    pub async fn close(self) -> Result<(), BlobEndpointError> {
        deadline(BlobOperation::Close, self.registration.close())
            .await?
            .map_err(BlobEndpointError::Sdk)
    }
}

/// One complete, hash-verified remote blob.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobFetchReceipt {
    /// Mutually authenticated peer that served the blob.
    pub remote_peer_id: PeerId,
    /// Requested lowercase SHA-256 content address.
    pub sha256: String,
    /// Complete bytes whose SHA-256 matches `sha256`.
    pub bytes: Vec<u8>,
    /// Whether the selected transport route used a relay circuit.
    pub relayed: bool,
}

impl BlobFetchReceipt {
    /// Borrow the verified complete blob.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the receipt and return the verified complete blob.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

async fn fetch_opened<F>(
    remote_peer_id: PeerId,
    sha256: String,
    opening: F,
) -> Result<BlobFetchReceipt, BlobEndpointError>
where
    F: Future<Output = Result<AuthenticatedRouteStream, AukiProtocolError>>,
{
    let mut stream = deadline(BlobOperation::Open, opening)
        .await?
        .map_err(BlobEndpointError::Sdk)?;
    let relayed = stream.is_relayed();
    let transfer = deadline(
        BlobOperation::Fetch,
        assemble_blob(&mut stream, sha256.as_str()),
    )
    .await
    .and_then(|result| result);
    let cleanup = deadline(BlobOperation::Close, stream.close())
        .await
        .and_then(|result| result.map_err(|error| BlobEndpointError::Close(error.to_string())));
    let bytes = prefer_primary(transfer, cleanup)?;

    Ok(BlobFetchReceipt {
        remote_peer_id,
        sha256,
        bytes,
        relayed,
    })
}

async fn assemble_blob<S>(stream: &mut S, sha256: &str) -> Result<Vec<u8>, BlobEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    validate_sha256(sha256.to_owned())?;
    let mut bytes = Vec::new();
    let mut offset = 0_u64;
    let mut expected_total = None;
    let mut complete = false;

    for _ in 0..MAX_BLOB_ROUNDS {
        let request = BlobRequest {
            sha256: sha256.to_owned(),
            offset,
            max_len: MAX_BLOB_CHUNK_BYTES,
        };
        let (meta, chunk) = blob_round_trip(stream, &request).await?;

        match expected_total {
            None => {
                expected_total = Some(meta.total_size);
                let capacity = usize::try_from(meta.total_size).map_err(|_| {
                    BlobEndpointError::InvalidResponse(
                        "advertised blob size does not fit this platform".into(),
                    )
                })?;
                bytes.try_reserve_exact(capacity).map_err(|_| {
                    BlobEndpointError::InvalidResponse(
                        "cannot reserve the advertised blob size".into(),
                    )
                })?;
            }
            Some(expected) if expected != meta.total_size => {
                return Err(BlobEndpointError::SizeMismatch {
                    expected,
                    actual: meta.total_size,
                });
            }
            Some(_) => {}
        }

        if meta.total_size == 0 {
            complete = true;
            break;
        }
        if chunk.is_empty() {
            return Err(BlobEndpointError::InvalidResponse(
                "empty chunk before end of blob".into(),
            ));
        }

        bytes.extend_from_slice(&chunk);
        offset = offset
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| BlobEndpointError::InvalidResponse("blob offset overflow".into()))?;
        if offset == meta.total_size {
            complete = true;
            break;
        }
        if offset > meta.total_size {
            return Err(BlobEndpointError::InvalidResponse(
                "chunk exceeds advertised size".into(),
            ));
        }
    }

    if !complete {
        return Err(BlobEndpointError::TooManyRounds);
    }
    let expected_total = expected_total.ok_or(BlobEndpointError::TooManyRounds)?;
    if bytes.len() as u64 != expected_total {
        return Err(BlobEndpointError::InvalidResponse(
            "assembled size differs from advertised size".into(),
        ));
    }
    if auki_registry::sha256_hex(&bytes) != sha256 {
        return Err(BlobEndpointError::HashMismatch);
    }
    Ok(bytes)
}

async fn blob_round_trip<S>(
    stream: &mut S,
    request: &BlobRequest,
) -> Result<(BlobChunkMeta, Vec<u8>), BlobEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    deadline(
        BlobOperation::WriteRequest,
        write_blob_request(stream, request),
    )
    .await?
    .map_err(BlobEndpointError::Codec)?;

    let (response, chunk) = deadline(BlobOperation::ReadResponse, read_blob_response(stream))
        .await?
        .map_err(BlobEndpointError::Codec)?;

    match response {
        BlobResponse::NotFound => Err(BlobEndpointError::NotFound),
        BlobResponse::Error { reason } => Err(BlobEndpointError::RemoteError(reason)),
        BlobResponse::Ok(meta) => {
            if meta.sha256 != request.sha256 || meta.offset != request.offset {
                return Err(BlobEndpointError::InvalidResponse(
                    "response address or offset differs from request".into(),
                ));
            }
            if meta.chunk_len > request.max_len || chunk.len() > request.max_len as usize {
                return Err(BlobEndpointError::InvalidResponse(
                    "response chunk exceeds the requested maximum".into(),
                ));
            }
            Ok((meta, chunk))
        }
    }
}

async fn serve_and_close<P>(
    stream: &mut AukiProtocolStream,
    provider: &P,
) -> Result<(), BlobEndpointError>
where
    P: BlobProvider,
{
    let remote_peer = stream.remote_peer().clone();
    let serving = serve_requests(stream, &remote_peer, provider).await;
    let cleanup = deadline(BlobOperation::Close, AsyncWriteExt::close(stream))
        .await
        .and_then(|result| result.map_err(|error| BlobEndpointError::Close(error.to_string())));
    prefer_primary(serving, cleanup)
}

async fn serve_requests<S, P>(
    stream: &mut S,
    remote_peer: &AuthenticatedPeer,
    provider: &P,
) -> Result<(), BlobEndpointError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: BlobProvider,
{
    for _ in 0..MAX_BLOB_ROUNDS {
        let request = match deadline(BlobOperation::ReadRequest, read_blob_request(stream)).await? {
            Ok(request) => request,
            Err(BlobsProtocolError::Io(error)) if is_normal_stream_end(error.kind()) => {
                return Ok(());
            }
            Err(error) => return Err(BlobEndpointError::Codec(error)),
        };

        let provided = deadline(
            BlobOperation::Provide,
            provider.provide(remote_peer, &request),
        )
        .await?;
        let (response, chunk) = provider_response(&request, provided);
        deadline(
            BlobOperation::WriteResponse,
            write_blob_response(stream, &response, &chunk),
        )
        .await?
        .map_err(BlobEndpointError::Codec)?;
    }
    Err(BlobEndpointError::TooManyRounds)
}

fn provider_response(
    request: &BlobRequest,
    provided: Result<Option<ProvidedBlobChunk>, BlobProviderError>,
) -> (BlobResponse, Vec<u8>) {
    let provided = match provided {
        Ok(Some(provided)) => provided,
        Ok(None) => return (BlobResponse::NotFound, Vec::new()),
        Err(error) => {
            return (
                BlobResponse::Error {
                    reason: error.to_string(),
                },
                Vec::new(),
            );
        }
    };

    let chunk_len = match u32::try_from(provided.bytes.len()) {
        Ok(chunk_len) => chunk_len,
        Err(_) => return invalid_provider_response("chunk length does not fit u32"),
    };
    let remaining = match provided.total_size.checked_sub(request.offset) {
        Some(remaining) => remaining,
        None => return invalid_provider_response("request offset exceeds total size"),
    };
    if provided.total_size > MAX_BLOB_BYTES {
        return invalid_provider_response("total size exceeds MAX_BLOB_BYTES");
    }
    if chunk_len > request.max_len || chunk_len > MAX_BLOB_CHUNK_BYTES {
        return invalid_provider_response("chunk exceeds the requested maximum");
    }
    if u64::from(chunk_len) > remaining {
        return invalid_provider_response("chunk exceeds the remaining blob bytes");
    }
    if chunk_len == 0 && request.offset < provided.total_size {
        return invalid_provider_response("empty chunk before end of blob");
    }

    (
        BlobResponse::Ok(BlobChunkMeta {
            sha256: request.sha256.clone(),
            offset: request.offset,
            total_size: provided.total_size,
            chunk_len,
        }),
        provided.bytes,
    )
}

fn invalid_provider_response(reason: &str) -> (BlobResponse, Vec<u8>) {
    (
        BlobResponse::Error {
            reason: format!("invalid blob provider response: {reason}"),
        },
        Vec::new(),
    )
}

fn validate_sha256(sha256: String) -> Result<String, BlobEndpointError> {
    if is_sha256_hex(&sha256) {
        Ok(sha256)
    } else {
        Err(BlobEndpointError::InvalidRequest(
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

async fn deadline<T>(
    operation: BlobOperation,
    future: impl Future<Output = T>,
) -> Result<T, BlobEndpointError> {
    deadline_after(operation, operation.timeout(), future).await
}

async fn deadline_after<T>(
    operation: BlobOperation,
    duration: Duration,
    future: impl Future<Output = T>,
) -> Result<T, BlobEndpointError> {
    support_deadline_after(duration, future, || BlobEndpointError::Timeout(operation)).await
}

/// One fixed-deadline Blob v1 operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlobOperation {
    /// Open and mutually authenticate one application stream.
    Open,
    /// Assemble and verify one complete blob.
    Fetch,
    /// Write one chunk request.
    WriteRequest,
    /// Read one chunk response.
    ReadResponse,
    /// Read one inbound chunk request.
    ReadRequest,
    /// Await one application-owned provider lookup.
    Provide,
    /// Write one inbound chunk response.
    WriteResponse,
    /// Close one stream or protocol registration.
    Close,
}

impl BlobOperation {
    fn timeout(self) -> Duration {
        match self {
            Self::Open => OPEN_TIMEOUT,
            Self::Fetch => FETCH_TIMEOUT,
            Self::WriteRequest
            | Self::ReadResponse
            | Self::ReadRequest
            | Self::Provide
            | Self::WriteResponse => ROUND_TIMEOUT,
            Self::Close => CLOSE_TIMEOUT,
        }
    }
}

impl fmt::Display for BlobOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Open => "open",
            Self::Fetch => "fetch",
            Self::WriteRequest => "request write",
            Self::ReadResponse => "response read",
            Self::ReadRequest => "request read",
            Self::Provide => "provider lookup",
            Self::WriteResponse => "response write",
            Self::Close => "close",
        })
    }
}

/// Failure from the shared Blob v1 endpoint.
#[derive(Debug, thiserror::Error)]
pub enum BlobEndpointError {
    /// The SDK protocol surface rejected registration, opening, or shutdown.
    #[error("Auki protocol operation failed: {0}")]
    Sdk(#[from] AukiProtocolError),
    /// A supplied content address is invalid.
    #[error("blob request is invalid: {0}")]
    InvalidRequest(String),
    /// The remote peer does not have the requested blob.
    #[error("blob was not found")]
    NotFound,
    /// The remote provider reported an application failure.
    #[error("remote blob error: {0}")]
    RemoteError(String),
    /// Blob v1 framing or validation failed.
    #[error("blob codec failed: {0}")]
    Codec(#[from] BlobsProtocolError),
    /// A fixed-deadline endpoint operation did not complete.
    #[error("blob {0} timed out")]
    Timeout(BlobOperation),
    /// Authenticated stream cleanup failed.
    #[error("close authenticated blob stream: {0}")]
    Close(String),
    /// Complete bytes do not match the requested SHA-256.
    #[error("assembled bytes fail SHA-256 verification")]
    HashMismatch,
    /// The remote peer changed its advertised size during one fetch.
    #[error("remote total_size changed mid-fetch: was {expected}, got {actual}")]
    SizeMismatch {
        /// First advertised complete size.
        expected: u64,
        /// Later inconsistent complete size.
        actual: u64,
    },
    /// A response was valid on the wire but inconsistent with the conversation.
    #[error("invalid remote blob response: {0}")]
    InvalidResponse(String),
    /// The bounded conversation did not complete within [`MAX_BLOB_ROUNDS`].
    #[error("blob transfer exceeded MAX_BLOB_ROUNDS")]
    TooManyRounds,
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };

    use futures::io::Cursor;

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

    #[test]
    fn spec_mounts_the_exact_blob_contract() {
        let spec = blob_protocol_spec().unwrap();
        assert_eq!(spec.protocol_id(), ID);
        assert_eq!(spec.max_concurrency(), BLOB_MAX_CONCURRENCY);
        assert_eq!(spec.max_frame_bytes(), MAX_BLOB_META_BYTES);
    }

    #[tokio::test]
    async fn multi_round_fetch_returns_only_verified_complete_bytes() {
        let payload = b"abcdef";
        let sha256 = auki_registry::sha256_hex(payload);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &sha256, 0, 6, b"abc").await;
        encoded_ok(&mut inbound, &sha256, 3, 6, b"def").await;
        let mut stream = ScriptedStream::new(inbound);

        let bytes = assemble_blob(&mut stream, &sha256).await.unwrap();

        assert_eq!(bytes, payload);
        assert!(!stream.outbound.is_empty(), "requests must be written");
    }

    #[tokio::test]
    async fn final_hash_mismatch_returns_no_partial_value() {
        let requested_sha = "a".repeat(64);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &requested_sha, 0, 3, b"bad").await;
        let mut stream = ScriptedStream::new(inbound);

        let result = assemble_blob(&mut stream, &requested_sha).await;

        assert!(matches!(result, Err(BlobEndpointError::HashMismatch)));
    }

    #[tokio::test]
    async fn changed_total_size_rejects_the_whole_assembly() {
        let requested_sha = "b".repeat(64);
        let mut inbound = Vec::new();
        encoded_ok(&mut inbound, &requested_sha, 0, 4, b"ab").await;
        encoded_ok(&mut inbound, &requested_sha, 2, 5, b"cd").await;
        let mut stream = ScriptedStream::new(inbound);

        let result = assemble_blob(&mut stream, &requested_sha).await;

        assert!(matches!(
            result,
            Err(BlobEndpointError::SizeMismatch {
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

        let result = assemble_blob(&mut stream, &requested_sha).await;

        assert!(matches!(result, Err(BlobEndpointError::TooManyRounds)));
    }

    #[test]
    fn provider_output_is_fenced_to_the_request() {
        let request = BlobRequest {
            sha256: "d".repeat(64),
            offset: 3,
            max_len: 2,
        };
        let (response, bytes) = provider_response(
            &request,
            Ok(Some(ProvidedBlobChunk::new(8, b"toolong".to_vec()))),
        );

        assert!(matches!(
            response,
            BlobResponse::Error { ref reason }
                if reason == "invalid blob provider response: chunk exceeds the requested maximum"
        ));
        assert!(bytes.is_empty());
    }

    #[test]
    fn expired_deadline_reports_the_interrupted_operation() {
        let result = futures::executor::block_on(deadline_after(
            BlobOperation::Provide,
            Duration::ZERO,
            futures::future::pending::<()>(),
        ));
        assert!(matches!(
            result,
            Err(BlobEndpointError::Timeout(BlobOperation::Provide))
        ));
    }

    #[test]
    fn exchange_failure_wins_over_cleanup_failure() {
        assert_eq!(
            prefer_primary::<(), _>(Err("exchange"), Err("cleanup")),
            Err("exchange")
        );
        assert_eq!(prefer_primary(Ok(7), Err("cleanup")), Err("cleanup"));
        assert_eq!(prefer_primary::<_, &str>(Ok(7), Ok(())), Ok(7));
    }
}
