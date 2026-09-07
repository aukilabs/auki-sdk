//! Real-browser supervisor tests. The suspension case is driven by the CLI
//! harness documented in docs/zitadel-browser-tests.md, not a Node compile check.
use super::*;
use auki_auth::{AuthorityRenewalProvider, DomainDescriptor, RenewedAuthority};
use auki_p2p::{DdsVerificationKeys, P2P_TOKEN_TTL, SignedP2pCredential};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::DecodePrivateKey,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use wasm_bindgen::{JsValue, prelude::wasm_bindgen};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

const PRIVATE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k\nfqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw\nUphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo\n-----END PRIVATE KEY-----";
const PUBLIC_KEY: &str = "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD\n8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==\n-----END PUBLIC KEY-----";

fn material(peer: PeerId, domain: Uuid, issued_at: i64) -> RenewedAuthority {
    let exp = issued_at + P2P_TOKEN_TTL.as_secs() as i64;
    let claims = json!({"type":"p2p-access", "iss":"dds", "aud":["auki-p2p"],
        "sub":"203845773915743233", "peer_type":"user", "peer_id":peer.to_string(),
        "domain_ids":[domain], "scopes":["domain-data:r"], "iat":issued_at, "exp":exp});
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(br#"{"alg":"ES256","typ":"JWT"}"#),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap())
    );
    let key = SigningKey::from_pkcs8_pem(PRIVATE_KEY).unwrap();
    let signature: Signature = key.sign(signing_input.as_bytes());
    let credential = SignedP2pCredential::new(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    ))
    .unwrap();
    let expiry = DateTime::from_timestamp(exp, 0).unwrap();
    RenewedAuthority {
        domain: DomainDescriptor::assigned(domain),
        peer_id: peer,
        credential,
        verification_keys: DdsVerificationKeys::new(0, PUBLIC_KEY.as_bytes().to_vec(), None),
        credential_expires_at: expiry,
        renew_at: expiry - chrono::Duration::minutes(1),
    }
}

