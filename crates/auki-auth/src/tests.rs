use std::{collections::HashMap, time::Duration};

use auki_domain::{Domain, DomainConfig, DomainStatus, ServedProtocols};
use auki_p2p::{
    Identity, Multiaddr, P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL,
    P2P_TOKEN_TYPE, P2PAccessClaims, Protocol,
};
use auki_session::Peer;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{TimeZone, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    AppCredentials, AuthClient, AuthEnvironment, AuthLimits, Credentials, DomainSelection, Error,
    PreparedPeer, SecretString,
    client::{validate_challenge_at, verification_key_id},
    wire::PeerChallengeResponse,
};

const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

const ROTATED_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgwRbuxaM6rEI3vYEl
vRmIEsc1QtC3uPMWvXo1xXt+CcOhRANCAAQDFwBFAujMsiq78IWbq5vz0QSWEdc7
7h5NE8sDwgD6Js22t9Ztq84hhkS3Aad4m9FOi8evk5QYW7ef+Bc2oZsr
-----END PRIVATE KEY-----"#;

const ROTATED_DDS_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEAxcARQLozLIqu/CFm6ub89EElhHX
O+4eTRPLA8IA+ibNtrfWbavOIYZEtwGneJvRTovHr5OUGFu3n/gXNqGbKw==
-----END PUBLIC KEY-----"#;

#[derive(Clone, Debug)]
struct RecordedRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

struct MockResponse {
    status: u16,
    body: Vec<u8>,
    delay: Duration,
}

impl MockResponse {
    fn json(value: Value) -> Self {
        Self {
            status: 200,
            body: value.to_string().into_bytes(),
            delay: Duration::ZERO,
        }
    }

    fn status(status: u16) -> Self {
        Self {
            status,
            body: b"{}".to_vec(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct MockServer {
    base_url: String,
    requests: std::sync::Arc<Mutex<Vec<RecordedRequest>>>,
    task: JoinHandle<()>,
}

impl MockServer {
    async fn start(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = std::sync::Arc::new(Mutex::new(Vec::new()));
        let task_requests = requests.clone();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let request = read_request(&mut stream).await;
                task_requests.lock().await.push(request);
                if !response.delay.is_zero() {
                    tokio::time::sleep(response.delay).await;
                }
                let reason = match response.status {
                    200 => "OK",
                    401 => "Unauthorized",
                    404 => "Not Found",
                    500 => "Internal Server Error",
                    _ => "Test Status",
                };
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.status,
                    reason,
                    response.body.len()
                );
                let _ = stream.write_all(head.as_bytes()).await;
                let _ = stream.write_all(&response.body).await;
                let _ = stream.shutdown().await;
            }
        });
        Self {
            base_url: format!("http://{address}"),
            requests,
            task,
        }
    }

    async fn finish(self) -> Vec<RecordedRequest> {
        self.task.await.unwrap();
        self.requests.lock().await.clone()
    }
}

async fn read_request(stream: &mut TcpStream) -> RecordedRequest {
    let mut bytes = Vec::new();
    let header_end;
    loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "client closed before completing request headers");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(
            bytes.len() <= 128 * 1024,
            "mock request exceeded test bound"
        );
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
    }

    let header_text = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines.next().unwrap().split_whitespace();
    let method = request_line.next().unwrap().to_owned();
    let target = request_line.next().unwrap().to_owned();
    let mut headers = HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').unwrap();
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().unwrap())
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "client closed before completing request body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    RecordedRequest {
        method,
        target,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    }
}

fn client_for(server: &MockServer) -> AuthClient {
    let environment = AuthEnvironment::new(&server.base_url, &server.base_url).unwrap();
    AuthClient::new(environment).unwrap()
}

fn login_response(access: &str, refresh: &str) -> MockResponse {
    MockResponse::json(json!({"access_token": access, "refresh_token": refresh}))
}

fn service_response(access: &str) -> MockResponse {
    MockResponse::json(json!({"access_token": access}))
}

fn domains_response(domain_id: Uuid, organization_id: Uuid) -> MockResponse {
    domains_response_with_organization(domain_id, Some(organization_id))
}

fn domains_response_with_organization(
    domain_id: Uuid,
    organization_id: Option<Uuid>,
) -> MockResponse {
    MockResponse::json(json!({
        "domains": [{
            "id": domain_id,
            "name": "Robotics Lab",
            "description": "Robot experiments",
            "organization_id": organization_id
        }],
        "total": 1,
        "limit": 100,
        "offset": 0
    }))
}

fn domains_page_response(
    domain_ids: &[Uuid],
    organization_id: Uuid,
    total: u64,
    limit: u32,
    offset: u32,
) -> MockResponse {
    let domains: Vec<_> = domain_ids
        .iter()
        .map(|domain_id| {
            json!({
                "id": domain_id,
                "name": format!("Domain {domain_id}"),
                "description": "Paginated robot experiment",
                "organization_id": organization_id
            })
        })
        .collect();
    MockResponse::json(json!({
        "domains": domains,
        "total": total,
        "limit": limit,
        "offset": offset
    }))
}

