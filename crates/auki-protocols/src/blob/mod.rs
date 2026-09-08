//! Content-addressed blob protocols.

pub mod v1;

#[cfg(all(feature = "blob-fs-provider", not(target_arch = "wasm32")))]
pub use v1::FsBlobProvider;
#[cfg(feature = "blob-endpoint")]
pub use v1::{
    BLOB_MAX_CONCURRENCY, BlobClient, BlobEndpoint, BlobEndpointError, BlobFetchReceipt,
    BlobOperation, BlobProvider, BlobProviderError, BlobProviderFuture, CLOSE_TIMEOUT,
    FETCH_TIMEOUT, OPEN_TIMEOUT, ProvidedBlobChunk, ROUND_TIMEOUT, blob_protocol_spec,
};
