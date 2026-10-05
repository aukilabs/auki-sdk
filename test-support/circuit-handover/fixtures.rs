//! Shared loopback-only DNS, credential, relay and tunnel fault fixtures.
use auki_p2p::{
    DdsTokenVerifier, ExpectedRelayLimits, Identity, Node, P2PAccessClaims, PeerId, PeerRole,
    RelayProvider, SignedP2pCredential,
};
use auki_p2p::{
    P2P_TOKEN_AUDIENCE, P2P_TOKEN_ISSUER, P2P_TOKEN_SCOPE, P2P_TOKEN_TTL, P2P_TOKEN_TYPE,
};
use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig, ResolverOpts},
    proto::xfer::Protocol as DnsProtocol,
};
use jsonwebtoken::encode;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
// Shared by edition 2021 and 2024 crates, which sort mixed-case imports differently.
#[rustfmt::skip]
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    process::{Child, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;
async fn timeout<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(15), future)
        .await
        .expect("local fixture deadline")
}
const TEST_DDS_PRIVATE_KEY: &[u8] = br#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggm4twpf4y/yNNw/k
fqecEEl4zBTwZdRDFUFp/fSxV8qhRANCAARUxrDWJ0AtEGTAYZ4412VPHqMCKoPw
UphDkcOIk7SODsKwUvTIiUr11NbXBJmbBRfhERczsuK4PVha5eg0fVqo
-----END PRIVATE KEY-----"#;

const TEST_DDS_PUBLIC_KEY: &[u8] = br#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEVMaw1idALRBkwGGeONdlTx6jAiqD
8FKYQ5HDiJO0jg7CsFL0yIlK9dTW1wSZmwUX4REXM7LiuD1YWuXoNH1aqA==
-----END PUBLIC KEY-----"#;

pub(super) struct TestDns {
    address: SocketAddr,
    resolver_timeout: Duration,
    shutdown: Arc<AtomicBool>,
    hold_queries: Arc<AtomicBool>,
    held_query_seen: Arc<AtomicBool>,
    task: Option<thread::JoinHandle<()>>,
}

impl TestDns {
    pub(super) fn start() -> Self {
        Self::start_with_timeout(Duration::from_secs(1))
    }