fn challenge_response(id: &str, challenge: [u8; 32]) -> MockResponse {
    MockResponse::json(json!({
        "challenge_id": id,
        "challenge": URL_SAFE_NO_PAD.encode(challenge),
        "expires_at": Utc::now() + chrono::Duration::seconds(60)
    }))
}

fn keys_response() -> MockResponse {
    keys_response_with_generation(1)
}

fn keys_response_with_generation(generation: u64) -> MockResponse {
    verification_keys_response(generation, TEST_DDS_PUBLIC_KEY, None)
}

fn verification_keys_response(
    generation: u64,
    current: &str,
    previous: Option<&str>,
) -> MockResponse {
    let mut keys = vec![json!({
        "id": verification_key_id(current).unwrap(),
        "status": "current",
        "signing_method": "ES256",
        "public_key": current
    })];
    if let Some(previous) = previous {
        keys.push(json!({
            "id": verification_key_id(previous).unwrap(),
            "status": "previous",
            "signing_method": "ES256",
            "public_key": previous
        }));
    }
    MockResponse::json(json!({
        "version": 1,
        "generation": generation,
        "previous_key_overlap_seconds": 1860,
        "keys": keys
    }))
}

async fn start_domain(identity: Identity, prepared: &PreparedPeer) -> (Domain, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let peer = Peer::new(prepared.peer_id.to_string(), "auki-auth-domain-proof")
        .with_storage_root(root.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let domain = Domain::builder(
        &peer,
        &session,
        DomainConfig::new(prepared.domain.id, identity),
    )
    .authority(
        prepared.verification_keys.clone(),
        prepared.initial_credential.clone(),
    )
    .join()
    .await
    .unwrap();
    (domain, root)
}

async fn start_listening_domain(
    identity: Identity,
    prepared: &PreparedPeer,
) -> (Domain, tempfile::TempDir) {
    start_listening_protocol_domain(identity, prepared, None, ServedProtocols::none()).await
}

async fn start_listening_protocol_domain(
    identity: Identity,
    prepared: &PreparedPeer,
    route: Option<(auki_p2p::PeerId, Multiaddr)>,
    served_protocols: ServedProtocols,
) -> (Domain, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let peer = Peer::new(prepared.peer_id.to_string(), "auki-auth-listener-proof")
        .with_storage_root(root.path().to_path_buf());
    let session = peer.start_session().unwrap();
    let listener: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    let mut config = DomainConfig::new(prepared.domain.id, identity)
        .with_listen_addresses([listener])
        .unwrap();
    if let Some((peer_id, address)) = route {
        config = config.with_peer_routes(peer_id, [address]).unwrap();
    }
    let domain = Domain::builder(&peer, &session, config)
        .authority(
            prepared.verification_keys.clone(),
            prepared.initial_credential.clone(),
        )
        .served_protocols(served_protocols)
        .join()
        .await
        .unwrap();
    (domain, root)
}

fn tcp_port(address: &Multiaddr) -> u16 {
    address
        .iter()
        .find_map(|protocol| match protocol {
            Protocol::Tcp(port) => Some(port),
            _ => None,
        })
        .expect("test listener must contain a TCP port")
}

fn signed_peer_response(
    identity: &Identity,
    domain_id: Uuid,
    principal_kind: &str,
    issued_at: u64,
) -> MockResponse {
    signed_peer_response_with_key(
        identity,
        domain_id,
        principal_kind,
        issued_at,
        TEST_DDS_PRIVATE_KEY,
    )
}

fn signed_peer_response_with_key(
    identity: &Identity,
    domain_id: Uuid,
    principal_kind: &str,
    issued_at: u64,
    signing_key: &[u8],
) -> MockResponse {
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.to_owned(),
        iss: P2P_TOKEN_ISSUER.to_owned(),
        aud: vec![P2P_TOKEN_AUDIENCE.to_owned()],
        sub: Uuid::from_u128(0xfeed).to_string(),
        organization_id: None,
        peer_type: Some(principal_kind.to_owned()),
        peer_id: identity.peer_id().to_string(),
        domain_ids: vec![domain_id.to_string()],
        scopes: vec![P2P_TOKEN_SCOPE.to_owned()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: issued_at + P2P_TOKEN_TTL.as_secs(),
    };
    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(signing_key).unwrap(),
    )
    .unwrap();
    MockResponse::json(json!({
        "peer_id": identity.peer_id().to_string(),
        "domain_id": domain_id,
        "peer_type": principal_kind,
        "p2p_access_token": token,
        "p2p_access_expires_at": Utc.timestamp_opt(claims.exp as i64, 0).single().unwrap()
    }))
}

