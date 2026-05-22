//! Native `/auki/message/0.0.1` node facade for binding hosts.
//!
//! This module owns the small synchronous surface that UniFFI can expose
//! to Swift. Browser peers still use the generated JavaScript package and
//! jslibp2p; this node is the native/iOS side of that interop path.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use futures::StreamExt as _;
use libp2p::{
    Multiaddr, PeerId, StreamProtocol, Swarm, SwarmBuilder, identify, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
};
use prost::Message as _;
use thiserror::Error;
use tokio::{
    runtime::{Builder, Runtime},
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

use crate::{
    PeerIdentity,
    message_protocol::{
        MESSAGE_PROTOCOL, MessageAck, MessageEnvelope, MessageProtocolError, read_message_ack,
        read_message_envelope, write_message_ack, write_message_envelope,
    },
    swarm::webrtc_direct_transport,
};

const MESSAGE_NODE_COMMAND_BUFFER: usize = 16;
const MESSAGE_NODE_EVENT_BUFFER: usize = 64;
const MESSAGE_NODE_TIMEOUT: Duration = Duration::from_secs(10);
const MESSAGE_NODE_IDENTIFY_PROTOCOL: &str = "/auki/identify/0.0.1";

/// Configuration for the native message node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageNodeConfig {
    /// WebRTC Direct multiaddrs to listen on.
    pub listen_addresses: Vec<Multiaddr>,
    /// Reported in identify responses.
    pub agent_version: String,
}