    pub(super) fn start_with_timeout(resolver_timeout: Duration) -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let address = socket.local_addr().unwrap();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let hold_queries = Arc::new(AtomicBool::new(false));
        let thread_hold_queries = hold_queries.clone();
        let held_query_seen = Arc::new(AtomicBool::new(false));
        let thread_held_query_seen = held_query_seen.clone();
        let task = thread::spawn(move || {
            let mut query = [0; 512];
            while !thread_shutdown.load(Ordering::Acquire) {
                match socket.recv_from(&mut query) {
                    Ok((length, remote)) => {
                        if thread_hold_queries.load(Ordering::Acquire) {
                            thread_held_query_seen.store(true, Ordering::Release);
                            while thread_hold_queries.load(Ordering::Acquire)
                                && !thread_shutdown.load(Ordering::Acquire)
                            {
                                thread::sleep(Duration::from_millis(5));
                            }
                        }
                        if let Some(response) = dns_a_response(&query[..length]) {
                            socket.send_to(&response, remote).unwrap();
                        }
                    }
                    Err(error)
                        if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                    Err(error) => panic!("test DNS receive failed: {error}"),
                }
            }
        });
        Self {
            address,
            resolver_timeout,
            shutdown,
            hold_queries,
            held_query_seen,
            task: Some(task),
        }
    }

    pub(super) fn resolver(&self) -> (ResolverConfig, ResolverOpts) {
        let name_server = NameServerConfig::new(self.address, DnsProtocol::Udp);
        let config = ResolverConfig::from_parts(None, Vec::new(), vec![name_server]);
        let mut options = ResolverOpts::default();
        options.timeout = self.resolver_timeout;
        options.attempts = 1;
        (config, options)
    }

    pub(super) fn hold_queries(&self) {
        self.held_query_seen.store(false, Ordering::Release);
        self.hold_queries.store(true, Ordering::Release);
    }

    pub(super) async fn wait_for_held_query(&self) {
        timeout(async {
            while !self.held_query_seen.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    pub(super) fn release_queries(&self) {
        self.hold_queries.store(false, Ordering::Release);
    }
}

impl Drop for TestDns {
    fn drop(&mut self) {
        self.hold_queries.store(false, Ordering::Release);
        self.shutdown.store(true, Ordering::Release);
        if let Some(task) = self.task.take() {
            task.join().unwrap();
        }
    }
}

fn dns_a_response(query: &[u8]) -> Option<Vec<u8>> {
    if query.len() < 17 || u16::from_be_bytes([query[4], query[5]]) != 1 {
        return None;
    }
    let mut cursor = 12;
    loop {
        let label_length = *query.get(cursor)? as usize;
        cursor += 1;
        if label_length == 0 {
            break;
        }
        cursor = cursor.checked_add(label_length)?;
        if cursor > query.len() {
            return None;
        }
    }
    let question_end = cursor.checked_add(4)?;
    let question = query.get(12..question_end)?;
    let query_type = u16::from_be_bytes([query[cursor], query[cursor + 1]]);
    let missing = query[12..cursor]
        .windows(b"missing".len())
        .any(|window| window == b"missing");

    let mut response = Vec::with_capacity(question_end + 16);
    response.extend_from_slice(&query[..2]);
    response.extend_from_slice(&(if missing { 0x8183u16 } else { 0x8180u16 }).to_be_bytes());
    response.extend_from_slice(&1u16.to_be_bytes());
    response.extend_from_slice(&u16::from(query_type == 1 && !missing).to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes());
    response.extend_from_slice(&0u16.to_be_bytes());
    response.extend_from_slice(question);
    if query_type == 1 && !missing {
        response.extend_from_slice(&0xc00cu16.to_be_bytes());
        response.extend_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&60u32.to_be_bytes());
        response.extend_from_slice(&4u16.to_be_bytes());
        response.extend_from_slice(&Ipv4Addr::LOCALHOST.octets());
    }
    Some(response)
}

pub(super) fn node(dns: &TestDns) -> Node {
    let (resolver_config, resolver_options) = dns.resolver();
    Node::start_with_dns_config(
        Identity::generate(),
        verifier(),
        ["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        resolver_config,
        resolver_options,
    )
    .unwrap()
}

pub(super) async fn install_current_token(
    node: &Node,
    role: PeerRole,
    domain_ids: Vec<String>,
) -> String {
    let credentials = node.authority();
    let issued_at = credentials
        .current_claims()
        .await
        .map(|claims| claims.iat + 1)
        .unwrap_or_else(unix_time)
        .max(unix_time());
    let claims = P2PAccessClaims {
        token_type: P2P_TOKEN_TYPE.into(),
        iss: P2P_TOKEN_ISSUER.into(),
        aud: vec![P2P_TOKEN_AUDIENCE.into()],
        sub: Uuid::new_v4().to_string(),
        organization_id: None,
        peer_type: Some(role.to_string()),
        peer_id: node.peer_id().to_string(),
        domain_ids,
        scopes: vec![P2P_TOKEN_SCOPE.into()],
        application: None,
        iat: issued_at,
        nbf: None,
        exp: issued_at + P2P_TOKEN_TTL.as_secs(),
    };
    let token = encode(
        &Header::new(Algorithm::ES256),
        &claims,
        &EncodingKey::from_ec_pem(TEST_DDS_PRIVATE_KEY).unwrap(),
    )
    .unwrap();
    credentials
        .install_credential(SignedP2pCredential::new(token.clone()).unwrap())
        .await
        .unwrap();
    token
}

pub(super) fn verifier() -> DdsTokenVerifier {
    DdsTokenVerifier::from_es256_pem(TEST_DDS_PUBLIC_KEY).unwrap()
}

pub(super) fn unix_time() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

pub(super) struct GoRelay {
    child: Child,
    input: ChildStdin,
    output: std::sync::mpsc::Receiver<serde_json::Value>,
    reader: Option<thread::JoinHandle<()>>,
    provider: RelayProvider,
}

impl GoRelay {
    pub(super) fn start(seconds: u64) -> Self {
        let binary = std::env::var("AUKI_HANDOVER_GO_RELAY")
            .expect("set AUKI_HANDOVER_GO_RELAY to the locally built Go fixture");
        let mut child = Command::new(binary)
            .args(["-duration", &format!("{seconds}s")])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<serde_json::Value>();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if tx.send(value).is_err() {
                    break;
                }
            }
        });
        let ready = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let peer = ready["peer_id"]
            .as_str()
            .unwrap()
            .parse::<PeerId>()
            .unwrap();
        let port = ready["port"].as_str().unwrap();
        let provider = RelayProvider::new(
            peer,
            [format!(
                "/dns4/handover.relay.auki-p2p.dev/tcp/{port}/p2p/{peer}"
            )],
            ExpectedRelayLimits::new(Duration::from_secs(seconds), 64 * 1024 * 1024).unwrap(),
        )
        .unwrap();
        Self {
            child,
            input,
            output: rx,
            reader: Some(reader),
            provider,
        }
    }

    pub(super) fn command(&mut self, command: &str) -> serde_json::Value {
        writeln!(self.input, "{command}").unwrap();
        self.input.flush().unwrap();
        self.output.recv_timeout(Duration::from_secs(3)).unwrap()
    }

    pub(super) fn provider(&self) -> RelayProvider {
        self.provider.clone()
    }

    pub(super) async fn wait_active(&mut self, expected: u64) -> serde_json::Value {
        timeout(async {
            loop {
                let stats = self.command("stats");
                if stats["active"].as_u64() == Some(expected) {
                    break stats;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
    }
}

impl Drop for GoRelay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TunnelState {
    Forward,
    Stalled,
}

pub(super) struct FaultProxy {
    pub(super) port: u16,
    pub(super) accepted: mpsc::Receiver<watch::Sender<TunnelState>>,
    task: JoinHandle<()>,
}

impl FaultProxy {
    pub(super) async fn start(relay_port: u16) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, accepted) = mpsc::channel(8);
        let task = tokio::spawn(async move {
            let mut tunnels = JoinSet::new();
            loop {
                tokio::select! {
                    result = listener.accept() => {
                        let (client, _) = result.unwrap();
                        let server = TcpStream::connect((Ipv4Addr::LOCALHOST, relay_port)).await.unwrap();
                        client.set_nodelay(true).unwrap();
                        server.set_nodelay(true).unwrap();
                        let (control, state) = watch::channel(TunnelState::Forward);
                        sender.send(control).await.unwrap();
                        tunnels.spawn(async move {
                            let (client_read, client_write) = client.into_split();
                            let (server_read, server_write) = server.into_split();
                            tokio::select! {
                                _ = copy_controlled(client_read, server_write, state.clone()) => {},
                                _ = copy_controlled(server_read, client_write, state) => {},
                            }
                        });
                    },
                    _ = tunnels.join_next(), if !tunnels.is_empty() => {},
                }
            }
        });
        Self {
            port,
            accepted,
            task,
        }
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        // Aborting drops the JoinSet too, so no tunnel tasks outlive the test.
        self.task.abort();
    }
}

async fn copy_controlled<R, W>(mut read: R, mut write: W, mut state: watch::Receiver<TunnelState>)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buffer = [0; 8192];
    loop {
        let mode = *state.borrow_and_update();
        match mode {
            TunnelState::Stalled => {
                if state.changed().await.is_err() {
                    return;
                }
                continue;
            }
            TunnelState::Forward => {}
        }
        let size = tokio::select! {
            changed = state.changed() => {
                if changed.is_err() { return; }
                continue;
            },
            result = tokio::io::AsyncReadExt::read(&mut read, &mut buffer) => {
                match result { Ok(0) | Err(_) => return, Ok(size) => size }
            },
        };
        // Preserve bytes already read if the fault arrives between read/write.
        loop {
            let mode = *state.borrow_and_update();
            match mode {
                TunnelState::Forward => break,
                TunnelState::Stalled => {
                    if state.changed().await.is_err() {
                        return;
                    }
                }
            }
        }
        if tokio::io::AsyncWriteExt::write_all(&mut write, &buffer[..size])
            .await
            .is_err()
        {
            return;
        }
    }
}
