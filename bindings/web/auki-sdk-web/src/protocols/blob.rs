use std::cell::RefCell;

use auki_protocols::blob::{
    BlobClient, BlobEndpoint, BlobFetchReceipt, BlobProvider, BlobProviderError,
    BlobProviderFuture, ProvidedBlobChunk,
    v1::{BlobRequest, ID},
};
use auki_sdk::AuthenticatedPeer;
use js_sys::{Function, Promise, Reflect, Uint8Array};
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::{
    AukiPeer,
    protocol_support::{
        CloseBarrier, authenticated_peer_to_js, javascript_error_reason, js_context, js_error,
        parse_exact_target, peer_protocols, to_js_value,
    },
};

/// Outbound Blob v1 client backed by the portable Rust protocol.
#[wasm_bindgen]
pub struct AukiBlobClient {
    inner: BlobClient,
}

#[wasm_bindgen]
impl AukiBlobClient {
    /// Bind an outbound Blob client to one running browser peer.
    #[wasm_bindgen(constructor)]
    pub fn new(peer: &AukiPeer) -> Result<AukiBlobClient, JsValue> {
        Ok(Self {
            inner: BlobClient::new(peer_protocols(peer, "Blob")?),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this client.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Fetch and SHA-256-verify one complete blob through an exact advertised route.
    #[wasm_bindgen(js_name = fetchExact, unchecked_return_type = "AukiBlobReceipt")]
    pub async fn fetch_exact(
        &self,
        #[wasm_bindgen(unchecked_param_type = "AukiExactTarget")] target: JsValue,
        sha256: String,
    ) -> Result<JsValue, JsValue> {
        let (peer_id, route) = parse_exact_target(target)?;
        let receipt = self
            .inner
            .fetch_exact(peer_id, route, sha256)
            .await
            .map_err(|error| js_context("fetch blob", error))?;
        blob_receipt_to_js(receipt)
    }
}

/// Mounted inbound Blob v1 service backed by one asynchronous JavaScript provider.
#[wasm_bindgen]
pub struct AukiBlobEndpoint {
    inner: RefCell<Option<BlobEndpoint>>,
    closing: CloseBarrier,
}

#[wasm_bindgen]
impl AukiBlobEndpoint {
    /// Mount Blob v1 on one running browser peer.
    ///
    /// The provider may return either a value or a Promise. Rust retains all
    /// request validation, range bounds, framing, deadlines, and stream cleanup.
    #[wasm_bindgen(js_name = mount)]
    pub fn mount(
        peer: &AukiPeer,
        #[wasm_bindgen(unchecked_param_type = "AukiBlobProvider")] provider: Function,
    ) -> Result<AukiBlobEndpoint, JsValue> {
        let endpoint = BlobEndpoint::mount(
            peer_protocols(peer, "Blob endpoint")?,
            JavaScriptBlobProvider { provider },
        )
        .map_err(|error| js_context("mount Blob endpoint", error))?;
        Ok(Self {
            inner: RefCell::new(Some(endpoint)),
            closing: CloseBarrier::default(),
        })
    }

    /// Immutable authenticated protocol identifier implemented by this endpoint.
    #[wasm_bindgen(getter)]
    pub fn protocol(&self) -> String {
        ID.to_owned()
    }

    /// Idempotently stop accepting streams and await every admitted handler.
    #[wasm_bindgen(unchecked_return_type = "Promise<void>")]
    pub fn close(&self) -> Promise {
        self.closing.get_or_start(|| {
            let endpoint = self.inner.borrow_mut().take();
            future_to_promise(async move {
                if let Some(endpoint) = endpoint {
                    endpoint
                        .close()
                        .await
                        .map_err(|error| js_context("close Blob endpoint", error))?;
                }
                Ok(JsValue::UNDEFINED)
            })
        })
    }
}

struct JavaScriptBlobProvider {
    provider: Function,
}

impl BlobProvider for JavaScriptBlobProvider {
    fn provide<'a>(
        &'a self,
        remote_peer: &'a AuthenticatedPeer,
        request: &'a BlobRequest,
    ) -> BlobProviderFuture<'a> {
        let invocation = invoke_blob_provider(&self.provider, remote_peer, request);
        Box::pin(async move {
            let value = invocation?;
            resolve_blob_provider_result(value).await
        })
    }
}

fn invoke_blob_provider(
    provider: &Function,
    remote_peer: &AuthenticatedPeer,
    request: &BlobRequest,
) -> Result<JsValue, BlobProviderError> {
    let remote_peer = authenticated_peer_to_js("convert authenticated Blob requester", remote_peer)
        .map_err(|error| blob_provider_error("convert requester", &error))?;
    let request = blob_request_to_js(request)
        .map_err(|error| blob_provider_error("convert request", &error))?;
    provider
        .call2(&JsValue::UNDEFINED, &remote_peer, &request)
        .map_err(|error| blob_provider_error("callback threw", &error))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BlobRequestRecord<'a> {
    sha256: &'a str,
    offset: u64,
    max_len: u32,
}

fn blob_request_to_js(request: &BlobRequest) -> Result<JsValue, JsValue> {
    to_js_value(
        "convert Blob provider request",
        &BlobRequestRecord {
            sha256: &request.sha256,
            offset: request.offset,
            max_len: request.max_len,
        },
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProvidedBlobChunkRecord {
    total_size: u64,
}

fn blob_provider_result_from_js(
    value: JsValue,
) -> Result<Option<ProvidedBlobChunk>, BlobProviderError> {
    if value.is_null() || value.is_undefined() {
        return Ok(None);
    }
    let record: ProvidedBlobChunkRecord = serde_wasm_bindgen::from_value(value.clone())
        .map_err(|error| BlobProviderError::new(format!("invalid response: {error}")))?;
    let bytes = Reflect::get(&value, &JsValue::from_str("bytes"))
        .map_err(|error| blob_provider_error("read response bytes", &error))?;
    if !bytes.is_instance_of::<Uint8Array>() {
        return Err(BlobProviderError::new(
            "invalid response: bytes must be a Uint8Array",
        ));
    }
    Ok(Some(ProvidedBlobChunk::new(
        record.total_size,
        Uint8Array::new(&bytes).to_vec(),
    )))
}

async fn resolve_blob_provider_result(
    value: JsValue,
) -> Result<Option<ProvidedBlobChunk>, BlobProviderError> {
    let value = JsFuture::from(Promise::resolve(&value))
        .await
        .map_err(|error| blob_provider_error("callback rejected", &error))?;
    blob_provider_result_from_js(value)
}

fn blob_provider_error(context: &'static str, error: &JsValue) -> BlobProviderError {
    BlobProviderError::new(format!("{context}: {}", javascript_error_reason(error)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BlobReceiptRecord {
    peer_id: String,
    sha256: String,
    relayed: bool,
}

fn blob_receipt_to_js(receipt: BlobFetchReceipt) -> Result<JsValue, JsValue> {
    let bytes = Uint8Array::from(receipt.bytes.as_slice());
    let value = to_js_value(
        "convert blob receipt",
        &BlobReceiptRecord {
            peer_id: receipt.remote_peer_id.to_string(),
            sha256: receipt.sha256,
            relayed: receipt.relayed,
        },
    )?;
    set_property(&value, "bytes", bytes.as_ref())?;
    Ok(value)
}

fn set_property(target: &JsValue, name: &str, value: &JsValue) -> Result<(), JsValue> {
    if Reflect::set(target, &JsValue::from_str(name), value)? {
        Ok(())
    } else {
        Err(js_error(format!("set blob receipt {name}")))
    }
}

#[wasm_bindgen(typescript_custom_section)]
const BLOB_RECEIPT_TYPESCRIPT: &str = r#"
/** Complete SHA-256-verified bytes returned by Blob v1. */
export interface AukiBlobReceipt {
    readonly peerId: string;
    readonly sha256: string;
    readonly bytes: Uint8Array;
    readonly relayed: boolean;
}

/** One exact, endpoint-validated range requested from browser storage. */
export interface AukiBlobProviderRequest {
    readonly sha256: string;
    readonly offset: bigint;
    readonly maxLen: number;
}

/** One request-relative range returned by browser storage. */
export interface AukiProvidedBlobChunk {
    readonly totalSize: bigint;
    readonly bytes: Uint8Array;
}

export type AukiBlobProviderResult = AukiProvidedBlobChunk | null | undefined;

export type AukiBlobProvider = (
    requester: AukiAuthenticatedPeer,
    request: AukiBlobProviderRequest,
) => AukiBlobProviderResult | Promise<AukiBlobProviderResult>;
"#;

#[cfg(test)]
mod tests {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::wasm_bindgen_test;

    use super::*;

    fn peer_id() -> auki_sdk::PeerId {
        "12D3KooWH3okqZcRaHwy4keYWo9eAaCDwhePYajtHsCM4Egsptan"
            .parse()
            .unwrap()
    }

    #[wasm_bindgen_test]
    fn blob_receipt_is_a_plain_record_with_typed_bytes() {
        let expected_peer = peer_id();
        let expected_hash = "00".repeat(32);
        let value = blob_receipt_to_js(BlobFetchReceipt {
            remote_peer_id: expected_peer,
            sha256: expected_hash.clone(),
            bytes: vec![0, 1, 127, 255],
            relayed: true,
        })
        .unwrap();

        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("peerId"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some(expected_peer.to_string().as_str())
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("sha256"))
                .unwrap()
                .as_string()
                .as_deref(),
            Some(expected_hash.as_str())
        );
        let bytes = Reflect::get(&value, &JsValue::from_str("bytes")).unwrap();
        assert!(bytes.is_instance_of::<Uint8Array>());
        assert_eq!(
            bytes.unchecked_into::<Uint8Array>().to_vec(),
            [0, 1, 127, 255]
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("relayed"))
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert!(!Reflect::has(&value, &JsValue::from_str("free")).unwrap());
    }

    #[wasm_bindgen_test]
    fn blob_provider_requests_preserve_bigint_offsets() {
        let value = blob_request_to_js(&BlobRequest {
            sha256: "0".repeat(64),
            offset: u64::MAX,
            max_len: 1024,
        })
        .unwrap();

        assert!(
            Reflect::get(&value, &JsValue::from_str("offset"))
                .unwrap()
                .is_bigint()
        );
        assert_eq!(
            Reflect::get(&value, &JsValue::from_str("maxLen"))
                .unwrap()
                .as_f64(),
            Some(1024.0)
        );
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ProvidedBlobFixture {
        total_size: u64,
    }

    fn provided_blob_fixture() -> JsValue {
        let value = to_js_value("convert fixture", &ProvidedBlobFixture { total_size: 4 }).unwrap();
        set_property(
            &value,
            "bytes",
            Uint8Array::from([1, 2, 3, 4].as_slice()).as_ref(),
        )
        .unwrap();
        value
    }

    #[wasm_bindgen_test]
    fn blob_provider_results_require_typed_bytes_and_preserve_size() {
        assert_eq!(
            blob_provider_result_from_js(provided_blob_fixture()).unwrap(),
            Some(ProvidedBlobChunk::new(4, vec![1, 2, 3, 4]))
        );
        assert!(
            blob_provider_result_from_js(JsValue::NULL)
                .unwrap()
                .is_none()
        );

        let invalid = to_js_value(
            "convert invalid fixture",
            &ProvidedBlobFixture { total_size: 4 },
        )
        .unwrap();
        set_property(&invalid, "bytes", &JsValue::from_str("not bytes")).unwrap();
        assert!(
            blob_provider_result_from_js(invalid)
                .unwrap_err()
                .to_string()
                .contains("Uint8Array")
        );
    }

    #[wasm_bindgen_test(async)]
    async fn blob_provider_accepts_plain_values_and_promises() {
        let plain = resolve_blob_provider_result(provided_blob_fixture())
            .await
            .unwrap();
        let promised =
            resolve_blob_provider_result(Promise::resolve(&provided_blob_fixture()).into())
                .await
                .unwrap();
        assert_eq!(plain, promised);

        let error =
            resolve_blob_provider_result(Promise::reject(&JsValue::from_str("denied")).into())
                .await
                .unwrap_err();
        assert!(error.to_string().contains("denied"));
    }
}