impl Default for MessageNodeConfig {
    fn default() -> Self {
        Self {
            listen_addresses: vec![],
            agent_version: format!("auki-sdk/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// One inbound message delivered to the binding host.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageNodeEvent {
    /// The libp2p peer that opened the message substream.
    pub peer_id: PeerId,
    /// The decoded protobuf envelope.
    pub envelope: MessageEnvelope,
}

/// Failure modes for the native message node facade.
#[derive(Debug, Error)]
pub enum MessageNodeError {
    /// Tokio runtime construction failed.
    #[error("runtime setup failed: {0}")]
    Runtime(String),
    /// Swarm construction or listen setup failed.
    #[error("swarm setup failed: {0}")]
    Swarm(String),
    /// The node driver task is no longer accepting commands.
    #[error("node is stopped")]
    Stopped,
    /// Driver command failed.
    #[error("command failed: {0}")]
    Command(String),
    /// Message protocol framing failed.
    #[error("message protocol: {0}")]
    Protocol(#[from] MessageProtocolError),
    /// Raw envelope bytes were not a protobuf `MessageEnvelope`.
    #[error("protobuf decode: {0}")]
    Decode(#[source] prost::DecodeError),
    /// A blocking host call waited too long for completion.
    #[error("operation timed out after {0:?}")]
    Timeout(Duration),
}

/// Synchronous handle to a native message node.
pub struct MessageNode {
    runtime: Runtime,
    command_tx: mpsc::Sender<MessageNodeCommand>,
    event_rx: Mutex<mpsc::Receiver<MessageNodeEvent>>,
    listen_addrs: Arc<Mutex<Vec<Multiaddr>>>,
    local_peer_id: PeerId,
    task: Mutex<Option<JoinHandle<()>>>,
}

#[derive(NetworkBehaviour)]
struct MessageNodeBehaviour {
    identify: identify::Behaviour,
    ping: ping::Behaviour,
    stream: libp2p_stream::Behaviour,
}

enum MessageNodeCommand {
    Dial {
        peer_id: PeerId,
        addrs: Vec<Multiaddr>,
        ack: oneshot::Sender<Result<(), MessageNodeError>>,
    },
    SendEnvelope {
        peer_id: PeerId,
        envelope: MessageEnvelope,
        ack: oneshot::Sender<Result<MessageAck, MessageNodeError>>,
    },
    Shutdown,
}

impl MessageNode {
    /// Spawn a native message node and start its background libp2p task.
    pub fn spawn(
        identity: PeerIdentity,
        config: MessageNodeConfig,
    ) -> Result<Self, MessageNodeError> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|err| MessageNodeError::Runtime(err.to_string()))?;

        let local_peer_id = identity.peer_id();
        let listen_addrs = Arc::new(Mutex::new(Vec::new()));
        let (command_tx, command_rx) = mpsc::channel(MESSAGE_NODE_COMMAND_BUFFER);
        let (event_tx, event_rx) = mpsc::channel(MESSAGE_NODE_EVENT_BUFFER);

        let swarm = {
            let _guard = runtime.enter();
            build_message_node_swarm(&identity, config)?
        };
        let listen_addrs_for_task = listen_addrs.clone();
        let task = runtime.spawn(run_message_node(
            swarm,
            command_rx,
            event_tx,
            listen_addrs_for_task,
        ));

        Ok(Self {
            runtime,
            command_tx,
            event_rx: Mutex::new(event_rx),
            listen_addrs,
            local_peer_id,
            task: Mutex::new(Some(task)),
        })
    }

    /// Local libp2p peer id.
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// Listen addresses emitted by libp2p so far.
    pub fn listen_addrs(&self) -> Vec<Multiaddr> {
        self.listen_addrs
            .lock()
            .expect("message node listen addrs mutex poisoned")
            .clone()
    }

    /// Request a dial to `peer_id` through the supplied multiaddrs.
    pub fn dial(&self, peer_id: PeerId, addrs: Vec<Multiaddr>) -> Result<(), MessageNodeError> {
        let (ack, rx) = oneshot::channel();
        self.runtime.block_on(async {
            self.command_tx
                .send(MessageNodeCommand::Dial {
                    peer_id,
                    addrs,
                    ack,
                })
                .await
                .map_err(|_| MessageNodeError::Stopped)?;
            tokio::time::timeout(MESSAGE_NODE_TIMEOUT, rx)
                .await
                .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
                .map_err(|_| MessageNodeError::Stopped)?
        })
    }

    /// Decode `envelope_bytes`, send the envelope, and return the peer's ack.
    pub fn send_envelope_bytes(
        &self,
        peer_id: PeerId,
        envelope_bytes: Vec<u8>,
    ) -> Result<MessageAck, MessageNodeError> {
        let envelope =
            MessageEnvelope::decode(&*envelope_bytes).map_err(MessageNodeError::Decode)?;
        let (ack, rx) = oneshot::channel();
        self.runtime.block_on(async {
            self.command_tx
                .send(MessageNodeCommand::SendEnvelope {
                    peer_id,
                    envelope,
                    ack,
                })
                .await
                .map_err(|_| MessageNodeError::Stopped)?;
            tokio::time::timeout(MESSAGE_NODE_TIMEOUT, rx)
                .await
                .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
                .map_err(|_| MessageNodeError::Stopped)?
        })
    }

    /// Poll one inbound message event without blocking the host thread.
    pub fn next_event(&self) -> Result<Option<MessageNodeEvent>, MessageNodeError> {
        let mut rx = self
            .event_rx
            .lock()
            .expect("message node event receiver mutex poisoned");
        match rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MessageNodeError::Stopped),
        }
    }

    /// Stop the background task.
    pub fn shutdown(&self) {
        let _ = self.command_tx.try_send(MessageNodeCommand::Shutdown);
        if let Some(task) = self
            .task
            .lock()
            .expect("message node task mutex poisoned")
            .take()
        {
            task.abort();
        }
    }
}

impl Drop for MessageNode {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn build_message_node_swarm(
    identity: &PeerIdentity,
    config: MessageNodeConfig,
) -> Result<Swarm<MessageNodeBehaviour>, MessageNodeError> {
    let agent_version = config.agent_version;
    let listen_addresses = config.listen_addresses;
    let mut swarm = SwarmBuilder::with_existing_identity(identity.keypair().clone())
        .with_tokio()
        .with_other_transport(webrtc_direct_transport)
        .map_err(|err| MessageNodeError::Swarm(err.to_string()))?
        .with_behaviour(|key| MessageNodeBehaviour {
            identify: identify::Behaviour::new(
                identify::Config::new(MESSAGE_NODE_IDENTIFY_PROTOCOL.into(), key.public())
                    .with_agent_version(agent_version),
            ),
            ping: ping::Behaviour::default(),
            stream: libp2p_stream::Behaviour::new(),
        })
        .expect("message node behaviour construction is infallible")
        .build();

    for addr in listen_addresses {
        swarm
            .listen_on(addr.clone())
            .map_err(|err| MessageNodeError::Swarm(format!("listen {addr}: {err}")))?;
    }

    Ok(swarm)
}

async fn run_message_node(
    mut swarm: Swarm<MessageNodeBehaviour>,
    mut command_rx: mpsc::Receiver<MessageNodeCommand>,
    event_tx: mpsc::Sender<MessageNodeEvent>,
    listen_addrs: Arc<Mutex<Vec<Multiaddr>>>,
) {
    let mut control = swarm.behaviour().stream.new_control();
    let proto = StreamProtocol::try_from_owned(MESSAGE_PROTOCOL.to_string())
        .expect("MESSAGE_PROTOCOL is a valid libp2p protocol id");
    let mut incoming: std::pin::Pin<
        Box<dyn futures::Stream<Item = (PeerId, libp2p::Stream)> + Send>,
    > = match control.accept(proto.clone()) {
        Ok(stream) => stream.boxed(),
        Err(_) => futures::stream::pending().boxed(),
    };

    loop {
        tokio::select! {
            event = swarm.next() => {
                match event {
                    Some(SwarmEvent::NewListenAddr { address, .. }) => {
                        listen_addrs
                            .lock()
                            .expect("message node listen addrs mutex poisoned")
                            .push(address);
                    }
                    Some(_) => {}
                    None => return,
                }
            }
            inbound = incoming.next() => {
                let Some((peer_id, substream)) = inbound else { return; };
                let tx = event_tx.clone();
                tokio::spawn(handle_inbound_message(peer_id, substream, tx));
            }
            command = command_rx.recv() => {
                let Some(command) = command else { return; };
                match command {
                    MessageNodeCommand::Dial { peer_id, addrs, ack } => {
                        let result = dial_message_peer(&mut swarm, peer_id, addrs);
                        let _ = ack.send(result);
                    }
                    MessageNodeCommand::SendEnvelope { peer_id, envelope, ack } => {
                        let mut outbound_control = control.clone();
                        let proto = proto.clone();
                        tokio::spawn(async move {
                            let result = send_outbound_message(
                                peer_id,
                                &mut outbound_control,
                                proto,
                                envelope,
                            )
                            .await;
                            let _ = ack.send(result);
                        });
                    }
                    MessageNodeCommand::Shutdown => return,
                }
            }
        }
    }
}

fn dial_message_peer(
    swarm: &mut Swarm<MessageNodeBehaviour>,
    peer_id: PeerId,
    addrs: Vec<Multiaddr>,
) -> Result<(), MessageNodeError> {
    if addrs.is_empty() {
        return Err(MessageNodeError::Command(
            "dial requires at least one multiaddr".into(),
        ));
    }

    for addr in addrs {
        let dial_addr = if addr
            .iter()
            .any(|proto| matches!(proto, libp2p::multiaddr::Protocol::P2p(_)))
        {
            addr
        } else {
            addr.with(libp2p::multiaddr::Protocol::P2p(peer_id))
        };
        swarm
            .dial(dial_addr.clone())
            .map_err(|err| MessageNodeError::Command(format!("dial {dial_addr}: {err}")))?;
    }

    Ok(())
}

async fn handle_inbound_message(
    peer_id: PeerId,
    mut substream: libp2p::Stream,
    event_tx: mpsc::Sender<MessageNodeEvent>,
) {
    let envelope = match read_message_envelope(&mut substream).await {
        Ok(envelope) => envelope,
        Err(err) => {
            eprintln!("auki-network: message from {peer_id}: read failed: {err}");
            return;
        }
    };
    let ack = MessageAck {
        request_id: envelope.request_id.clone(),
        accepted: true,
        detail: "accepted".to_string(),
    };
    let _ = write_message_ack(&mut substream, &ack).await;
    let _ = event_tx.send(MessageNodeEvent { peer_id, envelope }).await;
}

async fn send_outbound_message(
    peer_id: PeerId,
    control: &mut libp2p_stream::Control,
    proto: StreamProtocol,
    envelope: MessageEnvelope,
) -> Result<MessageAck, MessageNodeError> {
    let open = control.open_stream(peer_id, proto);
    let mut substream = tokio::time::timeout(MESSAGE_NODE_TIMEOUT, open)
        .await
        .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
        .map_err(|err| MessageNodeError::Command(err.to_string()))?;
    write_message_envelope(&mut substream, &envelope).await?;
    tokio::time::timeout(MESSAGE_NODE_TIMEOUT, read_message_ack(&mut substream))
        .await
        .map_err(|_| MessageNodeError::Timeout(MESSAGE_NODE_TIMEOUT))?
        .map_err(MessageNodeError::Protocol)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope_bytes(request_id: &str) -> Vec<u8> {
        let envelope = MessageEnvelope {
            type_url: "auki.test/ping".to_string(),
            body: b"hello".to_vec(),
            request_id: request_id.to_string(),
        };
        envelope.encode_to_vec()
    }

    fn wait_for_first_listen_addr(node: &MessageNode) -> Multiaddr {
        for _ in 0..100 {
            if let Some(addr) = node.listen_addrs().into_iter().next() {
                return addr;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("message node did not produce a listen addr");
    }

    fn wait_for_first_event(node: &MessageNode) -> MessageNodeEvent {
        for _ in 0..100 {
            if let Some(event) = node.next_event().expect("event poll") {
                return event;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("message node did not produce an event");
    }

    #[test]
    fn message_node_config_defaults_to_no_listeners() {
        let config = MessageNodeConfig::default();
        assert!(config.listen_addresses.is_empty());
        assert!(config.agent_version.starts_with("auki-sdk/"));
    }

    #[test]
    fn message_node_spawn_uses_identity_peer_id() {
        let identity = PeerIdentity::from_seed(&[71u8; 32]);
        let node =
            MessageNode::spawn(identity.clone(), MessageNodeConfig::default()).expect("spawn node");
        assert_eq!(node.local_peer_id(), identity.peer_id());
        node.shutdown();
    }

    #[test]
    fn send_envelope_bytes_rejects_bad_protobuf() {
        let identity = PeerIdentity::from_seed(&[72u8; 32]);
        let node = MessageNode::spawn(identity, MessageNodeConfig::default()).expect("spawn node");
        let err = node
            .send_envelope_bytes(PeerIdentity::from_seed(&[73u8; 32]).peer_id(), vec![0xff])
            .expect_err("bad protobuf should fail");
        assert!(matches!(err, MessageNodeError::Decode(_)));
        node.shutdown();
    }

    #[test]
    fn next_event_is_non_blocking_when_empty() {
        let identity = PeerIdentity::from_seed(&[74u8; 32]);
        let node = MessageNode::spawn(identity, MessageNodeConfig::default()).expect("spawn node");
        assert_eq!(node.next_event().expect("event poll"), None);
        node.shutdown();
    }

    #[test]
    fn send_envelope_bytes_accepts_protobuf_before_transport_attempt() {
        let identity = PeerIdentity::from_seed(&[75u8; 32]);
        let node = MessageNode::spawn(identity, MessageNodeConfig::default()).expect("spawn node");
        let err = node
            .send_envelope_bytes(
                PeerIdentity::from_seed(&[76u8; 32]).peer_id(),
                envelope_bytes("req-1"),
            )
            .expect_err("unconnected peer should fail after decode");
        assert!(!matches!(err, MessageNodeError::Decode(_)));
        node.shutdown();
    }

    #[test]
    fn two_message_nodes_exchange_envelope_over_webrtc_direct() {
        let id_a = PeerIdentity::from_seed(&[77u8; 32]);
        let id_b = PeerIdentity::from_seed(&[78u8; 32]);
        let node_a = MessageNode::spawn(
            id_a.clone(),
            MessageNodeConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/udp/0/webrtc-direct".parse().unwrap()],
                ..MessageNodeConfig::default()
            },
        )
        .expect("spawn node a");
        let node_b = MessageNode::spawn(
            id_b.clone(),
            MessageNodeConfig {
                listen_addresses: vec!["/ip4/127.0.0.1/udp/0/webrtc-direct".parse().unwrap()],
                ..MessageNodeConfig::default()
            },
        )
        .expect("spawn node b");
        let addr_a = wait_for_first_listen_addr(&node_a);
        let _addr_b = wait_for_first_listen_addr(&node_b);

        node_b
            .dial(id_a.peer_id(), vec![addr_a])
            .expect("dial node a");
        let ack = node_b
            .send_envelope_bytes(id_a.peer_id(), envelope_bytes("req-webrtc-direct"))
            .expect("send envelope");
        assert_eq!(ack.request_id, "req-webrtc-direct");
        assert!(ack.accepted);

        let event = wait_for_first_event(&node_a);
        assert_eq!(event.peer_id, id_b.peer_id());
        assert_eq!(event.envelope.request_id, "req-webrtc-direct");

        node_a.shutdown();
        node_b.shutdown();
    }
}
