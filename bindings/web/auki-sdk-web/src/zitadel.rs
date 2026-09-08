//! Browser-only values; all refresh/storage ordering remains in AuthSession.
use auki_sdk::{
    AukiPeerBootstrapError, AuthError, AuthFailureKind, ZitadelSessionCredentials,
    ZitadelSessionStore,
};
use chrono::{DateTime, SecondsFormat, Utc};
use js_sys::{Function, Promise, Reflect};
use send_wrapper::SendWrapper;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** Trusted host PKCE result. Stop all other refresh owners before importing. */
export interface ZitadelSessionCredentials {
  accessToken: string;
  refreshToken: string;
  clientId: string;
  issuer: string;
  /** RFC 3339; snapshots return UTC with a Z suffix, preserving nanoseconds. */
  accessTokenExpiresAt?: string | null;
}
/** Settle ALL writes before resolve/reject. No detached writes or session reentry. */
export type ZitadelSessionStore = (credentials: ZitadelCredentialsSnapshot) => Promise<void>;
export type AukiAuthFailureCode = "authentication_required" | "configuration" |
  "authorization_denied" | "persistence" | "transient" | "cancelled" | "closed";
/** Auth operation/startup and terminal waitStopped errors have this shape. */
export interface AukiAuthError extends Error { readonly code: AukiAuthFailureCode; }
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "ZitadelSessionCredentials")]
    pub type CredentialsPayload;
    #[wasm_bindgen(typescript_type = "ZitadelSessionStore")]
    pub type StoreCallback;
}

pub(crate) fn parse_credentials(value: &JsValue) -> Result<ZitadelSessionCredentials, JsValue> {
    let invalid = || auth_error(AuthFailureKind::Configuration);
    let field = |name: &str| Reflect::get(value, &name.into()).map_err(|_| invalid());
    let string = |name: &str| field(name)?.as_string().ok_or_else(invalid);
    let expiry = field("accessTokenExpiresAt")?;
    let expiry = if expiry.is_null() || expiry.is_undefined() {
        None
    } else {
        Some(
            DateTime::parse_from_rfc3339(&expiry.as_string().ok_or_else(invalid)?)
                .map_err(|_| invalid())?
                .with_timezone(&Utc),
        )
    };
    ZitadelSessionCredentials::new(
        string("accessToken")?,
        string("refreshToken")?,
        string("clientId")?,
        string("issuer")?.parse().map_err(|_| invalid())?,
        expiry,
    )
    .map_err(|e| auth_error(e.kind()))
}

/// An opaque, redacted credential snapshot. Only explicit methods expose tokens.
#[wasm_bindgen]
pub struct ZitadelCredentialsSnapshot {
    credentials: ZitadelSessionCredentials,
}

impl ZitadelCredentialsSnapshot {
    fn copy_from(c: &ZitadelSessionCredentials) -> Self {
        Self {
            credentials: ZitadelSessionCredentials::new(
                c.access_token().expose_secret(),
                c.refresh_token().expose_secret(),
                c.client_id(),
                c.issuer().clone(),
                c.access_token_expires_at(),
            )
            .expect("copy of validated credentials"),
        }
    }
}

#[wasm_bindgen]
impl ZitadelCredentialsSnapshot {
    #[wasm_bindgen(js_name = exposeAccessToken)]
    pub fn expose_access_token(&self) -> String {
        self.credentials.access_token().expose_secret().into()
    }
    #[wasm_bindgen(js_name = exposeRefreshToken)]
    pub fn expose_refresh_token(&self) -> String {
        self.credentials.refresh_token().expose_secret().into()
    }
    #[wasm_bindgen(getter, js_name = clientId)]
    pub fn client_id(&self) -> String {
        self.credentials.client_id().into()
    }
    #[wasm_bindgen(getter)]
    pub fn issuer(&self) -> String {
        self.credentials.issuer().to_string()
    }
    #[wasm_bindgen(getter, js_name = accessTokenExpiresAt)]
    pub fn access_token_expires_at(&self) -> Option<String> {
        self.credentials
            .access_token_expires_at()
            .map(|t| t.to_rfc3339_opts(SecondsFormat::AutoSi, true))
    }
    #[wasm_bindgen(js_name = toJSON)]
    pub fn to_json(&self) -> String {
        "[redacted Zitadel credentials]".into()
    }
    #[wasm_bindgen(js_name = toString)]
    pub fn to_string_redacted(&self) -> String {
        self.to_json()
    }
}