enum Step {
    Fail(auki_auth::Error),
    Return(RenewedAuthority),
    Pending,
}
#[derive(Clone)]
struct Renewal {
    steps: Arc<StdMutex<VecDeque<Step>>>,
    calls: Arc<AtomicUsize>,
}
impl Renewal {
    fn new(steps: impl IntoIterator<Item = Step>) -> Self {
        Self {
            steps: Arc::new(StdMutex::new(steps.into_iter().collect())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}
#[async_trait(?Send)]
impl AuthorityRenewalProvider for Renewal {
    async fn renew_authority(
        &self,
        cancellation: &CancellationToken,
    ) -> auki_auth::Result<RenewedAuthority> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra renewal");
        match step {
            Step::Fail(error) => Err(error),
            Step::Return(update) => Ok(update),
            Step::Pending => {
                cancellation.cancelled().await;
                Err(auki_auth::Error::Cancelled {
                    endpoint: "browser-test",
                })
            }
        }
    }
}

async fn supervisor(
    initial: RenewedAuthority,
    identity: Identity,
    renewal: Renewal,
) -> (Rc<BrowserNode>, Arc<AuthoritySupervisor>) {
    let header = initial.credential.to_sensitive_bearer_header().unwrap();
    let node = Rc::new(
        BrowserNode::start(
            identity,
            PeerAuthorityUpdate::new(
                initial.domain.id,
                initial.peer_id,
                initial.verification_keys,
                initial.credential,
                initial.credential_expires_at,
            ),
        )
        .await
        .unwrap(),
    );
    let authority = Arc::new(AuthoritySupervisor::new(
        node.authority(),
        AuthorityRenewal::new(renewal),
        CurrentAuthority {
            header,
            revision: 1,
            renew_at: Utc::now(),
            expires_at: initial.credential_expires_at,
        },
    ));
    (node, authority)
}

#[wasm_bindgen_test(async)]
async fn browser_terminal_failures_stop_retry_and_retain_actionable_lifecycle_reason() {
    for error in [
        auki_auth::Error::AuthenticationRequired,
        auki_auth::Error::DomainNotAccessible,
        auki_auth::Error::InvalidConfiguration("client"),
        auki_auth::Error::SessionClosed,
    ] {
        let kind = error.kind();
        let identity = Identity::generate();
        let peer_id = identity.peer_id();
        let initial = material(peer_id, Uuid::new_v4(), Utc::now().timestamp() - 1);
        let renewal = Renewal::new([Step::Fail(error)]);
        let (node, authority) = supervisor(initial, identity, renewal.clone()).await;
        let protocols = AukiPeerProtocols::new(node.clone(), node.domain_id());
        let runtime = OutboundSupervisor::start(authority, node, protocols);
        let exit = runtime.lifecycle().wait_stopped().await;
        assert_eq!(
            exit,
            AukiPeerExit::Failed(AukiPeerFailure::Authentication(kind))
        );
        assert_eq!(renewal.calls.load(Ordering::SeqCst), 1);
        assert_eq!(peer_exit(runtime.stop().await), exit);
    }
}

#[wasm_bindgen_test(async)]
async fn browser_transient_and_persistence_failures_back_off_and_preserve_peer_id() {
    for error in [
        auki_auth::Error::Persistence,
        auki_auth::Error::Transport {
            endpoint: "fixture",
        },
    ] {
        let identity = Identity::generate();
        let peer = identity.peer_id();
        let domain = Uuid::new_v4();
        let now = Utc::now().timestamp();
        let renewal = Renewal::new([Step::Fail(error), Step::Return(material(peer, domain, now))]);
        let (node, authority) =
            supervisor(material(peer, domain, now - 1), identity, renewal.clone()).await;
        assert_eq!(
            authority.maintain().await.unwrap(),
            AuthorityMaintenance::Retry
        );
        assert_eq!(
            authority.maintain().await.unwrap(),
            AuthorityMaintenance::Retry
        );
        assert!(matches!(
            authority.renew_after_unauthorized(1).await,
            Err(AuthoritySupervisorError::RetryPending)
        ));
        assert_eq!(renewal.calls.load(Ordering::SeqCst), 1);
        // Controlled backoff clock only; token validation continues to use UTC.
        authority.state.lock().await.retry_at = Some(Utc::now());
        assert_eq!(
            authority.maintain().await.unwrap(),
            AuthorityMaintenance::Renewed
        );
        assert_eq!(node.peer_id(), peer);
        assert_eq!(authority.state.lock().await.current.revision, 2);
        assert_eq!(renewal.calls.load(Ordering::SeqCst), 2);
        node.shutdown().await.unwrap();
        authority.stop().await;
    }
}

#[wasm_bindgen_test(async)]
async fn browser_renewal_timeout_is_bounded_and_can_retry_same_peer() {
    let identity = Identity::generate();
    let peer = identity.peer_id();
    let domain = Uuid::new_v4();
    let now = Utc::now().timestamp();
    // Preserve the normal 30-minute lifetime and leave enough headroom for cold
    // Wasm crypto initialization. The production ten-second attempt bound wins.
    let initial = material(peer, domain, now - P2P_TOKEN_TTL.as_secs() as i64 + 15);
    let renewal = Renewal::new([Step::Pending, Step::Return(material(peer, domain, now))]);
    let (node, authority) = supervisor(initial, identity, renewal.clone()).await;
    assert_eq!(AUTHORITY_ATTEMPT_TIMEOUT, Duration::from_secs(10));
    assert_eq!(
        authority.maintain().await.unwrap(),
        AuthorityMaintenance::Retry
    );
    assert_eq!(renewal.calls.load(Ordering::SeqCst), 1);
    authority.state.lock().await.retry_at = Some(Utc::now());
    assert_eq!(
        authority.maintain().await.unwrap(),
        AuthorityMaintenance::Renewed
    );
    assert_eq!(node.peer_id(), peer);
    node.shutdown().await.unwrap();
    authority.stop().await;
}

#[wasm_bindgen(
    inline_js = "export function suspensionMarker() { globalThis.__z07Suspend = 'ready'; } export function suspensionResumed() { return globalThis.__z07Suspend === 'resumed'; }"
)]
extern "C" {
    fn suspensionMarker();
    fn suspensionResumed() -> bool;
}