#[tokio::test(flavor = "multi_thread")]
async fn user_flow_starts_domain_and_explicitly_renews_authority_in_place() {
    let identity = Identity::from_ed25519_seed(&[0x31; 32]);
    let domain_id = Uuid::from_u128(0xd0);
    let organization_id = Uuid::from_u128(0xa0);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let first_challenge = [0x41; 32];
    let second_challenge = [0x42; 32];
    let server = MockServer::start(vec![
        login_response("api-access", "api-refresh"),
        service_response("dds-service"),
        domains_response(domain_id, organization_id),
        domains_response(domain_id, organization_id),
        challenge_response("challenge-1", first_challenge),
        signed_peer_response(&identity, domain_id, "user", now),
        keys_response(),
        domains_response(domain_id, organization_id),
        challenge_response("challenge-2", second_challenge),
        signed_peer_response_with_key(
            &identity,
            domain_id,
            "user",
            now + 1,
            ROTATED_DDS_PRIVATE_KEY,
        ),
        verification_keys_response(2, ROTATED_DDS_PUBLIC_KEY, Some(TEST_DDS_PUBLIC_KEY)),
        domains_page_response(&[], organization_id, 0, 100, 0),
    ])
    .await;
    let client = client_for(&server);

    let session = client
        .authenticate(Credentials::user_password(
            "roboticist@example.com",
            "correct horse battery staple",
        ))
        .await
        .unwrap();
    let domains = session.accessible_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].domain.id, domain_id);

    let prepared = session
        .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
        .await
        .unwrap();
    assert_eq!(prepared.domain.id, domain_id);
    assert_eq!(prepared.peer_id, identity.peer_id());
    assert_eq!(prepared.verification_keys.generation(), 1);
    assert!(prepared.renew_at < prepared.credential_expires_at);
    assert!(!format!("{prepared:?}").contains("dds-service"));

    let (domain, _storage) = start_listening_domain(identity.clone(), &prepared).await;
    assert_eq!(domain.status(), DomainStatus::Ready);
    assert_eq!(domain.domain_id(), domain_id);
    assert_eq!(domain.peer_id(), prepared.peer_id);
    assert_eq!(domain.listen_addresses().len(), 1);
    let listener_port = tcp_port(&domain.listen_addresses()[0]);
    let authority = domain.authority();
    assert_eq!(authority.domain_id(), domain_id);
    assert_eq!(authority.peer_id(), prepared.peer_id);

    let renewed = prepared.renewal.renew().await.unwrap();
    assert!(renewed.credential_expires_at > prepared.credential_expires_at);
    assert_eq!(renewed.peer_id, prepared.peer_id);
    assert_eq!(renewed.verification_keys.generation(), 2);

    // Update authority on the same live Domain. Keys are intentionally
    // installed first so a rotated-key credential can never race ahead of
    // the verifier that admits it.
    authority
        .install_verification_keys(renewed.verification_keys)
        .await
        .unwrap();
    authority
        .install_credential(renewed.credential)
        .await
        .unwrap();
    assert_eq!(domain.status(), DomainStatus::Ready);
    assert_eq!(authority.domain_id(), domain_id);
    assert_eq!(authority.peer_id(), prepared.peer_id);

    let status = domain.subscribe_status();
    assert!(matches!(
        prepared.renewal.renew().await.unwrap_err(),
        Error::DomainNotAccessible
    ));
    assert_eq!(domain.status(), DomainStatus::Ready);
    assert_eq!(*status.borrow(), DomainStatus::Ready);
    assert!(!status.has_changed().unwrap());

    tokio::time::timeout(Duration::from_secs(5), domain.leave())
        .await
        .expect("explicit Domain fence must release listener and tasks")
        .unwrap();
    assert_eq!(*status.borrow(), DomainStatus::Stopped);
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, listener_port)).unwrap();

    let requests = server.finish().await;
    assert_eq!(requests.len(), 12);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].target, "/user/login");
    assert_eq!(
        serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
        json!({
            "email": "roboticist@example.com",
            "password": "correct horse battery staple"
        })
    );
    assert_eq!(
        requests[1].headers.get("authorization").unwrap(),
        "Bearer api-access"
    );
    assert_eq!(
        requests[2].target,
        "/api/v1/accessible-domains?limit=100&offset=0"
    );
    assert_eq!(
        requests[4].target,
        format!("/api/v1/domains/{domain_id}/p2p/challenge")
    );
    assert_eq!(
        requests[5].target,
        format!("/api/v1/domains/{domain_id}/p2p/verify")
    );
    assert_peer_signature(&identity, first_challenge, &requests[5]);
    assert_peer_signature(&identity, second_challenge, &requests[9]);
    assert_eq!(
        requests[11].target,
        "/api/v1/accessible-domains?limit=100&offset=0"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_user_peers_fetch_resources_over_direct_tcp_across_live_renewal() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let a_identity = Identity::from_ed25519_seed(&[0x71; 32]);
        let b_identity = Identity::from_ed25519_seed(&[0x72; 32]);
        let a_peer = a_identity.peer_id();
        let b_peer = b_identity.peer_id();
        assert_ne!(a_peer, b_peer);
        let domain_id = Uuid::from_u128(0xd1ec7);
        let organization_id = Uuid::from_u128(0xa11ce);
        let now = u64::try_from(Utc::now().timestamp()).unwrap();
        let a_first_challenge = [0x73; 32];
        let b_challenge = [0x74; 32];
        let a_renewal_challenge = [0x75; 32];
        let server = MockServer::start(vec![
            login_response("api-a", "refresh-a"),
            service_response("dds-a"),
            login_response("api-b", "refresh-b"),
            service_response("dds-b"),
            domains_response(domain_id, organization_id),
            challenge_response("challenge-a-1", a_first_challenge),
            signed_peer_response(&a_identity, domain_id, "user", now),
            keys_response(),
            domains_response(domain_id, organization_id),
            challenge_response("challenge-b", b_challenge),
            signed_peer_response(&b_identity, domain_id, "user", now),
            keys_response(),
            domains_response(domain_id, organization_id),
            challenge_response("challenge-a-2", a_renewal_challenge),
            signed_peer_response(&a_identity, domain_id, "user", now + 1),
            keys_response(),
        ])
        .await;
        let client = client_for(&server);

        let a_auth = client
            .authenticate(Credentials::user_password(
                "peer-a@example.com",
                "peer-a-password",
            ))
            .await
            .unwrap();
        let b_auth = client
            .authenticate(Credentials::user_password(
                "peer-b@example.com",
                "peer-b-password",
            ))
            .await
            .unwrap();
        let a_prepared = a_auth
            .authorize_peer(DomainSelection::new(domain_id), &a_identity.proof())
            .await
            .unwrap();
        let b_prepared = b_auth
            .authorize_peer(DomainSelection::new(domain_id), &b_identity.proof())
            .await
            .unwrap();
        assert_eq!(a_prepared.peer_id, a_peer);
        assert_eq!(b_prepared.peer_id, b_peer);
        assert_eq!(a_prepared.domain.id, domain_id);
        assert_eq!(b_prepared.domain.id, domain_id);

        let (a, _a_storage) = start_listening_protocol_domain(
            a_identity.clone(),
            &a_prepared,
            None,
            ServedProtocols::none().with_resources_v2(),
        )
        .await;
        assert_eq!(a.status(), DomainStatus::Ready);
        assert_eq!(a.served_protocol_ids().len(), 1);
        let a_address = a.listen_addresses()[0].clone();
        let a_port = tcp_port(&a_address);

        let (b, _b_storage) = start_listening_protocol_domain(
            b_identity.clone(),
            &b_prepared,
            Some((a_peer, a_address.clone())),
            ServedProtocols::none(),
        )
        .await;
        assert_eq!(b.status(), DomainStatus::Ready);
        assert!(b.served_protocol_ids().is_empty());
        let b_port = tcp_port(&b.listen_addresses()[0]);

        let first = b.fetch_resources_catalog(a_peer).await.unwrap();
        assert!(first.resources.is_empty());
        assert_eq!(a.known_peers().peer_count(), 1);
        assert_eq!(b.known_peers().peer_count(), 1);

        let renewed = a_prepared.renewal.renew().await.unwrap();
        assert_eq!(renewed.verification_keys.generation(), 1);
        let authority = a.authority();
        authority
            .install_verification_keys(renewed.verification_keys)
            .await
            .unwrap();
        authority
            .install_credential(renewed.credential)
            .await
            .unwrap();
        assert_eq!(a.status(), DomainStatus::Ready);
        assert_eq!(a.listen_addresses(), [a_address]);

        // The second stream uses the same live Domains and direct route;
        // neither runtime is rebuilt or explicitly reconnected after renewal.
        let second = b.fetch_resources_catalog(a_peer).await.unwrap();
        assert!(second.resources.is_empty());
        assert_eq!(a.known_peers().peer_count(), 1);
        assert_eq!(b.known_peers().peer_count(), 1);

        let a_status = a.subscribe_status();
        let b_status = b.subscribe_status();
        b.leave().await.unwrap();
        a.leave().await.unwrap();
        assert_eq!(*b_status.borrow(), DomainStatus::Stopped);
        assert_eq!(*a_status.borrow(), DomainStatus::Stopped);
        let _b_rebound = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, b_port))
            .expect("B listener must be reusable immediately after ordered leave");
        let _a_rebound = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, a_port))
            .expect("A listener must be reusable immediately after ordered leave");

        let requests = server.finish().await;
        assert_eq!(requests.len(), 16);
        assert_eq!(
            requests
                .iter()
                .map(|request| request.target.as_str())
                .collect::<Vec<_>>(),
            [
                "/user/login",
                "/service/domains-access-token",
                "/user/login",
                "/service/domains-access-token",
                "/api/v1/accessible-domains?limit=100&offset=0",
                &format!("/api/v1/domains/{domain_id}/p2p/challenge"),
                &format!("/api/v1/domains/{domain_id}/p2p/verify"),
                "/service/p2p-verification-keys",
                "/api/v1/accessible-domains?limit=100&offset=0",
                &format!("/api/v1/domains/{domain_id}/p2p/challenge"),
                &format!("/api/v1/domains/{domain_id}/p2p/verify"),
                "/service/p2p-verification-keys",
                "/api/v1/accessible-domains?limit=100&offset=0",
                &format!("/api/v1/domains/{domain_id}/p2p/challenge"),
                &format!("/api/v1/domains/{domain_id}/p2p/verify"),
                "/service/p2p-verification-keys",
            ]
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
            json!({"email": "peer-a@example.com", "password": "peer-a-password"})
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[2].body).unwrap(),
            json!({"email": "peer-b@example.com", "password": "peer-b-password"})
        );
        assert_eq!(requests[1].headers["authorization"], "Bearer api-a");
        assert_eq!(requests[3].headers["authorization"], "Bearer api-b");
        for index in [4, 5, 6, 12, 13, 14] {
            assert_eq!(requests[index].headers["authorization"], "Bearer dds-a");
        }
        for index in [8, 9, 10] {
            assert_eq!(requests[index].headers["authorization"], "Bearer dds-b");
        }
        for index in [7, 11, 15] {
            assert!(!requests[index].headers.contains_key("authorization"));
        }
        assert_peer_challenge(&a_identity, &requests[5]);
        assert_peer_challenge(&b_identity, &requests[9]);
        assert_peer_challenge(&a_identity, &requests[13]);
        assert_peer_signature(&a_identity, a_first_challenge, &requests[6]);
        assert_peer_signature(&b_identity, b_challenge, &requests[10]);
        assert_peer_signature(&a_identity, a_renewal_challenge, &requests[14]);
    })
    .await
    .expect("local two-peer auth and direct-TCP proof must remain bounded");
}