// This facade and the core's spawn_local owner live on one JS agent. The checked
// wrapper satisfies the cross-target store trait without unsafe JS Send impls.
pub(crate) struct BrowserStore(SendWrapper<Function>);
impl BrowserStore {
    pub fn new(callback: StoreCallback) -> Result<Self, JsValue> {
        Ok(Self(SendWrapper::new(
            callback
                .dyn_into::<Function>()
                .map_err(|_| auth_error(AuthFailureKind::Configuration))?,
        )))
    }
}

#[async_trait::async_trait(?Send)]
impl ZitadelSessionStore for BrowserStore {
    async fn save(&self, credentials: &ZitadelSessionCredentials) -> Result<(), AuthError> {
        let snapshot: JsValue = ZitadelCredentialsSnapshot::copy_from(credentials).into();
        let result = self
            .0
            .call1(&JsValue::UNDEFINED, &snapshot)
            .map_err(|_| AuthError::Persistence)?;
        let promise = result
            .dyn_into::<Promise>()
            .map_err(|_| AuthError::Persistence)?;
        JsFuture::from(promise)
            .await
            .map_err(|_| AuthError::Persistence)?;
        Ok(())
    }
}

pub(crate) fn auth_error(kind: AuthFailureKind) -> JsValue {
    let (code, message) = match kind {
        AuthFailureKind::AuthenticationRequired => ("authentication_required", "Sign in again"),
        AuthFailureKind::Configuration => ("configuration", "Invalid authentication configuration"),
        AuthFailureKind::AuthorizationDenied => {
            ("authorization_denied", "Domain read access is required")
        }
        AuthFailureKind::Persistence => (
            "persistence",
            "Could not persist the replacement session; retry on this handle",
        ),
        AuthFailureKind::Transient => (
            "transient",
            "Authentication is temporarily unavailable; retry on this handle",
        ),
        AuthFailureKind::Cancelled => ("cancelled", "Authentication operation was cancelled"),
        AuthFailureKind::Closed => ("closed", "The session is closed"),
    };
    let error = js_sys::Error::new(message);
    error.set_name("AukiAuthError");
    // Error is newly allocated, extensible, and never exposed before this set.
    Reflect::set(&error, &"code".into(), &code.into()).expect("fresh Error code");
    error.into()
}

pub(crate) fn bootstrap_error(context: &'static str, error: AukiPeerBootstrapError) -> JsValue {
    match error {
        AukiPeerBootstrapError::ConfigureAuthentication(e)
        | AukiPeerBootstrapError::Authenticate(e)
        | AukiPeerBootstrapError::ListDomains(e)
        | AukiPeerBootstrapError::AuthorizePeer(e) => auth_error(e.kind()),
        other => crate::protocol_support::js_context(context, other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};
    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn timestamp_and_secret_snapshot_roundtrip() {
        let value = js_sys::JSON::parse(r#"{"accessToken":"ACCESS_SECRET","refreshToken":"REFRESH_SECRET","clientId":"client","issuer":"https://issuer.example","accessTokenExpiresAt":"2026-09-07T12:00:00.123456789+07:00"}"#).unwrap();
        let credentials = parse_credentials(&value).unwrap();
        let snapshot = ZitadelCredentialsSnapshot::copy_from(&credentials);
        assert_eq!(
            snapshot.access_token_expires_at().as_deref(),
            Some("2026-09-07T05:00:00.123456789Z")
        );
        assert_eq!(snapshot.expose_refresh_token(), "REFRESH_SECRET");
        assert!(!snapshot.to_json().contains("SECRET"));
        Reflect::set(&value, &"accessTokenExpiresAt".into(), &JsValue::NULL).unwrap();
        assert_eq!(
            parse_credentials(&value).unwrap().access_token_expires_at(),
            None
        );
    }

    #[wasm_bindgen_test]
    fn all_actionable_error_codes_are_redacted() {
        for (kind, code) in [
            (
                AuthFailureKind::AuthenticationRequired,
                "authentication_required",
            ),
            (AuthFailureKind::Configuration, "configuration"),
            (AuthFailureKind::AuthorizationDenied, "authorization_denied"),
            (AuthFailureKind::Persistence, "persistence"),
            (AuthFailureKind::Transient, "transient"),
            (AuthFailureKind::Cancelled, "cancelled"),
            (AuthFailureKind::Closed, "closed"),
        ] {
            let error = auth_error(kind);
            assert_eq!(
                Reflect::get(&error, &"code".into())
                    .unwrap()
                    .as_string()
                    .as_deref(),
                Some(code)
            );
        }
        let invalid = JsValue::from_str("DO_NOT_LEAK_SECRET");
        let error = js_sys::Error::from(parse_credentials(&invalid).unwrap_err());
        assert!(!error.message().as_string().unwrap().contains("DO_NOT_LEAK"));
    }
}
