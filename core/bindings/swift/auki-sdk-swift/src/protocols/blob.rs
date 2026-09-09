//! Swift adapter for bounded content-addressed Blob v1 transfers.

use std::{collections::BTreeMap, sync::Arc};

use auki_protocols::blob::v1::{
    BlobClient, BlobEndpoint, BlobProvider, BlobProviderError, BlobProviderFuture, BlobRequest, ID,
    MAX_BLOB_BYTES, ProvidedBlobChunk,
};
use auki_sdk_rs::AuthenticatedPeer;
use parking_lot::RwLock;
use sha2::{Digest, Sha256};

use crate::{AukiPeer, AukiPeerTarget, AukiSdkError, operation_error, wait_cleanup};

use super::finite_support::{CloseFuture, EndpointOwner, exact_target};

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiStoredBlob {
    pub sha256: String,
    pub total_size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, uniffi::Record)]
pub struct AukiBlobReceipt {
    pub remote_peer_id: String,
    pub sha256: String,
    pub bytes: Vec<u8>,
    pub relayed: bool,
}

#[derive(Clone, Default)]
struct BlobStore {
    blobs: Arc<RwLock<BTreeMap<String, Arc<[u8]>>>>,
}

impl BlobStore {
    fn put(&self, bytes: Vec<u8>) -> Result<AukiStoredBlob, AukiSdkError> {
        self.put_with_store_limit(bytes, MAX_BLOB_BYTES)
    }

    fn put_with_store_limit(
        &self,
        bytes: Vec<u8>,
        maximum_store_bytes: u64,
    ) -> Result<AukiStoredBlob, AukiSdkError> {
        let total_size = bytes.len() as u64;
        ensure_blob_size(total_size)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let mut blobs = self.blobs.write();
        let replaced_size = blobs
            .get(&sha256)
            .map_or(0, |existing| existing.len() as u64);
        let stored_size = blobs
            .values()
            .fold(0_u64, |total, blob| total.saturating_add(blob.len() as u64));
        let next_stored_size = stored_size
            .saturating_sub(replaced_size)
            .saturating_add(total_size);
        if next_stored_size > maximum_store_bytes {
            return Err(operation_error(
                "put Blob",
                format!(
                    "in-memory Blob store would contain {next_stored_size} bytes; maximum is {maximum_store_bytes}"
                ),
            ));
        }
        blobs.insert(sha256.clone(), Arc::from(bytes));
        Ok(AukiStoredBlob { sha256, total_size })
    }

    fn remove(&self, sha256: &str) -> bool {
        self.blobs.write().remove(sha256).is_some()
    }

    fn clear(&self) {
        self.blobs.write().clear();
    }
}

fn ensure_blob_size(total_size: u64) -> Result<(), AukiSdkError> {
    if total_size > MAX_BLOB_BYTES {
        Err(operation_error(
            "put Blob",
            format!("blob is {total_size} bytes; maximum is {MAX_BLOB_BYTES}"),
        ))
    } else {
        Ok(())
    }
}

impl BlobProvider for BlobStore {
    fn provide<'a>(
        &'a self,
        _remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        let bytes = self.blobs.read().get(&request.sha256).cloned();
        let offset = request.offset;
        let maximum = request.max_len;
        Box::pin(async move {
            let Some(bytes) = bytes else {
                return Ok(None);
            };
            let start = usize::try_from(offset)
                .map_err(|_| BlobProviderError::new("blob offset does not fit this platform"))?;
            if start > bytes.len() {
                return Err(BlobProviderError::new("blob offset exceeds stored size"));
            }
            let requested = usize::try_from(maximum)
                .map_err(|_| BlobProviderError::new("blob range does not fit this platform"))?;
            let end = start.saturating_add(requested).min(bytes.len());
            Ok(Some(ProvidedBlobChunk::new(
                bytes.len() as u64,
                bytes[start..end].to_vec(),
            )))
        })
    }
}

fn close_endpoint(endpoint: BlobEndpoint) -> CloseFuture {
    Box::pin(async move { endpoint.close().await.map_err(|error| error.to_string()) })
}

#[derive(uniffi::Object)]
pub struct AukiBlobClient {
    inner: BlobClient,
    domain_id: String,
}