#[tokio::test(flavor = "multi_thread")]
async fn malformed_rotation_is_rejected_without_advancing_renewal_version() {
    let identity = Identity::from_ed25519_seed(&[0x34; 32]);
    let domain_id = Uuid::from_u128(0xd6);
    let organization_id = Uuid::from_u128(0xa6);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        domains_response(domain_id, organization_id),
        challenge_response("initial-challenge", [0x53; 32]),
        signed_peer_response(&identity, domain_id, "app", now),
        keys_response(),
        domains_response(domain_id, organization_id),
        challenge_response("malformed-rotation", [0x54; 32]),
        signed_peer_response_with_key(
            &identity,
            domain_id,
            "app",
            now + 1,
            ROTATED_DDS_PRIVATE_KEY,
        ),
        verification_keys_response(2, ROTATED_DDS_PUBLIC_KEY, None),
        domains_response(domain_id, organization_id),
        challenge_response("valid-rotation", [0x55; 32]),
        signed_peer_response_with_key(
            &identity,
            domain_id,
            "app",
            now + 1,
            ROTATED_DDS_PRIVATE_KEY,
        ),
        verification_keys_response(2, ROTATED_DDS_PUBLIC_KEY, Some(TEST_DDS_PUBLIC_KEY)),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    let prepared = session
        .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
        .await
        .unwrap();

    assert!(matches!(
        prepared.renewal.renew().await.unwrap_err(),
        Error::InvalidP2pAuthority(auki_p2p::Error::VerificationKeyRotationMissingPrevious)
    ));

    // The valid retry intentionally uses the same issued-at. It can succeed
    // only if rejected material did not advance the renewal version.
    let renewed = prepared.renewal.renew().await.unwrap();
    assert_eq!(renewed.verification_keys.generation(), 2);
    assert_eq!(
        renewed.credential_expires_at.timestamp(),
        (now + 1 + P2P_TOKEN_TTL.as_secs()) as i64
    );
    server.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn app_flow_uses_basic_exchange_and_starts_the_same_domain_shape() {
    let identity = Identity::from_ed25519_seed(&[0x32; 32]);
    let domain_id = Uuid::from_u128(0xd1);
    let organization_id = Uuid::from_u128(0xa1);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        domains_response(domain_id, organization_id),
        challenge_response("app-challenge", [0x51; 32]),
        signed_peer_response(&identity, domain_id, "app", now),
        keys_response(),
        domains_response(domain_id, organization_id),
        challenge_response("app-renewal", [0x56; 32]),
        signed_peer_response(&identity, domain_id, "app", now + 1),
        keys_response_with_generation(2),
    ])
    .await;
    let client = client_for(&server);
    let credentials = AppCredentials::new("app-key", "app-secret")
        .with_gateway_mac("aa:bb:cc:dd:ee:ff")
        .unwrap();
    let session = client
        .authenticate(Credentials::AppCredentials(credentials))
        .await
        .unwrap();
    let prepared = session
        .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
        .await
        .unwrap();
    let (domain, _storage) = start_domain(identity.clone(), &prepared).await;
    assert_eq!(domain.status(), DomainStatus::Ready);
    assert_eq!(domain.domain_id(), domain_id);
    assert_eq!(domain.peer_id(), prepared.peer_id);
    let authority = domain.authority();
    let renewed = prepared.renewal.renew().await.unwrap();
    assert_eq!(renewed.peer_id, prepared.peer_id);
    assert_eq!(renewed.verification_keys.generation(), 2);
    authority
        .install_verification_keys(renewed.verification_keys)
        .await
        .unwrap();
    authority
        .install_credential(renewed.credential)
        .await
        .unwrap();
    assert_eq!(domain.status(), DomainStatus::Ready);
    assert_eq!(authority.domain_id(), domain_id);
    assert_eq!(authority.peer_id(), prepared.peer_id);
    tokio::time::timeout(Duration::from_secs(5), domain.leave())
        .await
        .expect("renewed App Domain must leave cleanly")
        .unwrap();

    let requests = server.finish().await;
    assert_eq!(requests.len(), 9);
    assert_eq!(
        requests[0].headers.get("authorization").unwrap(),
        &format!("Basic {}", STANDARD.encode("app-key:app-secret"))
    );
    for request_index in [1, 2, 3, 5, 6, 7] {
        let request = &requests[request_index];
        assert_eq!(
            request.headers.get("posemesh-gateway-mac").unwrap(),
            "AA:BB:CC:DD:EE:FF"
        );
        assert_eq!(
            request.headers.get("authorization").unwrap(),
            "Bearer app-dds-service"
        );
    }
    for request_index in [4, 8] {
        assert!(
            !requests[request_index]
                .headers
                .contains_key("authorization")
        );
        assert!(
            !requests[request_index]
                .headers
                .contains_key("posemesh-gateway-mac")
        );
    }
    assert_peer_signature(&identity, [0x51; 32], &requests[3]);
    assert_peer_signature(&identity, [0x56; 32], &requests[7]);
}