#[wasm_bindgen_test(async)]
async fn browser_suspension_resume_fences_expired_authority_and_recovers_same_peer() {
    let identity = Identity::generate();
    let peer = identity.peer_id();
    let domain = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let initial = material(peer, domain, now - P2P_TOKEN_TTL.as_secs() as i64 + 15);
    let renewal = Renewal::new([
        Step::Fail(auki_auth::Error::Persistence),
        Step::Return(material(peer, domain, now)),
    ]);
    let (node, authority) = supervisor(initial, identity, renewal.clone()).await;
    suspensionMarker();
    let deadline = Utc::now() + chrono::Duration::seconds(45);
    while !suspensionResumed() {
        assert!(
            Utc::now() < deadline,
            "run the documented Playwright freeze/resume harness"
        );
        Delay::new(Duration::from_millis(20)).await;
    }
    assert!(authority.state.lock().await.current.expires_at <= Utc::now());
    assert_eq!(
        authority.maintain().await.unwrap(),
        AuthorityMaintenance::Retry
    );
    assert!(
        authority.authorization().await.is_err(),
        "expired DMS authority must remain fenced"
    );
    authority.state.lock().await.retry_at = Some(Utc::now());
    assert_eq!(
        authority.maintain().await.unwrap(),
        AuthorityMaintenance::Renewed
    );
    assert!(authority.authorization().await.is_ok());
    assert_eq!(node.peer_id(), peer);
    assert_eq!(renewal.calls.load(Ordering::SeqCst), 2);
    js_sys::Reflect::set(
        &js_sys::global(),
        &JsValue::from_str("__z07Suspend"),
        &JsValue::from_str("passed"),
    )
    .unwrap();
    node.shutdown().await.unwrap();
    authority.stop().await;
}

