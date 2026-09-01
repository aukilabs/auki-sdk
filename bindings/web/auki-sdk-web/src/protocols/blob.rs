use auki_protocols::blob::{BlobClient, BlobFetchReceipt, v1::ID};
use js_sys::{Reflect, Uint8Array};
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::{
    AukiPeer,
    protocol_support::{js_context, js_error, parse_exact_target, peer_protocols, to_js_value},
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
}
