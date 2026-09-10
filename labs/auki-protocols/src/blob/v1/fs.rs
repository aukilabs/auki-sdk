//! Native immutable-filesystem provider for Blob v1.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use auki_sdk::AuthenticatedPeer;

use super::endpoint::{BlobProviderError, BlobProviderFuture, ProvidedBlobChunk};
use super::{BlobRequest, endpoint::BlobProvider};

/// Read-only Blob v1 provider rooted at one application directory.
///
/// The root cannot be changed after construction. Requests are resolved only
/// through [`auki_registry::read_blob_range`], which validates the lowercase
/// SHA-256 address, bounds the on-disk size, and reads at most the requested
/// chunk without loading the complete blob.
#[derive(Clone, Debug)]
pub struct FsBlobProvider {
    app_root: PathBuf,
}

impl FsBlobProvider {
    /// Bind one immutable application root.
    pub fn new(app_root: impl Into<PathBuf>) -> Self {
        Self {
            app_root: app_root.into(),
        }
    }

    /// Borrow the fixed application root.
    pub fn app_root(&self) -> &Path {
        &self.app_root
    }

    fn read_range(
        &self,
        request: &BlobRequest,
    ) -> Result<Option<ProvidedBlobChunk>, BlobProviderError> {
        match auki_registry::read_blob_range(
            &self.app_root,
            &request.sha256,
            request.offset,
            request.max_len,
        ) {
            Ok(None) => Ok(None),
            Ok(Some(range)) => Ok(Some(ProvidedBlobChunk::new(range.total_size, range.chunk))),
            Err(auki_registry::Error::BlobOffsetPastEnd) => {
                Err(BlobProviderError::new("offset past end of blob"))
            }
            Err(auki_registry::Error::InvalidBlob(_))
            | Err(auki_registry::Error::BlobHashMismatch) => {
                Err(BlobProviderError::new("invalid blob"))
            }
            Err(_) => Err(BlobProviderError::new("blob read failed")),
        }
    }
}

impl BlobProvider for FsBlobProvider {
    fn provide<'a>(
        &'a self,
        _remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        Box::pin(async move { self.read_range(request) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bounded_ranges_only_from_the_fixed_root() {
        let root = tempfile::tempdir().unwrap();
        let other_root = tempfile::tempdir().unwrap();
        let bytes = b"immutable blob";
        let sha256 = auki_registry::put_blob(root.path(), bytes).unwrap();
        let provider = FsBlobProvider::new(root.path());

        let chunk = provider
            .read_range(&BlobRequest {
                sha256: sha256.clone(),
                offset: 2,
                max_len: 4,
            })
            .unwrap()
            .unwrap();
        assert_eq!(chunk.total_size, bytes.len() as u64);
        assert_eq!(chunk.bytes, bytes[2..6]);

        let other_provider = FsBlobProvider::new(other_root.path());
        assert_eq!(
            other_provider
                .read_range(&BlobRequest {
                    sha256,
                    offset: 0,
                    max_len: 4,
                })
                .unwrap(),
            None
        );
    }

    #[test]
    fn maps_invalid_addresses_and_offsets_to_path_free_errors() {
        let root = tempfile::tempdir().unwrap();
        let sha256 = auki_registry::put_blob(root.path(), b"blob").unwrap();
        let provider = FsBlobProvider::new(root.path());

        let invalid = provider
            .read_range(&BlobRequest {
                sha256: "../outside".into(),
                offset: 0,
                max_len: 1,
            })
            .unwrap_err();
        assert_eq!(invalid.reason(), "invalid blob");

        let past_end = provider
            .read_range(&BlobRequest {
                sha256,
                offset: 5,
                max_len: 1,
            })
            .unwrap_err();
        assert_eq!(past_end.reason(), "offset past end of blob");
    }
}
