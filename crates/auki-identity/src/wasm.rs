use crate::core;
use js_sys::Error;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct Wallet {
    inner: core::Wallet,
}

#[wasm_bindgen]
impl Wallet {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self {
            inner: core::Wallet::new(),
        }
    }

    #[wasm_bindgen(js_name = fromSeed)]
    pub fn from_seed(seed: &[u8]) -> Result<Wallet, JsValue> {
        Ok(Self {
            inner: core::Wallet::from_seed(&seed32(seed)?),
        })
    }

    pub fn seed(&self) -> Vec<u8> {
        self.inner.seed().to_vec()
    }

    #[wasm_bindgen(js_name = publicKey)]
    pub fn public_key(&self) -> Vec<u8> {
        self.inner.public_key().0.to_vec()
    }

    pub fn id(&self) -> String {
        self.inner.id().0
    }

    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.inner.sign(msg).0.to_vec()
    }

    #[wasm_bindgen(js_name = signCanonicalJson)]
    pub fn sign_canonical_json(&self, json: String) -> Result<SignedCanonicalJson, JsValue> {
        let value: serde_json::Value = serde_json::from_str(&json)
            .map_err(|err| js_error(format!("JSON is not valid: {err}")))?;
        let (canonical_bytes, signature) = self.inner.sign_canonical_json(&value);
        Ok(SignedCanonicalJson {
            canonical_bytes,
            signature: signature.0.to_vec(),
        })
    }

    #[wasm_bindgen(js_name = deriveChild)]
    pub fn derive_child(&self, label: String) -> Wallet {
        Self {
            inner: self.inner.derive_child(&label),
        }
    }

    #[wasm_bindgen(js_name = issueCreationCert)]
    pub fn issue_creation_cert(
        &self,
        child: &Wallet,
        label: String,
        created_at_ns: i64,
    ) -> CreationCert {
        CreationCert {
            inner: self
                .inner
                .issue_creation_cert(&child.inner, &label, created_at_ns),
        }
    }
}

impl Default for Wallet {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
pub struct SignedCanonicalJson {
    canonical_bytes: Vec<u8>,
    signature: Vec<u8>,
}

#[wasm_bindgen]
impl SignedCanonicalJson {
    #[wasm_bindgen(getter, js_name = canonicalBytes)]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.canonical_bytes.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn signature(&self) -> Vec<u8> {
        self.signature.clone()
    }
}

#[wasm_bindgen]
pub struct CreationCert {
    inner: core::CreationCert,
}

#[wasm_bindgen]
impl CreationCert {
    #[wasm_bindgen(getter, js_name = parentPubkey)]
    pub fn parent_pubkey(&self) -> Vec<u8> {
        self.inner.parent_pubkey.0.to_vec()
    }

    #[wasm_bindgen(getter, js_name = childPubkey)]
    pub fn child_pubkey(&self) -> Vec<u8> {
        self.inner.child_pubkey.0.to_vec()
    }

    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.inner.label.clone()
    }

    #[wasm_bindgen(getter, js_name = createdAtNs)]
    pub fn created_at_ns(&self) -> i64 {
        self.inner.created_at_ns
    }

    #[wasm_bindgen(getter)]
    pub fn signature(&self) -> Vec<u8> {
        self.inner.signature.0.to_vec()
    }

    pub fn verify(&self) -> Result<(), JsValue> {
        self.inner.verify().map_err(verify_error_to_js)
    }
}

#[wasm_bindgen]
pub fn verify(pubkey: &[u8], msg: &[u8], signature: &[u8]) -> Result<(), JsValue> {
    core::verify(
        &core::PublicKey(bytes32(pubkey, "public key")?),
        msg,
        &core::Signature(bytes64(signature)?),
    )
    .map_err(verify_error_to_js)
}

#[wasm_bindgen(js_name = loadOrMintSeed)]
pub fn load_or_mint_seed(storage_key: String) -> Result<Vec<u8>, JsValue> {
    if storage_key.is_empty() {
        return Err(js_error("storage key must not be empty"));
    }

    let storage = browser_storage()?;
    if let Some(encoded) = storage.get_item(&storage_key).map_err(storage_error)? {
        return decode_seed_hex(&encoded);
    }

    let seed = core::Wallet::new().seed();
    storage
        .set_item(&storage_key, &encode_hex(&seed))
        .map_err(storage_error)?;
    Ok(seed.to_vec())
}

fn browser_storage() -> Result<web_sys::Storage, JsValue> {
    let window = web_sys::window().ok_or_else(|| js_error("window is not available"))?;
    window
        .local_storage()
        .map_err(storage_error)?
        .ok_or_else(|| js_error("localStorage is not available"))
}

fn seed32(seed: &[u8]) -> Result<[u8; 32], JsValue> {
    bytes32(seed, "seed")
}

fn bytes32(bytes: &[u8], name: &str) -> Result<[u8; 32], JsValue> {
    if bytes.len() != 32 {
        return Err(js_error(format!(
            "{name} must be exactly 32 bytes, found {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn bytes64(bytes: &[u8]) -> Result<[u8; 64], JsValue> {
    if bytes.len() != 64 {
        return Err(js_error(format!(
            "signature must be exactly 64 bytes, found {}",
            bytes.len()
        )));
    }
    let mut out = [0u8; 64];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn encode_hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

fn decode_seed_hex(encoded: &str) -> Result<Vec<u8>, JsValue> {
    if encoded.len() != 64 {
        return Err(js_error(format!(
            "stored seed must be exactly 64 hex characters, found {}",
            encoded.len()
        )));
    }

    let mut out = [0u8; 32];
    for (i, pair) in encoded.as_bytes().chunks(2).enumerate() {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        out[i] = (high << 4) | low;
    }
    Ok(out.to_vec())
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => char::from(b'0' + nibble),
        10..=15 => char::from(b'a' + nibble - 10),
        _ => unreachable!("nibble is masked"),
    }
}

fn hex_digit(c: u8) -> Result<u8, JsValue> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(js_error("stored seed contains non-hex characters")),
    }
}

fn verify_error_to_js(err: core::VerifyError) -> JsValue {
    js_error(err.to_string())
}

fn storage_error(err: JsValue) -> JsValue {
    if err.is_undefined() || err.is_null() {
        js_error("browser storage operation failed")
    } else {
        err
    }
}

fn js_error(message: impl AsRef<str>) -> JsValue {
    Error::new(message.as_ref()).into()
}