#[tokio::test]
async fn selected_domain_authorization_paginates_beyond_the_first_page() {
    let identity = Identity::from_ed25519_seed(&[0x33; 32]);
    let organization_id = Uuid::from_u128(0xa2);
    let first_page: Vec<_> = (1..=100).map(Uuid::from_u128).collect();
    let selected_domain = Uuid::from_u128(101);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        domains_page_response(&first_page, organization_id, 101, 100, 0),
        domains_page_response(&[selected_domain], organization_id, 101, 100, 100),
        challenge_response("page-two-challenge", [0x52; 32]),
        signed_peer_response(&identity, selected_domain, "app", now),
        keys_response(),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();

    let prepared = session
        .authorize_peer(DomainSelection::new(selected_domain), &identity.proof())
        .await
        .unwrap();
    assert_eq!(prepared.domain.id, selected_domain);

    let requests = server.finish().await;
    assert_eq!(
        requests[1].target,
        "/api/v1/accessible-domains?limit=100&offset=0"
    );
    assert_eq!(
        requests[2].target,
        "/api/v1/accessible-domains?limit=100&offset=100"
    );
}

#[tokio::test]
async fn accessible_domain_pagination_rejects_bad_echo_duplicates_and_unstable_totals() {
    let organization_id = Uuid::from_u128(0xa3);
    let first_page: Vec<_> = (1..=100).map(Uuid::from_u128).collect();

    let bad_echo = MockServer::start(vec![
        service_response("dds"),
        domains_page_response(&[], organization_id, 0, 99, 0),
    ])
    .await;
    let session = client_for(&bad_echo)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    bad_echo.finish().await;

    let duplicate = MockServer::start(vec![
        service_response("dds"),
        domains_page_response(&first_page, organization_id, 101, 100, 0),
        domains_page_response(&[first_page[0]], organization_id, 101, 100, 100),
    ])
    .await;
    let session = client_for(&duplicate)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    duplicate.finish().await;

    let unstable_total = MockServer::start(vec![
        service_response("dds"),
        domains_page_response(&first_page, organization_id, 101, 100, 0),
        domains_page_response(&[], organization_id, 100, 100, 100),
    ])
    .await;
    let session = client_for(&unstable_total)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    unstable_total.finish().await;
}

