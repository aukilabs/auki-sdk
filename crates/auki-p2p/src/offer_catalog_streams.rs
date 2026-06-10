//! libp2p stream helpers for offer-catalog loading and serving.

use crate::{
    OfferCatalogClient, OfferCatalogClientError, OfferLoadContext, OfferLoadError, OfferLoadReport,
    PeerRelationship, RuntimeLimits, load_remote_offers_with_client,
    protocols::offer_catalog_protocol,
};
use auki_protocol::v1::{
    error,
    frame::{self, FrameError},
    offer::{OfferCatalogRequest, OfferCatalogRequestError, OfferCatalogResponse},
};
use futures::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use libp2p::PeerId;
use std::{fmt, io};

/// libp2p-stream-backed client for outgoing offer-catalog loads.
pub struct Libp2pOfferCatalogClient {
    control: libp2p_stream::Control,
    limits: RuntimeLimits,
}

/// Errors produced while serving one inbound offer-catalog request.
#[derive(Debug)]
pub enum OfferCatalogServeError {
    /// Underlying stream I/O failed.
    Io(io::Error),
    /// RFC JSON frame encoding or decoding failed.
    Frame(FrameError),
    /// Decoded frame was not a valid offer-catalog request.
    Request(OfferCatalogRequestError),
}

struct CapturedOfferCatalogClient {
    frame: Result<Vec<u8>, OfferCatalogClientError>,
}

impl Libp2pOfferCatalogClient {
    /// Create an offer-catalog client from a raw libp2p stream control handle.
    pub fn new(control: libp2p_stream::Control, limits: RuntimeLimits) -> Self {
        Self { control, limits }
    }

    /// Borrow the configured runtime limits.
    pub fn limits(&self) -> RuntimeLimits {
        self.limits
    }

    /// Fetch one encoded v1 offer-catalog response frame over libp2p.
    pub async fn fetch_offer_catalog_frame(
        &mut self,
        peer_id: PeerId,
        request: OfferCatalogRequest,
    ) -> Result<Vec<u8>, OfferCatalogClientError> {
        let mut stream = self
            .control
            .open_stream(peer_id, offer_catalog_protocol())
            .await
            .map_err(open_stream_error)?;

        write_json_frame(
            &mut stream,
            request.value(),
            self.limits.catalog_response_frame_body_bytes,
            "offer catalog request",
        )
        .await?;

        let response = read_frame_bytes(
            &mut stream,
            self.limits.catalog_response_frame_body_bytes,
            "offer catalog response",
        )
        .await?;
        close_stream(&mut stream, "offer catalog").await?;
        Ok(response)
    }
}

impl OfferCatalogClient for CapturedOfferCatalogClient {
    fn fetch_offer_catalog_frame(
        &mut self,
        _peer_id: PeerId,
    ) -> Result<Vec<u8>, OfferCatalogClientError> {
        self.frame.clone()
    }
}

/// Accept inbound offer-catalog streams on a libp2p-stream control.
pub fn accept_offer_catalog_streams(
    control: &mut libp2p_stream::Control,
) -> Result<libp2p_stream::IncomingStreams, libp2p_stream::AlreadyRegistered> {
    control.accept(offer_catalog_protocol())
}

/// Fetch and load remote offers through a libp2p offer-catalog stream.
pub async fn load_remote_offers_over_libp2p(
    relationship: &mut PeerRelationship,
    client: &mut Libp2pOfferCatalogClient,
    request: OfferCatalogRequest,
    context: OfferLoadContext<'_>,
) -> Result<OfferLoadReport, OfferLoadError> {
    if !relationship.authorized {
        let mut skipped_client = CapturedOfferCatalogClient {
            frame: Ok(Vec::new()),
        };
        return load_remote_offers_with_client(relationship, &mut skipped_client, context);
    }

    let frame = client
        .fetch_offer_catalog_frame(relationship.peer_id, request)
        .await;
    let mut captured_client = CapturedOfferCatalogClient { frame };
    load_remote_offers_with_client(relationship, &mut captured_client, context)
}

/// Serve one inbound offer-catalog request with a complete local catalog response.
pub async fn serve_offer_catalog_response<S>(
    stream: &mut S,
    response: &OfferCatalogResponse,
    limits: RuntimeLimits,
) -> Result<OfferCatalogRequest, OfferCatalogServeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let request_frame =
        read_offer_catalog_request_frame(stream, limits.catalog_response_frame_body_bytes).await?;
    let request =
        OfferCatalogRequest::from_value(request_frame).map_err(OfferCatalogServeError::Request)?;
    write_offer_catalog_response_frame(stream, response, limits.catalog_response_frame_body_bytes)
        .await?;
    close_offer_catalog_stream(stream).await?;
    Ok(request)
}