#[wasm_bindgen_test(async)]
async fn browser_owned_session_survives_cancelled_wait_and_rejected_persistence() {
    use auki_auth::{
        AuthClient, AuthEnvironment, AuthSession, ZitadelSessionCredentials, ZitadelSessionStore,
    };
    use futures::future::{AbortHandle, Abortable};
    use tokio::sync::Semaphore;

    const BASE: &str = "http://127.0.0.1:18109";
    struct Store {
        entered: Semaphore,
        release: Semaphore,
        calls: AtomicUsize,
        fail_first: bool,
    }
    #[async_trait(?Send)]
    impl ZitadelSessionStore for Store {
        async fn save(&self, credentials: &ZitadelSessionCredentials) -> auki_auth::Result<()> {
            assert_eq!(
                credentials.access_token().expose_secret(),
                "browser-new-access"
            );
            assert_eq!(
                credentials.refresh_token().expose_secret(),
                "browser-new-refresh"
            );
            assert!(credentials.access_token_expires_at().is_some());
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.add_permits(1);
            self.release.acquire().await.unwrap().forget();
            if self.fail_first && call == 0 {
                Err(auki_auth::Error::Persistence)
            } else {
                Ok(())
            }
        }
    }
    struct SessionRenewal {
        session: AuthSession,
        material: RenewedAuthority,
    }
    #[async_trait(?Send)]
    impl AuthorityRenewalProvider for SessionRenewal {
        async fn renew_authority(
            &self,
            cancellation: &CancellationToken,
        ) -> auki_auth::Result<RenewedAuthority> {
            self.session
                .accessible_domains_with_cancellation(cancellation)
                .await?;
            Ok(self.material.clone())
        }
    }
    let http = reqwest::Client::new();
    for fail_first in [false, true] {
        assert!(
            http.post(format!("{BASE}/__reset"))
                .send()
                .await
                .expect("start test-support/zitadel-browser-fixture.mjs")
                .status()
                .is_success()
        );
        let store = Arc::new(Store {
            entered: Semaphore::new(0),
            release: Semaphore::new(0),
            calls: AtomicUsize::new(0),
            fail_first,
        });
        let session = AuthClient::new(AuthEnvironment::new(BASE, BASE).unwrap())
            .unwrap()
            .import_zitadel_session(
                ZitadelSessionCredentials::new(
                    "browser-old-access",
                    "browser-old-refresh",
                    "browser-client",
                    BASE.parse().unwrap(),
                    Some(Utc::now()),
                )
                .unwrap(),
                store.clone(),
            )
            .unwrap();
        let identity = Identity::generate();
        let peer = identity.peer_id();
        let domain = Uuid::new_v4();
        let now = Utc::now().timestamp();
        let (node, authority) =
            supervisor(material(peer, domain, now - 1), identity, Renewal::new([])).await;
        authority.state.lock().await.renewal = AuthorityRenewal::new(SessionRenewal {
            session: session.clone(),
            material: material(peer, domain, now),
        });
        let (abort, registration) = AbortHandle::new_pair();
        let (tx, rx) = oneshot::channel();
        spawn_local({
            let authority = authority.clone();
            async move {
                let _ = tx.send(Abortable::new(authority.maintain(), registration).await);
            }
        });
        let mut first = Some(rx);
        if !fail_first {
            loop {
                let counts: serde_json::Value = http
                    .get(format!("{BASE}/__stats"))
                    .send()
                    .await
                    .unwrap()
                    .json()
                    .await
                    .unwrap();
                if counts["refresh"] == 1 {
                    break;
                }
                Delay::new(Duration::from_millis(5)).await;
            }
            abort.abort();
            assert!(first.take().unwrap().await.unwrap().is_err());
        }
        store.entered.acquire().await.unwrap().forget();
        let counts: serde_json::Value = http
            .get(format!("{BASE}/__stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(counts, json!({"refresh":1,"exchange":0,"domains":0}));
        if fail_first {
            store.release.add_permits(1);
            assert_eq!(
                first.take().unwrap().await.unwrap().unwrap().unwrap(),
                AuthorityMaintenance::Retry
            );
            authority.state.lock().await.retry_at = Some(Utc::now());
        }
        let (tx, rx) = oneshot::channel();
        spawn_local({
            let authority = authority.clone();
            async move {
                let _ = tx.send(authority.maintain().await);
            }
        });
        if fail_first {
            store.entered.acquire().await.unwrap().forget();
        }
        assert_eq!(
            store.calls.load(Ordering::SeqCst),
            if fail_first { 2 } else { 1 }
        );
        let counts: serde_json::Value = http
            .get(format!("{BASE}/__stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(counts, json!({"refresh":1,"exchange":0,"domains":0}));
        store.release.add_permits(1);
        assert_eq!(rx.await.unwrap().unwrap(), AuthorityMaintenance::Renewed);
        let counts: serde_json::Value = http
            .get(format!("{BASE}/__stats"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(counts, json!({"refresh":1,"exchange":1,"domains":1}));
        assert_eq!(node.peer_id(), peer);
        node.shutdown().await.unwrap();
        authority.stop().await;
        session.close().await;
    }
}

#[wasm_bindgen_test(async)]
async fn browser_expired_pending_install_is_replaced_and_domain_denial_is_isolated() {
    let a = Identity::generate();
    let b = Identity::generate();
    let domain_a = Uuid::new_v4();
    let domain_b = Uuid::new_v4();
    let now = Utc::now().timestamp();
    let renewal = Renewal::new([
        Step::Fail(auki_auth::Error::DomainNotAccessible),
        Step::Return(material(b.peer_id(), domain_b, now)),
    ]);
    let (node_a, authority_a) =
        supervisor(material(a.peer_id(), domain_a, now - 2), a, renewal.clone()).await;
    let (node_b, authority_b) =
        supervisor(material(b.peer_id(), domain_b, now - 2), b, renewal.clone()).await;
    assert!(authority_a.maintain().await.is_err());
    assert!(matches!(
        authority_a.maintain().await,
        Err(AuthoritySupervisorError::AuthenticationFailed(
            AuthFailureKind::AuthorizationDenied
        ))
    ));
    let expired = material(
        node_b.peer_id(),
        domain_b,
        now - P2P_TOKEN_TTL.as_secs() as i64 - 1,
    );
    authority_b.state.lock().await.pending = Some(PendingAuthority {
        header: expired.credential.to_sensitive_bearer_header().unwrap(),
        update: PeerAuthorityUpdate::new(
            domain_b,
            node_b.peer_id(),
            expired.verification_keys,
            expired.credential,
            expired.credential_expires_at,
        ),
        renew_at: expired.renew_at,
        expires_at: expired.credential_expires_at,
    });
    assert_eq!(
        authority_b.maintain().await.unwrap(),
        AuthorityMaintenance::Renewed
    );
    assert!(authority_b.authorization().await.is_ok());
    assert_eq!(renewal.calls.load(Ordering::SeqCst), 2);
    node_a.shutdown().await.unwrap();
    node_b.shutdown().await.unwrap();
    authority_a.stop().await;
    authority_b.stop().await;
}