#[tokio::test]
async fn accessible_domain_pagination_enforces_page_and_total_bounds() {
    let organization_id = Uuid::from_u128(0xa4);
    let one_domain = [Uuid::from_u128(1)];
    let inconsistent_page = MockServer::start(vec![
        service_response("dds"),
        domains_page_response(&one_domain, organization_id, 2, 100, 0),
    ])
    .await;
    let session = client_for(&inconsistent_page)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    inconsistent_page.finish().await;

    let first_page: Vec<_> = (1..=100).map(Uuid::from_u128).collect();
    let excessive_total = MockServer::start(vec![
        service_response("dds"),
        domains_page_response(&first_page, organization_id, 1_025, 100, 0),
    ])
    .await;
    let session = client_for(&excessive_total)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::AccessibleDomainsTruncated {
            total: 1_025,
            returned: 100
        }
    ));
    excessive_total.finish().await;
}

#[tokio::test]
async fn accessible_domain_preserves_null_organization() {
    let domain_id = Uuid::from_u128(0xd15);
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        domains_response_with_organization(domain_id, None),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();

    let domains = session.accessible_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].domain.id, domain_id);
    assert_eq!(domains[0].domain.organization_id, None);
    server.finish().await;
}

#[tokio::test]
async fn accessible_domain_accepts_absent_legacy_display_metadata() {
    let domain_id = Uuid::from_u128(0xd16);
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        MockResponse::json(json!({
            "domains": [{
                "id": domain_id,
                "name": "   ",
                "description": " \t ",
                "organization_id": null
            }],
            "total": 1,
            "limit": 100,
            "offset": 0
        })),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();

    let domains = session.accessible_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0].domain.id, domain_id);
    assert_eq!(domains[0].domain.name, None);
    assert_eq!(domains[0].domain.description, None);
    assert_eq!(domains[0].domain.organization_id, None);
    server.finish().await;
}