async fn write_json_frame<S>(
    stream: &mut S,
    value: &serde_json::Value,
    max_body_len: u64,
    label: &'static str,
) -> Result<(), OfferCatalogClientError>
where
    S: AsyncWrite + Unpin,
{
    let frame = frame::encode_json_frame(value, max_body_len).map_err(|error| {
        OfferCatalogClientError::new(
            frame_error_code(&error),
            format!("{label} frame encode failed: {error}"),
            false,
        )
    })?;
    stream
        .write_all(&frame)
        .await
        .map_err(|error| client_io_error(label, error))?;
    stream
        .flush()
        .await
        .map_err(|error| client_io_error(label, error))
}

async fn read_frame_bytes<S>(
    stream: &mut S,
    max_body_len: u64,
    label: &'static str,
) -> Result<Vec<u8>, OfferCatalogClientError>
where
    S: AsyncRead + Unpin,
{
    read_complete_frame(stream, max_body_len)
        .await
        .map_err(|error| match error {
            FrameReadError::Io(error) => client_io_error(label, error),
            FrameReadError::Frame(error) => OfferCatalogClientError::new(
                frame_error_code(&error),
                format!("{label} frame decode failed: {error}"),
                false,
            ),
        })
}

async fn close_stream<S>(stream: &mut S, label: &'static str) -> Result<(), OfferCatalogClientError>
where
    S: AsyncWrite + Unpin,
{
    stream
        .close()
        .await
        .map_err(|error| client_io_error(label, error))
}

async fn read_offer_catalog_request_frame<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<serde_json::Value, OfferCatalogServeError>
where
    S: AsyncRead + Unpin,
{
    let frame = read_complete_frame(stream, max_body_len)
        .await
        .map_err(serve_frame_read_error)?;
    let (value, consumed) =
        frame::decode_json_frame(&frame, max_body_len).map_err(OfferCatalogServeError::Frame)?;
    debug_assert_eq!(consumed, frame.len());
    Ok(value)
}

async fn write_offer_catalog_response_frame<S>(
    stream: &mut S,
    response: &OfferCatalogResponse,
    max_body_len: u64,
) -> Result<(), OfferCatalogServeError>
where
    S: AsyncWrite + Unpin,
{
    let frame = frame::encode_json_frame(response.value(), max_body_len)
        .map_err(OfferCatalogServeError::Frame)?;
    stream
        .write_all(&frame)
        .await
        .map_err(OfferCatalogServeError::Io)?;
    stream.flush().await.map_err(OfferCatalogServeError::Io)?;
    Ok(())
}

async fn close_offer_catalog_stream<S>(stream: &mut S) -> Result<(), OfferCatalogServeError>
where
    S: AsyncWrite + Unpin,
{
    stream.close().await.map_err(OfferCatalogServeError::Io)
}

enum FrameReadError {
    Io(io::Error),
    Frame(FrameError),
}

async fn read_complete_frame<S>(
    stream: &mut S,
    max_body_len: u64,
) -> Result<Vec<u8>, FrameReadError>
where
    S: AsyncRead + Unpin,
{
    let mut prefix = Vec::with_capacity(frame::MAX_LEB128_U64_BYTES);

    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(FrameReadError::Io)?;
        prefix.push(byte[0]);

        match frame::decode_length(&prefix, max_body_len) {
            Ok((body_len, prefix_len)) => {
                debug_assert_eq!(prefix_len, prefix.len());
                let body_len = usize::try_from(body_len)
                    .map_err(|_| FrameReadError::Frame(FrameError::LengthOverflow))?;
                let mut body = vec![0u8; body_len];
                stream
                    .read_exact(&mut body)
                    .await
                    .map_err(FrameReadError::Io)?;

                let mut complete = prefix;
                complete.extend_from_slice(&body);
                return Ok(complete);
            }
            Err(FrameError::UnexpectedEof) if prefix.len() < frame::MAX_LEB128_U64_BYTES => {}
            Err(error) => return Err(FrameReadError::Frame(error)),
        }
    }
}

fn open_stream_error(error: libp2p_stream::OpenStreamError) -> OfferCatalogClientError {
    OfferCatalogClientError::new(
        error::TRANSPORT_FAILED,
        format!("offer catalog stream open failed: {error}"),
        true,
    )
}

fn client_io_error(label: &'static str, error: io::Error) -> OfferCatalogClientError {
    OfferCatalogClientError::new(
        error::TRANSPORT_FAILED,
        format!("{label} stream io failed: {error}"),
        true,
    )
}

fn frame_error_code(error: &FrameError) -> &'static str {
    match error {
        FrameError::BodyTooLarge { .. } => error::MESSAGE_PAYLOAD_TOO_LARGE,
        _ => error::TRANSPORT_FAILED,
    }
}

fn serve_frame_read_error(error: FrameReadError) -> OfferCatalogServeError {
    match error {
        FrameReadError::Io(error) => OfferCatalogServeError::Io(error),
        FrameReadError::Frame(error) => OfferCatalogServeError::Frame(error),
    }
}

impl fmt::Display for OfferCatalogServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "offer catalog stream io: {error}"),
            Self::Frame(error) => write!(f, "offer catalog frame: {error}"),
            Self::Request(error) => write!(f, "offer catalog request: {error}"),
        }
    }
}

impl std::error::Error for OfferCatalogServeError {}