impl AukiBlobClient {
    fn from_inner(inner: BlobClient, domain_id: String) -> Arc<Self> {
        Arc::new(Self { inner, domain_id })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiBlobClient {
    #[uniffi::constructor]
    pub fn new(peer: Arc<AukiPeer>) -> Arc<Self> {
        Self::from_inner(BlobClient::new(peer.rust_protocols()), peer.domain_id())
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    /// Fetch one complete blob only after Rust verifies its SHA-256 address.
    pub async fn fetch_exact(
        &self,
        target: AukiPeerTarget,
        sha256: String,
    ) -> Result<AukiBlobReceipt, AukiSdkError> {
        let (peer_id, route) = exact_target(&self.domain_id, target)?;
        let receipt = self
            .inner
            .fetch_exact(peer_id, route, sha256)
            .await
            .map_err(|error| operation_error("fetch Blob", error))?;
        Ok(AukiBlobReceipt {
            remote_peer_id: receipt.remote_peer_id.to_string(),
            sha256: receipt.sha256,
            bytes: receipt.bytes,
            relayed: receipt.relayed,
        })
    }
}

/// Blob v1 endpoint backed by one bounded process-local content-addressed map.
#[derive(uniffi::Object)]
pub struct AukiBlobEndpoint {
    owner: EndpointOwner<BlobEndpoint>,
    store: BlobStore,
    client: Arc<AukiBlobClient>,
}

#[uniffi::export(async_runtime = "tokio")]
impl AukiBlobEndpoint {
    #[uniffi::constructor]
    pub async fn mount(peer: Arc<AukiPeer>) -> Result<Arc<Self>, AukiSdkError> {
        let store = BlobStore::default();
        let endpoint = BlobEndpoint::mount(peer.rust_protocols(), store.clone())
            .map_err(|error| operation_error("mount Blob endpoint", error))?;
        let client = AukiBlobClient::from_inner(endpoint.client(), peer.domain_id());
        Ok(Arc::new(Self {
            owner: EndpointOwner::new(endpoint, close_endpoint),
            store,
            client,
        }))
    }

    pub fn protocol(&self) -> String {
        ID.into()
    }

    pub fn client(&self) -> Arc<AukiBlobClient> {
        Arc::clone(&self.client)
    }

    /// Hash and atomically insert one bounded blob.
    pub fn put(&self, bytes: Vec<u8>) -> Result<AukiStoredBlob, AukiSdkError> {
        self.owner.ensure_open("put Blob")?;
        self.store.put(bytes)
    }

    pub fn remove(&self, sha256: String) -> Result<bool, AukiSdkError> {
        self.owner.ensure_open("remove Blob")?;
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(operation_error(
                "remove Blob",
                "sha256 must be 64 lowercase hexadecimal characters",
            ));
        }
        Ok(self.store.remove(&sha256))
    }

    pub fn clear(&self) -> Result<(), AukiSdkError> {
        self.owner.ensure_open("clear Blobs")?;
        self.store.clear();
        Ok(())
    }

    pub async fn close(&self) -> Result<(), AukiSdkError> {
        wait_cleanup(self.owner.begin_close())
            .await
            .map_err(|error| operation_error("close Blob endpoint", error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_hashes_content_and_serves_exact_ranges() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let store = BlobStore::default();
            let stored = store.put(b"hello blob".to_vec()).unwrap();
            assert_eq!(
                stored.sha256,
                "e997afd18e5f6be004fc193aed2c90291e68ab2c7599a62538c935b7fca6ab0f"
            );
            let requester = super::super::finite_support::authenticated_peer(
                auki_sdk_rs::Identity::generate().peer_id(),
            );
            let request = BlobRequest {
                sha256: stored.sha256,
                offset: 6,
                max_len: 4,
            };
            let chunk = store.provide(&requester, &request).await.unwrap().unwrap();
            assert_eq!(chunk.total_size, 10);
            assert_eq!(chunk.bytes, b"blob");
        });
    }

    #[test]
    fn store_rejects_the_protocol_maximum() {
        assert!(ensure_blob_size(MAX_BLOB_BYTES).is_ok());
        assert!(ensure_blob_size(MAX_BLOB_BYTES + 1).is_err());
    }

    #[test]
    fn store_bounds_aggregate_memory_and_does_not_double_count_replacement() {
        let store = BlobStore::default();
        store.put_with_store_limit(b"abc".to_vec(), 5).unwrap();
        store.put_with_store_limit(b"abc".to_vec(), 5).unwrap();
        assert!(store.put_with_store_limit(b"def".to_vec(), 5).is_err());
    }
}