#[tokio::test]
async fn accessible_domain_normalizes_or_omits_legacy_display_metadata() {
    let padded_domain_id = Uuid::from_u128(0xd17);
    let oversized_domain_id = Uuid::from_u128(0xd18);
    let oversized_name = "n".repeat(257);
    let oversized_description = "d".repeat(4 * 1024 + 1);
    let server = MockServer::start(vec![
        service_response("app-dds-service"),
        MockResponse::json(json!({
            "domains": [
                {
                    "id": padded_domain_id,
                    "name": "  Legacy Robotics Lab  ",
                    "description": "\t Robot experiments \n",
                    "organization_id": null
                },
                {
                    "id": oversized_domain_id,
                    "name": oversized_name,
                    "description": oversized_description,
                    "organization_id": null
                }
            ],
            "total": 2,
            "limit": 100,
            "offset": 0
        })),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();

    let domains = session.accessible_domains().await.unwrap();
    assert_eq!(domains.len(), 2);
    assert_eq!(domains[0].domain.id, padded_domain_id);
    assert_eq!(
        domains[0].domain.name.as_deref(),
        Some("Legacy Robotics Lab")
    );
    assert_eq!(
        domains[0].domain.description.as_deref(),
        Some("Robot experiments")
    );
    assert_eq!(domains[1].domain.id, oversized_domain_id);
    assert_eq!(domains[1].domain.name, None);
    assert_eq!(domains[1].domain.description, None);
    server.finish().await;
}

#[tokio::test]
async fn unauthorized_dds_call_refreshes_user_and_retries_once() {
    let domain_id = Uuid::from_u128(0xd2);
    let organization_id = Uuid::from_u128(0xa2);
    let server = MockServer::start(vec![
        login_response("access-1", "refresh-1"),
        service_response("dds-1"),
        MockResponse::status(401),
        login_response("access-2", "refresh-2"),
        service_response("dds-2"),
        domains_response(domain_id, organization_id),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap();
    assert_eq!(session.accessible_domains().await.unwrap().len(), 1);

    let requests = server.finish().await;
    assert_eq!(requests[3].target, "/user/refresh");
    assert_eq!(
        requests[3].headers.get("authorization").unwrap(),
        "Bearer refresh-1"
    );
    assert_eq!(
        requests[4].headers.get("authorization").unwrap(),
        "Bearer access-2"
    );
    assert_eq!(
        requests[5].headers.get("authorization").unwrap(),
        "Bearer dds-2"
    );
}

#[tokio::test]
async fn rotated_refresh_token_survives_failed_service_exchange() {
    let domain_id = Uuid::from_u128(0xd3);
    let organization_id = Uuid::from_u128(0xa3);
    let server = MockServer::start(vec![
        login_response("access-1", "refresh-1"),
        service_response("dds-1"),
        MockResponse::status(401),
        login_response("access-2", "refresh-2"),
        MockResponse::status(500),
        MockResponse::status(401),
        login_response("access-3", "refresh-3"),
        service_response("dds-3"),
        domains_response(domain_id, organization_id),
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap();
    assert!(matches!(
        session.accessible_domains().await.unwrap_err(),
        Error::HttpStatus { status: 500, .. }
    ));
    assert_eq!(session.accessible_domains().await.unwrap().len(), 1);

    let requests = server.finish().await;
    assert_eq!(
        requests[6].headers.get("authorization").unwrap(),
        "Bearer refresh-2"
    );
}

#[tokio::test]
async fn domain_race_and_signed_peer_mismatch_fail_closed() {
    let identity = Identity::from_ed25519_seed(&[0x61; 32]);
    let domain_id = Uuid::from_u128(0xd4);
    let organization_id = Uuid::from_u128(0xa4);
    let race = MockServer::start(vec![
        login_response("access", "refresh"),
        service_response("dds"),
        domains_response(domain_id, organization_id),
        MockResponse::status(404),
    ])
    .await;
    let session = client_for(&race)
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap();
    assert!(matches!(
        session
            .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
            .await
            .unwrap_err(),
        Error::DomainNotAccessible
    ));
    race.finish().await;

    let other_identity = Identity::from_ed25519_seed(&[0x62; 32]);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let mut mismatched = signed_peer_response(&other_identity, domain_id, "user", now);
    let mut envelope: Value = serde_json::from_slice(&mismatched.body).unwrap();
    envelope["peer_id"] = json!(identity.peer_id().to_string());
    mismatched.body = envelope.to_string().into_bytes();
    let mismatch = MockServer::start(vec![
        login_response("access", "refresh"),
        service_response("dds"),
        domains_response(domain_id, organization_id),
        challenge_response("mismatch", [0x63; 32]),
        mismatched,
        keys_response(),
    ])
    .await;
    let session = client_for(&mismatch)
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap();
    assert!(matches!(
        session
            .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
            .await
            .unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    mismatch.finish().await;
}

#[tokio::test]
async fn verification_key_id_must_match_the_canonical_pkix_fingerprint() {
    let identity = Identity::from_ed25519_seed(&[0x64; 32]);
    let domain_id = Uuid::from_u128(0xd5);
    let organization_id = Uuid::from_u128(0xa5);
    let now = u64::try_from(Utc::now().timestamp()).unwrap();
    let mut mismatched_keys = keys_response();
    let mut key_set: Value = serde_json::from_slice(&mismatched_keys.body).unwrap();
    key_set["keys"][0]["id"] =
        json!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    mismatched_keys.body = key_set.to_string().into_bytes();
    let server = MockServer::start(vec![
        service_response("dds"),
        domains_response(domain_id, organization_id),
        challenge_response("fingerprint-challenge", [0x65; 32]),
        signed_peer_response(&identity, domain_id, "app", now),
        mismatched_keys,
    ])
    .await;
    let session = client_for(&server)
        .authenticate(Credentials::app("app-key", "app-secret"))
        .await
        .unwrap();

    assert!(matches!(
        session
            .authorize_peer(DomainSelection::new(domain_id), &identity.proof())
            .await
            .unwrap_err(),
        Error::InvalidResponse { .. }
    ));
    server.finish().await;
}

#[test]
fn challenge_expiration_accepts_exact_clock_skew_edges_and_rejects_outside_them() {
    let now = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
    let response_at = |expires_at| PeerChallengeResponse {
        challenge_id: "bounded-challenge".to_owned(),
        challenge: URL_SAFE_NO_PAD.encode([0x66; 32]),
        expires_at,
    };

    // Bound both a slow response from a server clock behind by 60 seconds and
    // a server clock ahead by 60 seconds. DDS still consumes/verifies the
    // one-time challenge authoritatively.
    assert!(validate_challenge_at(&response_at(now - chrono::Duration::seconds(60)), now).is_ok());
    assert!(validate_challenge_at(&response_at(now + chrono::Duration::seconds(120)), now).is_ok());
    assert!(
        validate_challenge_at(
            &response_at(now - chrono::Duration::seconds(60) - chrono::Duration::nanoseconds(1)),
            now
        )
        .is_err()
    );
    assert!(
        validate_challenge_at(
            &response_at(now + chrono::Duration::seconds(120) + chrono::Duration::nanoseconds(1)),
            now
        )
        .is_err()
    );
}

#[tokio::test]
async fn strict_json_size_and_cancellation_bounds_fail_closed() {
    let unknown = MockServer::start(vec![MockResponse::json(json!({
        "access_token": "access",
        "refresh_token": "refresh",
        "unexpected": true
    }))])
    .await;
    let error = client_for(&unknown)
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::InvalidResponse { .. }));
    unknown.finish().await;

    let oversized = MockServer::start(vec![MockResponse {
        status: 200,
        body: vec![b'x'; 1024],
        delay: Duration::ZERO,
    }])
    .await;
    let environment = AuthEnvironment::new(&oversized.base_url, &oversized.base_url).unwrap();
    let client = AuthClient::with_limits(
        environment,
        AuthLimits {
            max_response_bytes: 128,
            ..AuthLimits::default()
        },
    )
    .unwrap();
    let error = client
        .authenticate(Credentials::user_password("person@example.com", "password"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::ResponseTooLarge { maximum: 128, .. }
    ));
    oversized.finish().await;

    let delayed = MockServer::start(vec![
        login_response("access", "refresh").delayed(Duration::from_millis(150)),
    ])
    .await;
    let client = client_for(&delayed);
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
    });
    let error = client
        .authenticate_with_cancellation(
            Credentials::user_password("person@example.com", "password"),
            &cancellation,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Cancelled { .. }));
    delayed.finish().await;
}

#[test]
fn environment_and_secret_surfaces_are_safe_by_default() {
    let dev = AuthEnvironment::dev();
    assert_eq!(dev.api_base_url(), "https://api.dev.aukiverse.com/");
    assert_eq!(dev.dds_base_url(), "https://dds.dev.aukiverse.com/");
    assert!(AuthEnvironment::new("http://127.0.0.2:8000", "http://localhost:9000").is_ok());
    assert!(AuthEnvironment::new("http://192.168.1.2", "https://dds.example.com").is_err());
    assert!(
        AuthEnvironment::new("https://api.example.com/prefix", "https://dds.example.com").is_err()
    );

    let secret = SecretString::new("do-not-print-me");
    assert!(!format!("{secret:?}").contains("do-not-print-me"));
    assert!(!format!("{secret}").contains("do-not-print-me"));
    let credentials = Credentials::user_password("secret@example.com", "do-not-print-me");
    let debug = format!("{credentials:?}");
    assert!(!debug.contains("secret@example.com"));
    assert!(!debug.contains("do-not-print-me"));
}

fn assert_peer_signature(identity: &Identity, challenge: [u8; 32], request: &RecordedRequest) {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let signature = URL_SAFE_NO_PAD
        .decode(body["signature"].as_str().unwrap())
        .unwrap();
    assert!(identity.public_key().verify(&challenge, &signature));
}

fn assert_peer_challenge(identity: &Identity, request: &RecordedRequest) {
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({
            "peer_id": identity.peer_id().to_string(),
            "public_key": URL_SAFE_NO_PAD.encode(identity.public_key_protobuf())
        })
    );
}
